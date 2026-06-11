//! Local persistence — SQLite in the app data dir.
//! `notes`: the remember/recall store. `kv`: small settings (city, etc.).

use rusqlite::Connection;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub struct Db(pub Mutex<Connection>);

pub fn init(app: &AppHandle) -> Result<Db, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let conn = Connection::open(dir.join("jarvis.db"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notes (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           ts INTEGER NOT NULL,
           note TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS kv (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS todos (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           created_ts INTEGER NOT NULL,
           text TEXT NOT NULL,
           source TEXT NOT NULL DEFAULT 'manual',
           status TEXT NOT NULL DEFAULT 'open',
           origin_key TEXT,
           link TEXT,
           due_ts INTEGER,
           snoozed_until INTEGER,
           done_ts INTEGER
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_todos_origin
           ON todos(origin_key) WHERE origin_key IS NOT NULL;",
    )?;
    Ok(Db(Mutex::new(conn)))
}

pub fn kv_get(db: &Db, key: &str) -> Option<String> {
    let conn = db.0.lock().unwrap();
    conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

pub fn kv_set(db: &Db, key: &str, value: &str) -> Result<(), String> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

// ---------- Tauri commands (settings the UI owns) ----------

#[tauri::command]
pub fn setting_get(db: State<'_, Db>, key: String) -> Option<String> {
    kv_get(&db, &key)
}

#[tauri::command]
pub fn setting_set(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    kv_set(&db, &key, &value)
}
