//! Agents — JARVIS operates the portfolio.
//!
//! JARVIS dispatches the Claude Code CLI headlessly inside an ALLOWLISTED
//! project directory to make edits sir asks for, by text or by voice. Two
//! hard safety rules shape the whole design:
//!
//!   1. DEPLOY GATE — `git push` NEVER happens automatically. Every job ends
//!      in the Review queue; only sir's explicit Approve & Deploy pushes.
//!   2. SCOPED EDITS — the agent runs in `acceptEdits` permission mode (never
//!      `--dangerously-skip-permissions`), so it may edit files in its own
//!      directory and nothing else; non-edit actions (Bash, network) are not
//!      auto-granted and simply don't run in headless mode.
//!
//! Mechanism: one-shot `claude -p` with stream-json output. The instruction
//! is fed on stdin as plain text (never as a `-p "..."` argument) so no
//! fragile Windows command-line quoting is involved — the same trick the
//! brain's warm session uses. Crucially, the AGENT only edits files; JARVIS
//! (Rust, here) owns every git operation. That keeps the deploy gate airtight
//! (the agent has no path to `git push` at all) and lets preflight guarantee
//! the diff sir reviews is exactly the agent's work and nothing else.
//!
//! Job lifecycle: queued → running → review → deploying → done
//!                                         ↘ discarded / cancelled / failed
//! Concurrency is capped at 2 running jobs; the rest queue.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::db::{self, Db};

const MAX_RUNNING: usize = 2;
const DIFF_CAP: usize = 200_000; // per-file diff clamp for the UI

// ---------- allowlist ----------

/// One editable target. `cf_project` (optional) lets Approve & Deploy confirm
/// the Cloudflare Pages build after pushing.
fn default_allowlist() -> Vec<Value> {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    let projects = PathBuf::from(&home).join("Documents").join("Projects");
    let entry = |key: &str, folder: &str, cf: &str| {
        json!({
            "key": key,
            "path": projects.join(folder).to_string_lossy(),
            "cf_project": cf,
        })
    };
    vec![
        entry("myfreepdfedit", "myfreepdfedit", "myfreepdfedit"),
        entry("myfreeimagetool", "myfreeimagetool", "myfreeimagetool"),
        entry("myfreeaudiotool", "myfreeaudiotool", "myfreeaudiotool"),
        entry("myfreevideotool", "myfreevideotool", "myfreevideotool"),
        entry("myfreeinvoicetool", "myfreeinvoicetool", "myfreeinvoicetool"),
        entry("portfolio-analytics", "portfolio-analytics", ""),
        entry("JARVIS", "JARVIS", ""),
    ]
}

pub fn allowlist(app: &AppHandle) -> Vec<Value> {
    let db = app.state::<Db>();
    db::kv_get(&db, "agents_allowlist")
        .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(default_allowlist)
}

fn allow_entry(app: &AppHandle, key: &str) -> Option<Value> {
    allowlist(app).into_iter().find(|e| e["key"] == key)
}

fn agents_enabled(app: &AppHandle) -> bool {
    let db = app.state::<Db>();
    db::kv_get(&db, "agents_enabled").as_deref() == Some("1")
}

// ---------- git plumbing (JARVIS owns all git) ----------

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn is_git_repo(dir: &Path) -> bool {
    run_git(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

fn is_dirty(dir: &Path) -> bool {
    run_git(dir, &["status", "--porcelain"]).map(|s| !s.trim().is_empty()).unwrap_or(true)
}

fn head_sha(dir: &Path) -> String {
    run_git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
}

/// Stage the agent's changes and produce per-file unified diffs for review.
fn collect_changes(dir: &Path) -> (Vec<Value>, usize) {
    let _ = run_git(dir, &["add", "-A"]); // stage so new files appear in the diff
    let name_status = run_git(dir, &["diff", "--cached", "--name-status"]).unwrap_or_default();
    let mut files = Vec::new();
    let mut insertions = 0usize;
    for line in name_status.lines() {
        let mut parts = line.splitn(2, '\t');
        let status = parts.next().unwrap_or("").trim().to_string();
        let path = parts.next().unwrap_or("").trim().to_string();
        if path.is_empty() {
            continue;
        }
        let mut diff = run_git(dir, &["diff", "--cached", "--", &path]).unwrap_or_default();
        insertions += diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
        if diff.len() > DIFF_CAP {
            diff.truncate(DIFF_CAP);
            diff.push_str("\n… diff truncated (file too large to display in full) …");
        }
        files.push(json!({ "status": status, "path": path, "diff": diff }));
    }
    (files, insertions)
}

// ---------- job model ----------

#[derive(Clone)]
struct Job {
    id: String,
    site: String,
    dir: String,
    instruction: String,
    session_id: Option<String>,
    status: String, // queued running review deploying done failed cancelled discarded superseded
    base_sha: String,
    started_at: i64,
    ended_at: Option<i64>,
    log: Vec<String>,
    summary: String,
    files: Vec<Value>,
    commit_hash: Option<String>,
    error: Option<String>,
}

impl Job {
    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "site": self.site,
            "instruction": self.instruction,
            "status": self.status,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "log": self.log,
            "summary": self.summary,
            "files": self.files,
            "files_changed": self.files.len(),
            "commit_hash": self.commit_hash,
            "error": self.error,
        })
    }
}

