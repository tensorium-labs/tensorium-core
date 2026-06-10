# TensorHash v1 Phase A3 — Pool Share Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `tensorium-pool` validates stratum shares with TensorHash v1 via the consensus `BlockHeader` type and forwards `epoch_seed` to miners; share-difficulty defaults retuned for MH/s-class hashrates.

**Architecture:** All changes in `crates/tensorium-pool`. The pool stops hand-rolling header bytes + SHA256d: it constructs `tensorium_core::BlockHeader` from `StratumJob` and calls `.pow_hash(epoch_seed)` for the share check and `.hash()` for the ledger id-hash (id-hash stays double-SHA256 by consensus). `epoch_seed` flows node-template → `StratumJob` → `mining.notify` (64-hex, miner already parses it). Template parsing is factored out of the HTTP fetch for testability.

**Tech Stack:** Rust; `tensorium-core` (already a dependency — `BlockHeader`, `Hash256`); deletes the `sha2` dependency.

**Spec:** `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a3-design.md`
**Repo:** `/root/.openclaw/workspace/tensorium-core` — work on branch `feature/tensorhash-a3` (created in Task 1, merged in Task 4).

**Reference vectors (computed against the current code before any change):**
- Fixed test job: `chain_id="tensorium-testnet-0"`, `version=1`, `height=7`, `previous_hash=[0x11;32]`, `merkle_root=[0x22;32]`, `timestamp=1_780_000_000`, `difficulty_bits=20`, `nonce=424242` → serialized header is 112 bytes and its **id-hash (double-SHA256)** is
  `dae08fc31e66282b654c853bf490a0677f0fd6f508ff4d1539ca4c3b88f84302`
  (verified independently with Python hashlib; pins ledger block-hash stability across the refactor).

---

### Task 1: `epoch_seed` plumbing (StratumJob, template parsing, notify)

**Files:**
- Modify: `crates/tensorium-pool/src/stratum.rs` (`StratumJob` ~line 39, `fetch_job` ~line 275, `notify_msg` ~line 391; new `parse_job_response`; new `#[cfg(test)] mod tests` at end of file)

- [ ] **Step 1: Create the branch**

```bash
cd /root/.openclaw/workspace/tensorium-core
git checkout -b feature/tensorhash-a3
```

- [ ] **Step 2: Write the failing tests**

Add at the very end of `crates/tensorium-pool/src/stratum.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_template_response(with_epoch_seed: bool) -> String {
        let seed_field = if with_epoch_seed {
            format!("\"epoch_seed\": {:?},", vec![7u8; 32])
        } else {
            String::new()
        };
        format!(
            r#"{{
                "chain_id": "tensorium-testnet-0",
                {seed_field}
                "height": 7,
                "leading_zero_bits": 20,
                "previous_hash": {prev:?},
                "template": {{
                    "header": {{
                        "version": 1,
                        "chain_id": "tensorium-testnet-0",
                        "height": 7,
                        "previous_hash": {prev:?},
                        "merkle_root": {merkle:?},
                        "timestamp_seconds": 1780000000,
                        "leading_zero_bits": 20,
                        "nonce": 0
                    }},
                    "transactions": []
                }},
                "tx_count": 0
            }}"#,
            prev = vec![0x11u8; 32],
            merkle = vec![0x22u8; 32],
        )
    }

    #[test]
    fn parse_job_response_reads_epoch_seed() {
        let (job, _raw) = parse_job_response(&fixture_template_response(true))
            .expect("template with epoch_seed must parse");
        assert_eq!(job.epoch_seed, [7u8; 32]);
        assert_eq!(job.height, 7);
        assert_eq!(job.chain_id, "tensorium-testnet-0");
    }

    #[test]
    fn parse_job_response_rejects_template_without_epoch_seed() {
        // A node that does not send epoch_seed is too old for TensorHash —
        // the pool must refuse the job rather than guess a seed.
        assert!(parse_job_response(&fixture_template_response(false)).is_none());
    }

    #[test]
    fn notify_msg_carries_epoch_seed_hex() {
        let job = StratumJob {
            job_id: "h7-test".into(),
            chain_id: "tensorium-testnet-0".into(),
            height: 7,
            previous_hash: [0x11; 32],
            merkle_root: [0x22; 32],
            epoch_seed: [7u8; 32],
            timestamp: 1_780_000_000,
            difficulty_bits: 20,
            version: 1,
        };
        let msg = notify_msg(&job, 1 << 28);
        assert_eq!(
            msg["params"]["epoch_seed"].as_str().unwrap(),
            "07".repeat(32),
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tensorium-pool 2>&1 | tail -5`
Expected: compile error — `epoch_seed` field and `parse_job_response` don't exist.

