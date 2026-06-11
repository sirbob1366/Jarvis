//! The Brain — Anthropic Messages API (raw HTTP; no official Rust SDK exists).
//! Streams responses over SSE and emits Tauri events the UI renders live:
//!   jarvis-delta { text }   incremental assistant text
//!   jarvis-done  { text }   full reply (also the TTS payload in Stage 2)
//!   jarvis-error { error }
//!
//! Session memory is a rolling window held in app state; the persistent
//! notes store arrives with the tools in Stage 3.

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::secrets;

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MAX_TOKENS: u32 = 1024;
/// Rolling window: keep the last N turns (user+assistant pairs count as 2).
const HISTORY_WINDOW: usize = 24;

const SYSTEM_PROMPT: &str = "You are J.A.R.V.I.S., the user's private desktop assistant — the same intelligence \
that runs his portfolio analytics. Address him as \"sir\". Voice: crisp, capable butler-AI; concise by default; \
dry wit welcome; never robotic filler, never bullet-point spam in conversation. Your replies are usually spoken \
aloud, so keep them short and speakable — one to three sentences unless sir asks for detail. Use your tools when \
they answer the question; otherwise answer from general knowledge. If a tool fails, say so plainly and move on.";

#[derive(Default)]
pub struct Session {
    /// Anthropic-shaped message objects: {role, content}.
    pub messages: Mutex<Vec<Value>>,
}

#[derive(Clone, Serialize)]
struct DeltaPayload<'a> {
    text: &'a str,
}

#[derive(Clone, Serialize)]
struct ErrorPayload {
    error: String,
}

fn trim_history(messages: &mut Vec<Value>) {
    if messages.len() > HISTORY_WINDOW {
        let cut = messages.len() - HISTORY_WINDOW;
        messages.drain(..cut);
        // History must start with a user turn.
        while messages
            .first()
            .map(|m| m["role"] != "user")
            .unwrap_or(false)
        {
            messages.remove(0);
        }
    }
}

/// Stream one assistant turn for the accumulated history.
/// Returns the full assistant text. Tool-use support arrives in Stage 3.
async fn stream_turn(app: &AppHandle, api_key: &str, messages: &[Value]) -> Result<String, String> {
    let body = json!({
        "model": DEFAULT_MODEL,
        "max_tokens": MAX_TOKENS,
        "system": SYSTEM_PROMPT,
        "stream": true,
        "messages": messages,
    });

    let res = reqwest::Client::new()
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        return Err(format!("API {status}: {msg}"));
    }

    let mut full = String::new();
    let mut buffer = String::new();
    let mut stream = res.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // SSE events are separated by a blank line.
        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            for line in event_block.lines() {
                let Some(data) = line.strip_prefix("data: ") else { continue };
                let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
                match value["type"].as_str() {
                    Some("content_block_delta") => {
                        if let Some(t) = value["delta"]["text"].as_str() {
                            full.push_str(t);
                            let _ = app.emit("jarvis-delta", DeltaPayload { text: t });
                        }
                    }
                    Some("error") => {
                        let msg = value["error"]["message"]
                            .as_str()
                            .unwrap_or("unknown stream error")
                            .to_string();
                        return Err(format!("API stream error: {msg}"));
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(full)
}

#[tauri::command]
pub async fn ask_jarvis(
    app: AppHandle,
    session: State<'_, Session>,
    message: String,
) -> Result<(), String> {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Ok(());
    }

    let Some(api_key) = secrets::get(secrets::ANTHROPIC_API_KEY)? else {
        let err = "No Anthropic API key configured, sir. Open Settings (gear icon) and add one.";
        let _ = app.emit("jarvis-error", ErrorPayload { error: err.into() });
        return Ok(());
    };

    let history = {
        let mut messages = session.messages.lock().unwrap();
        messages.push(json!({ "role": "user", "content": message }));
        trim_history(&mut messages);
        messages.clone()
    };

    match stream_turn(&app, &api_key, &history).await {
        Ok(full) => {
            session
                .messages
                .lock()
                .unwrap()
                .push(json!({ "role": "assistant", "content": full }));
            let _ = app.emit("jarvis-done", DeltaPayload { text: &full });
        }
        Err(err) => {
            // Drop the failed user turn so a retry doesn't double it.
            let mut messages = session.messages.lock().unwrap();
            if messages.last().map(|m| m["role"] == "user").unwrap_or(false) {
                messages.pop();
            }
            let _ = app.emit("jarvis-error", ErrorPayload { error: err });
        }
    }
    Ok(())
}

#[tauri::command]
pub fn clear_session(session: State<'_, Session>) -> Result<(), String> {
    session.messages.lock().unwrap().clear();
    Ok(())
}