#[derive(Default)]
pub struct Agents {
    jobs: Mutex<Vec<Job>>,
    procs: Mutex<HashMap<String, Child>>, // running children, for cancel
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn snapshot_job(app: &AppHandle, id: &str) -> Option<Job> {
    let agents = app.state::<Agents>();
    let jobs = agents.jobs.lock().unwrap();
    jobs.iter().find(|j| j.id == id).cloned()
}

/// Update a job in place by id, run a mutation, then emit it. Returns false if gone.
fn with_job<F: FnOnce(&mut Job)>(app: &AppHandle, id: &str, f: F) -> bool {
    let agents = app.state::<Agents>();
    let snapshot = {
        let mut jobs = agents.jobs.lock().unwrap();
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else { return false };
        f(job);
        job.clone()
    };
    let _ = app.emit("agent-update", snapshot.to_value());
    true
}

fn push_log(app: &AppHandle, id: &str, line: String) {
    {
        let agents = app.state::<Agents>();
        let mut jobs = agents.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.log.push(line.clone());
        }
    }
    let _ = app.emit("agent-log", json!({ "id": id, "line": line }));
}

/// Jobs actively occupying an agent slot.
fn running_count(app: &AppHandle) -> usize {
    let agents = app.state::<Agents>();
    let jobs = agents.jobs.lock().unwrap();
    jobs.iter().filter(|j| j.status == "running").count()
}

// ---------- dispatch ----------

/// Preflight + create a job. Starts it now if a slot is free, else queues it.
/// `resume` carries a prior agent session id for "request changes" follow-ups.
pub fn dispatch(app: &AppHandle, site: &str, instruction: &str, resume: Option<String>) -> Result<Value, String> {
    if !agents_enabled(app) {
        return Err("Agents are disabled. Enable them in Settings → Agents first, sir.".into());
    }
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err("An instruction is required.".into());
    }
    let entry = allow_entry(app, site).ok_or_else(|| {
        format!("'{site}' is not on the agent allowlist. Add it in Settings → Agents.")
    })?;
    let dir = PathBuf::from(entry["path"].as_str().unwrap_or(""));
    if !dir.exists() {
        return Err(format!("Directory not found: {}", dir.display()));
    }
    if !is_git_repo(&dir) {
        return Err(format!("{} is not a git repository — agents require git for the review/deploy gate.", dir.display()));
    }
    if is_dirty(&dir) {
        return Err(format!(
            "{site} has uncommitted changes. Agents refuse a dirty repo so the review diff is purely the agent's work — commit or stash first, sir."
        ));
    }

    let job = Job {
        id: format!("job-{}", now_ms()),
        site: site.to_string(),
        dir: dir.to_string_lossy().to_string(),
        instruction: instruction.to_string(),
        session_id: resume.filter(|s| !s.is_empty()),
        status: "queued".into(),
        base_sha: head_sha(&dir),
        started_at: now_ms(),
        ended_at: None,
        log: Vec::new(),
        summary: String::new(),
        files: Vec::new(),
        commit_hash: None,
        error: None,
    };
    let out = job.to_value();
    {
        let agents = app.state::<Agents>();
        agents.jobs.lock().unwrap().push(job);
    }
    let _ = app.emit("agent-update", out.clone());
    try_promote(app);
    Ok(out)
}

