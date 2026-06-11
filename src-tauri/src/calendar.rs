//! Google Calendar (personal account) — read/create tools on top of the
//! shared google_auth loopback flow. The work calendar lives in work.rs and
//! reuses the same listing helpers with the work token.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::google_auth;
use crate::secrets;
use crate::tools::ist_now;

const API: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

/// Settings → "Connect Calendar": full consent flow. Returns a status line.
#[tauri::command]
pub async fn calendar_connect(app: AppHandle) -> Result<String, String> {
    google_auth::connect(&app, secrets::GOOGLE_OAUTH_TOKEN, google_auth::PERSONAL_SCOPES)
        .await
        .map(|_| "Calendar linked. At your service, sir.".into())
}

/// Board "Today" card — today's events without a model round-trip.
#[tauri::command]
pub async fn calendar_today() -> Result<Value, String> {
    run_tool(&json!({ "action": "today" })).await
}

// ---------- shared event helpers (also used by work.rs) ----------

pub fn simplify(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .map(|e| {
            json!({
                "title": e["summary"],
                "start": e["start"]["dateTime"].as_str().or(e["start"]["date"].as_str()),
                "end": e["end"]["dateTime"].as_str().or(e["end"]["date"].as_str()),
                "location": e["location"],
            })
        })
        .collect()
}

pub async fn list_events(token: &str, time_min: &str, time_max: &str, max: u32) -> Result<Value, String> {
    let res: Value = reqwest::Client::new()
        .get(API)
        .bearer_auth(token)
        .query(&[
            ("timeMin", time_min),
            ("timeMax", time_max),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", &max.to_string()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    if res["error"].is_object() {
        return Err(format!("Calendar API: {}", res["error"]["message"]));
    }
    let empty = vec![];
    Ok(json!({ "events": simplify(res["items"].as_array().unwrap_or(&empty)) }))
}

/// Today's IST window as RFC3339 strings.
pub fn today_window() -> (String, String) {
    let now = ist_now();
    let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(1);
    (
        format!("{}+05:30", start.format("%Y-%m-%dT%H:%M:%S")),
        format!("{}+05:30", end.format("%Y-%m-%dT%H:%M:%S")),
    )
}

// ---------- the personal calendar tool ----------

pub async fn run_tool(input: &Value) -> Result<Value, String> {
    let token = google_auth::access_token(secrets::GOOGLE_OAUTH_TOKEN).await?;
    let now = ist_now();

    match input["action"].as_str().unwrap_or("") {
        "today" => {
            let (start, end) = today_window();
            list_events(&token, &start, &end, 15).await
        }
        "next" => {
            let in_30d = now + chrono::Duration::days(30);
            list_events(&token, &now.to_rfc3339(), &in_30d.to_rfc3339(), 1).await
        }
        "week" => {
            let in_7d = now + chrono::Duration::days(7);
            list_events(&token, &now.to_rfc3339(), &in_7d.to_rfc3339(), 25).await
        }
        "create" => {
            let title = input["title"].as_str().filter(|t| !t.is_empty()).ok_or("title required")?;
            let start = input["start_iso"].as_str().ok_or("start_iso required (RFC3339 with +05:30)")?;
            let start_dt = chrono::DateTime::parse_from_rfc3339(start)
                .map_err(|e| format!("Bad start_iso: {e}"))?;
            let minutes = input["duration_minutes"].as_i64().filter(|m| (1..=1440).contains(m)).unwrap_or(30);
            let end_dt = start_dt + chrono::Duration::minutes(minutes);

            let res: Value = reqwest::Client::new()
                .post(API)
                .bearer_auth(&token)
                .json(&json!({
                    "summary": title,
                    "start": { "dateTime": start_dt.to_rfc3339() },
                    "end": { "dateTime": end_dt.to_rfc3339() },
                }))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;

            if res["error"].is_object() {
                return Err(format!("Calendar API: {}", res["error"]["message"]));
            }
            Ok(json!({ "ok": true, "created": title, "start": start_dt.to_rfc3339(), "link": res["htmlLink"] }))
        }
        other => Err(format!("Unknown calendar action: {other}")),
    }
}