- [ ] **Step 4: Implement**

(a) `StratumJob` gains the field (after `merkle_root`):

```rust
    pub merkle_root:     [u8; 32],
    pub epoch_seed:      [u8; 32],
    pub timestamp:       u64,
```

(b) Split `fetch_job` so parsing is testable. Replace the body of `fetch_job` and add `parse_job_response` directly below it:

```rust
/// Fetch a job from the node.  Returns `(job, raw_template_json)` on success.
pub fn fetch_job(node_rpc: &str, treasury: &str) -> Option<(StratumJob, String)> {
    let url  = format!("http://{}/getblocktemplate/{}", node_rpc, treasury);
    let resp = http_get_body(&url)?;
    parse_job_response(&resp)
}

/// Parse a /getblocktemplate response body into a StratumJob.
/// Returns None (and logs) when the node does not send `epoch_seed` —
/// such a node predates TensorHash v1 and cannot be pool-mined against.
fn parse_job_response(resp: &str) -> Option<(StratumJob, String)> {
    let v: Value = serde_json::from_str(resp).ok()?;

    let hdr       = v["template"]["header"].as_object()?;
    let chain_id  = hdr["chain_id"].as_str()?.to_string();
    let height    = hdr["height"].as_u64()?;
    let diff_bits = hdr["leading_zero_bits"].as_u64()? as u8;
    let timestamp = hdr["timestamp_seconds"].as_u64()?;
    let version   = hdr["version"].as_u64().unwrap_or(1) as u32;
    let prev      = parse_byte_array(hdr.get("previous_hash")?)?;
    let mroot     = parse_byte_array(hdr.get("merkle_root")?)?;

    // Top-level field, added by the node in Phase A2.
    let epoch_seed = match v.get("epoch_seed").and_then(parse_byte_array) {
        Some(seed) => seed,
        None => {
            eprintln!(
                "[stratum] node template has no epoch_seed — node too old \
                 for TensorHash v1, upgrade tensorium-node"
            );
            return None;
        }
    };

    let ms     = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_millis();
    let job_id = format!("h{}-{}", height, ms);

    let job = StratumJob { job_id, chain_id, height, previous_hash: prev,
                           merkle_root: mroot, epoch_seed, timestamp,
                           difficulty_bits: diff_bits, version };
    Some((job, resp.to_string()))
}
```

Note: `parse_byte_array` takes `&Value` — `and_then(parse_byte_array)` works because `v.get(...)` yields `Option<&Value>`. If the existing signature differs, adapt the call, not the helper.

(c) `notify_msg` gains the seed (after `merkle_root`):

