---
name: onboard
description: JARVIS-led interview that fills CLAUDE.md and seeds the three domain files. Adapted from AIS-OS /onboard (MIT, Nate Herk) to the three-domain JARVIS layout.
---

# /onboard — the seven questions, three-domain version

Run as a spoken/typed interview. PRE-FILL everything JARVIS already knows
(the 5 sites, the analytics platform, the work tool names, contacts already
in work/context.md) and confirm rather than re-ask — keep it short.

Ask, one at a time, adapting follow-ups:
1. Role & responsibilities at Enertiv — what does a good week look like?
2. The active work projects (confirm: Element, Northbridge, Principal,
   Mortenson) — status and the next milestone of each.
3. Key people per project (confirm Jon Roman / Mad Dash, Melvin, Clint) and
   the recurring blockers (confirm COI).
4. Business: per-site monetization status and the 90-day plan; status of
   paper-trading and Little Hills Books.
5. Personal goals this quarter (health, travel, finance arcs — no numbers
   needed, no credentials ever).
6. Preferences: briefing time window, what belongs in the morning brief,
   what should never be spoken aloud.
7. Anything that should be in the vault that nothing above covered?

Then: update work/context.md, business/context.md, personal/context.md and
CLAUDE.md via update_context (confirm each write), and log one decision:
"Vault onboarded" [personal]. Finish with a one-line spoken summary.
