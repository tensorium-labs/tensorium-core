# Known Issues — Tensorium Testnet

Status: Public Testnet GPU-first (Phase 6 complete, Phase 7 preparation started)
Last updated: 2026-05-31

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

### KI-003: No automatic peer discovery

**Severity:** Medium
**Component:** tensorium-node P2P
**Description:** Peer connections are configured entirely via `TENSORIUM_PEERS` environment variable. There is no DNS seed, peer exchange (PEX), or automatic discovery. New nodes must be pointed at a known seed node manually.
**Workaround:** Set `TENSORIUM_PEERS=157.230.44.162:23333` and use `tensorium-node sync 157.230.44.162:23333` for initial sync.
**Fix planned:** Phase 7 readiness — DNS seeds and/or documented seed list.

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

- No mining pool support — Phase 7 readiness
- No peer exchange / DNS seeds — Phase 7 readiness
- No security audit — required before mainnet
- No rate limiting on RPC — testnet only, RPC should be localhost-bound
