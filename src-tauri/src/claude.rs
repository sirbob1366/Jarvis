//! The Brain — Anthropic Messages API (raw HTTP; no official Rust SDK exists).
//! Streams responses over SSE, runs the tool-use agentic loop, and emits
//! Tauri events the UI renders live:
//!   jarvis-delta { text }   incremental assistant text
//!   jarvis-tool  { name }   a tool is being consulted
//!   jarvis-done  { text }   full reply (also the TTS payload)
//!   jarvis-error { error }
//!
//! Session memory is a rolling window in app state; the persistent notes
//! store is exposed to the model through the remember/recall tools.

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::secrets;
use crate::tools;

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MAX_TOKENS: u32 = 1024;
/// Rolling window: keep the last N message objects.
const HISTORY_WINDOW: usize = 24;
/// Safety valve on the tool loop.
const MAX_TOOL_ROUNDS: usize = 5;

const SYSTEM_PROMPT: &str = "You are J.A.R.V.I.S., the user's private desktop assistant — the same intelligence \
that runs his portfolio analytics. Address him as \"sir\". Voice: crisp, capable butler-AI; concise by default; \
dry wit welcome; never robotic filler, never bullet-point spam in conversation. Your replies are usually spoken \
aloud, so keep them short and speakable — one to three sentences unless sir asks for detail. Use your tools when \
they answer the question (traffic, weather, timers, notes, time); otherwise answer from general knowledge. When a \
tool returns numbers, use the actual numbers. If a tool fails, say so plainly and move on.";

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
struct ToolPayload<'a> {
    name: &'a str,
}

#[derive(Clone, Serialize)]
struct ErrorPayload {
    error: String,
}

fn trim_history(messages: &mut Vec<Value>) {
    if messages.len() > HISTORY_WINDOW {
        let cut = messages.len() - HISTORY_WINDOW;
        messages.drain(..cut);
        // History must start with a plain-text user turn (a leading tool_result
        // without its tool_use would be rejected).
        while messages
            .first()
            .map(|m| m["role"] != "user" || m["content"].is_array())
            .unwrap_or(false)
        {
            messages.remove(0);
        }
    }
}

/// One streamed assistant turn.
struct Turn {
    stop_reason: String,
    /// Anthropic content blocks: text + tool_use.
    content: Vec<Value>,
}

async fn stream_turn(app: &AppHandle, api_key: &str, messages: &[Value]) -> Result<Turn, String> {
    let body = json!({
        "model": DEFAULT_MODEL,
        "max_tokens": MAX_TOKENS,
        "system": SYSTEM_PROMPT,
        "stream": true,
        "tools": tools::definitions(),
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

    // Accumulators per content-block index.
    enum Block {
        Text(String),
        ToolUse { id: String, name: String, json: String },
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut stop_reason = String::from("end_turn");
    let mut buffer = String::new();
    let mut stream = res.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            for line in event_block.lines() {
                let Some(data) = line.strip_prefix("data: ") else { continue };
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                match v["type"].as_str() {
                    Some("content_block_start") => {
                        let cb = &v["content_block"];
                        match cb["type"].as_str() {
                            Some("tool_use") => {
                                blocks.push(Block::ToolUse {
                                    id: cb["id"].as_str().unwrap_or_default().into(),
                                    name: cb["name"].as_str().unwrap_or_default().into(),
                                    json: String::new(),
                                });
                                if let Block::ToolUse { name, .. } = blocks.last().unwrap() {
                                    let _ = app.emit("jarvis-tool", ToolPayload { name });
                                }
                            }
                            _ => blocks.push(Block::Text(String::new())),
                        }
                    }
                    Some("content_block_delta") => match v["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(t) = v["delta"]["text"].as_str() {
                                if let Some(Block::Text(s)) = blocks.last_mut() {
                                    s.push_str(t);
                                }
                                let _ = app.emit("jarvis-delta", DeltaPayload { text: t });
                            }
                        }
                        Some("input_json_delta") => {
                            if let (Some(p), Some(Block::ToolUse { json, .. })) =
                                (v["delta"]["partial_json"].as_str(), blocks.last_mut())
                            {
                                json.push_str(p);
                            }
                        }
                        _ => {}
                    },
                    Some("message_delta") => {
                        if let Some(sr) = v["delta"]["stop_reason"].as_str() {
                            stop_reason = sr.to_string();
                        }
                    }
                    Some("error") => {
                        let msg = v["error"]["message"].as_str().unwrap_or("stream error");
                        return Err(format!("API stream error: {msg}"));
                    }
                    _ => {}
                }
            }
        }
    }

    let content = blocks
        .into_iter()
        .filter_map(|b| match b {
            Block::Text(s) if s.is_empty() => None,
            Block::Text(s) => Some(json!({ "type": "text", "text": s })),
            Block::ToolUse { id, name, json: input } => {
                let input: Value = serde_json::from_str(&input).unwrap_or(json!({}));
                Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
            }
        })
        .collect();

    Ok(Turn { stop_reason, content })
}

fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("")
}

/// Run a full exchange: stream, execute tools, repeat until end_turn.
/// Used by the chat command and (Stage 4) the morning briefing.
pub async fn run_exchange(app: &AppHandle, api_key: &str, history: Vec<Value>) -> Result<(String, Vec<Value>), String> {
    let mut messages = history;

    for _round in 0..MAX_TOOL_ROUNDS {
        let turn = stream_turn(app, api_key, &messages).await?;
        messages.push(json!({ "role": "assistant", "content": turn.content }));

        if turn.stop_reason != "tool_use" {
            return Ok((text_of(&turn.content), messages));
        }

        let tool_uses: Vec<Value> = turn
            .content
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .cloned()
            .collect();

        let mut results = Vec::new();
        for tu in &tool_uses {
            let name = tu["name"].as_str().unwrap_or("");
            let outcome = tools::run(app, name, &tu["input"]).await;
            let (body, is_error) = match outcome {
                Ok(v) => (v.to_string(), false),
                Err(e) => (e, true),
            };
            results.push(json!({
                "type": "tool_result",
                "tool_use_id": tu["id"],
                "content": body,
                "is_error": is_error,
            }));
        }
        messages.push(json!({ "role": "user", "content": results }));
    }

    Err("Tool loop exceeded its round limit, sir — something is misbehaving.".into())
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

    match run_exchange(&app, &api_key, history).await {
        Ok((full, new_history)) => {
            let mut messages = session.messages.lock().unwrap();
            *messages = new_history;
            trim_history(&mut messages);
            drop(messages);
            let _ = app.emit("jarvis-done", DeltaPayload { text: &full });
        }
        Err(err) => {
            let mut messages = session.messages.lock().unwrap();
            if messages.last().map(|m| m["role"] == "user").unwrap_or(false) {
                messages.pop();
            }
            drop(messages);
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
