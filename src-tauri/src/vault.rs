//! JARVIS-OS — the second brain. A local, git-versioned, plain-markdown
//! vault at ~/JARVIS-OS, bootstrapped from Nate Herk's AIS-OS kit (MIT;
//! LICENSE and attribution kept in the vault README) and adapted to three
//! domains: WORK / BUSINESS / PERSONAL.
//!
//! Brain integration: in CLI mode the vault IS the brain's working directory
//! (brain::brain_dir), so CLAUDE.md's routing tree loads natively and the
//! model Reads domain files itself. In API mode, context_for() injects
//! CLAUDE.md plus the relevant domain's files into the system prompt,
//! routed by the domain pin or a keyword classifier (token budget: route by
//! domain rather than loading everything).
//!
//! Memory write-back: log_decision / save_note / update_context tools. Every
//! change auto-commits to the vault's local git. Nothing is written silently:
//! while the Settings "ask before writing" toggle is on (default), tool calls
//! without confirmed:true are refused with an instruction to ask sir first.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::{self, Db};

const KIT_REPO: &str = "https://github.com/nateherkai/AIS-OS";
const DOMAINS: [&str; 3] = ["work", "business", "personal"];

pub fn vault_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("JARVIS-OS")
}

pub fn exists() -> bool {
    vault_dir().join("CLAUDE.md").exists()
}

// ---------- git plumbing ----------

fn run_hidden(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn git(args: &[&str]) -> Result<String, String> {
    run_hidden("git", args, Some(&vault_dir()))
}

/// Commit everything pending. Failure is non-fatal (vault works ungitted).
fn commit(message: &str) {
    let _ = git(&["add", "-A"]);
    let _ = git(&[
        "-c", "user.name=JARVIS",
        "-c", "user.email=jarvis@local",
        "commit", "-m", message,
    ]);
}

fn emit_changed(app: &AppHandle, path: &str) {
    let _ = app.emit("vault-changed", json!({ "path": path }));
}

// ---------- bootstrap ----------

const CLAUDE_MD: &str = include_str!("vault_templates/CLAUDE.md");
const CONNECTIONS_MD: &str = include_str!("vault_templates/connections.md");
const README_MD: &str = include_str!("vault_templates/README.md");
const WORK_MD: &str = include_str!("vault_templates/work-context.md");
const BUSINESS_MD: &str = include_str!("vault_templates/business-context.md");
const PERSONAL_MD: &str = include_str!("vault_templates/personal-context.md");
const SKILL_ONBOARD: &str = include_str!("vault_templates/skill-onboard.md");
const SKILL_AUDIT: &str = include_str!("vault_templates/skill-audit.md");
const SKILL_LEVELUP: &str = include_str!("vault_templates/skill-level-up.md");

/// Create ~/JARVIS-OS: clone the AIS-OS kit as skeleton when git+network
/// allow, then adapt to the three-domain layout. Idempotent.
#[tauri::command]
pub async fn vault_init(app: AppHandle) -> Result<String, String> {
    let dir = vault_dir();
    if exists() {
        return Ok(format!("Vault already online at {}", dir.display()));
    }

    let cloned = tauri::async_runtime::spawn_blocking(move || {
        if vault_dir().exists() {
            return false; // half-made dir: adapt in place, don't clone over it
        }
        let ok = run_hidden(
            "git",
            &["clone", "--depth", "1", KIT_REPO, &vault_dir().to_string_lossy()],
            None,
        )
        .is_ok();
        if ok {
            // Detach from the kit's history; the vault gets its own repo.
            let _ = run_hidden("cmd", &["/C", "rmdir", "/S", "/Q", ".git"], Some(&vault_dir()));
        }
        ok
    })
    .await
    .map_err(|e| e.to_string())?;

    let dir = vault_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Three-domain layout + kit dirs we keep (references, archives, decisions).
    for sub in ["work", "business", "personal", "references", "archives", "decisions", ".claude/skills/onboard", ".claude/skills/audit", ".claude/skills/level-up"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|e| e.to_string())?;
    }
    // The kit's context/ folder (if cloned) is superseded by the domains.
    if cloned && dir.join("context").exists() {
        let _ = std::fs::rename(dir.join("context"), dir.join("archives").join("kit-context"));
    }

    let write_if = |rel: &str, content: &str, force: bool| -> Result<(), String> {
        let p = dir.join(rel);
        if force || !p.exists() {
            std::fs::write(&p, content).map_err(|e| e.to_string())?;
        }
        Ok(())
    };

    // Adapted core (always ours) — kit LICENSE stays as cloned.
    write_if("CLAUDE.md", CLAUDE_MD, true)?;
    write_if("connections.md", CONNECTIONS_MD, true)?;
    write_if("README.md", README_MD, true)?;
    write_if(
        "decisions/log.md",
        "# Decision log\n\nAppend-only. Newest first. Format:\n`## YYYY-MM-DD [domain] — decision`\nfollowed by *Why.*\n",
        false,
    )?;
    write_if("work/context.md", WORK_MD, false)?;
    write_if("business/context.md", BUSINESS_MD, false)?;
    write_if("personal/context.md", PERSONAL_MD, false)?;
    write_if(".claude/skills/onboard/SKILL.md", SKILL_ONBOARD, true)?;
    write_if(".claude/skills/audit/SKILL.md", SKILL_AUDIT, true)?;
    write_if(".claude/skills/level-up/SKILL.md", SKILL_LEVELUP, true)?;
    if !dir.join("LICENSE").exists() {
        write_if(
            "LICENSE",
            "Skeleton structure adapted from AIS-OS (c) Nate Herk, MIT License.\nhttps://github.com/nateherkai/AIS-OS\n",
            false,
        )?;
    }

    // Fresh repo + first commit.
    let _ = git(&["init"]);
    commit("JARVIS-OS: initial vault (adapted from AIS-OS, MIT)");

    emit_changed(&app, "");
    Ok(format!(
        "Vault online at {}{}. The brain now works from it.",
        dir.display(),
        if cloned { " (AIS-OS skeleton cloned + adapted)" } else { " (kit unreachable — built from embedded skeleton)" }
    ))
}

