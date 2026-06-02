# Known Issues — Tensorium Mainnet Operations

Status: Mainnet live; this file tracks active runtime and operator-facing issues
Last updated: 2026-06-02

---

## Active Known Issues

### KI-001: Stale block logged as accepted by losing miner

**Severity:** Low (cosmetic)
**Component:** txmminer
**Description:** When two miners mine the same block simultaneously, the second miner to submit receives `AlreadyKnown` from the node, which returns `accepted: true` with the existing height. The miner prints `✓` rather than logging a stale event. Functionally the miner continues correctly to the next block.
**Impact:** None — mining continues normally. Log is slightly misleading.
**Fix planned:** Phase 7 readiness — return `accepted: false` for `AlreadyKnown` in RPC response.

---

### KI-002: `mine-once` is single-threaded

**Severity:** Low
**Component:** tensorium-node (mine-once command)
**Description:** `tensorium-node mine-once` uses a single-threaded nonce search. At current GPU-first mainnet difficulty, this command is only useful for development diagnostics and should not be treated as a real miner.
**Workaround:** Use `txmminer-cuda` for GPU mining or `txmminer` only for low-difficulty/dev testing.

---

### KI-003: No peer exchange; discovery still depends on static seeds

**Severity:** Medium
**Component:** tensorium-node P2P
**Description:** Nodes now have built-in default seeds, and the mainnet-candidate chain has a DNS seed, but there is still no peer exchange (PEX) or richer discovery layer. If the published seeds are unreachable, operators still need manual peer configuration.
**Workaround:** Default seeds should work in normal cases. If not, set `TENSORIUM_PEERS` manually and run `tensorium-node sync <peer>` against a healthy node.
**Fix planned:** Post-launch networking hardening — add peer exchange and redundancy beyond static seeds/DNS seeds.

---

### KI-004: Legacy JSON chain state migration may leave `.json.migrated` backups behind

**Severity:** Low
**Component:** tensorium-node state management
**Description:** RocksDB migration is complete, but nodes that auto-migrate from legacy `state.json` intentionally keep a `state.json.migrated` backup on disk for rollback safety.
**Current impact:** Minor disk usage until operators manually remove the backup after verifying the migrated DB.
**Workaround:** After verifying `tensorium-node status` and RPC responses on the migrated DB, delete the `.json.migrated` backup manually if disk space matters.

---

### KI-005: P2P sync does not request missing blocks on broadcast rejection

**Severity:** Low
**Component:** P2P networking
**Description:** When a node receives a broadcast block whose parent is not known (gap in chain), it rejects with "parent not known" and does not automatically request the missing blocks. The syncing node must manually run `tensorium-node sync` to fill gaps.
**Workaround:** Run `tensorium-node sync <peer>` after any detected gap.

---

### KI-006: No transaction fee enforcement

**Severity:** Low
**Component:** Consensus / mempool
**Description:** The current consensus rules do not enforce minimum transaction fees. Miners accept zero-fee transactions. Fee market mechanics remain deferred to a later mainnet economics pass.

---

### KI-007: Wallet balance shows 0 until full chain scan

**Severity:** Low (UX)
**Component:** txmwallet
**Description:** `txmwallet balance` scans the canonical chain sequentially to rebuild UTXOs. It no longer depends on the old JSON state file, but it still has no dedicated address/UTXO index for fast lookups.
**Fix planned:** Post-launch wallet/indexing improvement after the storage migration baseline.

---

## Fixed Issues

| ID | Description | Fixed in |
|----|-------------|---------|
| FI-001 | Genesis block non-deterministic (different hash per node) | v0.1.1 — fixed timestamp |
| FI-002 | txmminer fails at difficulty > ~23 bits (nonce limit too low) | v0.1.1 — raised to u64::MAX |
| FI-003 | Invalid POST body returns HTTP 500 instead of HTTP 400 | v0.1.2 — parse errors now 400 |
| FI-004 | No production GPU miner path available | v0.2.0-pre-mainnet — CUDA miner released |
| FI-005 | GPU-first genesis took too long to initialize on normal nodes | v0.2.0-pre-mainnet — pre-mined genesis nonce |

---

## Deferred / Out Of Scope

These are known limitations that remain intentionally deferred after launch:

- No peer exchange (PEX) / richer peer discovery beyond static seeds
- No external security audit yet
- Public RPC still relies on nginx rate limiting and localhost-only node binds
