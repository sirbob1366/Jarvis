//! HUD plumbing — the authenticated bridge between the webview and the
//! analytics Worker. The UI never holds the Cloudflare Access service token;
//! every call routes through here, where the token pair is read from the
//! Credential Manager and attached as headers. Paths are allowlisted to /api/.

use serde_json::Value;

use crate::secrets;

pub const WORKER_BASE: &str = "https://analytics.myfreepdfedit.com";

fn access_headers() -> Result<(String, String), String> {
    let id = secrets::get(secrets::CF_ACCESS_CLIENT_ID)?
        .ok_or("Cloudflare Access service token not configured (Settings).")?;
    let secret = secrets::get(secrets::CF_ACCESS_CLIENT_SECRET)?
        .ok_or("Cloudflare Access service token not configured (Settings).")?;
    Ok((id, secret))
}

fn check_path(path: &str) -> Result<(), String> {
    if !path.starts_with("/api/") || path.contains("..") {
        return Err(format!("Refused non-API path: {path}"));
    }
    Ok(())
}

pub async fn worker_get(path: &str) -> Result<Value, String> {
    check_path(path)?;
    let (id, secret) = access_headers()?;
    let res = reqwest::Client::new()
        .get(format!("{WORKER_BASE}{path}"))
        .header("CF-Access-Client-Id", id)
        .header("CF-Access-Client-Secret", secret)
        .send()
        .await
        .map_err(|e| format!("Worker unreachable: {e}"))?;

    let status = res.status();
    if !status.is_success() {
        return Err(format!("Worker API {status} for {path}"));
    }
    res.json().await.map_err(|e| format!("Bad JSON from Worker: {e}"))
}

/// POST/DELETE for the revenue ledger (the Worker already gates these behind
/// the same Access service token).
pub async fn worker_send(method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
    check_path(path)?;
    let (id, secret) = access_headers()?;
    let client = reqwest::Client::new();
    let mut req = match method {
        "POST" => client.post(format!("{WORKER_BASE}{path}")),
        "DELETE" => client.delete(format!("{WORKER_BASE}{path}")),
        other => return Err(format!("Method {other} not allowed")),
    };
    req = req
        .header("CF-Access-Client-Id", id)
        .header("CF-Access-Client-Secret", secret);
    if let Some(b) = body {
        req = req.json(b);
    }
    let res = req.send().await.map_err(|e| format!("Worker unreachable: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(format!("Worker API {status}: {text}"));
    }
    res.json().await.map_err(|e| format!("Bad JSON from Worker: {e}"))
}

// ---------- Tauri commands ----------

/// Generic GET for the HUD/board (e.g. "/api/overview", "/api/live").
#[tauri::command]
pub async fn worker_api(path: String) -> Result<Value, String> {
    worker_get(&path).await
}

/// Mutations — revenue entries + fx rate only.
#[tauri::command]
pub async fn worker_mutate(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    if !path.starts_with("/api/revenue") {
        return Err("Only the revenue ledger accepts writes.".into());
    }
    worker_send(&method, &path, body.as_ref()).await
}
