# Post-Launch Mainnet Phases — Execution Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development. Complete phases in order unless an incident forces reprioritization.

**Goal:** Turn the current "mainnet live" state into a production-grade operating posture: recoverable, observable, compartmentalized, and ready for third-party integrations.

**Current baseline (2026-06-02):**
- Mainnet chain live
- RocksDB storage migration complete
- Explorer no longer rescans chain per address/tx request
- Backup script exists, but restore workflow and service segregation still need to be formalized

---

## Phase 10A — Recovery & Restore

**Objective:** Make backup and restore a rehearsed operational procedure rather than an implied capability.

**Why first:** Backups without a validated restore path are paperwork, not safety.

**Deliverables:**
- `docs/operations/RESTORE_RUNBOOK.md` with exact recovery steps for the active mainnet environment
- Verified restore drill on a clean host or temp data directory
- Explicit rules for when to restore `*.db/` versus when to fall back to `*.json.migrated`
- Service restart order documented for RPC, P2P, explorer, and pool

**Required tasks:**
- Define canonical backup artifact set:
  - `state.db/`
  - `mempool.json`
  - `banlist.json`
  - optional `*.json.migrated`
- Add restore commands for:
  - stop services
  - extract archive
  - validate file ownership and permissions
  - start services
  - verify `/getblockcount`, `/health`, explorer status
- Run one real restore drill and capture timings + gotchas

**Exit criteria:**
- A non-author operator can restore from the latest backup using only the runbook
- Recovery procedure is stored in repo and referenced from `MAINNET_READINESS.md`

**Progress note (2026-06-02):**
- `docs/operations/RESTORE_RUNBOOK.md` created
- restore drill completed in temp environment
- measured timings captured in `docs/operations/RESTORE_RUNBOOK.md`

---

## Phase 10B — Explorer Durability

**Objective:** Move the explorer from "good enough for low traffic" to stable under sustained mainnet usage.

**Why second:** User-facing data access is now part of the trust surface.

**Deliverables:**
- Persistent explorer index snapshot strategy or replay budget decision documented
- Reorg handling policy documented and tested
- Explorer process/service deployment documented separately from node
- Basic explorer health probes and memory budget defined

**Required tasks:**
- Decide one of:
  - keep in-memory index and accept replay-on-restart
  - persist compact explorer index to disk
- Add a startup sync status display or warmup status response
- Document expected rebuild time at current block heights
- Add tests or smoke scripts for:
  - explorer restart
  - chain tip advance
  - simple reorg/reset behavior

**Exit criteria:**
- Explorer restart behavior is predictable and documented
- Operators know whether restart cost is seconds, minutes, or unacceptable

**Progress note (2026-06-02):**
- explorer index persistence implemented via `txindex.json`
- `/api/indexer/status` now reports snapshot path, `persisted_at`, and `loaded_from_disk`
- restart smoke verified that explorer can reload the persisted index snapshot

---

## Phase 10C — Pool Custody & Payout Separation

**Objective:** Separate pool operations from treasury/cold-wallet trust.

**Why third:** Funds risk is more important than convenience once mainnet traffic increases.

**Deliverables:**
- Payout hot wallet policy
- Max hot wallet balance rule
- Refill flow from treasury to hot wallet
- Pool payout runbook

**Required tasks:**
- Define:
  - treasury wallet
  - payout hot wallet
  - refill operator
  - payout cadence
- Add explicit daily/weekly operational checks
- Document how to rotate payout wallet if compromise is suspected
- Update readiness checklist to mark explorer service and payout hot wallet separation properly

**Exit criteria:**
- No operator needs founder cold wallet access for normal pool payouts
- Pool payout process is repeatable and bounded by a hot-wallet cap

**Progress note (2026-06-02):**
- `docs/operations/POOL_PAYOUT_RUNBOOK.md` created
- `tensorium-pool` now exposes custody metadata via CLI and HTTP
- treasury / payout-hot-wallet separation is now explicit in operator-facing docs

---

## Phase 10D — Public RPC & Ops Hardening

**Objective:** Harden the public edges around the live chain.

**Why fourth:** By this point core data and custody paths should already be stable.

**Deliverables:**
- Public RPC fronting policy
- nginx/rate-limit templates
- incident response checklist for node lag, RPC abuse, or disk pressure
- explicit service ownership split for node / explorer / pool

**Required tasks:**
- Document and validate:
  - rate limits
  - connection caps
  - log rotation
  - alert thresholds
  - disk usage alarms for RocksDB growth
- Add operator checklist for:
  - chain stall
  - peer isolation
  - explorer divergence
  - backup failure

**Exit criteria:**
- Public RPC posture is documented as an ops standard, not tribal knowledge
- Main failure modes have a written first response

**Progress note (2026-06-02):**
- `docs/operations/PUBLIC_RPC_HARDENING_RUNBOOK.md` created
- `templates/nginx-public-rpc.conf` added
- installer RPC systemd default corrected back to localhost-only bind

---

## Phase 10E — Data Provider & Listing Readiness

**Objective:** Package the chain for third-party consumption cleanly.

**Why fifth:** External integrations are easier once internal ops are boring.

**Deliverables:**
- CoinGecko / CMC submission checklist
- public API surface summary
- explorer / RPC SLA assumptions
- canonical asset metadata document

**Required tasks:**
- Publish one concise document with:
  - chain identifiers
  - explorer URLs
  - RPC URLs
  - bridge URLs
  - token metadata
  - support contact
- Verify data consistency across:
  - website
  - explorer
  - docs
  - listing package

**Exit criteria:**
- Exchange/data-provider packet can be handed off without ad-hoc fact gathering

**Progress note (2026-06-02):**
- `docs/integrations/CANONICAL_ASSET_METADATA.md` created as the single-source packet for chain identifiers, URLs, bridge metadata, tokenomics, and support contact
- `docs/integrations/CEX_LISTING_PACKAGE.md` now points integrator-facing consumers to the canonical packet

---

## Recommended Execution Order

1. Phase 10A — Recovery & Restore
2. Phase 10B — Explorer Durability
3. Phase 10C — Pool Custody & Payout Separation
4. Phase 10D — Public RPC & Ops Hardening
5. Phase 10E — Data Provider & Listing Readiness

## Phase 10 Closure

Status: **COMPLETE** on 2026-06-02.

Closed with these repo artifacts:

- `docs/operations/RESTORE_RUNBOOK.md`
- `docs/operations/POOL_PAYOUT_RUNBOOK.md`
- `docs/operations/PUBLIC_RPC_HARDENING_RUNBOOK.md`
- `docs/operations/PUBLIC_RPC_POSTURE.md`
- `templates/nginx-public-rpc.conf`
- `docs/integrations/CANONICAL_ASSET_METADATA.md`

Handoff prompt for the next worker:

- `docs/superpowers/prompts/2026-06-02-claude-code-phase11-handoff.md`

## Non-Goals For This Plan

- L1 scripting / HTLC / atomic swap
- governance mechanism
- mobile wallet
- bridge contract redesign

Those belong to the longer-term protocol/product roadmap, not immediate post-launch operations.