// ---------- tree / read / write (UI + Mind Map) ----------

fn build_tree(dir: &Path, rel: &Path, depth: u8) -> Vec<Value> {
    if depth > 4 {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<Value> = Vec::new();
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    for e in items {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" || name == "LICENSE" {
            continue;
        }
        let p = e.path();
        let rel_p = rel.join(&name);
        if p.is_dir() {
            out.push(json!({
                "name": name,
                "path": rel_p.to_string_lossy().replace('\\', "/"),
                "dir": true,
                "children": build_tree(&p, &rel_p, depth + 1),
            }));
        } else if name.ends_with(".md") {
            let preview = std::fs::read_to_string(&p)
                .ok()
                .map(|c| c.lines().take(6).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            out.push(json!({
                "name": name,
                "path": rel_p.to_string_lossy().replace('\\', "/"),
                "dir": false,
                "preview": preview,
            }));
        }
    }
    out
}

fn safe_path(rel: &str) -> Result<PathBuf, String> {
    if rel.contains("..") || rel.starts_with('/') || rel.contains(':') {
        return Err("Path escapes the vault.".into());
    }
    Ok(vault_dir().join(rel))
}

#[tauri::command]
pub fn vault_tree() -> Result<Value, String> {
    if !exists() {
        return Err("Vault not initialized.".into());
    }
    Ok(json!({
        "root": vault_dir().to_string_lossy(),
        "tree": build_tree(&vault_dir(), Path::new(""), 0),
    }))
}

#[tauri::command]
pub fn vault_read_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(safe_path(&path)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn vault_write_file(app: AppHandle, path: String, content: String) -> Result<(), String> {
    let p = safe_path(&path)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, content).map_err(|e| e.to_string())?;
    commit(&format!("edit: {path}"));
    emit_changed(&app, &path);
    Ok(())
}

#[tauri::command]
pub fn vault_status(app: AppHandle) -> Result<Value, String> {
    let db = app.state::<Db>();
    if !exists() {
        return Ok(json!({ "exists": false }));
    }
    let dirty = git(&["status", "--porcelain"]).map(|s| !s.trim().is_empty()).unwrap_or(false);
    Ok(json!({
        "exists": true,
        "root": vault_dir().to_string_lossy(),
        "git_dirty": dirty,
        "last_audit_date": db::kv_get(&db, "vault_last_audit_date"),
        "last_audit": db::kv_get(&db, "vault_last_audit"),
    }))
}

// ---------- model tools: the write-back path ----------

fn writeback_guard(app: &AppHandle, input: &Value) -> Result<(), String> {
    let db = app.state::<Db>();
    let ask = db::kv_get(&db, "vault_writeback_ask").as_deref() != Some("0");
    if ask && input["confirmed"].as_bool() != Some(true) {
        return Err("Write-back confirmation is on: ask sir first (e.g. \"Shall I log that decision, sir?\"), then retry this call with confirmed: true.".into());
    }
    Ok(())
}

fn valid_domain(d: &str) -> Result<&str, String> {
    if DOMAINS.contains(&d) {
        Ok(d)
    } else {
        Err(format!("domain must be one of {DOMAINS:?}"))
    }
}

fn slugify(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(48).collect()
}

pub fn log_decision(app: &AppHandle, input: &Value) -> Result<Value, String> {
    if !exists() {
        return Err("Vault not initialized — sir can do that from the Mind Map tab.".into());
    }
    writeback_guard(app, input)?;
    let domain = valid_domain(input["domain"].as_str().unwrap_or(""))?;
    let decision = input["decision"].as_str().filter(|s| !s.trim().is_empty()).ok_or("decision required")?;
    let why = input["why"].as_str().unwrap_or("");
    let date = crate::tools::ist_now().format("%Y-%m-%d").to_string();

    let path = vault_dir().join("decisions/log.md");
    let prior = std::fs::read_to_string(&path).unwrap_or_default();
    // Append-only, newest entries right under the header.
    let (head, rest) = prior.split_once("\n\n").unwrap_or((prior.as_str(), ""));
    let entry = format!("## {date} [{domain}] — {}\n*Why:* {}\n\n", decision.trim(), why.trim());
    let next = format!("{head}\n\n{entry}{rest}");
    std::fs::write(&path, next).map_err(|e| e.to_string())?;
    commit(&format!("decision [{domain}]: {}", &decision.chars().take(60).collect::<String>()));
    emit_changed(app, "decisions/log.md");
    Ok(json!({ "ok": true, "logged": decision, "domain": domain }))
}

pub fn save_note(app: &AppHandle, input: &Value) -> Result<Value, String> {
    if !exists() {
        return Err("Vault not initialized — sir can do that from the Mind Map tab.".into());
    }
    writeback_guard(app, input)?;
    let domain = valid_domain(input["domain"].as_str().unwrap_or(""))?;
    let topic = input["topic"].as_str().filter(|s| !s.trim().is_empty()).ok_or("topic required")?;
    let content = input["content"].as_str().filter(|s| !s.trim().is_empty()).ok_or("content required")?;
    let date = crate::tools::ist_now().format("%Y-%m-%d").to_string();

    let rel = format!("{domain}/{}.md", slugify(topic));
    let p = vault_dir().join(&rel);
    let mut body = std::fs::read_to_string(&p).unwrap_or_else(|_| format!("# {topic}\n"));
    body.push_str(&format!("\n## {date}\n{}\n", content.trim()));
    std::fs::write(&p, body).map_err(|e| e.to_string())?;
    commit(&format!("note [{domain}]: {topic}"));
    emit_changed(app, &rel);
    Ok(json!({ "ok": true, "file": rel }))
}

pub fn update_context(app: &AppHandle, input: &Value) -> Result<Value, String> {
    if !exists() {
        return Err("Vault not initialized — sir can do that from the Mind Map tab.".into());
    }
    writeback_guard(app, input)?;
    let file = input["file"].as_str().filter(|s| s.ends_with(".md")).ok_or("file (.md, vault-relative) required")?;
    let p = safe_path(file)?;
    let old = input["old_text"].as_str().unwrap_or("");
    let new = input["new_text"].as_str().ok_or("new_text required")?;

    let body = std::fs::read_to_string(&p).map_err(|_| format!("{file} does not exist — use save_note to create notes."))?;
    let next = if old.is_empty() {
        format!("{body}\n{new}\n") // append mode
    } else {
        if !body.contains(old) {
            return Err("old_text not found in the file — re-read it and retry with the exact text.".into());
        }
        body.replacen(old, new, 1)
    };
    std::fs::write(&p, next).map_err(|e| e.to_string())?;
    commit(&format!("update: {file}"));
    emit_changed(app, file);
    Ok(json!({ "ok": true, "file": file }))
}

pub fn vault_search(input: &Value) -> Result<Value, String> {
    if !exists() {
        return Err("Vault not initialized.".into());
    }
    let query = input["query"].as_str().filter(|s| !s.trim().is_empty()).ok_or("query required")?;
    let q = query.to_lowercase();
    let mut hits = Vec::new();
    fn walk(dir: &Path, rel: &Path, q: &str, hits: &mut Vec<Value>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            let rel_p = rel.join(&name);
            if p.is_dir() {
                walk(&p, &rel_p, q, hits);
            } else if name.ends_with(".md") {
                if let Ok(body) = std::fs::read_to_string(&p) {
                    for (i, line) in body.lines().enumerate() {
                        if line.to_lowercase().contains(q) && hits.len() < 20 {
                            hits.push(json!({
                                "file": rel_p.to_string_lossy().replace('\\', "/"),
                                "line": i + 1,
                                "text": line.trim().chars().take(200).collect::<String>(),
                            }));
                        }
                    }
                }
            }
        }
    }
    walk(&vault_dir(), Path::new(""), &q, &mut hits);
    Ok(json!({ "query": query, "hits": hits }))
}