```rust
            "merkle_root":      bytes_to_hex(&job.merkle_root),
            "epoch_seed":       bytes_to_hex(&job.epoch_seed),
            "timestamp":        job.timestamp,
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p tensorium-pool 2>&1 | tail -5`
Expected: the 3 new tests PASS (plus existing pool tests, if any).

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-pool/src/stratum.rs
git commit -m "feat(pool): parse epoch_seed from node templates and forward it in mining.notify"
```

---

### Task 2: share validation via the consensus `BlockHeader`

**Files:**
- Modify: `crates/tensorium-pool/src/stratum.rs` (imports ~line 9; delete `sha256d` ~181 and `build_header` ~199; `validate_share` ~231; block id-hash ~582; tests)
- Modify: `crates/tensorium-pool/Cargo.toml` (drop `sha2`)

- [ ] **Step 1: Write the failing tests**

Add inside the existing `mod tests` from Task 1:

```rust
    fn fixture_job() -> StratumJob {
        StratumJob {
            job_id: "h7-test".into(),
            chain_id: "tensorium-testnet-0".into(),
            height: 7,
            previous_hash: [0x11; 32],
            merkle_root: [0x22; 32],
            epoch_seed: [0u8; 32],
            timestamp: 1_780_000_000,
            difficulty_bits: 20,
            version: 1,
        }
    }

    /// LE-hex encoding of a nonce, as miners submit it.
    fn nonce_to_le_hex(nonce: u64) -> String {
        nonce.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// CPU-mine the fixture job at `bits` leading zeros (zero epoch seed).
    /// At 12 bits this is ~4k pow_hash calls — fast even unoptimized.
    fn mine_fixture_nonce(bits: u8) -> u64 {
        let job = fixture_job();
        for nonce in 0u64.. {
            let hash = job_header(&job, nonce).pow_hash(Hash256(job.epoch_seed));
            if leading_zero_bits(&hash.0) >= bits {
                return nonce;
            }
        }
        unreachable!()
    }

    #[test]
    fn validate_share_accepts_tensorhash_share() {
        let job = fixture_job();
        let nonce = mine_fixture_nonce(12);
        let (zeros, is_share, _) =
            validate_share(&job, &nonce_to_le_hex(nonce), 12).expect("nonce parses");
        assert!(zeros >= 12);
        assert!(is_share);
    }

    #[test]
    fn validate_share_rejects_wrong_nonce_and_wrong_seed() {
        let job = fixture_job();
        let nonce = mine_fixture_nonce(12);

        // Neighbouring nonce: fails the 12-bit share check (deterministic
        // for these fixed inputs; a priori odds of passing were 2^-12).
        let (_, is_share, _) =
            validate_share(&job, &nonce_to_le_hex(nonce + 1), 12).unwrap();
        assert!(!is_share, "nonce+1 must not satisfy the share target");

        // Same nonce under a different epoch seed: different dataset,
        // different pow hash — must also fail.
        let mut other_seed_job = job.clone();
        other_seed_job.epoch_seed = [9u8; 32];
        let (_, is_share, _) =
            validate_share(&other_seed_job, &nonce_to_le_hex(nonce), 12).unwrap();
        assert!(!is_share, "share must be bound to the epoch seed");
    }

    #[test]
    fn job_header_id_hash_matches_pre_refactor_vector() {
        // Pins ledger block-hash stability: BlockHeader::hash() (double-
        // SHA256) must equal what sha256d(build_header(..)) produced before
        // this refactor. Vector computed against the pre-change code and
        // cross-checked with Python hashlib.
        let header = job_header(&fixture_job(), 424242);
        assert_eq!(
            bytes_to_hex(&header.hash().0),
            "dae08fc31e66282b654c853bf490a0677f0fd6f508ff4d1539ca4c3b88f84302",
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-pool 2>&1 | tail -5`
Expected: compile error — `job_header` doesn't exist.

- [ ] **Step 3: Implement**

(a) Imports: replace the `sha2` line (`use sha2::{Digest, Sha256};`) with:

```rust
use tensorium_core::{block::BlockHeader, hash::Hash256};
```

(adjust the module paths to however `tensorium-core` re-exports them — check `crates/tensorium-core/src/lib.rs`; `use tensorium_core::{BlockHeader, Hash256};` if they are re-exported at the crate root).

(b) Delete the `sha256d` function (~line 181) and the whole `build_header` function (~line 199). Keep `leading_zero_bits` unchanged.

(c) Add the consensus-header builder where `build_header` was:

```rust
// ── Consensus header builder ──────────────────────────────────────────────────

/// Build the consensus `BlockHeader` for a job + nonce. Using the consensus
/// type means the pool's PoW check and ledger id-hash can never drift from
/// the node's serialization.
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
```

(d) `validate_share` switches to TensorHash:

```rust
fn validate_share(
    job:              &StratumJob,
    nonce_hex:        &str,
    worker_diff_bits: u8,
) -> Option<(u8, bool, bool)> {
    let nonce  = le_hex_to_u64(nonce_hex)?;
    let hash   = job_header(job, nonce).pow_hash(Hash256(job.epoch_seed));
    let zeros  = leading_zero_bits(&hash.0);
    let is_share = zeros >= worker_diff_bits;
    let is_block = zeros >= job.difficulty_bits;
    Some((zeros, is_share, is_block))
}
```

(e) The ledger block id-hash (~line 582) — replace:

```rust
                                    let hash_bytes = sha256d(&build_header(job, nonce));
                                    let block_hash = bytes_to_hex(&hash_bytes);
```

with:

```rust
                                    let block_hash = bytes_to_hex(&job_header(job, nonce).hash().0);
```

(f) `Cargo.toml`: delete the `sha2 = "0.10"` line (grep first —
`grep -rn "sha2\|Sha256" crates/tensorium-pool/src/` must show no remaining
uses).

(g) `BlockHeader`'s fields must be public for (c) to compile — they are
(`pub` struct with `pub` fields in `crates/tensorium-core/src/block.rs`).
`Hash256` is a `pub` tuple struct (`Hash256(pub [u8; 32])`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p tensorium-pool 2>&1 | tail -5`
Expected: all pool tests PASS, including the pre-refactor id-hash pin.

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-pool/src/stratum.rs crates/tensorium-pool/Cargo.toml Cargo.lock
git commit -m "feat(pool): validate shares with TensorHash v1 via the consensus BlockHeader"
```

---

### Task 3: share-difficulty defaults for TensorHash rates

**Files:**
- Modify: `crates/tensorium-pool/src/main.rs` (~line 151 default; ~line 173 banner)
- Modify: `crates/tensorium-pool/src/stratum.rs` (vardiff constants ~lines 6, 31–34)

- [ ] **Step 1: `main.rs` default 2^20 → 2^28**

```rust
    let share_diff: u64 = std::env::var("TENSORIUM_POOL_SHARE_DIFF")
        .ok()
        .and_then(|s| s.parse().ok())
        // 2^28 ≈ 49 shares/min for a 220 MH/s GPU (RTX 5090, measured) —
        // inside the vardiff 15–60/min target band from the first second.
        .unwrap_or(268_435_456);
```

and the banner line (vardiff upper bound changes in Step 2):

```rust
    println!("  share_diff   = {} ({}bits, vardiff 16–40bits target {}-{}/min)",
             share_diff, stratum::diff_to_bits(share_diff),
             stratum::VARDIFF_TARGET_MIN, stratum::VARDIFF_TARGET_MAX);
```

- [ ] **Step 2: `stratum.rs` vardiff bound + stale comments**

`VARDIFF_MAX_BITS` and its doc comment:

```rust
/// Maximum per-worker share difficulty.  2 bits below MAINNET network diff
/// (42) so there is always a gap between "valid share" and "valid block".
pub const VARDIFF_MAX_BITS: u8 = 40;
```

and the module header comment (line ~6): change
`Bounds: 16 bit (min) … 38 bit (max, 2 below` →
`Bounds: 16 bit (min) … 40 bit (max, 2 below`.

- [ ] **Step 3: Build + test**

Run: `cargo test -p tensorium-pool 2>&1 | tail -3 && cargo clippy -p tensorium-pool 2>&1 | tail -2`
Expected: tests PASS; clippy no hard errors.

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-pool/src/main.rs crates/tensorium-pool/src/stratum.rs
git commit -m "feat(pool): retune share-diff defaults for TensorHash MH/s hashrates (2^28 initial, vardiff cap 40)"
```

---

### Task 4: validation sweep + merge + push

**Files:** none (verification)

- [ ] **Step 1: Workspace gates**

```bash
cargo test --workspace 2>&1 | grep "test result" | awk -F'[. ;]+' '{p+=$4; f+=$6} END {print "TOTAL passed="p" failed="f}'
cargo clippy --workspace --all-targets 2>&1 | tail -2
```
Expected: failed=0 (≈226 passed: 220 prior + 6 new pool tests); clippy warnings only.

- [ ] **Step 2: Merge + push**

```bash
git checkout main
git merge --ff-only feature/tensorhash-a3
git push origin main
git branch -d feature/tensorhash-a3
```

- [ ] **Step 3: Record the launch-checklist item**

Append to `docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md`:

```markdown

## Pre-launch checklist addition (Phase A3, 2026-06-10)

Pool↔GPU live test — bundle with the genesis re-mine rental: devnet node +
`tensorium-pool` + `tensorium-miner --mode pool`; verify the miner receives
`epoch_seed` in `mining.notify`, shares are accepted at the vardiff target
rate, a found block is submitted and accepted by the node, and the PPLNS
split is recorded in the ledger.
```

```bash
git add docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md
git commit -m "docs: add pool live-test to the pre-launch checklist (Phase A3)"
git push origin main
```

---

## Self-Review Notes

- **Spec coverage:** §1 consensus-type validation → Task 2; §2 epoch_seed plumbing → Task 1; §3 defaults → Task 3; §4 tests 1–5 → Tasks 1–2 (test 5 = id-hash pin), GPU live test → Task 4 Step 3 checklist. Out-of-scope untouched.
- **Vector provenance:** the id-hash pin was computed against the current (pre-refactor) serialization and cross-checked with an independent Python implementation before writing this plan.
- **Type consistency:** `job_header(&StratumJob, u64) -> BlockHeader` used identically in Tasks 1–2 tests and implementation; `parse_job_response(&str) -> Option<(StratumJob, String)>` matches its callers.
