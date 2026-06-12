//! Proactive behaviors:
//! 1. Morning briefing — on the first wake of the day (inside a configurable
//!    IST window, default 06:00–12:00), JARVIS greets sir with a compact
//!    spoken briefing built from his tools (portfolio + weather + calendar
//!    once Stage 5 lands). Streams into the normal chat view.
//! 2. Anomaly watch — polls the Worker's /api/anomalies every 30 minutes;
//!    a hit raises a native notification + a one-line spoken heads-up
//!    (max one alert per site per 6h, mirroring the Discord throttle).

use chrono::Timelike;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::claude;
use crate::db::{self, Db};
use crate::secrets;
use crate::tools;

const ANOMALY_POLL_SECS: u64 = 30 * 60;
const ALERT_COOLDOWN_MS: i64 = 6 * 60 * 60 * 1000;

#[derive(Default)]
pub struct AlertLog(Mutex<HashMap<String, i64>>);

const BRIEFING_PROMPT: &str = "It is the first time sir has woken me today. Deliver his morning briefing: \
use portfolio_stats (query today, and week for context) and weather (today) — plus the calendar if available — \
then give a compact spoken-style briefing: greeting, the most important portfolio change, weather in one line, \
and one actionable observation. Keep it under six sentences; this is read aloud.";

const WORK_BRIEFING_ADDON: &str = " It is a weekday, so also fold in work: work_calendar (today) for the first \
meeting and free gaps, work_email (unread_count) and work_slack (mentions) if connected — one sentence on what \
needs attention. If a work tool is not connected, skip it silently.";

const VAULT_BRIEFING_ADDON: &str = " Also consult the JARVIS-OS vault (vault_search / the vault context you \
already have): if a logged decision has a deadline near today or a project note matches today's meetings, \
mention it in one sentence — at most one item, only if genuinely relevant.";

fn briefing_prompt(app: &AppHandle) -> String {
    use chrono::Datelike;
    let now = tools::ist_now();
    let weekday = !matches!(now.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
    let mut p = BRIEFING_PROMPT.to_string();
    if weekday {
        p.push_str(WORK_BRIEFING_ADDON);
    }
    let pending = crate::agents::pending_review_count(app);
    if pending > 0 {
        p.push_str(&format!(
            " Also mention, in one sentence, that {pending} agent {} awaiting his review on the Agents tab.",
            if pending == 1 { "job is" } else { "jobs are" }
        ));
    }
    if crate::vault::exists() {
        p.push_str(VAULT_BRIEFING_ADDON);
        // Monday: fold in Sunday's context-audit health note.
        let db = app.state::<Db>();
        if now.weekday() == chrono::Weekday::Mon {
            if let Some(audit) = db::kv_get(&db, "vault_last_audit") {
                let clip: String = audit.chars().take(400).collect();
                p.push_str(&format!(
                    " Include one short context-audit health note based on Sunday's audit: {clip}"
                ));
            }
        }
    }
    p
}

/// True if the briefing hasn't run today and we're inside the window.
fn briefing_due(app: &AppHandle) -> bool {
    let db = app.state::<Db>();
    let now = tools::ist_now();
    let today = now.format("%Y-%m-%d").to_string();

    if db::kv_get(&db, "last_briefing_date").as_deref() == Some(today.as_str()) {
        return false;
    }
    let start: u32 = db::kv_get(&db, "briefing_window_start").and_then(|v| v.parse().ok()).unwrap_or(6);
    let end: u32 = db::kv_get(&db, "briefing_window_end").and_then(|v| v.parse().ok()).unwrap_or(12);
    (start..end).contains(&now.hour())
}

/// Run the briefing if due. Called at startup and whenever the window is summoned.
pub fn maybe_brief(app: &AppHandle) {
    if !briefing_due(app) {
        return;
    }
    // Mark first so a slow run can't double-fire.
    {
        let db = app.state::<Db>();
        let today = tools::ist_now().format("%Y-%m-%d").to_string();
        let _ = db::kv_set(&db, "last_briefing_date", &today);
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // The board stagger-paints in sync with the spoken briefing.
        let _ = app.emit("briefing-start", ());

        // Routed through the brain (subscription CLI first, API fallback);
        // the session keeps the exchange so follow-up questions have context.
        match crate::brain::converse(&app, &briefing_prompt(&app)).await {
            Ok(text) => {
                let _ = app.emit("jarvis-done", json!({ "text": text }));
            }
            Err(e) => {
                let _ = app.emit("jarvis-error", json!({ "error": format!("Briefing failed: {e}") }));
            }
        }
    });
}