pub fn vault_read_tool(input: &Value) -> Result<Value, String> {
    let file = input["file"].as_str().ok_or("file required")?;
    let body = std::fs::read_to_string(safe_path(file)?).map_err(|e| e.to_string())?;
    Ok(json!({ "file": file, "content": body.chars().take(12_000).collect::<String>() }))
}

// ---------- API-mode context injection (domain routing) ----------

const WORK_HINTS: &[&str] = &["enertiv", "element", "northbridge", "principal", "mortenson", "meeting", "coi", "client", "slack", "work", "jon", "melvin", "clint", "mad dash"];
const BUSINESS_HINTS: &[&str] = &["pdfedit", "imagetool", "audiotool", "videotool", "invoicetool", "site", "traffic", "revenue", "adsense", "ezoic", "seo", "analytics", "portfolio", "deploy", "worker", "paper-trading", "little hills"];
const PERSONAL_HINTS: &[&str] = &["gym", "health", "fitness", "travel", "trip", "finance", "goal", "family", "personal"];

fn classify(message: &str) -> Vec<&'static str> {
    let m = message.to_lowercase();
    let mut hit: Vec<&str> = Vec::new();
    if WORK_HINTS.iter().any(|h| m.contains(h)) {
        hit.push("work");
    }
    if BUSINESS_HINTS.iter().any(|h| m.contains(h)) {
        hit.push("business");
    }
    if PERSONAL_HINTS.iter().any(|h| m.contains(h)) {
        hit.push("personal");
    }
    hit
}

