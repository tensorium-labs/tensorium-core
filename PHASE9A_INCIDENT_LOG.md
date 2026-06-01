# Phase 9A Incident Log

Status: active log for bridge incidents and anomalies.
Last updated: 2026-06-01

## Log Location

Primary: `/root/.openclaw/workspace/tensorium-core/PHASE9A_INCIDENT_LOG.md`

This file is the canonical incident record. Each entry is appended
chronologically. Do not edit or delete past entries.

## Entry Format

```
### INC-<YYYY-MM-DD>-<N>

**Severity:** LOW | MEDIUM | HIGH | CRITICAL
**Status:** OPEN | RESOLVED | MONITORING
**Detected:** <ISO timestamp UTC>
**Resolved:** <ISO timestamp UTC> or OPEN

**Summary:** One sentence describing what happened.

**Timeline:**
- <time> — <event>
- <time> — <event>

**Impact:**
- Describe what was affected and for how long.

**Root Cause:**
- Describe what caused the incident.

**Resolution:**
- Describe what was done to fix it.

**Follow-up:**
- Any post-incident action items.
```

## Severity Definitions

| Severity | Description |
|---|---|
| LOW | No user impact. Operator anomaly noticed and resolved quickly. |
| MEDIUM | Delayed processing or minor user impact. No funds at risk. |
| HIGH | User transaction delayed or stuck. Possible funds at risk. |
| CRITICAL | Funds at risk. Bridge paused. Immediate operator response required. |

## Communication Path

- Internal log: this file
- User-facing status: `status.tensoriumlabs.com`
- Real-time communication: Telegram @tensoriumlabs
- Post-incident summary: posted to Telegram within 24h of resolution

## When to Pause the Bridge

Pause immediately if any of the following:
- Unexpected mint or burn without matching ledger entry
- Custody address balance does not match circulating wTXM supply
- Relayer bot stops responding and cannot be restarted within 30 min
- Smart contract owner key suspected compromised
- Anomalous transaction volume (>3x daily average in <1h)

Pause command (Optimism mainnet):
```
controller.pause() — call from owner address 0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB
```

## Active Incidents

None. Bridge live since 2026-06-01. No incidents recorded.

---

*First entry will be added here when the first incident occurs.*
