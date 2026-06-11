---
name: audit
description: Four-Cs health check of the vault (Current, Complete, Consistent, Connected). Adapted from AIS-OS /audit (MIT, Nate Herk).
---

# /audit — Four-Cs gap report

For each domain (work / business / personal) score 0–10 and give ONE line of
evidence each:
- **Current** — are the files fresh? (flag anything > 3 weeks stale that
  changes weekly, e.g. work contacts, project status)
- **Complete** — are the sections filled or still skeleton stubs?
- **Consistent** — do files contradict each other or decisions/log.md?
- **Connected** — does connections.md match what JARVIS actually reaches?

Output format (this exact shape — the Mind Map renders it as a scorecard):

```
AUDIT YYYY-MM-DD
work:      C? ? ? ?  <one-line gap>
business:  C? ? ? ?  <one-line gap>
personal:  C? ? ? ?  <one-line gap>
top fix:   <the single highest-leverage fix>
```

End with one spoken sentence (e.g. "Context audit: business domain is
current; the work contacts file is three weeks stale, sir.").
