//! The Work stage — Gmail, Slack, and the work calendar. READ-ONLY by design:
//! nothing here can send, modify, or delete anything on the work accounts.
//! Gmail + work calendar ride the second Google account (work_google tokens);
//! Slack uses a user (xoxp) token with read scopes only.
//!
//! Action items: extraction is model-driven — the work_email/work_slack tools
//! return candidate messages (with deep links / permalinks), the model picks
//! the real action items and files them through work_todos {action:"suggest"},
//! where they stay "suggested" until sir confirms. origin_key dedupes across
//! days and re-scans.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::calendar;
use crate::google_auth;
use crate::secrets;
use crate::tools::ist_now;

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const SLACK: &str = "https://slack.com/api";

async fn work_token() -> Result<String, String> {
    google_auth::access_token(secrets::WORK_GOOGLE_OAUTH_TOKEN)
        .await
        .map_err(|_| "Work Google account not connected — Settings → Connect Work Google, sir.".into())
}

fn slack_token() -> Result<String, String> {
    secrets::get(secrets::SLACK_TOKEN)?
        .ok_or_else(|| "Slack token not configured — Settings, sir.".to_string())
}

fn urlenc(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.~".contains(c) {
                c.to_string()
            } else {
                c.to_string()
                    .bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect()
            }
        })
        .collect()
}

// ==========================================================================
// Gmail (gmail.readonly)
// ==========================================================================

async fn gmail_get(token: &str, path: &str) -> Result<Value, String> {
    let res = reqwest::Client::new()
        .get(format!("{GMAIL}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Gmail unreachable: {e}"))?;
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    if v["error"].is_object() {
        return Err(format!("Gmail API: {}", v["error"]["message"]));
    }
    Ok(v)
}

