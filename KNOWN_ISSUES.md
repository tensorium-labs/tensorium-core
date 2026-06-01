# Known Issues — Tensorium Testnet

Status: Public testnet active, mainnet-candidate pre-launch checklist in progress
Last updated: 2026-06-01

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
**Description:** `tensorium-node mine-once` uses a single-threaded nonce search. At current GPU-first testnet difficulty, this command is only useful for development diagnostics and should not be treated as a real miner.
**Workaround:** Use `txmminer-cuda` for GPU mining or `txmminer` only for low-difficulty/dev testing.

---

### KI-003: No peer exchange; discovery still depends on static seeds

**Severity:** Medium
**Component:** tensorium-node P2P
**Description:** Nodes now have built-in default seeds, and the mainnet-candidate chain has a DNS seed, but there is still no peer exchange (PEX) or richer discovery layer. If the published seeds are unreachable, operators still need manual peer configuration.
**Workaround:** Default seeds should work in normal cases. If not, set `TENSORIUM_PEERS` manually and run `tensorium-node sync <peer>` against a healthy node.
**Fix planned:** Post-launch networking hardening — add better peer discovery and redundancy beyond static seeds.

---

### KI-004: Chain state stored in single JSON file

**Severity:** Medium (scalability)
**Component:** tensorium-node state management
**Description:** The entire chain state is loaded into memory and serialized as a single JSON file on every block write. This is fast and simple for testnet (few hundred blocks) but will not scale to hundreds of thousands of blocks without significant memory and I/O overhead.
**Current impact:** None at testnet scale. At 100,000 blocks (~70 days), state file would be ~500MB+.
**Fix planned:** Pre-mainnet — migrate to embedded key-value store (e.g. sled or RocksDB).

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
**Description:** The current consensus rules do not enforce minimum transaction fees. Miners accept zero-fee transactions. Fee market mechanics are deferred to testnet Phase 5 / mainnet design.

---

### KI-007: Wallet balance shows 0 until full chain scan

**Severity:** Low (UX)
**Component:** txmwallet
**Description:** `txmwallet balance` scans the entire chain state file sequentially. On large chains this is slow. There is no UTXO index for fast lookups.
**Fix planned:** Pre-mainnet with the state store migration (KI-004).

---

## Fixed Issues

| ID | Description | Fixed in |
|----|-------------|---------|
| FI-001 | Genesis block non-deterministic (different hash per node) | v0.1.1 — fixed timestamp |
| FI-002 | txmminer fails at difficulty > ~23 bits (nonce limit too low) | v0.1.1 — raised to u64::MAX |
| FI-003 | Invalid POST body returns HTTP 500 instead of HTTP 400 | v0.1.2 — parse errors now 400 |
| FI-004 | No GPU miner available for public testnet | v0.2.0-testnet — CUDA miner released |
| FI-005 | GPU-first genesis took too long to initialize on normal nodes | v0.2.0-testnet — pre-mined genesis nonce |

---

## Out of Scope for Testnet

These are known limitations that are intentional for testnet and will be addressed before mainnet:

- No peer exchange (PEX) / richer peer discovery beyond static seeds
- No security audit — required before mainnet
- Public RPC still relies on nginx rate limiting and localhost-only node binds