/// Promote queued jobs into running while a slot is free.
fn try_promote(app: &AppHandle) {
    loop {
        if running_count(app) >= MAX_RUNNING {
            return;
        }
        let next = {
            let agents = app.state::<Agents>();
            let jobs = agents.jobs.lock().unwrap();
            jobs.iter().find(|j| j.status == "queued").map(|j| j.id.clone())
        };
        let Some(id) = next else { return };
        with_job(app, &id, |j| j.status = "running".into());
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            run_job(app2, id).await;
        });
    }
}

// ---------- the agent run ----------

async fn run_job(app: AppHandle, id: String) {
    let Some(job) = snapshot_job(&app, &id) else { return };
    let dir = PathBuf::from(&job.dir);

    let bin = match crate::brain::resolve_cli(&app, false) {
        Some(b) => b,
        None => {
            finish_failed(&app, &id, "Claude Code CLI not found on PATH — install it to dispatch agents.".into());
            try_promote(&app);
            return;
        }
    };

    // One-shot print mode, streaming JSON, edits auto-accepted, scoped to dir.
    // Resume the session for follow-up ("request changes") so context carries.
    let mut args: Vec<String> = vec![
        "-p".into(),
        "--input-format".into(), "stream-json".into(),
        "--output-format".into(), "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--permission-mode".into(), "acceptEdits".into(),
    ];
    if let Some(sid) = &job.session_id {
        args.push("--resume".into());
        args.push(sid.clone());
    }

    let mut cmd = if bin.via_cmd {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&bin.path).args(&args);
        c
    } else {
        let mut c = Command::new(&bin.path);
        c.args(&args);
        c
    };
    #[cfg(windows)]
    {
        // tokio's Command exposes creation_flags inherently (no import needed).
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = match cmd
        .current_dir(&dir)
        .env_remove("ANTHROPIC_API_KEY") // subscription billing, never credits
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            finish_failed(&app, &id, format!("Failed to launch agent: {e}"));
            try_promote(&app);
            return;
        }
    };

    // Feed the instruction as one stream-json user message, then close stdin so
    // the one-shot turn runs to completion. JSON encoding escapes the
    // instruction — no shell/Windows quoting is ever involved.
    if let Some(mut stdin) = child.stdin.take() {
        let line = json!({
            "type": "user",
            "message": { "role": "user", "content": [ { "type": "text", "text": job.instruction } ] }
        })
        .to_string()
            + "\n";
        let _ = stdin.write_all(line.as_bytes()).await;
        let _ = stdin.flush().await;
        drop(stdin); // EOF ends the one-shot prompt
    }
    let stdout = child.stdout.take();

    // Park the child so cancel() can reach it.
    {
        let agents = app.state::<Agents>();
        agents.procs.lock().unwrap().insert(id.clone(), child);
    }

    let mut summary = String::new();
    let mut session_id: Option<String> = None;
    let mut is_error = false;
    let mut err_text = String::new();

    if let Some(stdout) = stdout {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(sid) = v["session_id"].as_str() {
                session_id = Some(sid.to_string());
            }
            match v["type"].as_str() {
                Some("stream_event") => {
                    let ev = &v["event"];
                    if ev["type"] == "content_block_start" && ev["content_block"]["type"] == "tool_use" {
                        let name = ev["content_block"]["name"].as_str().unwrap_or("tool");
                        let tgt = ev["content_block"]["input"]["file_path"].as_str()
                            .or_else(|| ev["content_block"]["input"]["command"].as_str())
                            .unwrap_or("");
                        push_log(&app, &id, format!("· {name} {tgt}").trim_end().to_string());
                    }
                }
                Some("assistant") => {
                    if let Some(content) = v["message"]["content"].as_array() {
                        for block in content {
                            if let Some(t) = block["text"].as_str() {
                                if !t.trim().is_empty() {
                                    push_log(&app, &id, t.trim().to_string());
                                }
                            }
                        }
                    }
                }
                Some("result") => {
                    is_error = v["is_error"].as_bool().unwrap_or(false)
                        || v["subtype"].as_str().map(|s| s != "success").unwrap_or(false);
                    summary = v["result"].as_str().unwrap_or("").to_string();
                    if is_error && summary.trim().is_empty() {
                        err_text = v["error"].as_str().unwrap_or("agent run failed").to_string();
                    }
                }
                _ => {}
            }
        }
    }

    // Reap the child (also clears the cancel handle). If cancel() already
    // pulled it, the job is terminal and we must not overwrite that.
    let was_cancelled = {
        let agents = app.state::<Agents>();
        let mut procs = agents.procs.lock().unwrap();
        match procs.remove(&id) {
            Some(mut c) => { let _ = c.start_kill(); false }
            None => true,
        }
    };
    if was_cancelled {
        try_promote(&app);
        return;
    }

    if is_error {
        finish_failed(&app, &id, if err_text.is_empty() { summary } else { err_text });
        try_promote(&app);
        return;
    }

    // The agent edited files; JARVIS now stages and packages the diff.
    let (files, insertions) = collect_changes(&dir);
    if files.is_empty() {
        with_job(&app, &id, |j| {
            j.status = "done".into();
            j.ended_at = Some(now_ms());
            j.session_id = session_id.clone();
            j.summary = if summary.trim().is_empty() { "The agent made no file changes.".into() } else { summary.clone() };
        });
        try_promote(&app);
        return;
    }

    with_job(&app, &id, |j| {
        j.status = "review".into();
        j.ended_at = Some(now_ms());
        j.session_id = session_id.clone();
        j.summary = summary.clone();
        j.files = files.clone();
    });
    let _ = app.emit("agent-review-ready", json!({
        "id": id, "site": job.site, "files_changed": files.len(), "insertions": insertions,
    }));
    try_promote(&app);
}

