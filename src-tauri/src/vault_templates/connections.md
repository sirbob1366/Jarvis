# Connections — every live system JARVIS reaches

| System | What | How JARVIS reaches it |
| --- | --- | --- |
| Portfolio analytics API | pageviews/uniques/live/anomalies/revenue for the 5 sites | `analytics.myfreepdfedit.com/api/*` via Cloudflare Access service token (Credential Manager) |
| The 5 sites | pdfedit (myfreepdfedit.com), imagetool, audiotool, videotool, invoicetool | tracked by the beacon → analytics Worker (D1) |
| Work Gmail | unread, today's mail, search, action items — READ-ONLY | second Google account OAuth (gmail.readonly) |
| Work calendar | meetings, gaps — READ-ONLY | same work account (calendar.readonly) |
| Personal Google Calendar | today/next/week/create | personal account OAuth (calendar) |
| Slack (work) | mentions, unread DMs, search — READ-ONLY | xoxp user token (search:read + history + users:read) |
| Weather | Open-Meteo, no key | city from app Settings (default Pune, IN) |
| Discord | anomaly alerts from the analytics Worker (separate from the app) | Worker-side webhook |
| Claude brain | Claude Code CLI (subscription) with API fallback | local MCP shim exposes all app tools |

Credentials live ONLY in the Windows Credential Manager (service "JARVIS").
This vault never stores secrets.
