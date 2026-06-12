//! Secrets live in the Windows Credential Manager via the `keyring` crate —
//! never in files, never in the repo. Service name "JARVIS", one entry per key.

use keyring::Entry;

const SERVICE: &str = "JARVIS";

/// Keys we manage. Kept as an allowlist so arbitrary strings can't be probed.
pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";
pub const CF_ACCESS_CLIENT_ID: &str = "cf_access_client_id";
pub const CF_ACCESS_CLIENT_SECRET: &str = "cf_access_client_secret";
pub const GOOGLE_OAUTH_TOKEN: &str = "google_oauth_token";
pub const GOOGLE_CLIENT_ID: &str = "google_client_id";
pub const GOOGLE_CLIENT_SECRET: &str = "google_client_secret";
pub const WORK_GOOGLE_OAUTH_TOKEN: &str = "work_google_oauth_token";
pub const SLACK_TOKEN: &str = "slack_token";
/// Cloudflare API token — used only to confirm Pages deploy status after an
/// agent push (Settings → Agents). Read-scoped is sufficient.
pub const CF_API_TOKEN: &str = "cloudflare_api_token";

const ALLOWED: &[&str] = &[
    ANTHROPIC_API_KEY,
    CF_ACCESS_CLIENT_ID,
    CF_ACCESS_CLIENT_SECRET,
    GOOGLE_OAUTH_TOKEN,
    GOOGLE_CLIENT_ID,
    GOOGLE_CLIENT_SECRET,
    WORK_GOOGLE_OAUTH_TOKEN,
    SLACK_TOKEN,
    CF_API_TOKEN,
];

fn entry(key: &str) -> Result<Entry, String> {
    if !ALLOWED.contains(&key) {
        return Err(format!("Unknown secret key: {key}"));
    }
    Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

pub fn get(key: &str) -> Result<Option<String>, String> {
    match entry(key)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return delete(key);
    }
    entry(key)?
        .set_password(value.trim())
        .map_err(|e| e.to_string())
}

pub fn delete(key: &str) -> Result<(), String> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------- Tauri commands ----------

#[tauri::command]
pub fn secret_set(key: String, value: String) -> Result<(), String> {
    set(&key, &value)
}

#[tauri::command]
pub fn secret_exists(key: String) -> Result<bool, String> {
    Ok(get(&key)?.is_some())
}
