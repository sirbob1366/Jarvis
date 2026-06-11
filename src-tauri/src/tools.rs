//! JARVIS's tools — definitions (Anthropic tool schemas) + execution.
//! Every tool runs locally in Rust; the model only ever supplies arguments.

use chrono::{FixedOffset, Utc};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::db::{self, Db};
use crate::hud::worker_get;

const SITES: [&str; 5] = ["pdfedit", "imagetool", "audiotool", "videotool", "invoicetool"];

pub fn ist_now() -> chrono::DateTime<FixedOffset> {
    Utc::now().with_timezone(&FixedOffset::east_opt(5 * 3600 + 1800).unwrap())
}

/// Tool schemas sent with every Anthropic request.
pub fn definitions() -> Value {
    json!([
      {
        "name": "portfolio_stats",
        "description": "Live and historical analytics for sir's five web properties (pdfedit=myfreepdfedit.com, imagetool, audiotool, videotool, invoicetool). Use for any question about site traffic, visitors, pages, or what's happening right now.",
        "input_schema": {
          "type": "object",
          "properties": {
            "query": { "type": "string", "enum": ["today", "week", "top_pages", "live"],
                       "description": "today = today-so-far summary (IST); week = last-7-day summary; top_pages = most visited pages (7d); live = per-site traffic in the last 30 minutes" },
            "site": { "type": "string", "enum": SITES,
                      "description": "Optional. Omit for the whole portfolio." }
          },
          "required": ["query"]
        }
      },
      {
        "name": "weather",
        "description": "Current conditions or forecast for sir's configured city (Settings; default Pune, IN).",
        "input_schema": {
          "type": "object",
          "properties": {
            "when": { "type": "string", "enum": ["now", "today", "tomorrow"] }
          },
          "required": ["when"]
        }
      },
      {
        "name": "set_timer",
        "description": "Set a local timer or reminder. Fires a Windows notification and a spoken alert.",
        "input_schema": {
          "type": "object",
          "properties": {
            "seconds": { "type": "integer", "minimum": 5, "maximum": 86400, "description": "Duration from now." },
            "label": { "type": "string", "description": "What to announce when it fires." }
          },
          "required": ["seconds", "label"]
        }
      },
      {
        "name": "list_timers",
        "description": "List timers/reminders currently running.",
        "input_schema": { "type": "object", "properties": {} }
      },
      {
        "name": "system",
        "description": "Local system actions: open a URL or app in the default handler, or report the current date/time.",
        "input_schema": {
          "type": "object",
          "properties": {
            "action": { "type": "string", "enum": ["open_url", "current_time"] },
            "url": { "type": "string", "description": "Required for open_url. https:// URL or app path." }
          },
          "required": ["action"]
        }
      },
      {
        "name": "calendar",
        "description": "Sir's Google Calendar: read today's events, the next event, this week, or create an event.",
        "input_schema": {
          "type": "object",
          "properties": {
            "action": { "type": "string", "enum": ["today", "next", "week", "create"] },
            "title": { "type": "string", "description": "For create: the event title." },
            "start_iso": { "type": "string", "description": "For create: RFC3339 start with IST offset, e.g. 2026-06-13T15:00:00+05:30." },
            "duration_minutes": { "type": "integer", "minimum": 1, "maximum": 1440, "description": "For create. Default 30." }
          },
          "required": ["action"]
        }
      },
      {
        "name": "work_todos",
        "description": "Sir's unified to-do list. list = current items (confirmed first, then suggested); add a task; complete/confirm/snooze/dismiss an existing one by id or fuzzy text.",
        "input_schema": {
          "type": "object",
          "properties": {
            "action": { "type": "string", "enum": ["list", "add", "complete", "confirm", "snooze", "dismiss"] },
            "text": { "type": "string", "description": "Task text (add), or fuzzy match (complete/confirm/snooze/dismiss)." },
            "id": { "type": "integer", "description": "Exact todo id, if known." },
            "snooze_hours": { "type": "integer", "minimum": 1, "maximum": 336, "description": "For snooze. Default 24." }
          },
          "required": ["action"]
        }
      },
      {
        "name": "remember",
        "description": "Save a note to sir's persistent local notes store (survives restarts).",
        "input_schema": {
          "type": "object",
          "properties": { "note": { "type": "string" } },
          "required": ["note"]
        }
      },
      {
        "name": "recall",
        "description": "Search the persistent notes store. Empty query returns the most recent notes.",
        "input_schema": {
          "type": "object",
          "properties": { "query": { "type": "string" } }
        }
      }
    ])
}

