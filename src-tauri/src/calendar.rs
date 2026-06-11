//! Google Calendar — OAuth 2.0 desktop (loopback) flow + read/create tools.
//! Tokens live in the Windows Credential Manager; nothing touches disk.
//!
//! Connect once from Settings (after creating an OAuth *Desktop app* client in
//! Google Cloud Console — see README): JARVIS opens the consent page in the
//! browser, catches the redirect on 127.0.0.1, exchanges the code, and stores
//! {access, refresh, expiry}. Refreshes silently from then on.

use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::secrets;
use crate::tools::ist_now;

const REDIRECT_PORT: u16 = 17821;
const SCOPE: &str = "https://www.googleapis.com/auth/calendar";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

fn client_pair() -> Result<(String, String), String> {
    let id = secrets::get(secrets::GOOGLE_CLIENT_ID)?
        .ok_or("Google OAuth client not configured — Settings, sir.")?;
    let secret = secrets::get(secrets::GOOGLE_CLIENT_SECRET)?
        .ok_or("Google OAuth client secret not configured — Settings, sir.")?;
    Ok((id, secret))
}

fn save_tokens(access: &str, refresh: Option<&str>, expires_in: i64) -> Result<(), String> {
    // Keep the old refresh token when Google doesn't resend it.
    let prior_refresh = load_tokens().ok().and_then(|t| t["refresh_token"].as_str().map(String::from));
    let tokens = json!({
        "access_token": access,
        "refresh_token": refresh.map(String::from).or(prior_refresh),
        "expires_at": chrono::Utc::now().timestamp() + expires_in - 60,
    });
    secrets::set(secrets::GOOGLE_OAUTH_TOKEN, &tokens.to_string())
}

fn load_tokens() -> Result<Value, String> {
    let raw = secrets::get(secrets::GOOGLE_OAUTH_TOKEN)?
        .ok_or("Calendar not connected — Settings → Connect Calendar, sir.")?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

async fn access_token() -> Result<String, String> {
    let tokens = load_tokens()?;
    let now = chrono::Utc::now().timestamp();
    if tokens["expires_at"].as_i64().unwrap_or(0) > now {
        return Ok(tokens["access_token"].as_str().unwrap_or_default().to_string());
    }

    // Refresh.
    let refresh = tokens["refresh_token"]
        .as_str()
        .ok_or("Calendar session expired (no refresh token) — reconnect in Settings.")?;
    let (id, secret) = client_pair()?;
    let res: Value = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", id.as_str()),
            ("client_secret", secret.as_str()),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let access = res["access_token"]
        .as_str()
        .ok_or_else(|| format!("Token refresh failed: {res}"))?;
    save_tokens(access, None, res["expires_in"].as_i64().unwrap_or(3500))?;
    Ok(access.to_string())
}

/// Settings → "Connect Calendar": full consent flow. Returns a status line.
#[tauri::command]
pub async fn calendar_connect(app: AppHandle) -> Result<String, String> {
    let (id, secret) = client_pair()?;
    let redirect = format!("http://127.0.0.1:{REDIRECT_PORT}");

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| format!("Loopback port {REDIRECT_PORT} unavailable: {e}"))?;

    let auth_url = format!(
        "{AUTH_URL}?client_id={id}&redirect_uri={redirect}&response_type=code&scope={}&access_type=offline&prompt=consent",
        SCOPE.replace(':', "%3A").replace('/', "%2F")
    );
    app.opener().open_url(&auth_url, None::<&str>).map_err(|e| e.to_string())?;

    // Wait (max 3 min) for Google to redirect the browser back to us.
    let (mut stream, _) = tokio::time::timeout(std::time::Duration::from_secs(180), listener.accept())
        .await
        .map_err(|_| "Timed out waiting for the Google consent page.")?
        .map_err(|e| e.to_string())?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let code = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|path| path.split("code=").nth(1))
        .map(|c| c.split('&').next().unwrap_or(c).to_string())
        .filter(|c| !c.is_empty());

    let body = if code.is_some() {
        "<html><body style='background:#05080d;color:#4fd8ff;font-family:monospace;text-align:center;padding-top:20vh'>\
         <h2>JARVIS — CALENDAR LINKED</h2>You may close this window, sir.</body></html>"
    } else {
        "<html><body style='background:#05080d;color:#ff5252;font-family:monospace;text-align:center;padding-top:20vh'>\
         <h2>JARVIS — AUTHORIZATION DECLINED</h2></body></html>"
    };
    let _ = stream
        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}").as_bytes())
        .await;

    let code = code.ok_or("Google returned no authorization code (consent declined?).")?;

    let res: Value = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", id.as_str()),
            ("client_secret", secret.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let access = res["access_token"]
        .as_str()
        .ok_or_else(|| format!("Token exchange failed: {res}"))?;
    save_tokens(access, res["refresh_token"].as_str(), res["expires_in"].as_i64().unwrap_or(3500))?;
    Ok("Calendar linked. At your service, sir.".into())
}

// ---------- the calendar tool ----------

fn simplify(items: &[Value]) -> Vec<Value> {
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

async fn list_events(token: &str, time_min: &str, time_max: &str, max: u32) -> Result<Value, String> {
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

pub async fn run_tool(input: &Value) -> Result<Value, String> {
    let token = access_token().await?;
    let now = ist_now();

    match input["action"].as_str().unwrap_or("") {
        "today" => {
            let start = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
            let end = start + chrono::Duration::days(1);
            list_events(
                &token,
                &format!("{}+05:30", start.format("%Y-%m-%dT%H:%M:%S")),
                &format!("{}+05:30", end.format("%Y-%m-%dT%H:%M:%S")),
                15,
            )
            .await
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
