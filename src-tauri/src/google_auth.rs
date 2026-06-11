//! Google OAuth 2.0 desktop (loopback) — shared by the personal calendar and
//! the work account (Gmail + work calendar, both read-only). One OAuth client
//! (Settings), two independent token entries in the Credential Manager, so
//! sir can be signed into two Google accounts at once.

use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::secrets;

const REDIRECT_PORT: u16 = 17821;
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub const PERSONAL_SCOPES: &str = "https://www.googleapis.com/auth/calendar";
pub const WORK_SCOPES: &str =
    "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/calendar.readonly";

fn client_pair() -> Result<(String, String), String> {
    let id = secrets::get(secrets::GOOGLE_CLIENT_ID)?
        .ok_or("Google OAuth client not configured — Settings, sir.")?;
    let secret = secrets::get(secrets::GOOGLE_CLIENT_SECRET)?
        .ok_or("Google OAuth client secret not configured — Settings, sir.")?;
    Ok((id, secret))
}

fn save_tokens(token_key: &str, access: &str, refresh: Option<&str>, expires_in: i64) -> Result<(), String> {
    // Keep the old refresh token when Google doesn't resend it.
    let prior_refresh = load_tokens(token_key)
        .ok()
        .and_then(|t| t["refresh_token"].as_str().map(String::from));
    let tokens = json!({
        "access_token": access,
        "refresh_token": refresh.map(String::from).or(prior_refresh),
        "expires_at": chrono::Utc::now().timestamp() + expires_in - 60,
    });
    secrets::set(token_key, &tokens.to_string())
}

fn load_tokens(token_key: &str) -> Result<Value, String> {
    let raw = secrets::get(token_key)?
        .ok_or("Google account not connected — Settings, sir.")?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

pub async fn access_token(token_key: &str) -> Result<String, String> {
    let tokens = load_tokens(token_key)?;
    let now = chrono::Utc::now().timestamp();
    if tokens["expires_at"].as_i64().unwrap_or(0) > now {
        return Ok(tokens["access_token"].as_str().unwrap_or_default().to_string());
    }

    let refresh = tokens["refresh_token"]
        .as_str()
        .ok_or("Google session expired (no refresh token) — reconnect in Settings.")?;
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
    save_tokens(token_key, access, None, res["expires_in"].as_i64().unwrap_or(3500))?;
    Ok(access.to_string())
}

/// Full consent flow: open browser, catch loopback redirect, exchange code.
pub async fn connect(app: &AppHandle, token_key: &str, scopes: &str) -> Result<String, String> {
    let (id, secret) = client_pair()?;
    let redirect = format!("http://127.0.0.1:{REDIRECT_PORT}");

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| format!("Loopback port {REDIRECT_PORT} unavailable: {e}"))?;

    let scope_enc: String = scopes
        .chars()
        .map(|c| match c {
            ':' => "%3A".into(),
            '/' => "%2F".into(),
            ' ' => "%20".into(),
            c => c.to_string(),
        })
        .collect();
    // select_account so sir can pick the work account on the second connect.
    let auth_url = format!(
        "{AUTH_URL}?client_id={id}&redirect_uri={redirect}&response_type=code&scope={scope_enc}\
         &access_type=offline&prompt=consent%20select_account"
    );
    app.opener().open_url(&auth_url, None::<&str>).map_err(|e| e.to_string())?;

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
         <h2>JARVIS — ACCOUNT LINKED</h2>You may close this window, sir.</body></html>"
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
    save_tokens(token_key, access, res["refresh_token"].as_str(), res["expires_in"].as_i64().unwrap_or(3500))?;
    Ok("Account linked. At your service, sir.".into())
}
