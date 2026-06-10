# TensorHash v1 — Phase A1 (Algorithm + Core Integration) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SHA256d proof-of-work with TensorHash v1 (a memory-hard,
GPU-first algorithm) at the consensus layer, fully tested via `cargo test`,
with no CPU-mining production tooling left in the workspace.

**Architecture:** A new pure-Rust crate `tensorium-tensorhash` implements the
TensorHash v1 hash function (dataset-element function + TensorMix mixing).
`tensorium-core` gains a `BlockHeader::pow_hash(epoch_seed)` method that calls
into this crate, used only by `pow::header_meets_work`. The existing
`BlockHeader::hash()` (double-SHA256, used for chain linkage/merkle/storage)
is untouched. `crates/txmminer` (CPU miner) is deleted.

**Tech Stack:** Rust 1.76, Cargo workspace, `blake2` crate (already in
`Cargo.lock` as a transitive dependency).

**Out of scope (separate plans):** CUDA miner kernel (`tools/tensorium-miner`,
needs GPU hardware not available in this environment), pool share validation
(Phase A3), new genesis/chain ID/tokenomics (Phase B).

**Reference design doc:** `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a-design.md`

---

### Task 1: Scaffold the `tensorium-tensorhash` crate

**Files:**
- Create: `crates/tensorium-tensorhash/Cargo.toml`
- Create: `crates/tensorium-tensorhash/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add `blake2` to workspace dependencies and register the new crate**

In `Cargo.toml` (workspace root), add `"crates/tensorium-tensorhash"` to
`members` (keep alphabetical-ish grouping, just add it near
`tensorium-core`):

```toml
[workspace]
members = [
    "crates/tensorium-core",
    "crates/tensorium-tensorhash",
    "crates/tensorium-node",
    "crates/txmwallet",
    "crates/txmminer",
    "crates/tensorium-pool",
    "crates/txm-rpc-adapter",
    "crates/tensorium-indexer",
]
```

(`crates/txmminer` is removed in Task 9 — leave it for now so the workspace
still builds during this task.)

Add `blake2` to `[workspace.dependencies]`:

```toml
[workspace.dependencies]
argon2 = "0.5"
base64ct = "=1.6.0"
bech32 = "0.9"
blake2 = "0.10"
chacha20poly1305 = "0.10"
hex = "0.4"
k256 = { version = "0.13", features = ["ecdsa"] }
rand_core = { version = "0.6", features = ["getrandom"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "1"
```

- [ ] **Step 2: Create the crate's `Cargo.toml`**

```toml
[package]
name = "tensorium-tensorhash"
version = "0.1.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]
blake2.workspace = true
```

- [ ] **Step 3: Create a placeholder `src/lib.rs` so the workspace builds**

```rust
//! TensorHash v1 — Tensorium's memory-hard, GPU-first proof-of-work
//! algorithm. Pure-Rust reference implementation used by `tensorium-core`
//! for block validation (light verification — see crate docs in
//! `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a-design.md`).

pub const ELEMENT_SIZE: usize = 32;
pub const DATASET_N: u64 = 600_000_000;
pub const EPOCH_LENGTH: u64 = 8_192;
pub const K: usize = 32;
```

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo build --workspace`
Expected: builds successfully (the new crate is currently unused, that's fine).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/tensorium-tensorhash
git commit -m "feat(tensorhash): scaffold tensorium-tensorhash crate"
```

---

### Task 2: Implement `dataset_element`

**Files:**
- Modify: `crates/tensorium-tensorhash/src/lib.rs`

- [ ] **Step 1: Add the dataset element function and a determinism test**

Append to `crates/tensorium-tensorhash/src/lib.rs`:

```rust
use blake2::digest::{consts::U32, Digest};
use blake2::Blake2b;

type Blake2b256 = Blake2b<U32>;

/// One element of the TensorHash dataset for the given epoch seed.
///
/// Computable on demand — this is what makes verification cheap (a verifier
/// recomputes only the `K` elements a given attempt touches) while mining is
/// memory-hard (a miner materializes all `DATASET_N` elements into VRAM
/// because recomputing per-attempt is ~`K`x slower).
pub fn dataset_element(epoch_seed: &[u8; 32], index: u64) -> [u8; 32] {
    let mut hasher = Blake2b256::new();
    hasher.update(epoch_seed);
    hasher.update(&index.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_element_is_deterministic() {
        let seed = [0u8; 32];
        assert_eq!(dataset_element(&seed, 42), dataset_element(&seed, 42));
    }

    #[test]
    fn dataset_element_differs_by_index() {
        let seed = [0u8; 32];
        assert_ne!(dataset_element(&seed, 0), dataset_element(&seed, 1));
    }

    #[test]
    fn dataset_element_differs_by_seed() {
        let a = dataset_element(&[0u8; 32], 7);
        let b = dataset_element(&[1u8; 32], 7);
        assert_ne!(a, b);
    }

    #[test]
    fn dataset_element_zero_zero_known_value() {
        // Locks the exact byte layout (Blake2b-256 of 32 zero bytes ||
        // 8 zero bytes for index 0) — this is the cross-check value the
        // future CUDA implementation's --selftest must reproduce.
        let seed = [0u8; 32];
        let elem = dataset_element(&seed, 0);
        let hex = hex::encode(elem);
        println!("dataset_element([0;32], 0) = {hex}");
        // Computed by Blake2b-256("\x00"*32 || "\x00"*8):
        assert_eq!(
            hex,
            "1c0b5ee32cb55626b1aa72b2dd29c0aa70eaaedfddc18ab0c97a8aef1bf17e94"
        );
    }
}
```

Note: `hex` is not yet a dependency of this crate — add it.

- [ ] **Step 2: Add `hex` (dev-dependency) to the crate's `Cargo.toml`**

```toml
[dependencies]
blake2.workspace = true

[dev-dependencies]
hex.workspace = true
```

- [ ] **Step 3: Run the tests — the known-value test will likely fail first time**

Run: `cargo test -p tensorium-tensorhash -- --nocapture`

The first three tests should pass. `dataset_element_zero_zero_known_value`
may fail because the placeholder hex string above is a guess. If it fails,
copy the **actual** printed value from the `--nocapture` output
(`dataset_element([0;32], 0) = <actual_hex>`) and replace the `assert_eq!`
expected string with that exact value.

- [ ] **Step 4: Re-run until green**

Run: `cargo test -p tensorium-tensorhash`
Expected: all tests pass (4/4).

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-tensorhash
git commit -m "feat(tensorhash): implement dataset_element"
```

---

### Task 3: Implement `pow_hash` (full TensorHash v1 algorithm)

**Files:**
- Modify: `crates/tensorium-tensorhash/src/lib.rs`

- [ ] **Step 1: Add the helper conversions and the `pow_hash` function**

Append to `crates/tensorium-tensorhash/src/lib.rs` (outside the `tests`
module, alongside `dataset_element`):

```rust
fn bytes_to_u64x4(bytes: &[u8; 32]) -> [u64; 4] {
    let mut out = [0u64; 4];
    for (i, word) in out.iter_mut().enumerate() {
        *word = u64::from_le_bytes(bytes[i * 8..(i + 1) * 8].try_into().unwrap());
    }
    out
}

fn u64x4_to_bytes(words: &[u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// TensorHash v1 proof-of-work hash.
///
/// `header_bytes` is the nonce-independent serialized header prefix
/// (`BlockHeader::pow_prefix_bytes` in `tensorium-core`). `epoch_seed` is the
/// dataset seed for the block's epoch (id-hash of the last block of the
/// previous epoch; `[0u8; 32]` for epoch 0).
///
/// Algorithm:
/// 1. `digest = Blake2b256(header_bytes || nonce_le)`
/// 2. Initialize a 4xu64 accumulator from `digest`.
/// 3. For `j in 0..K`: derive an index from `Blake2b256(digest || j_le)`,
///    look up `dataset_element(epoch_seed, idx)`, and fold it into the
///    accumulator via the TensorMix multiply-rotate-add step.
/// 4. Return `Blake2b256(header_bytes || nonce_le || acc_bytes)`.
pub fn pow_hash(header_bytes: &[u8], nonce: u64, epoch_seed: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Blake2b256::new();
    hasher.update(header_bytes);
    hasher.update(&nonce.to_le_bytes());
    let digest_full = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&digest_full);

    let mut acc = bytes_to_u64x4(&digest);

    for j in 0..K as u64 {
        let mut h = Blake2b256::new();
        h.update(&digest);
        h.update(&j.to_le_bytes());
        let idx_seed = h.finalize();
        let idx = u64::from_le_bytes(idx_seed[0..8].try_into().unwrap()) % DATASET_N;

        let elem = bytes_to_u64x4(&dataset_element(epoch_seed, idx));
        let mut next = [0u64; 4];
        for m in 0..4 {
            next[m] = acc[m]
                .wrapping_mul(elem[m] | 1)
                .wrapping_add(elem[(m + 1) % 4].rotate_left(13));
        }
        acc = next;
    }

    let acc_bytes = u64x4_to_bytes(&acc);
    let mut h = Blake2b256::new();
    h.update(header_bytes);
    h.update(&nonce.to_le_bytes());
    h.update(&acc_bytes);
    let final_full = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&final_full);
    out
}
```

- [ ] **Step 2: Add tests — determinism, sensitivity, and a KAT**

Add to the `tests` module in `crates/tensorium-tensorhash/src/lib.rs`:

```rust
#[test]
fn pow_hash_is_deterministic() {
    let seed = [0u8; 32];
    let header = b"test-header-bytes";
    assert_eq!(pow_hash(header, 0, &seed), pow_hash(header, 0, &seed));
}

#[test]
fn pow_hash_changes_with_nonce() {
    let seed = [0u8; 32];
    let header = b"test-header-bytes";
    assert_ne!(pow_hash(header, 0, &seed), pow_hash(header, 1, &seed));
}

#[test]
fn pow_hash_changes_with_epoch_seed() {
    let header = b"test-header-bytes";
    let a = pow_hash(header, 0, &[0u8; 32]);
    let b = pow_hash(header, 0, &[1u8; 32]);
    assert_ne!(a, b);
}

#[test]
fn pow_hash_known_answer_vector() {
    // Locks down the full algorithm output for a fixed input — this is the
    // primary cross-check value the future CUDA --selftest must reproduce
    // bit-for-bit.
    let seed = [0u8; 32];
    let header = b"tensorhash-v1-kat-vector";
    let hash = pow_hash(header, 12345, &seed);
    let hex = hex::encode(hash);
    println!("pow_hash KAT = {hex}");
    assert_eq!(
        hex,
        "0000000000000000000000000000000000000000000000000000000000000000"
    );
}
```

- [ ] **Step 3: Run tests, capture the real KAT value, and fix the assertion**

Run: `cargo test -p tensorium-tensorhash -- --nocapture`

`pow_hash_is_deterministic`, `pow_hash_changes_with_nonce`, and
`pow_hash_changes_with_epoch_seed` should pass immediately.
`pow_hash_known_answer_vector` will fail (the placeholder hex above is the
wrong length/value on purpose). Copy the printed `pow_hash KAT = <hex>`
value (64 hex chars) and replace the `assert_eq!` expected string with it.

- [ ] **Step 4: Re-run until green**

Run: `cargo test -p tensorium-tensorhash`
Expected: all tests pass (8/8).

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-tensorhash
git commit -m "feat(tensorhash): implement pow_hash with TensorMix mixing"
```

---

### Task 4: Wire `BlockHeader::pow_hash` into `tensorium-core`

**Files:**
- Modify: `crates/tensorium-core/Cargo.toml`
- Modify: `crates/tensorium-core/src/block.rs`

- [ ] **Step 1: Add the crate dependency**

In `crates/tensorium-core/Cargo.toml`, add to `[dependencies]`:

```toml
tensorium-tensorhash = { path = "../tensorium-tensorhash" }
```

- [ ] **Step 2: Add `pow_prefix_bytes` and `pow_hash` to `BlockHeader`**

In `crates/tensorium-core/src/block.rs`, inside `impl BlockHeader` (after the
existing `hash()` method, around line 159), add:

```rust
    /// Serialized header bytes excluding `nonce` — the nonce-independent
    /// prefix fed into TensorHash's `pow_hash`. Mirrors the field order of
    /// `hash()` minus the trailing nonce bytes.
    fn pow_prefix_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(120);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(self.chain_id.as_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.previous_hash.0);
        bytes.extend_from_slice(&self.merkle_root.0);
        bytes.extend_from_slice(&self.timestamp_seconds.to_le_bytes());
        bytes.push(self.leading_zero_bits);
        bytes
    }

    /// TensorHash v1 proof-of-work hash for this header, given the dataset
    /// epoch seed for the epoch containing `self.height`. Used only by
    /// `pow::header_meets_work` — chain linkage, merkle roots, and storage
    /// keys continue to use `hash()` (double-SHA256), unaffected by this.
    pub fn pow_hash(&self, epoch_seed: Hash256) -> Hash256 {
        let prefix = self.pow_prefix_bytes();
        Hash256(tensorium_tensorhash::pow_hash(&prefix, self.nonce, &epoch_seed.0))
    }
```

- [ ] **Step 3: Add a test**

In the `#[cfg(test)] mod tests` block of `crates/tensorium-core/src/block.rs`
(create one if it doesn't exist — check the end of the file first), add:

```rust
#[cfg(test)]
mod pow_hash_tests {
    use super::*;

    #[test]
    fn pow_hash_differs_from_id_hash() {
        let header = BlockHeader {
            version: 1,
            chain_id: "tensorium-testnet-0".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1_700_000_000,
            leading_zero_bits: 8,
            nonce: 0,
        };
        assert_ne!(header.hash(), header.pow_hash(Hash256::ZERO));
    }

    #[test]
    fn pow_hash_changes_with_nonce() {
        let mut header = BlockHeader {
            version: 1,
            chain_id: "tensorium-testnet-0".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1_700_000_000,
            leading_zero_bits: 8,
            nonce: 0,
        };
        let h0 = header.pow_hash(Hash256::ZERO);
        header.nonce = 1;
        let h1 = header.pow_hash(Hash256::ZERO);
        assert_ne!(h0, h1);
    }
}
```

- [ ] **Step 4: Build and test**

Run: `cargo test -p tensorium-core block::`
Expected: both new tests pass. (The crate as a whole won't fully compile yet
— `pow.rs`, `validation.rs`, `state.rs` still call the old signatures. That's
fixed in the next tasks. If `cargo test -p tensorium-core block::` fails to
even compile due to those other files, that's expected at this point —
proceed to Task 5.)

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/Cargo.toml crates/tensorium-core/src/block.rs
git commit -m "feat(core): add BlockHeader::pow_hash via tensorium-tensorhash"
```

---

### Task 5: Update `pow.rs` to use TensorHash

**Files:**
- Modify: `crates/tensorium-core/src/pow.rs`

- [ ] **Step 1: Replace `header_meets_work` and `mine_header`**

Replace the full contents of `crates/tensorium-core/src/pow.rs` with:

```rust
use crate::{block::BlockHeader, hash::Hash256};

pub fn hash_meets_work(hash: Hash256, leading_zero_bits: u8) -> bool {
    hash.leading_zero_bits() >= u32::from(leading_zero_bits)
}

/// Checks whether `header` satisfies its declared `leading_zero_bits` target
/// under TensorHash v1, given the dataset `epoch_seed` for its epoch.
pub fn header_meets_work(header: &BlockHeader, epoch_seed: Hash256) -> bool {
    hash_meets_work(header.pow_hash(epoch_seed), header.leading_zero_bits)
}

/// Brute-force nonce search (used by tests and the node's CPU devnet mining
/// path — TEST_PARAMS difficulty only). Production GPU mining lives in
/// `tools/tensorium-miner`.
pub fn mine_header(mut header: BlockHeader, epoch_seed: Hash256, max_nonce: u64) -> Option<BlockHeader> {
    for nonce in 0..=max_nonce {
        header.nonce = nonce;
        if header_meets_work(&header, epoch_seed) {
            return Some(header);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_zero_work_check_is_monotonic() {
        let hash = Hash256([0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(hash_meets_work(hash, 16));
        assert!(!hash_meets_work(hash, 17));
    }

    #[test]
    fn mine_header_finds_a_satisfying_nonce_at_low_difficulty() {
        let header = BlockHeader {
            version: 1,
            chain_id: "tensorium-testnet-0".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1_700_000_000,
            leading_zero_bits: 8,
            nonce: 0,
        };
        let mined = mine_header(header.clone(), Hash256::ZERO, 100_000)
            .expect("difficulty 8 should be found within 100k nonces");
        assert!(header_meets_work(&mined, Hash256::ZERO));
    }
}
```

- [ ] **Step 2: Build (compile errors in other files are expected for now)**

Run: `cargo build -p tensorium-core 2>&1 | tail -40`
Expected: errors in `validation.rs` and `state.rs` about `header_meets_work`
and `mine_header` argument counts — that's the work for Tasks 6-7.

- [ ] **Step 3: Run the new pow tests in isolation once Task 6/7 are done**

(Defer running `cargo test -p tensorium-core pow::` to the end of Task 7,
once the crate compiles again.)

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-core/src/pow.rs
git commit -m "feat(core): pow.rs uses TensorHash pow_hash"
```

---

### Task 6: Update `validation.rs` to thread `epoch_seed`

**Files:**
- Modify: `crates/tensorium-core/src/validation.rs`

- [ ] **Step 1: Add the `epoch_seed` parameter to `validate_block`**

In `crates/tensorium-core/src/validation.rs`:

1. Add `hash::Hash256` to the `use crate::{...}` import at the top:

```rust
use crate::{
    block::{merkle_root, Block},
    chain::ConsensusParams,
    hash::Hash256,
    pow::header_meets_work,
};
```

2. Change the `validate_block` signature and the PoW check (around lines
31-70):

```rust
pub fn validate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    block: &Block,
    now_seconds: u64,
    expected_leading_zero_bits: u8,
    epoch_seed: Hash256,
) -> Result<(), ValidationError> {
```

and:

```rust
    if !header_meets_work(&block.header, epoch_seed) {
        return Err(ValidationError::InvalidProofOfWork);
    }
```

(the rest of the function body is unchanged).

- [ ] **Step 2: Update the test module's calls**

In the `#[cfg(test)] mod tests` block of the same file, every call to
`validate_block(...)` needs `Hash256::ZERO` appended as the final argument
(all test blocks are at height 0/1, epoch 0, whose seed is always
`Hash256::ZERO`). For example, change:

```rust
        assert_eq!(
            validate_block(&TEST_PARAMS, None, block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits),
            Ok(())
        );
```

to:

```rust
        assert_eq!(
            validate_block(&TEST_PARAMS, None, block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
            Ok(())
        );
```

Apply this same `, Hash256::ZERO)` append to **every** `validate_block(...)`
call in this file's test module (there are 7 — search for
`validate_block(&TEST_PARAMS` and `validate_block(\n` to find them all).

Also, the test `accepts_coinbase_with_fees_above_base_reward` calls
`mine_header(block.header.clone(), 1_000_000)` — update it to
`mine_header(block.header.clone(), Hash256::ZERO, 1_000_000)`.

- [ ] **Step 3: Build**

Run: `cargo build -p tensorium-core 2>&1 | tail -40`
Expected: remaining errors should now only be in `state.rs` (Task 7).

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-core/src/validation.rs
git commit -m "feat(core): validate_block takes TensorHash epoch_seed"
```

---

### Task 7: Add `epoch_seed_for_height` and fix `state.rs` production paths

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Add the `epoch_seed_for_height` method**

In `crates/tensorium-core/src/state.rs`, in `impl ChainState`, right after
`get_block_by_height` (around line 280), add:

```rust
    /// The TensorHash dataset epoch seed for the block at `height`.
    ///
    /// Epoch 0 (heights `0..tensorium_tensorhash::EPOCH_LENGTH`) always uses
    /// the fixed zero seed — there is no prior epoch to derive it from.
    /// Later epochs derive their seed from the id-hash (`Block::hash`) of the
    /// last block of the previous epoch.
    pub fn epoch_seed_for_height(&self, height: u64) -> Hash256 {
        let epoch = height / tensorium_tensorhash::EPOCH_LENGTH;
        if epoch == 0 {
            return Hash256::ZERO;
        }
        let seed_height = epoch * tensorium_tensorhash::EPOCH_LENGTH - 1;
        self.get_block_by_height(seed_height)
            .map(|b| b.hash())
            .unwrap_or(Hash256::ZERO)
    }
```

- [ ] **Step 2: Fix `init_genesis` (around line 289-307)**

Replace:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, None);
        let block = mine_candidate_block(params, None, timestamp_seconds, "genesis", max_nonce, leading_zero_bits)?;
        validate_block(params, None, &block, timestamp_seconds, leading_zero_bits)?;
```

with:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, None);
        let epoch_seed = self.epoch_seed_for_height(0);
        let block = mine_candidate_block(params, None, timestamp_seconds, "genesis", max_nonce, leading_zero_bits, epoch_seed)?;
        validate_block(params, None, &block, timestamp_seconds, leading_zero_bits, epoch_seed)?;
```

- [ ] **Step 3: Fix `init_genesis_nonce` (around line 311-334)**

Replace:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, None);
        let mut block = candidate_block(params, None, timestamp_seconds, "genesis", vec![], 0);
        block.header.leading_zero_bits = leading_zero_bits;
        block.header.nonce = genesis_nonce;
        if !crate::pow::header_meets_work(&block.header) {
            return Err(StateError::MiningFailed);
        }
        validate_block(params, None, &block, timestamp_seconds, leading_zero_bits)?;
```

with:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, None);
        let epoch_seed = self.epoch_seed_for_height(0);
        let mut block = candidate_block(params, None, timestamp_seconds, "genesis", vec![], 0);
        block.header.leading_zero_bits = leading_zero_bits;
        block.header.nonce = genesis_nonce;
        if !crate::pow::header_meets_work(&block.header, epoch_seed) {
            return Err(StateError::MiningFailed);
        }
        validate_block(params, None, &block, timestamp_seconds, leading_zero_bits, epoch_seed)?;
```

- [ ] **Step 4: Fix `mine_next_block` (around line 336-354)**

Replace:

```rust
        let parent = self.tip().ok_or(StateError::MissingGenesis)?.clone();
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, Some(&parent));
        let block = mine_candidate_block(params, Some(&parent), timestamp_seconds, miner, max_nonce, leading_zero_bits)?;
        validate_block(params, Some(&parent), &block, timestamp_seconds, leading_zero_bits)?;
```

with:

```rust
        let parent = self.tip().ok_or(StateError::MissingGenesis)?.clone();
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, Some(&parent));
        let next_height = parent.header.height + 1;
        let epoch_seed = self.epoch_seed_for_height(next_height);
        let block = mine_candidate_block(params, Some(&parent), timestamp_seconds, miner, max_nonce, leading_zero_bits, epoch_seed)?;
        validate_block(params, Some(&parent), &block, timestamp_seconds, leading_zero_bits, epoch_seed)?;
```

- [ ] **Step 5: Fix `submit_block` (around line 462-465)**

Replace:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, Some(&parent));
        validate_block(params, Some(&parent), &block, now_seconds, leading_zero_bits)?;
```

with:

```rust
        let leading_zero_bits = self.expected_leading_zero_bits_for(params, Some(&parent));
        let epoch_seed = self.epoch_seed_for_height(block.header.height);
        validate_block(params, Some(&parent), &block, now_seconds, leading_zero_bits, epoch_seed)?;
```

- [ ] **Step 6: Fix the `mine_candidate_block` private helper (around line 671-684)**

Replace:

```rust
fn mine_candidate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    timestamp_seconds: u64,
    miner: &str,
    max_nonce: u64,
    leading_zero_bits: u8,
) -> Result<Block, StateError> {
    let mut block = candidate_block(params, parent, timestamp_seconds, miner, vec![], 0);
    block.header.leading_zero_bits = leading_zero_bits;
    let header = block.header;
    let mined_header = mine_header(header, max_nonce).ok_or(StateError::MiningFailed)?;
    Ok(Block::new(mined_header, block.transactions))
}
```

with:

```rust
fn mine_candidate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    timestamp_seconds: u64,
    miner: &str,
    max_nonce: u64,
    leading_zero_bits: u8,
    epoch_seed: Hash256,
) -> Result<Block, StateError> {
    let mut block = candidate_block(params, parent, timestamp_seconds, miner, vec![], 0);
    block.header.leading_zero_bits = leading_zero_bits;
    let header = block.header;
    let mined_header = mine_header(header, epoch_seed, max_nonce).ok_or(StateError::MiningFailed)?;
    Ok(Block::new(mined_header, block.transactions))
}
```

- [ ] **Step 7: Build to find the remaining (test-only) call sites**

Run: `cargo build -p tensorium-core 2>&1 | grep -E "^error|-->" | head -60`

Expected: a list of `error[E0061]: this function takes N arguments but M
arguments were supplied` (or similar), each with a `--> src/state.rs:LINE:COL`
location. These are all in the `#[cfg(test)] mod tests` block (around lines
730-1680).

- [ ] **Step 8: Fix every reported call site**

For each error location, the fix is mechanical based on which function is
being called:

- **`mine_header(<header_expr>, <max_nonce>)`** → add `Hash256::ZERO` as the
  second argument: **`mine_header(<header_expr>, Hash256::ZERO, <max_nonce>)`**.
  Example: `mine_header(c1.header.clone(), 10_000_000)` →
  `mine_header(c1.header.clone(), Hash256::ZERO, 10_000_000)`.

- **`validate_block(<params>, <parent>, <block>, <ts>, <bits>)`** → append
  `, Hash256::ZERO`: **`validate_block(<params>, <parent>, <block>, <ts>,
  <bits>, Hash256::ZERO)`**.

- **`header_meets_work(&h)`** (or similar) → append `, Hash256::ZERO`:
  **`header_meets_work(&h, Hash256::ZERO)`**.

All test chains in this file are far shorter than `EPOCH_LENGTH` (8,192
blocks), so every block in every test is in epoch 0, and
`epoch_seed_for_height(_) == Hash256::ZERO` for all of them — `Hash256::ZERO`
is the *correct* value here, not a stub.

After fixing a batch, re-run `cargo build -p tensorium-core 2>&1 | grep -E
"^error|-->" | head -60` and repeat until the error list is empty.

- [ ] **Step 9: Run the full crate test suite**

Run: `cargo test -p tensorium-core`
Expected: all tests pass. If any test fails (not just compile errors), it's
likely a test that mines at a height ≥ `EPOCH_LENGTH` or otherwise crosses an
epoch boundary — re-read that test, compute the epoch seed it should use via
`state.epoch_seed_for_height(height)` instead of `Hash256::ZERO`, and use
that. (Given `TEST_PARAMS` chains in this file are all under a few thousand
blocks, this should not occur, but check.)

- [ ] **Step 10: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "feat(core): thread TensorHash epoch_seed through ChainState"
```

---

### Task 8: Fix `tensorium-node` call sites

**Files:**
- Modify: `crates/tensorium-node/src/main.rs`

- [ ] **Step 1: Build the node crate to find call sites**

Run: `cargo build -p tensorium-node 2>&1 | grep -E "^error|-->" | head -30`

This should report two locations: the `header_meets_work(&h)` call in the CPU
devnet mining loop (around line 374) and the `mine_header(candidate.header.clone(),
1_000_000)` call in the `sync_blocks_walks_back_to_find_common_ancestor_on_fork_below_tip`
test (around line 2317).

- [ ] **Step 2: Fix the mining loop (around line 374)**

This loop mines genesis/devnet blocks at a fixed difficulty within epoch 0
(genesis is height 0). Change:

```rust
                    if header_meets_work(&h) {
```

to:

```rust
                    if header_meets_work(&h, Hash256::ZERO) {
```

If `Hash256` is not already imported at the top of the file, add it to the
existing `tensorium_core::{...}` import block (check `cargo build` output —
if it reports `cannot find type Hash256`, add `Hash256` to that import list;
it is likely already imported since `Hash256::ZERO` is used elsewhere in this
file for `previous_hash`).

- [ ] **Step 3: Fix the test call site (around line 2317)**

Change:

```rust
                let header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
```

to:

```rust
                let header = mine_header(candidate.header.clone(), Hash256::ZERO, 1_000_000).unwrap();
```

(`Hash256` should already be in scope in this test module via the crate's
top-level imports — if not, add `use tensorium_core::hash::Hash256;` to the
`use tensorium_core::{chain::TEST_PARAMS, pow::mine_header};` line on the
preceding line, making it `use tensorium_core::{chain::TEST_PARAMS, hash::Hash256, pow::mine_header};`.)

- [ ] **Step 4: Build and test**

Run: `cargo build -p tensorium-node 2>&1 | tail -20`
Expected: builds cleanly.

Run: `cargo test -p tensorium-node`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): pass TensorHash epoch_seed to pow checks"
```

---

### Task 9: Remove `crates/txmminer`

**Files:**
- Delete: `crates/txmminer/`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Remove the crate from the workspace members list**

In `Cargo.toml` (workspace root), remove the `"crates/txmminer",` line from
`members`.

- [ ] **Step 2: Delete the crate directory**

```bash
git rm -r crates/txmminer
```

- [ ] **Step 3: Build the full workspace**

Run: `cargo build --workspace 2>&1 | tail -20`
Expected: builds cleanly with no references to `txmminer` remaining.

- [ ] **Step 4: Search for any leftover references**

Run: `grep -rn "txmminer" --include=*.toml --include=*.rs --include=*.md . 2>/dev/null`
Expected: no matches (or only in `docs/superpowers/specs/` historical specs,
which is fine — don't edit those).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove txmminer (CPU mining retired, GPU-only via tools/tensorium-miner)"
```

---

### Task 10: Full workspace verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace 2>&1 | tail -60`
Expected: all crates build and all tests pass.

- [ ] **Step 2: Run clippy to catch obvious issues**

Run: `cargo clippy --workspace --all-targets 2>&1 | tail -60`
Expected: no new errors (warnings pre-existing in the codebase are fine; do
not fix unrelated warnings as part of this plan).

- [ ] **Step 3: Confirm the KAT values are recorded**

Run: `cargo test -p tensorium-tensorhash -- --nocapture 2>&1 | grep -E "dataset_element|pow_hash KAT"`
Expected: prints the two KAT hex values now hardcoded in
`crates/tensorium-tensorhash/src/lib.rs` — these are the values the future
CUDA `--selftest` (Phase A2) must reproduce.

- [ ] **Step 4: Final commit (if any cleanup was needed)**

If steps 1-3 required no further changes, this task is just verification —
no commit needed. Otherwise commit any fixes with a descriptive message.