/// Background loops: briefing check every minute, anomaly poll every 30 min.
pub fn start(app: &AppHandle) {
    let app1 = app.clone();
    tauri::async_runtime::spawn(async move {
        // Small delay so startup is quiet, then check once a minute.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            maybe_brief(&app1);
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(ANOMALY_POLL_SECS)).await;
            if let Err(e) = poll_anomalies(&app2).await {
                // Not configured / offline — stay quiet, try again next cycle.
                let _ = e;
            }
        }
    });

    // Weekly vault audit — Sunday evening (configurable), quiet run; the
    // result feeds Monday's briefing and the Mind Map scorecard overlay.
    let app3 = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
            maybe_audit(&app3).await;
        }
    });
}

const AUDIT_PROMPT: &str = "Run the vault /audit skill now: read CLAUDE.md, the three domain folders, \
decisions/log.md and connections.md (vault_read / vault_search), then produce the Four-Cs gap report in the \
exact AUDIT block format the skill defines, ending with the one spoken sentence.";

async fn maybe_audit(app: &AppHandle) {
    use chrono::Datelike;
    if !crate::vault::exists() {
        return;
    }
    let now = tools::ist_now();
    let today = now.format("%Y-%m-%d").to_string();
    let (day, hour) = {
        let db = app.state::<Db>();
        if db::kv_get(&db, "vault_last_audit_date").as_deref() == Some(today.as_str()) {
            return;
        }
        (
            db::kv_get(&db, "audit_day").unwrap_or_else(|| "Sun".into()),
            db::kv_get(&db, "audit_hour").and_then(|v| v.parse::<u32>().ok()).unwrap_or(19),
        )
    };
    if format!("{:?}", now.weekday()) != day || now.hour() < hour {
        return;
    }

    claude::QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = crate::brain::converse(app, AUDIT_PROMPT).await;
    claude::QUIET.store(false, std::sync::atomic::Ordering::Relaxed);

    if let Ok(text) = result {
        let db = app.state::<Db>();
        let _ = db::kv_set(&db, "vault_last_audit", &text);
        let _ = db::kv_set(&db, "vault_last_audit_date", &today);
        let _ = app.emit("vault-audit", json!({ "text": text }));
    }
}

async fn poll_anomalies(app: &AppHandle) -> Result<(), String> {
    let id = secrets::get(secrets::CF_ACCESS_CLIENT_ID)?.ok_or("no service token")?;
    let secret = secrets::get(secrets::CF_ACCESS_CLIENT_SECRET)?.ok_or("no service token")?;

    let v: Value = reqwest::Client::new()
        .get("https://analytics.myfreepdfedit.com/api/anomalies")
        .header("CF-Access-Client-Id", id)
        .header("CF-Access-Client-Secret", secret)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let Some(anomalies) = v["anomalies"].as_array() else { return Ok(()) };
    let now = chrono::Utc::now().timestamp_millis();
    let log = app.state::<AlertLog>();

    for a in anomalies {
        let site = a["site"].as_str().unwrap_or("?").to_string();
        {
            let mut seen = log.0.lock().unwrap();
            if seen.get(&site).map(|t| now - t < ALERT_COOLDOWN_MS).unwrap_or(false) {
                continue;
            }
            seen.insert(site.clone(), now);
        }

        let kind = if a["kind"] == "surge" { "surging" } else { "dropping" };
        let pct = a["pct"].as_i64().unwrap_or(0);
        let line = format!(
            "Sir, {site} is {kind} — {pct:+}% against its hourly norm.",
        );

        let _ = app
            .notification()
            .builder()
            .title("JARVIS — traffic anomaly")
            .body(&line)
            .show();
        let _ = app.emit("anomaly-alert", json!({ "text": line }));
    }
    Ok(())
}
