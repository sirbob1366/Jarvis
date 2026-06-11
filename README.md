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