/// Execute one tool call. Errors become strings the model can relay gracefully.
pub async fn run(app: &AppHandle, name: &str, input: &Value) -> Result<Value, String> {
    match name {
        "portfolio_stats" => portfolio_stats(input).await,
        "weather" => weather(app, input).await,
        "set_timer" => set_timer(app, input),
        "list_timers" => list_timers(app),
        "system" => system(app, input),
        "calendar" => crate::calendar::run_tool(input).await,
        "work_todos" => crate::todos::run_tool(app, input).await,
        "remember" => remember(app, input),
        "recall" => recall(app, input),
        other => Err(format!("Unknown tool: {other}")),
    }
}

// ---------- portfolio_stats ----------

async fn portfolio_stats(input: &Value) -> Result<Value, String> {
    let site = input["site"].as_str().filter(|s| SITES.contains(s));
    let site_q = site.map(|s| format!("&site={s}")).unwrap_or_default();
    let today = ist_now().format("%Y-%m-%d");

    match input["query"].as_str().unwrap_or("today") {
        "today" => worker_get(&format!("/api/summary?from={today}&to={today}{site_q}")).await,
        "week" => worker_get(&format!("/api/summary?{}", site_q.trim_start_matches('&'))).await,
        "top_pages" => worker_get(&format!("/api/breakdown?dim=page&limit=5{site_q}")).await,
        "live" => worker_get("/api/live").await.map(|mut v| {
            // The event list is noise for a spoken answer; counters suffice.
            if let Some(obj) = v.as_object_mut() {
                obj.remove("events");
            }
            v
        }),
        other => Err(format!("Unknown query: {other}")),
    }
}

// ---------- weather (Open-Meteo, no key) ----------

const WMO: &[(u8, &str)] = &[
    (0, "clear"), (1, "mostly clear"), (2, "partly cloudy"), (3, "overcast"),
    (45, "fog"), (48, "rime fog"), (51, "light drizzle"), (53, "drizzle"), (55, "heavy drizzle"),
    (61, "light rain"), (63, "rain"), (65, "heavy rain"), (66, "freezing rain"), (67, "freezing rain"),
    (71, "light snow"), (73, "snow"), (75, "heavy snow"), (80, "rain showers"), (81, "rain showers"),
    (82, "violent rain showers"), (95, "thunderstorm"), (96, "thunderstorm with hail"), (99, "thunderstorm with hail"),
];

fn wmo(code: u64) -> &'static str {
    WMO.iter()
        .rev()
        .find(|(c, _)| u64::from(*c) <= code)
        .map(|(_, s)| *s)
        .unwrap_or("unknown")
}

async fn weather(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let db = app.state::<Db>();
    let city = db::kv_get(&db, "city").unwrap_or_else(|| "Pune".into());

    let geo: Value = reqwest::get(format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1",
        urlencoding(&city)
    ))
    .await
    .map_err(|e| e.to_string())?
    .json()
    .await
    .map_err(|e| e.to_string())?;

    let place = geo["results"][0].clone();
    let (lat, lon) = (
        place["latitude"].as_f64().ok_or(format!("City '{city}' not found"))?,
        place["longitude"].as_f64().ok_or("geocode error")?,
    );

    let fc: Value = reqwest::get(format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m\
         &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code\
         &timezone=auto&forecast_days=2"
    ))
    .await
    .map_err(|e| e.to_string())?
    .json()
    .await
    .map_err(|e| e.to_string())?;

    let day = |i: usize| {
        json!({
            "high_c": fc["daily"]["temperature_2m_max"][i],
            "low_c": fc["daily"]["temperature_2m_min"][i],
            "rain_chance_pct": fc["daily"]["precipitation_probability_max"][i],
            "conditions": wmo(fc["daily"]["weather_code"][i].as_u64().unwrap_or(0)),
        })
    };

    let out = match input["when"].as_str().unwrap_or("now") {
        "tomorrow" => json!({ "city": city, "tomorrow": day(1) }),
        "today" => json!({ "city": city, "today": day(0) }),
        _ => json!({
            "city": city,
            "now": {
                "temp_c": fc["current"]["temperature_2m"],
                "feels_like_c": fc["current"]["apparent_temperature"],
                "humidity_pct": fc["current"]["relative_humidity_2m"],
                "wind_kmh": fc["current"]["wind_speed_10m"],
                "conditions": wmo(fc["current"]["weather_code"].as_u64().unwrap_or(0)),
            },
            "today": day(0),
        }),
    };
    Ok(out)
}