fn finish_failed(app: &AppHandle, id: &str, msg: String) {
    with_job(app, id, |j| {
        j.status = "failed".into();
        j.ended_at = Some(now_ms());
        j.error = Some(msg);
    });
}

// ---------- review actions ----------

/// Approve & Deploy: commit the staged changes, push, confirm Cloudflare.
async fn approve(app: &AppHandle, id: &str) -> Result<Value, String> {
    let job = snapshot_job(app, id).ok_or("job not found")?;
    if job.status != "review" {
        return Err("Only a job awaiting review can be deployed.".into());
    }
    let dir = PathBuf::from(&job.dir);
    with_job(app, id, |j| j.status = "deploying".into());

    let msg = format!(
        "{}\n\nDispatched via JARVIS agent.",
        job.instruction.lines().next().unwrap_or(&job.instruction).chars().take(72).collect::<String>()
    );
    // Changes were staged at review time.
    if let Err(e) = run_git(&dir, &["-c", "user.name=JARVIS", "-c", "user.email=jarvis@local", "commit", "-m", &msg]) {
        with_job(app, id, |j| j.status = "review".into());
        return Err(format!("Commit failed: {e}"));
    }
    let hash = head_sha(&dir);

    // THE deploy gate: this is the only place JARVIS ever pushes.
    if let Err(e) = run_git(&dir, &["push"]) {
        // Keep the local commit; surface the push failure for retry.
        with_job(app, id, |j| {
            j.status = "review".into();
            j.commit_hash = Some(hash.clone());
            j.error = Some(format!("Committed {} but push failed: {e}", short(&hash)));
        });
        return Err(format!("Commit {} is local only — push failed: {e}", short(&hash)));
    }

    record_history(app, &job.site, &job.instruction, &hash, "deploy", "");
    let cf = confirm_cloudflare(app, &job.site, &hash).await;
    with_job(app, id, |j| {
        j.status = "done".into();
        j.commit_hash = Some(hash.clone());
        j.error = None;
        j.summary = format!("{}\n\nDeployed · {}", j.summary, cf);
    });
    Ok(json!({ "ok": true, "commit": hash, "cloudflare": cf }))
}

/// Best-effort Cloudflare Pages deployment confirmation after a push.
async fn confirm_cloudflare(app: &AppHandle, site: &str, _hash: &str) -> String {
    let entry = match allow_entry(app, site) {
        Some(e) => e,
        None => return "pushed (no Cloudflare project mapped)".into(),
    };
    let project = entry["cf_project"].as_str().unwrap_or("");
    if project.is_empty() {
        return "pushed — Cloudflare will build from the new commit".into();
    }
    let (token, account) = {
        let db = app.state::<Db>();
        (
            crate::secrets::get(crate::secrets::CF_API_TOKEN).ok().flatten(),
            db::kv_get(&db, "cloudflare_account_id"),
        )
    };
    let (Some(token), Some(account)) = (token, account) else {
        return "pushed — Cloudflare build status not configured (add an API token in Settings)".into();
    };
    let url = format!(
        "https://api.cloudflare.com/client/v4/accounts/{account}/pages/projects/{project}/deployments?per_page=1"
    );
    match reqwest::Client::new().get(&url).bearer_auth(token).send().await {
        Ok(resp) => match resp.json::<Value>().await {
            Ok(v) => {
                let latest = &v["result"][0];
                let stage = latest["latest_stage"]["name"].as_str().unwrap_or("queued");
                let status = latest["latest_stage"]["status"].as_str().unwrap_or("active");
                format!("Cloudflare: {stage} ({status})")
            }
            Err(_) => "pushed — Cloudflare responded but status was unreadable".into(),
        },
        Err(e) => format!("pushed — couldn't reach Cloudflare ({e})"),
    }
}

