# Business — the portfolio

> Seeded by JARVIS with what it already runs; expand freely.

## The 5 sites
| id | domain | what it is |
| --- | --- | --- |
| pdfedit | myfreepdfedit.com | free PDF editing tools |
| imagetool | — | image tools |
| audiotool | — | audio tools |
| videotool | — | video tools |
| invoicetool | — | invoice generator |

Stack: static fronts + Cloudflare; analytics = custom Worker + D1
(portfolio-analytics repo), beacon on every site, HUD + Discord alerts.

## Monetization
Strategy & timeline: (AdSense/Ezoic/affiliate per site; revenue ledger lives
in the analytics platform — JARVIS can read and edit it from the HUD tab).

## Content plans
(per-site queues, keyword targets)

## JARVIS architecture
Desktop app (Tauri 2): command center, HUD, work stage, voice, this vault.
Brain: Claude Code CLI (subscription) with API fallback; tools over MCP.

## Other ventures
- Paper-trading system — (status)
- Little Hills Books — (status)
