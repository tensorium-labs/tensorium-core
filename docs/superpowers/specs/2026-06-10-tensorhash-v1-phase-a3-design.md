# TensorHash v1 — Phase A3 Design (Pool Share Validation)

**Status:** Approved
**Context:** Tensorium Clean Relaunch. Phases A1 (algorithm), B (MAINNET
params), and A2 (CUDA miner + node support, GPU-validated — see
`2026-06-10-phase-a2-gpu-validation-notes.md`) are complete on `main`
(`4e8c6b8`). This phase makes `tensorium-pool` validate stratum shares
against TensorHash v1 and forward `epoch_seed` to miners. The miner side is
already prepared: it parses `epoch_seed` from `mining.notify` and falls back
to the zero seed with a one-time warning when absent (correct only for
epoch 0, heights < 8192).

All changes live in `crates/tensorium-pool` (chiefly `src/stratum.rs`).

## 1. Share PoW validation via the consensus type

`validate_share` currently hashes a hand-rolled header byte string with
SHA256d:

```rust
let header = build_header(job, nonce);   // pool's own serialization
let hash   = sha256d(&header);
let zeros  = leading_zero_bits(&hash);
```

Replace with the exact consensus code path — construct a
`tensorium_core::BlockHeader` from the `StratumJob` and call its methods:

```rust
fn job_header(job: &StratumJob, nonce: u64) -> BlockHeader {
    BlockHeader {
        version: job.version,
        chain_id: job.chain_id.clone(),
        height: job.height,
        previous_hash: Hash256(job.previous_hash),
        merkle_root: Hash256(job.merkle_root),
        timestamp_seconds: job.timestamp,
        leading_zero_bits: job.difficulty_bits,
        nonce,
    }
}

// validate_share:
let header = job_header(job, nonce);
let zeros  = leading_zero_bits(&header.pow_hash(Hash256(job.epoch_seed)).0);
```

Rationale (decided): the pool can never drift from node serialization,
because it *is* the node's serialization. CPU cost per share is ~34 Blake2b
calls (the K=32 touched dataset elements are recomputed on demand) —
microseconds, negligible at share rates.

Consequences:

- `build_header` and `sha256d` are deleted from `stratum.rs`; the block
  id-hash for the payout ledger (currently `sha256d(build_header(...))`)
  becomes `job_header(job, nonce).hash()` — `BlockHeader::hash()` is still
  double-SHA256 (id-hash is unchanged by TensorHash; only the PoW hash
  changed).
- The `sha2` dependency is removed from `tensorium-pool/Cargo.toml` if
  nothing else in the crate uses it (verify with grep at implementation
  time; `accounting.rs`/`payout.rs`/`main.rs` are not expected to).
- `leading_zero_bits` helper stays (operates on any 32-byte hash).

## 2. `epoch_seed` plumbing

- `StratumJob` gains `pub epoch_seed: [u8; 32]`.
- `fetch_job` parses the **top-level** `epoch_seed` field of the node's
  `/getblocktemplate` response (a JSON array of 32 byte values, same serde
  as `previous_hash`; added in Phase A2 Task 2). Missing field → `None`
  (hard error path, logged) — a node without it is too old to pool-mine
  TensorHash.
- `notify_msg` adds `"epoch_seed": bytes_to_hex(&job.epoch_seed)` (64-char
  hex — the stratum convention, matching `previous_hash`/`merkle_root`;
  the miner's `parse_notify` already consumes exactly this).

## 3. Share-difficulty defaults for TensorHash rates

TensorHash hashrates are MH/s, not SHA256d's GH/s (measured: RTX 5090 =
220 MH/s). The SHA256d-era defaults would flood the pool:

| Constant | Old | New | Why |
|---|---|---|---|
| `TENSORIUM_POOL_SHARE_DIFF` default (`main.rs`) | `1_048_576` (2^20) | `268_435_456` (2^28) | 2^20 → ~210 shares/s from one 5090 for the ~8 min vardiff needs to climb; 2^28 → ~49 shares/min, inside the vardiff target band (15–60/min) from the first second. A ~50 MH/s card still produces ~11 shares/min — acceptable floor granularity. |
| `VARDIFF_MAX_BITS` (`stratum.rs`) | 38 | 40 | Keeps the "2 bits below network difficulty" rule (network is now 42, was 40). Update the stale doc comment. |
| `VARDIFF_MIN_BITS` | 16 | 16 (unchanged) | Harmless floor; enables CPU-mined shares in tests and tiny devices. |

Share difficulty is accounting granularity only — it does not affect how
many blocks the pool finds (that is purely total hashrate vs the 42-bit
network target). Env override `TENSORIUM_POOL_SHARE_DIFF` keeps working.

## 4. Testing (CPU-only gate; no GPU required)

Unit tests in `stratum.rs` (new `#[cfg(test)]` additions):

1. **Accept valid share:** CPU-mine a nonce at **12 bits** for a fixed
   `StratumJob` (zero epoch seed) by iterating
   `job_header(job, n).pow_hash(...)` — expected ~4k attempts, fast even in
   the unoptimized test profile — then assert `validate_share` returns
   `Some((zeros, true, _))` with `zeros >= 12` for that nonce's LE-hex
   (`worker_diff_bits` is a direct parameter, so the test is not bound by
   the production vardiff floor of 16).
2. **Reject invalid nonce:** `validate_share` with a different nonce
   (mined-nonce ± 1) at the same 12-bit diff returns `is_share == false`
   (probabilistically certain; deterministic for the fixed inputs once
   chosen — same KAT pattern used throughout the project).
3. **Epoch seed matters:** the same valid nonce validated under a different
   `epoch_seed` fails the share check (deterministic for fixed inputs).
4. **Notify carries the seed:** `notify_msg` output contains
   `epoch_seed` == the job's seed hex.
5. **id-hash unchanged:** `job_header(...).hash()` equals the old
   `sha256d(build_header(...))` for one fixed job — pin this with a
   hardcoded hex vector computed before deleting `build_header`, proving
   ledger block-hashes stay stable across the refactor.

Pool↔GPU live test (node devnet + pool + `tensorium-miner --mode pool`) is
**deferred to the pre-launch rental session** and bundled with the genesis
re-mine (one rental covers both). Recorded as a launch checklist item:
miner connects, receives `epoch_seed` in notify, shares accepted, a found
block submits and is accepted by the node, PPLNS split recorded.

## Dependencies & Out of Scope

- **Depends on:** Phase A2 (node sends `epoch_seed` in templates; miner
  parses `epoch_seed` from notify).
- **Out of scope:** payout/accounting logic (shares remain shares), pool
  HTTP proxy endpoints (`main.rs` passthroughs unchanged), deploy (Phase D),
  genesis re-mine (launch-time), Phases C/E.