fn header<'a>(msg: &'a Value, name: &str) -> &'a str {
    msg["payload"]["headers"]
        .as_array()
        .and_then(|hs| {
            hs.iter()
                .find(|h| h["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))
        })
        .and_then(|h| h["value"].as_str())
        .unwrap_or("")
}

/// List messages for a query and hydrate from/subject/snippet + deep link.
async fn gmail_messages(token: &str, q: &str, max: u32) -> Result<Vec<Value>, String> {
    let list = gmail_get(
        token,
        &format!("/messages?q={}&maxResults={max}", urlenc(q)),
    )
    .await?;
    let ids: Vec<(String, String)> = list["messages"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| {
            Some((
                m["id"].as_str()?.to_string(),
                m["threadId"].as_str().unwrap_or_default().to_string(),
            ))
        })
        .collect();

    let mut out = Vec::new();
    for (id, thread) in ids {
        let msg = gmail_get(
            token,
            &format!("/messages/{id}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date"),
        )
        .await?;
        out.push(json!({
            "id": id,
            "thread_id": thread,
            "from": header(&msg, "From"),
            "subject": header(&msg, "Subject"),
            "date": header(&msg, "Date"),
            "snippet": msg["snippet"],
            "unread": msg["labelIds"].as_array().map(|l| l.iter().any(|x| x == "UNREAD")).unwrap_or(false),
            "link": format!("https://mail.google.com/mail/u/0/#all/{thread}"),
        }));
    }
    Ok(out)
}

async fn gmail_unread_count(token: &str) -> Result<i64, String> {
    let v = gmail_get(token, "/messages?q=is%3Aunread%20category%3Aprimary&maxResults=1").await?;
    Ok(v["resultSizeEstimate"].as_i64().unwrap_or(0))
}

pub async fn email_tool(input: &Value) -> Result<Value, String> {
    let token = work_token().await?;
    match input["action"].as_str().unwrap_or("") {
        "unread_count" => Ok(json!({ "unread_primary": gmail_unread_count(&token).await? })),
        "today" => {
            let msgs = gmail_messages(&token, "category:primary newer_than:1d", 12).await?;
            Ok(json!({ "messages": msgs, "note": "Deep links included — say 'link on screen', never read URLs aloud." }))
        }
        "search" => {
            let q = input["query"].as_str().filter(|s| !s.is_empty()).ok_or("query required")?;
            let msgs = gmail_messages(&token, q, 10).await?;
            Ok(json!({ "messages": msgs }))
        }
        "extract_action_items" => {
            let msgs = gmail_messages(&token, "category:primary is:unread newer_than:2d", 15).await?;
            Ok(json!({
                "candidates": msgs,
                "instruction": "Pick the genuine action items addressed to sir and file each with work_todos {action:'suggest'} (origin_key = 'email:'+thread_id, link = the deep link). Skip newsletters and FYIs."
            }))
        }
        other => Err(format!("Unknown work_email action: {other}")),
    }
}

// ==========================================================================
// Slack (user token, read scopes: search/history/users)
// ==========================================================================

async fn slack_call(token: &str, method: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let res = reqwest::Client::new()
        .get(format!("{SLACK}/{method}"))
        .bearer_auth(token)
        .query(params)
        .send()
        .await
        .map_err(|e| format!("Slack unreachable: {e}"))?;
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    if v["ok"] != true {
        return Err(format!("Slack API {method}: {}", v["error"].as_str().unwrap_or("unknown error")));
    }
    Ok(v)
}

fn slack_match_json(m: &Value) -> Value {
    json!({
        "text": m["text"].as_str().map(|t| t.chars().take(280).collect::<String>()),
        "from": m["username"],
        "channel": m["channel"]["name"],
        "ts": m["ts"],
        "permalink": m["permalink"],
    })
}

/// Mentions of sir in the last day (search.messages with the user id).
async fn slack_mentions(token: &str) -> Result<Vec<Value>, String> {
    let auth = slack_call(token, "auth.test", &[]).await?;
    let uid = auth["user_id"].as_str().unwrap_or_default().to_string();
    let res = slack_call(
        token,
        "search.messages",
        &[("query", format!("<@{uid}>").as_str()), ("sort", "timestamp"), ("sort_dir", "desc"), ("count", "15")],
    )
    .await?;
    let day_ago = (chrono::Utc::now().timestamp() - 86_400) as f64;
    Ok(res["messages"]["matches"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter(|m| m["ts"].as_str().and_then(|t| t.parse::<f64>().ok()).map(|t| t > day_ago).unwrap_or(true))
        .map(slack_match_json)
        .collect())
}

/// Unread DMs/group DMs — conversations.list + per-channel unread counts.
async fn slack_unread_dms(token: &str) -> Result<Vec<Value>, String> {
    let list = slack_call(
        token,
        "conversations.list",
        &[("types", "im,mpim"), ("exclude_archived", "true"), ("limit", "30")],
    )
    .await?;
    let mut out = Vec::new();
    for ch in list["channels"].as_array().unwrap_or(&vec![]).iter().take(20) {
        let id = ch["id"].as_str().unwrap_or_default();
        let Ok(info) = slack_call(token, "conversations.info", &[("channel", id)]).await else { continue };
        let unread = info["channel"]["unread_count_display"].as_i64().unwrap_or(0);
        if unread == 0 {
            continue;
        }
        let name = if let Some(user) = ch["user"].as_str() {
            slack_call(token, "users.info", &[("user", user)])
                .await
                .ok()
                .and_then(|u| u["user"]["real_name"].as_str().map(String::from))
                .unwrap_or_else(|| user.to_string())
        } else {
            info["channel"]["name"].as_str().unwrap_or("group").to_string()
        };
        out.push(json!({ "with": name, "unread": unread, "channel_id": id }));
    }
    Ok(out)
}

pub async fn slack_tool(input: &Value) -> Result<Value, String> {
    let token = slack_token()?;
    match input["action"].as_str().unwrap_or("") {
        "unreads" => Ok(json!({ "unread_dms": slack_unread_dms(&token).await? })),
        "mentions" => Ok(json!({ "mentions": slack_mentions(&token).await? })),
        "search" => {
            let q = input["query"].as_str().filter(|s| !s.is_empty()).ok_or("query required")?;
            let res = slack_call(token.as_str(), "search.messages", &[("query", q), ("count", "10")]).await?;
            let matches: Vec<Value> = res["messages"]["matches"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(slack_match_json)
                .collect();
            Ok(json!({ "matches": matches }))
        }
        "extract_action_items" => {
            let mentions = slack_mentions(&token).await?;
            Ok(json!({
                "candidates": mentions,
                "instruction": "Pick the messages that ask sir to do something and file each with work_todos {action:'suggest'} (origin_key = 'slack:'+ts, link = the permalink). Skip pleasantries."
            }))
        }
        other => Err(format!("Unknown work_slack action: {other}")),
    }
}

// ==========================================================================
// Work calendar (calendar.readonly on the work account)
// ==========================================================================

/// Free gaps ≥ 30 min between events, 08:00–20:00 IST today.
fn find_gaps(events: &[Value]) -> Vec<Value> {
    let now = ist_now();
    let day = now.date_naive();
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    let mut busy: Vec<(i64, i64)> = events
        .iter()
        .filter_map(|e| {
            let s = parse(e["start"].as_str()?)?;
            let en = parse(e["end"].as_str()?)?;
            Some((s.timestamp(), en.timestamp()))
        })
        .collect();
    busy.sort();

    let floor = day.and_hms_opt(8, 0, 0).unwrap().and_utc().timestamp() - 19_800;
    let ceil = day.and_hms_opt(20, 0, 0).unwrap().and_utc().timestamp() - 19_800;
    let mut cursor = floor.max(now.timestamp());
    let mut gaps = Vec::new();
    for (s, e) in busy {
        if s - cursor >= 30 * 60 {
            gaps.push((cursor, s));
        }
        cursor = cursor.max(e);
    }
    if ceil - cursor >= 30 * 60 {
        gaps.push((cursor, ceil));
    }
    gaps.into_iter()
        .map(|(s, e)| {
            let f = |t: i64| {
                chrono::DateTime::from_timestamp(t, 0)
                    .unwrap()
                    .with_timezone(&chrono::FixedOffset::east_opt(19_800).unwrap())
                    .format("%H:%M")
                    .to_string()
            };
            json!({ "from": f(s), "to": f(e), "minutes": (e - s) / 60 })
        })
        .collect()
}

pub async fn calendar_tool(input: &Value) -> Result<Value, String> {
    let token = work_token().await?;
    let now = ist_now();
    match input["action"].as_str().unwrap_or("") {
        "today" => {
            let (start, end) = calendar::today_window();
            calendar::list_events(&token, &start, &end, 20).await
        }
        "next" => {
            let in_14d = now + chrono::Duration::days(14);
            calendar::list_events(&token, &now.to_rfc3339(), &in_14d.to_rfc3339(), 1).await
        }
        "week" => {
            let in_7d = now + chrono::Duration::days(7);
            calendar::list_events(&token, &now.to_rfc3339(), &in_7d.to_rfc3339(), 30).await
        }
        "gaps" => {
            let (start, end) = calendar::today_window();
            let v = calendar::list_events(&token, &start, &end, 20).await?;
            let empty = vec![];
            let events = v["events"].as_array().unwrap_or(&empty);
            Ok(json!({ "gaps": find_gaps(events), "events_today": events.len() }))
        }
        other => Err(format!("Unknown work_calendar action: {other}")),
    }
}

// ==========================================================================
// Tauri commands — direct UI data (no model round-trip)
// ==========================================================================

#[tauri::command]
pub async fn work_email_overview() -> Result<Value, String> {
    let token = work_token().await?;
    let (unread, messages) = tokio::join!(
        gmail_unread_count(&token),
        gmail_messages(&token, "category:primary newer_than:1d", 10),
    );
    Ok(json!({ "unread": unread?, "messages": messages? }))
}

#[tauri::command]
pub async fn work_slack_overview() -> Result<Value, String> {
    let token = slack_token()?;
    let (mentions, dms) = tokio::join!(slack_mentions(&token), slack_unread_dms(&token));
    Ok(json!({ "mentions": mentions?, "dms": dms? }))
}

#[tauri::command]
pub async fn work_calendar_today() -> Result<Value, String> {
    let v = calendar_tool(&json!({ "action": "today" })).await?;
    let empty = vec![];
    let events = v["events"].as_array().unwrap_or(&empty).clone();
    Ok(json!({ "events": events, "gaps": find_gaps(&events) }))
}

/// Settings → "Connect Work Google" (gmail.readonly + calendar.readonly).
#[tauri::command]
pub async fn work_google_connect(app: AppHandle) -> Result<String, String> {
    google_auth::connect(&app, secrets::WORK_GOOGLE_OAUTH_TOKEN, google_auth::WORK_SCOPES)
        .await
        .map(|_| "Work account linked (read-only). At your service, sir.".into())
}

/// "Scan for action items" — one model-driven synthesis pass over email +
/// slack; suggestions land in the todos store as 'suggested'.
#[tauri::command]
pub async fn work_scan(app: AppHandle) -> Result<String, String> {
    let prompt = "Scan my work inbox and Slack for action items: call work_email \
        {action:'extract_action_items'} and work_slack {action:'extract_action_items'}, judge which \
        candidates are genuine tasks for me, and file each via work_todos {action:'suggest'}. \
        Then reply with a one-line spoken summary of what you filed.";
    let text = crate::brain::converse(&app, prompt).await?;
    let _ = tauri::Emitter::emit(&app, "todos-changed", ());
    let _ = tauri::Emitter::emit(&app, "jarvis-done", json!({ "text": text }));
    Ok(text)
}
