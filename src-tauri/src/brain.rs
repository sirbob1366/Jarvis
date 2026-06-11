//! Brain routing — subscription-first.
//!
//! CLI mode (default): conversational turns run through the local Claude Code
//! CLI in headless mode, which bills sir's Max subscription instead of API
//! credits. Implementation: ONE warm persistent process
//!     claude -p --input-format stream-json --output-format stream-json
//! spawned in a dedicated sandbox folder. The sandbox is file-configured so
//! no fragile Windows argument quoting is needed:
//!     CLAUDE.md                    — the JARVIS persona (system prompt)
//!     .mcp.json                    — points at the app's local MCP shim
//!     .claude/settings.local.json  — allows mcp__jarvis__* + read-only
//!                                    built-ins, denies Bash/Edit/Write/web
//! The app's tool layer (portfolio, weather, calendars, work, todos,
//! navigate_app…) is served to the CLI over MCP (mcp.rs) — the same Rust
//! executors as API mode, so both brains have identical capabilities.
//!
//! API mode (fallback): the existing direct Anthropic path. Used when
//! selected in Settings, or automatically when the CLI is missing or a turn
//! errors (a visible status note + event explains the switch; subscription
//! limit exhaustion raises a distinct notice with a one-tap switch).
//!
//! Latency: the warm session removes the per-turn process spawn (~2-4s cold)
//! but still adds a few hundred ms over the raw API; first-delta and total
//! latencies are measured per turn and surfaced in Settings so the trade-off
//! is visible. Voice flows feel best under ~1s first-delta — if CLI mode
//! consistently misses that on this machine, Settings shows it and one tap
//! moves the brain to API mode.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use crate::claude;
use crate::db::{self, Db};

pub struct Brain {
    session: tokio::sync::Mutex<Option<CliSession>>,
    pub mcp_url: std::sync::Mutex<String>,
    cli: std::sync::Mutex<Option<CliBinary>>, // resolved once, refreshed on demand
}

impl Default for Brain {
    fn default() -> Self {
        Self {
            session: tokio::sync::Mutex::new(None),
            mcp_url: std::sync::Mutex::new(String::new()),
            cli: std::sync::Mutex::new(None),
        }
    }
}

#[derive(Clone)]
struct CliBinary {
    path: String,
    via_cmd: bool,
    version: String,
}

enum TurnEvent {
    Delta(String),
    Tool(String),
    Result { text: String, is_error: bool },
    Closed,
}

struct CliSession {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::UnboundedReceiver<TurnEvent>,
}

// ---------- CLI discovery ----------

fn locate_claude() -> Option<CliBinary> {
    let out = std::process::Command::new("where.exe")
        .arg("claude")
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let path = stdout.lines().map(str::trim).find(|l| !l.is_empty())?.to_string();
    let lower = path.to_lowercase();
    let via_cmd = lower.ends_with(".cmd") || lower.ends_with(".bat") || lower.ends_with(".ps1");

    // Version probe doubles as a health check.
    let mut probe = if via_cmd {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", &path, "--version"]);
        c
    } else {
        let mut c = std::process::Command::new(&path);
        c.arg("--version");
        c
    };
    let version = probe
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    Some(CliBinary { path, via_cmd, version })
}

fn cli_binary(app: &AppHandle, refresh: bool) -> Option<CliBinary> {
    let brain = app.state::<Brain>();
    let mut guard = brain.cli.lock().unwrap();
    if refresh || guard.is_none() {
        *guard = locate_claude();
    }
    guard.clone()
}

// ---------- sandbox ----------

/// Stage 6 makes the JARVIS-OS vault the brain's home when it exists, so the
/// routing tree (CLAUDE.md) loads natively; otherwise a dedicated sandbox.
pub fn brain_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    let vault = PathBuf::from(&home).join("JARVIS-OS");
    if vault.join("CLAUDE.md").exists() {
        return vault;
    }
    PathBuf::from(home).join("jarvis-brain")
}

const SANDBOX_CLAUDE_MD: &str = "\
# JARVIS brain — persona and rules

You are J.A.R.V.I.S., the user's private desktop assistant. Address him as \"sir\".
Voice: crisp, capable butler-AI; concise; dry wit welcome; never robotic filler.
Replies are usually spoken aloud — one to three sentences unless sir asks for detail.

Speech discipline: never read URLs, message ids, or permalinks aloud — say
\"link on screen\"; round large numbers when speaking (the app shows exact
figures); the app's board and HUD carry the detail, your voice carries the
summary. When a richer on-screen view exists, call mcp__jarvis__navigate_app.