/// Board greeting strip — compact current conditions, cache-friendly.
#[tauri::command]
pub async fn weather_now(app: AppHandle) -> Result<Value, String> {
    weather(&app, &json!({ "when": "now" })).await
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

// ---------- timers / reminders ----------

pub struct Timers {
    next_id: AtomicU64,
    pub active: Mutex<Vec<(u64, String, i64)>>, // (id, label, fires_at_epoch_ms)
}

impl Default for Timers {
    fn default() -> Self {
        Self { next_id: AtomicU64::new(1), active: Mutex::new(Vec::new()) }
    }
}

fn set_timer(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let seconds = input["seconds"].as_u64().filter(|s| (5..=86_400).contains(s))
        .ok_or("seconds must be 5..86400")?;
    let label = input["label"].as_str().unwrap_or("Timer").to_string();

    let timers = app.state::<Timers>();
    let id = timers.next_id.fetch_add(1, Ordering::Relaxed);
    let fires_at = Utc::now().timestamp_millis() + (seconds as i64) * 1000;
    timers.active.lock().unwrap().push((id, label.clone(), fires_at));

    let app2 = app.clone();
    let label2 = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        let timers = app2.state::<Timers>();
        let mut active = timers.active.lock().unwrap();
        let Some(pos) = active.iter().position(|(tid, ..)| *tid == id) else { return }; // cancelled
        active.remove(pos);
        drop(active);

        let _ = app2
            .notification()
            .builder()
            .title("JARVIS")
            .body(&label2)
            .show();
        // The UI speaks this (if unmuted) and prints it.
        let _ = app2.emit("timer-fired", json!({ "label": label2 }));
    });

    Ok(json!({ "ok": true, "id": id, "fires_in_seconds": seconds, "label": label }))
}

fn list_timers(app: &AppHandle) -> Result<Value, String> {
    let timers = app.state::<Timers>();
    let now = Utc::now().timestamp_millis();
    let list: Vec<Value> = timers
        .active
        .lock()
        .unwrap()
        .iter()
        .map(|(id, label, at)| json!({ "id": id, "label": label, "seconds_left": ((at - now) / 1000).max(0) }))
        .collect();
    Ok(json!({ "timers": list }))
}

// ---------- system ----------

fn system(app: &AppHandle, input: &Value) -> Result<Value, String> {
    match input["action"].as_str().unwrap_or("") {
        "current_time" => {
            let now = ist_now();
            Ok(json!({
                "local_time_ist": now.format("%A, %d %B %Y, %H:%M").to_string(),
                "timezone": "Asia/Kolkata (IST)"
            }))
        }
        "open_url" => {
            let url = input["url"].as_str().ok_or("url required")?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("Only http(s) URLs are allowed.".into());
            }
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true, "opened": url }))
        }
        other => Err(format!("Unknown action: {other}")),
    }
}

// ---------- remember / recall ----------

fn remember(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let note = input["note"].as_str().filter(|n| !n.trim().is_empty()).ok_or("note required")?;
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO notes (ts, note) VALUES (?1, ?2)",
        rusqlite::params![Utc::now().timestamp_millis(), note.trim()],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

fn recall(app: &AppHandle, input: &Value) -> Result<Value, String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let query = input["query"].as_str().unwrap_or("").trim().to_string();

    let mut stmt;
    let rows: Vec<(i64, String)> = if query.is_empty() {
        stmt = conn
            .prepare("SELECT ts, note FROM notes ORDER BY id DESC LIMIT 10")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    } else {
        stmt = conn
            .prepare("SELECT ts, note FROM notes WHERE note LIKE ?1 ORDER BY id DESC LIMIT 10")
            .map_err(|e| e.to_string())?;
        stmt.query_map([format!("%{query}%")], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };

    let notes: Vec<Value> = rows
        .into_iter()
        .map(|(ts, note)| {
            let when = chrono::DateTime::from_timestamp_millis(ts)
                .map(|d| d.with_timezone(&FixedOffset::east_opt(19800).unwrap()).format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            json!({ "date": when, "note": note })
        })
        .collect();
    Ok(json!({ "notes": notes, "count": notes.len() }))
}
