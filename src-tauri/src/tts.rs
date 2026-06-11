//! Text-to-speech — WinRT Windows.Media.SpeechSynthesis.
//!
//! Why a Rust path at all: WebView2's speechSynthesis only exposes legacy
//! SAPI voices — the modern "(Natural)" neural voices never appear there
//! (they are Edge-browser-exclusive). WinRT's SpeechSynthesizer sees every
//! voice pack installed through Windows Settings → Time & Language → Speech,
//! including the on-device neural ones on Windows 11. So: the UI enumerates
//! both engines, prefers any "(Natural)" voice (WinRT first), and falls back
//! down the en-GB male → en chain. Synthesis happens here; the WAV is handed
//! to the webview as base64 and played there, so mute/interrupt logic stays
//! in one place.

use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceGender};
use windows::Storage::Streams::DataReader;

fn werr(e: windows::core::Error) -> String {
    let m = e.message().to_string();
    if m.is_empty() { format!("TTS error {:#x}", e.code().0) } else { m }
}

/// Every voice the OS speech stack offers (the webview enumerates its own).
#[tauri::command]
pub fn tts_voices() -> Result<Vec<Value>, String> {
    let all = SpeechSynthesizer::AllVoices().map_err(werr)?;
    let mut out = Vec::new();
    for v in &all {
        let gender = match v.Gender() {
            Ok(VoiceGender::Male) => "male",
            Ok(VoiceGender::Female) => "female",
            _ => "unknown",
        };
        out.push(json!({
            "name": v.DisplayName().map_err(werr)?.to_string(),
            "lang": v.Language().map_err(werr)?.to_string(),
            "gender": gender,
        }));
    }
    Ok(out)
}

/// Synthesize text (or SSML) to a WAV, returned base64 for webview playback.
#[tauri::command]
pub async fn tts_synthesize(
    text: String,
    voice: String,
    rate: f64,
    ssml: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let synth = SpeechSynthesizer::new().map_err(werr)?;

        if !voice.is_empty() {
            let all = SpeechSynthesizer::AllVoices().map_err(werr)?;
            for v in &all {
                if v.DisplayName().map(|n| n.to_string() == voice).unwrap_or(false) {
                    synth.SetVoice(&v).map_err(werr)?;
                    break;
                }
            }
        }
        if let Ok(options) = synth.Options() {
            let _ = options.SetSpeakingRate(rate.clamp(0.5, 3.0));
        }

        let htext = HSTRING::from(&text);
        let stream = if ssml {
            synth.SynthesizeSsmlToStreamAsync(&htext).map_err(werr)?.get().map_err(werr)?
        } else {
            synth.SynthesizeTextToStreamAsync(&htext).map_err(werr)?.get().map_err(werr)?
        };

        let size = stream.Size().map_err(werr)? as u32;
        let input = stream.GetInputStreamAt(0).map_err(werr)?;
        let reader = DataReader::CreateDataReader(&input).map_err(werr)?;
        reader.LoadAsync(size).map_err(werr)?.get().map_err(werr)?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).map_err(werr)?;

        Ok(b64(&buf))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deep link into Windows voice settings (add a natural voice pack).
#[tauri::command]
pub fn open_voice_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("ms-settings:speech", None::<&str>)
        .map_err(|e| e.to_string())
}

// Minimal base64 (std has none; not worth a crate for one call site).
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}