/// Request changes: restore the tree and resume the agent session with a follow-up.
fn request_changes(app: &AppHandle, id: &str, follow_up: &str) -> Result<Value, String> {
    let job = snapshot_job(app, id).ok_or("job not found")?;
    let dir = PathBuf::from(&job.dir);
    let _ = run_git(&dir, &["reset", "--hard", &job.base_sha]);
    let _ = run_git(&dir, &["clean", "-fd"]);
    with_job(app, id, |j| { j.status = "superseded".into(); j.ended_at = Some(now_ms()); });
    dispatch(app, &job.site, follow_up, job.session_id)
}

/// Discard: throw away the agent's edits, restore the working tree.
fn discard(app: &AppHandle, id: &str) -> Result<Value, String> {
    let job = snapshot_job(app, id).ok_or("job not found")?;
    let dir = PathBuf::from(&job.dir);
    run_git(&dir, &["reset", "--hard", &job.base_sha])?;
    let _ = run_git(&dir, &["clean", "-fd"]);
    with_job(app, id, |j| {
        j.status = "discarded".into();
        j.ended_at = Some(now_ms());
        j.files = Vec::new();
    });
    Ok(json!({ "ok": true }))
}

fn cancel(app: &AppHandle, id: &str) -> Result<Value, String> {
    let taken = {
        let agents = app.state::<Agents>();
        let mut procs = agents.procs.lock().unwrap();
        procs.remove(id)
    };
    if let Some(mut child) = taken {
        let _ = child.start_kill();
    }
    let job = snapshot_job(app, id).ok_or("job not found")?;
    // Roll back any partial edits made before the kill.
    let dir = PathBuf::from(&job.dir);
    let _ = run_git(&dir, &["reset", "--hard", &job.base_sha]);
    let _ = run_git(&dir, &["clean", "-fd"]);
    with_job(app, id, |j| {
        j.status = "cancelled".into();
        j.ended_at = Some(now_ms());
    });
    try_promote(app);
    Ok(json!({ "ok": true }))
}

// ---------- history (durable, in SQLite) ----------

fn record_history(app: &AppHandle, site: &str, instruction: &str, hash: &str, action: &str, note: &str) {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let _ = conn.execute(
        "INSERT INTO agent_history (ts, site, instruction, commit_hash, action, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![now_ms(), site, instruction, hash, action, note],
    );
}

fn history_list(app: &AppHandle) -> Vec<Value> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let Ok(mut stmt) = conn.prepare(
        "SELECT ts, site, instruction, commit_hash, action, note FROM agent_history ORDER BY id DESC LIMIT 50",
    ) else { return vec![] };
    stmt.query_map([], |r| {
        Ok(json!({
            "ts": r.get::<_, i64>(0)?,
            "site": r.get::<_, String>(1)?,
            "instruction": r.get::<_, String>(2)?,
            "commit_hash": r.get::<_, String>(3)?,
            "action": r.get::<_, String>(4)?,
            "note": r.get::<_, String>(5)?,
        }))
    })
    .map(|it| it.filter_map(Result::ok).collect::<Vec<_>>())
    .unwrap_or_default()
}

