//! Speech-to-text — WinRT Windows.Media.SpeechRecognition.
//!
//! Why not the webview? WebView2 does not implement the Web Speech API's
//! SpeechRecognition (Edge-only feature). Why not whisper.cpp? It would add a
//! cmake/clang build chain and a ~75MB bundled model for quality the OS engine
//! already provides locally. The WinRT recognizer uses the OS speech stack:
//! no model to ship, microphone is only open during recognition, and it
//! auto-stops on silence (the push-to-talk endpoint).
//!
//! Events emitted: stt-started, stt-result { text }, stt-error { error }.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use windows::Media::SpeechRecognition::SpeechRecognizer;

#[derive(Default)]
pub struct SttState {
    /// Held while a recognition is in flight so a second hotkey press can stop it.
    active: Mutex<Option<SpeechRecognizer>>,
}

#[derive(Clone, Serialize)]
struct TextPayload {
    text: String,
}

#[derive(Clone, Serialize)]
struct ErrorPayload {
    error: String,
}

fn friendly_error(e: &windows::core::Error) -> String {
    let raw = e.message().to_string();
    if raw.contains("privacy") || e.code().0 as u32 == 0x80045509 {
        // SPERR_SPEECH_PRIVACY_POLICY_NOT_ACCEPTED
        "Windows speech recognition is disabled. Enable it under Settings → Privacy → Speech.".into()
    } else if raw.is_empty() {
        format!("Speech recognition failed ({:#x})", e.code().0)
    } else {
        raw
    }
}

/// One push-to-talk capture: listens until silence (or stt_stop), emits the transcript.
#[tauri::command]
pub async fn stt_listen(app: AppHandle, state: State<'_, SttState>) -> Result<(), String> {
    // Only one capture at a time.
    if state.active.lock().unwrap().is_some() {
        return Ok(());
    }

    let recognizer = SpeechRecognizer::new().map_err(|e| friendly_error(&e))?;
    *state.active.lock().unwrap() = Some(recognizer.clone());

    let _ = app.emit("stt-started", ());

    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let run = || -> windows::core::Result<String> {
            recognizer.CompileConstraintsAsync()?.get()?;
            let result = recognizer.RecognizeAsync()?.get()?;
            Ok(result.Text()?.to_string())
        };
        match run() {
            Ok(text) if !text.trim().is_empty() => {
                let _ = app2.emit("stt-result", TextPayload { text });
            }
            Ok(_) => {
                let _ = app2.emit("stt-error", ErrorPayload { error: "I didn't catch that, sir.".into() });
            }
            Err(e) => {
                let _ = app2.emit("stt-error", ErrorPayload { error: friendly_error(&e) });
            }
        }
        let s = app2.state::<SttState>();
        *s.active.lock().unwrap() = None;
    });

    Ok(())
}

/// Hotkey released in "hold" mode — end the capture early.
#[tauri::command]
pub fn stt_stop(state: State<'_, SttState>) -> Result<(), String> {
    if let Some(recognizer) = state.active.lock().unwrap().take() {
        if let Ok(op) = recognizer.StopRecognitionAsync() {
            let _ = op; // fire and forget; RecognizeAsync resolves with whatever was heard
        }
    }
    Ok(())
}
