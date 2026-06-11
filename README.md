# JARVIS Desktop — Command Center (v2)

Voice-driven native desktop command center for Windows — the desktop embodiment of the
[portfolio-analytics JARVIS](https://github.com/sirbob1366/portfolio-analytics). It lives in the
tray as an arc reactor, summons with **Ctrl+Shift+J**, speaks with a natural neural voice, briefs
you each morning, watches your traffic, reads your work inbox (read-only), keeps a unified to-do
list, and maintains a git-versioned second brain (JARVIS-OS).

Built with **Tauri 2** (Rust backend + WebView2 frontend). Installed size a few MB, idles near
zero CPU; all polling pauses while hidden except the 30-minute anomaly watch.

## The four tabs (left icon rail)

1. **Command** — a glance board, not a chat window: greeting strip (clock + weather), portfolio
   card (mini arc reactor vs 7-day average, deltas, 5-dot site strip, live counter, anomaly
   chips), Today card (next meeting countdown, free-gap hint), To-Do card, inbox/Slack strip.
   Paints instantly from cache, refreshes live (weather 30 min · portfolio 60 s · calendar/todos
   5 min). Conversation opens as an overlay drawer — the board never navigates away.
2. **HUD** — the analytics dashboard rebuilt natively against the same `/api/*` endpoints
   (service-token auth through Rust; no webview embedding, no Access login): Overview reactor
   with orbiting site nodes, site drill-ins (range picker, DPR-sharp charts, five breakdowns),
   Live (10 s stream), Revenue (view **and** edit the ledger + FX rate). Plus native anomaly
   toasts and a draggable always-on-top **mini reactor** widget (tray toggle).
3. **Work** — read-only by design: Gmail (unread/today/search/action items, deep links), Slack
   (mentions + unread DMs, permalinks), work calendar (next meeting, ≥30-min gaps), and the
   unified to-do list (suggested-until-confirmed, confirm/complete/snooze). "Catch me up" = the
   last 4 hours in under 20 spoken seconds.
4. **Mind Map** — the JARVIS-OS vault visualized (see below). **Settings** holds every credential,
   the brain mode, voice picker, briefing window, and a connection-status row per integration
   with the last error visible.

## The brain — subscription-first

Settings → Brain has two modes:

- **Claude Code mode (default):** turns route through the local Claude Code CLI headlessly
  (`claude -p`, stream-json in/out) — billing your **Max subscription**, not API credits. One
  warm persistent process per conversation removes the 2–4 s spawn cost; expect roughly
  +0.3–0.8 s to first word versus the raw API (both latencies are measured per turn and shown in
  Settings — if voice feels sluggish, one tap switches modes). The app's tools are served to the
  CLI via a **local MCP shim** (loopback HTTP, per-run token) — chosen over prompt-level
  tool-intents because the CLI keeps its native agentic loop and every tool (present and future)
  works identically in both modes. The CLI sandbox is file-configured
  (`CLAUDE.md` persona, `.mcp.json`, `.claude/settings.local.json` allowing `mcp__jarvis__*` +
  read-only built-ins, denying Bash/file-writes/web), and `ANTHROPIC_API_KEY` is stripped from
  its environment so it can never silently bill credits.
- **API mode:** the direct Anthropic path (`claude-sonnet-4-6`) — used when selected, and as
  automatic fallback when the CLI is missing or errors (visible status note). Subscription limit
  exhaustion raises a distinct notice with a one-tap switch.

## JARVIS-OS — the second brain

A plain-markdown, git-versioned vault at `~/JARVIS-OS` (skeleton adapted from
[AIS-OS by Nate Herk](https://github.com/nateherkai/AIS-OS), MIT — attribution kept in the vault),
organized into **work / business / personal**. In CLI mode the vault *is* the brain's working
directory, so the `CLAUDE.md` routing tree loads natively; in API mode the relevant domain's
files are injected into the system prompt (domain pin in the titlebar, else keyword routing).
Write-back happens only through tools — `log_decision`, `save_note`, `update_context` — every
change auto-committed to the vault's git, and **never silently**: while "ask before writing" is
on, unconfirmed writes are mechanically refused. The Mind Map tab renders the vault as a
collapsible tidy-tree (pan/zoom/search, edit-in-place, live node pulses on writes) with the
weekly Four-Cs audit as an overlay scorecard. JARVIS runs `/audit` quietly every Sunday evening
and folds a one-line health note into Monday's briefing.

## Voice

**Speaking:** dual-engine. The WinRT `Windows.Media.SpeechSynthesis` path (Rust) sees every
voice pack installed via **Windows Settings → Time & Language → Speech**, *including the
Windows 11 natural neural voices* — WebView2's `speechSynthesis` only ever exposes legacy SAPI
voices (the Edge "Online (Natural)" voices are browser-exclusive; that's why the Rust path
exists and is preferred). Preference order: Ryan (Natural) en-GB → any en-GB Natural male → any
Natural → legacy en-GB male → en. The voice picker flags Natural voices; if none is installed, a
hint deep-links to `ms-settings:speech`. Delivery: URLs/ids are never read aloud ("link on
screen"), big numbers are rounded in speech, a 240 ms pause follows "sir", and JARVIS never
talks over push-to-talk.

**Listening:** push-to-talk (hold **Ctrl+Shift+J**) via the WinRT recognizer — WebView2 lacks
Web Speech recognition; whisper.cpp would add a build chain + ~75 MB model for no gain. Mic is
open only while the waveform strip shows.

## Setup

| Requirement | Install |
| --- | --- |
| Rust (MSVC) + VS Build Tools (C++) | `winget install Rustlang.Rustup Microsoft.VisualStudio.2022.BuildTools` |
| Node.js ≥ 20 | for the Tauri CLI (`npm install`) |
| Claude Code CLI (optional, for subscription brain) | `npm i -g @anthropic-ai/claude-code` → `claude login` |
| git (optional, for vault versioning + kit clone) | `winget install Git.Git` |

```sh
npm install
npm run dev      # development app
npm run build    # → src-tauri/target/release/bundle/msi/JARVIS_0.2.0_x64_en-US.msi
```

The `.msi` is unsigned — SmartScreen will ask once (*More info → Run anyway*).

**First run:** tray reactor appears → Ctrl+Shift+J → Settings tab:
1. **Brain** — install/login Claude Code for subscription mode, and/or paste an Anthropic key.
2. **Portfolio** — Cloudflare Access service token pair (Zero Trust → Service Auth; add a
   Service-Auth policy on the analytics app). Powers the board card + the entire HUD tab.
3. **Personal Google** — OAuth Desktop client ID/secret → *Connect Calendar*.
4. **Work accounts** — *Connect Work Google* (pick the work account; `gmail.readonly` +
   `calendar.readonly` only) and a Slack user token (`xoxp-…`, scopes: `search:read`,
   `channels:history`, `groups:history`, `im:history`, `mpim:history`, `users:read`).
5. **Mind Map tab** → *Initialize the vault* → say "run onboarding" for the 7-question interview.

Every credential lives in the **Windows Credential Manager** (service "JARVIS") — never a file.

## Architecture

```
ui/                      vanilla JS, no bundler
  js/app.js              tab router, conversation drawer, domain pin, events
  js/board.js            Command glance board (cache-first, visibility-paused polling)
  js/hud.js              native HUD (overview/site/live/revenue, DPR-aware charts)
  js/work.js             work tab (today/inbox/slack/todos)
  js/mindmap.js          vault tidy-tree, side panel, audit overlay
  js/settings.js         credentials, brain mode, voice picker, connection statuses
  js/voice.js            dual-engine TTS + push-to-talk + delivery polish
  mini.html              always-on-top mini reactor window
src-tauri/src/
  lib.rs                 tray, hotkey, windows, single-instance
  brain.rs               CLI/API routing, warm session, latency, fallback
  mcp.rs                 local MCP shim (the CLI's bridge to the app tools)
  claude.rs              Anthropic streaming + session + vault context injection
  tools.rs               the model's toolbox (portfolio, weather, timers, system,
                         work_*, navigate_app, hud_data, vault tools, notes)
  work.rs / google_auth.rs / calendar.rs    read-only work stage + OAuth
  vault.rs (+vault_templates/)              JARVIS-OS bootstrap, git, write-back
  hud.rs / todos.rs / proactive.rs          worker proxy, todo store, briefing+audit loops
  tts.rs / stt.rs                           WinRT speech synthesis / recognition
  db.rs / secrets.rs                        SQLite (notes/kv/todos), Credential Manager
```

## Design system

8 px spacing grid · two type roles per card (11 px tracked uppercase label / tabular-mono value)
· 14 px card radius · hairline borders `rgba(78,216,255,.18)` · the cyan glow appears **only** on
live/active elements (reactor, live counters, listening waveform, active path) · amber strictly
for anomalies · 150–200 ms ease-out motion, stagger-ins, number tick-ups · DPR-aware canvas/SVG
everywhere · zero idle animation beyond the reactor pulse; everything stops when minimized.