fn read_clipped(p: &Path, max: usize) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.chars().take(max).collect())
}

/// Vault context for the API brain's system prompt — CLAUDE.md always,
/// plus the pinned/classified domain's files (budgeted, not everything).
pub fn context_for(app: &AppHandle, message: &str) -> String {
    if !exists() {
        return String::new();
    }
    let dir = vault_dir();
    let mut out = String::from("\n\n=== JARVIS-OS VAULT (sir's second brain; treat as ground truth) ===\n");
    if let Some(c) = read_clipped(&dir.join("CLAUDE.md"), 6000) {
        out.push_str(&c);
    }

    let db = app.state::<Db>();
    let pin = db::kv_get(&db, "domain_pin").unwrap_or_else(|| "all".into());
    let domains: Vec<String> = if DOMAINS.contains(&pin.as_str()) {
        vec![pin]
    } else {
        classify(message).into_iter().map(String::from).collect()
    };

    for d in &domains {
        // Most recently touched files carry the freshest context.
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(d))
            .map(|r| r.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "md").unwrap_or(false)).collect())
            .unwrap_or_default();
        files.sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
        for f in files.into_iter().take(3) {
            if let Some(c) = read_clipped(&f, 4000) {
                out.push_str(&format!("\n--- {}/{} ---\n{c}\n", d, f.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
    }
    // Recent decisions are always relevant.
    if let Some(c) = read_clipped(&dir.join("decisions/log.md"), 2500) {
        out.push_str(&format!("\n--- decisions/log.md (recent) ---\n{c}\n"));
    }
    out.push_str("=== END VAULT ===\n");
    out
}
