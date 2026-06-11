# JARVIS Desktop

Voice-driven native desktop assistant for Windows — the desktop embodiment of the
[portfolio-analytics JARVIS](https://github.com/sirbob1366/portfolio-analytics). It sits in the
system tray as an arc reactor, summons with **Ctrl+Shift+J**, listens, speaks, briefs you each
morning, and carries tools (portfolio stats, weather, timers, calendar, persistent notes).

Built with **Tauri 2** (Rust backend + WebView2 frontend) — not Electron. The installed app is a
few MB, idles near zero CPU, and uses the OS webview.

## Prerequisites (Windows)

| Requirement | Install |
| --- | --- |
| Rust (MSVC toolchain) | `winget install Rustlang.Rustup` then restart the shell (`cargo --version` to verify) |
| VS Build Tools (C++ workload) | `winget install Microsoft.VisualStudio.2022.BuildTools` → select "Desktop development with C++" |
| WebView2 runtime | Preinstalled on Windows 11; otherwise [Evergreen installer](https://developer.microsoft.com/microsoft-edge/webview2/) |
| Node.js ≥ 20 | for the Tauri CLI (`npm install`) |

## Build & run

```sh
npm install        # Tauri CLI
npm run dev        # development app with hot reload
npm run build      # release build → src-tauri/target/release/bundle/msi/JARVIS_*.msi
```

The `.msi` from `npm run build` is a real installable app: installs to Program Files,
Start-menu entry, tray icon, uninstaller. It is unsigned — Windows SmartScreen will show
"unrecognized app" once; choose *More info → Run anyway* (code-signing certificates are the
only way around that, and unnecessary for a personal install).

## First run

1. Launch JARVIS — the arc reactor appears in the system tray.
2. Left-click the tray icon (or **Ctrl+Shift+J**) to open the window.
3. Gear icon → paste your **Anthropic API key** → SAVE. The key goes into the
   **Windows Credential Manager** (service "JARVIS") — never a file, never the repo.
4. Type a message (voice arrives in Stage 2).

Tray right-click menu: **Open / Mute / Start with Windows / Quit**. Closing the window hides
to tray; the app keeps running.

## Keyboard

| Key | Action |
| --- | --- |
| `Ctrl+Shift+J` (global) | Summon window + start listening (push-to-talk) |
| `Esc` | Hide window |

## Voice

**Speaking (TTS):** webview `speechSynthesis` — WebView2 exposes the installed Windows
voices. JARVIS prefers a UK male voice (Ryan/George/Thomas) and falls back down the en-GB →
en chain. Rate and pitch live in Settings; Mute (tray or 🔊 button) silences him instantly.
Every spoken reply also renders as text.

**Listening (STT):** push-to-talk via **Ctrl+Shift+J** — hold and speak, release (or pause)
to send. Implementation: the WinRT `Windows.Media.SpeechRecognition` engine, called from
Rust. Why not the alternatives the spec offered:

- *Webview SpeechRecognition*: WebView2 does **not** implement the Web Speech API's
  recognition half (Edge-only). The UI still probes for it at runtime and would prefer it.
- *whisper.cpp*: would add a cmake/clang build chain and a ~75MB bundled model for accuracy
  the OS engine already provides locally, with worse latency.

The WinRT recognizer only opens the microphone during capture (the waveform strip +
"● LISTENING" tag make that visible) and auto-stops on silence.

**Troubleshooting:** if JARVIS says speech recognition is disabled, enable
**Settings → Privacy & security → Speech → Online speech recognition** in Windows.

## Tools

JARVIS answers with function-calling — every tool executes locally in Rust; the model only
supplies arguments:

| Tool | What it does |
| --- | --- |
| `portfolio_stats` | Today/week summaries, top pages, live counts from the analytics Worker (per site or portfolio-wide) |
| `weather` | Current + today/tomorrow forecast via Open-Meteo (no key; city in Settings, default Pune) |
| `set_timer` / `list_timers` | Local timers/reminders — native Windows notification + spoken alert |
| `system` | Open a URL in the default browser; current date/time (IST) |
| `remember` / `recall` | Persistent notes store (SQLite in `%APPDATA%\com.sirbob.jarvis`) |

Try: *"How's pdfedit doing today?"*, *"Weather tomorrow?"*, *"Remind me in 20 minutes to stretch"*,
*"Remember that the Ezoic payout lands on the 15th"*, *"What did I ask you to remember?"*

### Cloudflare Access service token (portfolio_stats)

The analytics Worker sits behind Cloudflare Access, so JARVIS authenticates with a
**service token** instead of a browser login:

1. [Zero Trust](https://one.dash.cloudflare.com) → **Access → Service auth → Service Tokens →
   Create Service Token** — name it `jarvis-desktop`, duration to taste. Copy the
   **Client ID** and **Client Secret** (the secret is shown once).
2. **Access → Applications → Portfolio Analytics HUD → Policies → Add a policy**:
   name `JARVIS desktop`, action **Service Auth**, include → **Service Token** →
   `jarvis-desktop`.
3. JARVIS Settings (gear) → paste both values → SAVE. They live in the Windows Credential
   Manager and ride along as `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers.

## Architecture

```
ui/                 frontend (vanilla JS, no bundler) — chat view, settings, voice (Stage 2)
src-tauri/src/
  lib.rs            shell: tray, hotkey, single-instance, autostart, mute state
  claude.rs         Anthropic streaming (SSE → Tauri events), session memory, tools (Stage 3)
  secrets.rs        Windows Credential Manager via keyring (allowlisted keys)
```

Model: `claude-sonnet-4-6` (the spec's `claude-sonnet-4-20250514` is deprecated and retires
2026-06-15, so the current Sonnet is used instead).

## Roadmap / stages

1. ✅ Tray shell, frameless HUD window, global hotkey, single-instance, autostart, streaming text chat
2. Voice: push-to-talk STT + TTS (UK male voice preferred), waveform, mute
3. Tools: portfolio_stats (Cloudflare Access service token), weather (Open-Meteo), timers/reminders, system, remember/recall (SQLite)
4. Morning briefing on first wake + 30-min anomaly polling with native notifications
5. Google Calendar (OAuth desktop loopback) + `.msi` installer

Stage-specific setup (Access service token, Google OAuth) is documented as those stages land.
