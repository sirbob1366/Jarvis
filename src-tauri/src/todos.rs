//! Unified to-do store — the work_todos synthesizer's backing table plus the
//! manual list on the Command board. Items extracted from email/slack/calendar
//! arrive as status='suggested' (with an origin_key so re-extraction dedupes);
//! confirming promotes them to 'open'. Manual adds are 'open' immediately.

use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::db::Db;

fn row_to_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "created_ts": r.get::<_, i64>(1)?,
        "text": r.get::<_, String>(2)?,
        "source": r.get::<_, String>(3)?,
        "status": r.get::<_, String>(4)?,
        "link": r.get::<_, Option<String>>(5)?,
        "due_ts": r.get::<_, Option<i64>>(6)?,
        "snoozed_until": r.get::<_, Option<i64>>(7)?,
    }))
}

const COLS: &str = "id, created_ts, text, source, status, link, due_ts, snoozed_until";

/// Open + suggested items, confirmed first, un-snoozed only.
pub fn list_active(db: &Db) -> Result<Vec<Value>, String> {
    let conn = db.0.lock().unwrap();
    let now = Utc::now().timestamp_millis();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM todos
             WHERE status IN ('open','suggested','snoozed')
               AND (snoozed_until IS NULL OR snoozed_until <= ?1)
             ORDER BY CASE status WHEN 'open' THEN 0 WHEN 'snoozed' THEN 0 ELSE 1 END,
                      due_ts IS NULL, due_ts, created_ts"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![now], |r| row_to_json(r))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn add(db: &Db, text: &str, source: &str, origin_key: Option<&str>, link: Option<&str>, suggested: bool) -> Result<i64, String> {
    let conn = db.0.lock().unwrap();
    let status = if suggested { "suggested" } else { "open" };
    // Dedupe synthesized items by origin_key across days.
    let res = conn.execute(
        "INSERT INTO todos (created_ts, text, source, status, origin_key, link)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(origin_key) DO NOTHING",
        params![Utc::now().timestamp_millis(), text.trim(), source, status, origin_key, link],
    );
    match res {
        Ok(0) => Ok(0), // duplicate suggestion — already known
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------- Tauri commands ----------

#[tauri::command]
pub fn todo_list(db: State<'_, Db>) -> Result<Value, String> {
    let items = list_active(&db)?;
    let open = items.iter().filter(|t| t["status"] != "suggested").count();
    Ok(json!({ "items": items, "open_count": open }))
}

#[tauri::command]
pub fn todo_add(db: State<'_, Db>, text: String) -> Result<Value, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("empty to-do".into());
    }
    let id = add(&db, &text, "manual", None, None, false)?;
    Ok(json!({ "ok": true, "id": id }))
}

#[tauri::command]
pub fn todo_complete(db: State<'_, Db>, id: i64) -> Result<Value, String> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE todos SET status = 'done', done_ts = ?1 WHERE id = ?2",
        params![Utc::now().timestamp_millis(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn todo_confirm(db: State<'_, Db>, id: i64) -> Result<Value, String> {
    let conn = db.0.lock().unwrap();
    conn.execute("UPDATE todos SET status = 'open' WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn todo_snooze(db: State<'_, Db>, id: i64, until_ts: i64) -> Result<Value, String> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE todos SET status = 'snoozed', snoozed_until = ?1 WHERE id = ?2",
        params![until_ts, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn todo_dismiss(db: State<'_, Db>, id: i64) -> Result<Value, String> {
    let conn = db.0.lock().unwrap();
    conn.execute("UPDATE todos SET status = 'dismissed' WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

/// The work_todos model tool (voice management: add/complete/snooze/list).
pub async fn run_tool(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let db = app.state::<Db>();
    match input["action"].as_str().unwrap_or("list") {
        "list" => {
            let items = list_active(&db)?;
            Ok(json!({ "todos": items, "count": items.len() }))
        }
        "add" => {
            let text = input["text"].as_str().filter(|t| !t.trim().is_empty()).ok_or("text required")?;
            let id = add(&db, text, "manual", None, None, false)?;
            Ok(json!({ "ok": true, "id": id }))
        }
        "complete" | "confirm" | "snooze" | "dismiss" => {
            // Resolve by id or by fuzzy text match.
            let id = match input["id"].as_i64() {
                Some(id) => id,
                None => {
                    let text = input["text"].as_str().ok_or("id or text required")?;
                    let conn = db.0.lock().unwrap();
                    conn.query_row(
                        "SELECT id FROM todos WHERE status IN ('open','suggested','snoozed') AND text LIKE ?1
                         ORDER BY created_ts DESC LIMIT 1",
                        params![format!("%{}%", text.trim())],
                        |r| r.get(0),
                    )
                    .map_err(|_| format!("No matching to-do for '{text}'"))?
                }
            };
            match input["action"].as_str().unwrap() {
                "complete" => {
                    let conn = db.0.lock().unwrap();
                    conn.execute(
                        "UPDATE todos SET status='done', done_ts=?1 WHERE id=?2",
                        params![Utc::now().timestamp_millis(), id],
                    )
                    .map_err(|e| e.to_string())?;
                }
                "confirm" => {
                    let conn = db.0.lock().unwrap();
                    conn.execute("UPDATE todos SET status='open' WHERE id=?1", params![id])
                        .map_err(|e| e.to_string())?;
                }
                "dismiss" => {
                    let conn = db.0.lock().unwrap();
                    conn.execute("UPDATE todos SET status='dismissed' WHERE id=?1", params![id])
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    let hours = input["snooze_hours"].as_i64().unwrap_or(24).clamp(1, 24 * 14);
                    let until = Utc::now().timestamp_millis() + hours * 3_600_000;
                    let conn = db.0.lock().unwrap();
                    conn.execute(
                        "UPDATE todos SET status='snoozed', snoozed_until=?1 WHERE id=?2",
                        params![until, id],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            let _ = tauri::Emitter::emit(app, "todos-changed", ());
            Ok(json!({ "ok": true, "id": id }))
        }
        other => Err(format!("Unknown todos action: {other}")),
    }
}