/// One-tap rollback: revert a deployed commit and push the revert.
async fn rollback(app: &AppHandle, site: &str, hash: &str) -> Result<Value, String> {
    let entry = allow_entry(app, site).ok_or("site not on allowlist")?;
    let dir = PathBuf::from(entry["path"].as_str().unwrap_or(""));
    if is_dirty(&dir) {
        return Err(format!("{site} has uncommitted changes — resolve them before rolling back."));
    }
    run_git(&dir, &["-c", "user.name=JARVIS", "-c", "user.email=jarvis@local", "revert", "--no-edit", hash])?;
    let revert_sha = head_sha(&dir);
    run_git(&dir, &["push"])?;
    record_history(app, site, &format!("rollback of {}", short(hash)), &revert_sha, "rollback", "");
    let cf = confirm_cloudflare(app, site, &revert_sha).await;
    Ok(json!({ "ok": true, "revert": revert_sha, "cloudflare": cf }))
}

// ---------- model tools (voice / chat) ----------

pub fn dispatch_tool(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let site = input["site"].as_str().ok_or("site required")?;
    let instruction = input["instruction"].as_str().ok_or("instruction required")?;
    let job = dispatch(app, site, instruction, None)?;
    let _ = app.emit("navigate", json!({ "tab": "agents" }));
    Ok(json!({
        "ok": true,
        "job_id": job["id"],
        "status": job["status"],
        "note": "Agent dispatched. It edits locally and stops at the review gate — nothing deploys until sir approves. Tell sir you will notify him when it is staged for review."
    }))
}

pub fn status_tool(app: &AppHandle) -> Result<Value, String> {
    let agents = app.state::<Agents>();
    let jobs = agents.jobs.lock().unwrap();
    let active: Vec<Value> = jobs.iter()
        .filter(|j| j.status == "running" || j.status == "queued" || j.status == "deploying")
        .map(|j| json!({ "site": j.site, "instruction": j.instruction, "status": j.status }))
        .collect();
    let reviews: Vec<Value> = jobs.iter()
        .filter(|j| j.status == "review")
        .map(|j| json!({ "site": j.site, "instruction": j.instruction, "files_changed": j.files.len() }))
        .collect();
    drop(jobs);
    Ok(json!({
        "enabled": agents_enabled(app),
        "active": active,
        "pending_reviews": reviews,
        "active_count": active.len(),
        "review_count": reviews.len(),
    }))
}

/// For the morning briefing: how many jobs await sir's review.
pub fn pending_review_count(app: &AppHandle) -> usize {
    let agents = app.state::<Agents>();
    let jobs = agents.jobs.lock().unwrap();
    jobs.iter().filter(|j| j.status == "review").count()
}

// ---------- Tauri commands ----------

#[tauri::command]
pub fn agents_config(app: AppHandle) -> Result<Value, String> {
    let cli = crate::brain::resolve_cli(&app, true);
    Ok(json!({
        "enabled": agents_enabled(&app),
        "allowlist": allowlist(&app),
        "cli_available": cli.is_some(),
        "cli_version": cli.map(|c| c.version),
        "max_concurrency": MAX_RUNNING,
    }))
}

#[tauri::command]
pub fn agents_set_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let db = app.state::<Db>();
    db::kv_set(&db, "agents_enabled", if enabled { "1" } else { "0" })
}

#[tauri::command]
pub fn agents_set_allowlist(app: AppHandle, allowlist: Value) -> Result<(), String> {
    let db = app.state::<Db>();
    db::kv_set(&db, "agents_allowlist", &allowlist.to_string())
}

#[tauri::command]
pub fn agents_jobs(app: AppHandle) -> Result<Value, String> {
    let agents = app.state::<Agents>();
    let list: Vec<Value> = agents.jobs.lock().unwrap().iter().rev().map(Job::to_value).collect();
    Ok(json!({ "jobs": list, "history": history_list(&app) }))
}

#[tauri::command]
pub fn agents_dispatch(app: AppHandle, site: String, instruction: String) -> Result<Value, String> {
    dispatch(&app, &site, &instruction, None)
}

#[tauri::command]
pub async fn agents_approve(app: AppHandle, id: String) -> Result<Value, String> {
    approve(&app, &id).await
}

#[tauri::command]
pub fn agents_request_changes(app: AppHandle, id: String, follow_up: String) -> Result<Value, String> {
    request_changes(&app, &id, &follow_up)
}

#[tauri::command]
pub fn agents_discard(app: AppHandle, id: String) -> Result<Value, String> {
    discard(&app, &id)
}

#[tauri::command]
pub fn agents_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    cancel(&app, &id)
}

#[tauri::command]
pub async fn agents_rollback(app: AppHandle, site: String, hash: String) -> Result<Value, String> {
    rollback(&app, &site, &hash).await
}