Your tools are the mcp__jarvis__* set (portfolio analytics, weather, timers,
calendars, work email/slack — strictly read-only — todos, notes, navigation).
Use them whenever they answer the question; use actual numbers from results.
If a tool fails, say so plainly and move on. Do not use Bash, file edits, or
web access — they are disabled here by design.
";

fn ensure_sandbox(mcp_url: &str) -> Result<PathBuf, String> {
    let dir = brain_dir();
    std::fs::create_dir_all(dir.join(".claude")).map_err(|e| e.to_string())?;

    // Persona — only when this is the plain sandbox (the vault owns its own CLAUDE.md).
    let claude_md = dir.join("CLAUDE.md");
    if dir.file_name().map(|n| n == "jarvis-brain").unwrap_or(false) {
        std::fs::write(&claude_md, SANDBOX_CLAUDE_MD).map_err(|e| e.to_string())?;
    }

    let mcp = json!({ "mcpServers": { "jarvis": { "type": "http", "url": mcp_url } } });
    std::fs::write(dir.join(".mcp.json"), serde_json::to_string_pretty(&mcp).unwrap())
        .map_err(|e| e.to_string())?;

    let settings = json!({
        "enableAllProjectMcpServers": true,
        "permissions": {
            "allow": ["mcp__jarvis__*", "Read(**)", "Glob(**)", "Grep(**)"],
            "deny": ["Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task"]
        }
    });
    std::fs::write(
        dir.join(".claude").join("settings.local.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    Ok(dir)
}

// ---------- warm session ----------

async fn spawn_session(app: &AppHandle) -> Result<CliSession, String> {
    let bin = cli_binary(app, false).ok_or("Claude Code CLI not found on PATH")?;
    let mcp_url = app.state::<Brain>().mcp_url.lock().unwrap().clone();
    let dir = ensure_sandbox(&mcp_url)?;

    let args = [
        "-p",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose",
        "--include-partial-messages",
    ];

    let mut cmd = if bin.via_cmd {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&bin.path).args(args);
        c
    } else {
        let mut c = Command::new(&bin.path);
        c.args(args);
        c
    };

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .current_dir(&dir)
        // Subscription auth only — never let a stray env key bill API credits.
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start Claude Code: {e}"))?;

    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

    let (tx, rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            match v["type"].as_str() {
                Some("stream_event") => {
                    let ev = &v["event"];
                    match ev["type"].as_str() {
                        Some("content_block_delta") => {
                            if let Some(t) = ev["delta"]["text"].as_str() {
                                let _ = tx.send(TurnEvent::Delta(t.to_string()));
                            }
                        }
                        Some("content_block_start") => {
                            if ev["content_block"]["type"] == "tool_use" {
                                let name = ev["content_block"]["name"].as_str().unwrap_or("tool");
                                let short = name.strip_prefix("mcp__jarvis__").unwrap_or(name);
                                let _ = tx.send(TurnEvent::Tool(short.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
                Some("result") => {
                    let is_error = v["is_error"].as_bool().unwrap_or(false)
                        || v["subtype"].as_str().map(|s| s != "success").unwrap_or(false);
                    let text = v["result"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v["error"].to_string());
                    let _ = tx.send(TurnEvent::Result { text, is_error });
                }
                _ => {}
            }
        }
        let _ = tx.send(TurnEvent::Closed);
    });

    Ok(CliSession { child, stdin, rx })
}

async fn cli_turn(app: &AppHandle, message: &str) -> Result<String, String> {
    let brain = app.state::<Brain>();
    let mut guard = brain.session.lock().await; // one turn at a time

    if guard.is_none() {
        *guard = Some(spawn_session(app).await?);
    }
    let sess = guard.as_mut().unwrap();

    let line = json!({
        "type": "user",
        "message": { "role": "user", "content": [ { "type": "text", "text": message } ] }
    })
    .to_string()
        + "\n";

    let t0 = Instant::now();
    if sess.stdin.write_all(line.as_bytes()).await.is_err() {
        *guard = None;
        return Err("CLI session pipe broke — restarting next turn.".into());
    }
    let _ = sess.stdin.flush().await;

    let mut first_delta_ms: Option<u128> = None;
    let mut streamed = String::new();

    loop {
        let ev = tokio::time::timeout(std::time::Duration::from_secs(240), sess.rx.recv())
            .await
            .map_err(|_| {
                "CLI turn timed out.".to_string()
            })?;
        match ev {
            Some(TurnEvent::Delta(t)) => {
                if first_delta_ms.is_none() {
                    first_delta_ms = Some(t0.elapsed().as_millis());
                }
                streamed.push_str(&t);
                let _ = app.emit("jarvis-delta", json!({ "text": t }));
            }
            Some(TurnEvent::Tool(name)) => {
                let _ = app.emit("jarvis-tool", json!({ "name": name }));
            }
            Some(TurnEvent::Result { text, is_error }) => {
                let total = t0.elapsed().as_millis();
                record_latency(app, "cli", first_delta_ms.unwrap_or(total), total);
                if is_error {
                    let _ = sess.child.start_kill();
                    *guard = None;
                    return Err(if text.trim().is_empty() { "CLI turn failed.".into() } else { text });
                }
                let final_text = if text.trim().is_empty() { streamed } else { text };
                return Ok(final_text);
            }
            Some(TurnEvent::Closed) | None => {
                *guard = None;
                return Err("Claude Code session ended unexpectedly.".into());
            }
        }
    }
}

fn record_latency(app: &AppHandle, mode: &str, first_ms: u128, total_ms: u128) {
    let db = app.state::<Db>();
    let _ = db::kv_set(&db, &format!("brain_{mode}_first_ms"), &first_ms.to_string());
    let _ = db::kv_set(&db, &format!("brain_{mode}_total_ms"), &total_ms.to_string());
}

fn is_limit_error(e: &str) -> bool {
    let l = e.to_lowercase();
    l.contains("limit") || l.contains("quota") || l.contains("rate")
}

// ---------- routing ----------

pub fn mode(app: &AppHandle) -> String {
    let db = app.state::<Db>();
    db::kv_get(&db, "brain_mode").unwrap_or_else(|| "cli".into())
}

/// One conversational turn through whichever brain is active.
/// Streams jarvis-delta / jarvis-tool either way; returns the final text.
pub async fn converse(app: &AppHandle, message: &str) -> Result<String, String> {
    if mode(app) == "cli" {
        match cli_turn(app, message).await {
            Ok(text) => {
                // Mirror the exchange into the API session so a mode switch
                // keeps conversational context.
                claude::mirror_into_session(app, message, &text);
                let _ = app.emit("brain-status", json!({ "active": "cli" }));
                return Ok(text);
            }
            Err(e) => {
                if is_limit_error(&e) {
                    let _ = app.emit("brain-limit", json!({ "error": e }));
                } else {
                    let _ = app.emit("brain-status", json!({
                        "active": "api",
                        "fallback": true,
                        "note": format!("Claude Code unavailable ({e}) — using the API for now, sir.")
                    }));
                }
                // fall through to API
            }
        }
    }
    let t0 = Instant::now();
    let out = claude::api_converse(app, message).await;
    if out.is_ok() {
        let total = t0.elapsed().as_millis();
        record_latency(app, "api", total, total);
        let _ = app.emit("brain-status", json!({ "active": "api" }));
    }
    out
}

/// Drop the warm CLI session (clear conversation / mode switch).
pub async fn reset(app: &AppHandle) {
    let brain = app.state::<Brain>();
    let mut guard = brain.session.lock().await;
    if let Some(sess) = guard.as_mut() {
        let _ = sess.child.start_kill();
    }
    *guard = None;
}

// ---------- Tauri commands ----------

#[tauri::command]
pub async fn brain_status(app: AppHandle) -> Result<Value, String> {
    let bin = cli_binary(&app, true);
    let db = app.state::<Db>();
    Ok(json!({
        "mode": mode(&app),
        "cli_available": bin.is_some(),
        "cli_version": bin.map(|b| b.version),
        "sandbox": brain_dir().to_string_lossy(),
        "cli_first_ms": db::kv_get(&db, "brain_cli_first_ms"),
        "cli_total_ms": db::kv_get(&db, "brain_cli_total_ms"),
        "api_total_ms": db::kv_get(&db, "brain_api_total_ms"),
    }))
}

#[tauri::command]
pub async fn brain_set_mode(app: AppHandle, mode: String) -> Result<Value, String> {
    if !["cli", "api"].contains(&mode.as_str()) {
        return Err("mode must be 'cli' or 'api'".into());
    }
    {
        let db = app.state::<Db>();
        db::kv_set(&db, "brain_mode", &mode)?;
    }
    if mode == "api" {
        reset(&app).await;
    }
    let _ = app.emit("brain-status", json!({ "active": mode }));
    Ok(json!({ "ok": true, "mode": mode }))
}
