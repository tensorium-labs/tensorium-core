# TensorHash v1 — Phase B (Genesis & Tokenomics v2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reconfigure `tensorium-core`'s mainnet consensus parameters for a clean, zero-premine, mining-only TXM relaunch under TensorHash v1, renaming `MAINNET_CANDIDATE` → `MAINNET` (`chain_id` `"tensorium-mainnet"`).

**Architecture:** This is a parameter/constant-rename change confined almost entirely to `crates/tensorium-core/src/chain.rs` (the `ConsensusParams::mainnet()` definition and the `MAINNET` const), with mechanical `MAINNET_CANDIDATE` → `MAINNET` token renames in the handful of other crates that reference the const. No new modules, no algorithm changes, no difficulty-logic changes — `difficulty.rs`'s existing `expected_leading_zero_bits` already supports `difficulty_retarget_activation_height: 0`.

**Tech Stack:** Rust, Cargo workspace, existing `tensorium-core` test suite (`cargo test`).

**Spec:** `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-b-design.md`

**Scope note (narrowed from the design doc's "rename throughout" framing):** The design doc's section 1 lists every file that references the `MAINNET_CANDIDATE` const, and all of those files DO need their `MAINNET_CANDIDATE` → `MAINNET` token usages updated (this plan covers all of them). However, CLI subcommand names (`mainnet-candidate` / `mc`), environment variable names (`TENSORIUM_MC_PEERS`, etc.), on-disk file path constants (`tensorium-mc-state.json`, etc.), and helper function names containing `mc`/`mainnet_candidate` (`mc_state_path_from_env`, `init_mainnet_candidate_state`, `peers_for_mainnet_candidate_uses_mc_peer_list`, etc.) are **left unchanged** — these are operational/deployment surface (systemd units, env files on the live VPS) and renaming them is a separate, deployment-coordinated change for a later phase. Only the two genesis-placeholder constants (`MC_GENESIS_TIMESTAMP` / `MC_GENESIS_NONCE`) are renamed, per the design doc's explicit instruction, since they are internal to `main.rs` and carry new TODO/placeholder semantics.

---

### Task 1: `chain.rs` — chain identity rename, zero-premine tokenomics, new emission & difficulty constants

**Files:**
- Modify: `crates/tensorium-core/src/chain.rs`

This task makes `crates/tensorium-core/src/chain.rs` the single source of truth for the new `MAINNET` config. It's done as one task because the constants, the `ConsensusParams::mainnet()` body, and this file's own tests must all change together to compile and pass.

- [ ] **Step 1: Update the top-of-file constants block (lines 3-21)**

Replace:

```rust
pub const COIN: u64 = 100_000_000;
pub const MAX_HALVING_ERAS: u32 = 10;
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;
/// Total genesis pre-mint (all allocation buckets combined).
pub const GENESIS_PRE_MINT_COINS: u64 = 8_000_000;
pub const MINING_ALLOCATION_COINS: u64 = 25_000_000;
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN;
/// Kept for backward-compat; equals GENESIS_PRE_MINT_COINS * COIN.
pub const FOUNDER_ALLOCATION_ATOMS: u64 = GENESIS_PRE_MINT_COINS * COIN;
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;

/// Genesis allocation buckets — (address, atoms). All minted at block 0.
/// Founder 1M | Liquidity pool 3M | Bridge reserve 2M | Ecosystem 2M = 8M total.
pub const MC_GENESIS_ALLOCATIONS: &[(&str, u64)] = &[
    ("txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d", 1_000_000 * COIN), // founder
    ("txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p", 3_000_000 * COIN), // liquidity pool
    ("txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq", 2_000_000 * COIN), // bridge reserve
    ("txm1jwz2nvfajy84kyypzxp0pq8n5vrwahu6yny9hf", 2_000_000 * COIN), // ecosystem/treasury
];
```

with:

```rust
pub const COIN: u64 = 100_000_000;
pub const MAX_HALVING_ERAS: u32 = 10;
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN;
/// Zero premine: the entire max supply is mining-only issuance.
pub const MINING_ALLOCATION_COINS: u64 = TOTAL_SUPPLY_COINS;
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;
```

- [ ] **Step 2: Rename the `ChainNetwork` variant**

Replace:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChainNetwork {
    Testnet,
    MainnetCandidate,
}
```

with:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChainNetwork {
    Testnet,
    Mainnet,
}
```

- [ ] **Step 3: Replace `mainnet_candidate()` with `mainnet()`**

Replace the entire `pub const fn mainnet_candidate() -> Self { ... }` method body (currently lines 96-120):

```rust
    pub const fn mainnet_candidate() -> Self {
        Self {
            network: ChainNetwork::MainnetCandidate,
            chain_id: "tensorium-mainnet-candidate-0",
            target_block_seconds: 60,
            halving_interval_blocks: 1_051_200,
            max_halving_eras: MAX_HALVING_ERAS,
            total_supply_atoms: TOTAL_SUPPLY_ATOMS,
            founder_allocation_atoms: FOUNDER_ALLOCATION_ATOMS, // = 8M total pre-mint
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS,   // = 25M
            genesis_allocations: MC_GENESIS_ALLOCATIONS,
            founder_address: "",
            initial_reward_atoms: 1_190_279_581,                // 11.9027... TXM/block for 25M over 10 eras
            initial_leading_zero_bits: 40,
            min_leading_zero_bits: 32,
            max_leading_zero_bits: 56,
            difficulty_adjustment_window: 120,
            // Disabled until a real activation height is chosen and coordinated
            // (network stays on fixed 40-bit difficulty until then).
            difficulty_retarget_activation_height: u64::MAX,
            coinbase_maturity_blocks: 10,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }
```

with:

```rust
    pub const fn mainnet() -> Self {
        Self {
            network: ChainNetwork::Mainnet,
            chain_id: "tensorium-mainnet",
            target_block_seconds: 60,
            halving_interval_blocks: 2_102_400,
            max_halving_eras: MAX_HALVING_ERAS,
            total_supply_atoms: TOTAL_SUPPLY_ATOMS,
            founder_allocation_atoms: 0,
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS, // = 33M (zero premine)
            genesis_allocations: &[],
            founder_address: "",
            initial_reward_atoms: 785_584_523, // ~7.8558 TXM/block for 33M over 10 eras
            initial_leading_zero_bits: 42,
            min_leading_zero_bits: 34,
            max_leading_zero_bits: 58,
            difficulty_adjustment_window: 60,
            // Active from genesis: this is a fresh chain with no blocks mined
            // under fixed difficulty, so there is no backward-compat concern.
            difficulty_retarget_activation_height: 0,
            coinbase_maturity_blocks: 10,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }
```

- [ ] **Step 4: Replace the `MAINNET_CANDIDATE` const and its comment block**

Replace (currently lines 128-135):

```rust
// MAINNET_CANDIDATE — tokenomics v2 (2026-06-02)
// chain_id:        tensorium-mainnet-candidate-0
// Initial diff:    40 bits (GPU-first, RTX 3060+)
// Genesis ts:      1_780_272_000 (2026-06-01 00:00:00 UTC)
// Genesis nonce:   TBD — re-mine after tokenomics update
// Pre-mint (8M):   founder 1M | liquidity 3M | bridge 2M | ecosystem 2M
// Mining (25M):    11.9027... TXM/block, 10 eras, ~20 years
pub const MAINNET_CANDIDATE: ConsensusParams = ConsensusParams::mainnet_candidate();
```

with:

```rust
// MAINNET — TensorHash v1 clean relaunch, tokenomics v2 (2026-06-10)
// chain_id:        tensorium-mainnet
// Algorithm:       TensorHash v1 (memory-hard, GPU-first)
// Initial diff:    42 bits equivalent, retargeting active from genesis (window 60 blocks)
// Genesis ts:      TBD — set at actual launch time
// Genesis nonce:   TBD — re-mine offline (CPU brute-force or GPU miner) before launch
// Pre-mint:        0 (zero premine, mining-only issuance)
// Mining (33M):    ~7.8558 TXM/block, halving every ~4 years, 10 eras, ~40 years
pub const MAINNET: ConsensusParams = ConsensusParams::mainnet();
```

- [ ] **Step 5: Update `chain.rs`'s own tests**

Replace the entire `#[cfg(test)] mod tests { ... }` block (currently lines 149-215) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn assert_supply_split(params: ConsensusParams) {
        assert_eq!(params.total_supply_atoms, 33_000_000 * COIN);
        assert_eq!(
            params.founder_allocation_atoms + params.mining_allocation_atoms,
            params.total_supply_atoms,
            "pre-mint + mining must equal total supply"
        );
    }

    #[test]
    fn halving_interval_is_two_years_for_one_minute_blocks() {
        assert_eq!(
            TESTNET.halving_interval_blocks,
            2 * ConsensusParams::blocks_per_year(60)
        );
    }

    #[test]
    fn testnet_tokenomics_match_phase_7_readiness_plan() {
        assert_eq!(TESTNET.chain_id, "tensorium-testnet-0");
        assert_eq!(TESTNET.target_block_seconds, 60);
        assert_eq!(TESTNET.max_halving_eras, 10);
        assert_eq!(TESTNET.initial_reward_atoms, 1_523_557_865);
        assert!(TESTNET.genesis_allocations.is_empty());
        assert_eq!(TESTNET.coinbase_maturity_blocks, 10);
        assert_eq!(TESTNET.max_future_block_time_seconds, 2 * 60 * 60);
        assert_supply_split(TESTNET);
    }

    #[test]
    fn mainnet_tokenomics_match_zero_premine_relaunch_plan() {
        assert_eq!(MAINNET.chain_id, "tensorium-mainnet");
        assert_eq!(MAINNET.target_block_seconds, TESTNET.target_block_seconds);
        // MAINNET uses a 4-year halving era (TESTNET uses 2 years) — different
        // emission schedule for the zero-premine relaunch.
        assert_eq!(MAINNET.halving_interval_blocks, 2_102_400);
        assert_eq!(MAINNET.max_halving_eras, TESTNET.max_halving_eras);
        // Zero premine: no genesis allocations, founder allocation is 0.
        assert_eq!(MAINNET.founder_allocation_atoms, 0);
        assert!(MAINNET.genesis_allocations.is_empty());
        let genesis_total: u64 = MAINNET.genesis_allocations.iter().map(|(_, a)| a).sum();
        assert_eq!(genesis_total, 0);
        // 33M mining allocation with new initial reward.
        assert_eq!(MAINNET.initial_reward_atoms, 785_584_523);
        assert_eq!(MAINNET.coinbase_maturity_blocks, 10);
        assert_supply_split(MAINNET);
    }

    #[test]
    fn mainnet_is_gpu_first_harder_than_reference_network() {
        assert!(MAINNET.initial_leading_zero_bits > TESTNET.initial_leading_zero_bits);
        assert!(MAINNET.min_leading_zero_bits > TESTNET.min_leading_zero_bits);
        assert!(MAINNET.max_leading_zero_bits > TESTNET.max_leading_zero_bits);
        assert!(MAINNET.min_leading_zero_bits <= MAINNET.initial_leading_zero_bits);
        assert!(MAINNET.initial_leading_zero_bits <= MAINNET.max_leading_zero_bits);
    }

    #[test]
    fn mainnet_retargeting_is_active_from_genesis() {
        // Fresh chain — retargeting is enabled from block 0, unlike the old
        // MAINNET_CANDIDATE which kept it disabled (u64::MAX).
        assert_eq!(MAINNET.difficulty_retarget_activation_height, 0);
        assert_eq!(MAINNET.difficulty_adjustment_window, 60);
        assert_eq!(MAINNET.initial_leading_zero_bits, 42);
    }
}
```

- [ ] **Step 6: Run the `tensorium-core` test suite to confirm `chain.rs` compiles and its tests pass**

Run: `cargo test -p tensorium-core chain:: 2>&1 | tail -20`

Expected: compile errors from other modules referencing `MAINNET_CANDIDATE` /
`mainnet_candidate` / `MainnetCandidate` (those are fixed in Tasks 2-7) — but
the `chain::tests` module itself should show `4 passed` once the rest of the
crate compiles. It's fine if this command currently fails to link due to
errors in `emission.rs`/`difficulty.rs`; just confirm the *reported* compiler
errors are only about `MAINNET_CANDIDATE`/`mainnet_candidate`/`MainnetCandidate`
references in `emission.rs` and `difficulty.rs`, not about anything in
`chain.rs` itself.

- [ ] **Step 7: Commit**

```bash
git add crates/tensorium-core/src/chain.rs
git commit -m "feat(chain): rename MAINNET_CANDIDATE to MAINNET, zero premine, new emission/difficulty params"
```

---

### Task 2: `emission.rs` — rename references, update emission test for new schedule

**Files:**
- Modify: `crates/tensorium-core/src/emission.rs`

- [ ] **Step 1: Update the test imports and the `reward_is_zero_after_final_halving_era` loop**

Replace:

```rust
#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET_CANDIDATE, TESTNET};

    use super::*;
```

with:

```rust
#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET, TESTNET};

    use super::*;
```

Replace:

```rust
    #[test]
    fn reward_is_zero_after_final_halving_era() {
        for params in [TESTNET, MAINNET_CANDIDATE] {
            let first_zero_height =
                params.halving_interval_blocks * u64::from(params.max_halving_eras);
            assert_eq!(reward_at_height(&params, first_zero_height), 0);
            assert_eq!(reward_at_height(&params, first_zero_height + 1), 0);
        }
    }
```

with:

```rust
    #[test]
    fn reward_is_zero_after_final_halving_era() {
        for params in [TESTNET, MAINNET] {
            let first_zero_height =
                params.halving_interval_blocks * u64::from(params.max_halving_eras);
            assert_eq!(reward_at_height(&params, first_zero_height), 0);
            assert_eq!(reward_at_height(&params, first_zero_height + 1), 0);
        }
    }
```

- [ ] **Step 2: Replace the MAINNET_CANDIDATE-specific emission test**

Replace:

```rust
    #[test]
    fn mainnet_candidate_emission_matches_reference_schedule() {
        // MC uses 25M mining supply with 1_190_279_581 initial reward (tokenomics v2).
        let mc_supply = emitted_supply_until_height(
            &MAINNET_CANDIDATE,
            MAINNET_CANDIDATE.halving_interval_blocks * 10,
        );
        // Verify total mining supply is close to 25M (minor rounding dust from bit-shifts).
        assert!(mc_supply <= 25_000_000 * 100_000_000, "must not exceed 25M TXM");
        assert!(mc_supply >= 25_000_000 * 100_000_000 - MAINNET_CANDIDATE.initial_reward_atoms,
                "dust must be less than one block reward");

        // MC initial reward differs from testnet (25M vs 32M mining supply).
        assert_eq!(MAINNET_CANDIDATE.initial_reward_atoms, 1_190_279_581);
        assert_eq!(TESTNET.initial_reward_atoms, 1_523_557_865);
    }
```

with:

```rust
    #[test]
    fn mainnet_emission_matches_zero_premine_schedule() {
        // MAINNET uses the full 33M supply as mining allocation (zero premine)
        // with a 4-year halving era and 785_584_523-atom initial reward.
        let mainnet_supply = emitted_supply_until_height(
            &MAINNET,
            MAINNET.halving_interval_blocks * 10,
        );
        assert_eq!(mainnet_supply, 3_299_999_986_972_800);

        let dust = MAINNET.mining_allocation_atoms - mainnet_supply;
        assert_eq!(dust, 13_027_200);
        assert!(dust < MAINNET.initial_reward_atoms, "dust must be less than one block reward");

        assert_eq!(MAINNET.initial_reward_atoms, 785_584_523);
        assert_eq!(TESTNET.initial_reward_atoms, 1_523_557_865);
    }
```

- [ ] **Step 3: Run the emission tests**

Run: `cargo test -p tensorium-core emission:: 2>&1 | tail -20`

Expected: All tests in `emission::tests` pass — `reward_halves_each_era_then_stops`,
`emission_never_exceeds_cap`, `ten_era_emission_matches_mining_allocation_with_rounding_dust`,
`mainnet_emission_matches_zero_premine_schedule`, `reward_is_zero_after_final_halving_era`
all `ok`.

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-core/src/emission.rs
git commit -m "test(emission): update tests for MAINNET zero-premine emission schedule"
```

---

### Task 3: `difficulty.rs` — rename references, update difficulty bounds test, add genesis-retargeting test

**Files:**
- Modify: `crates/tensorium-core/src/difficulty.rs`

- [ ] **Step 1: Update the test import**

Replace:

```rust
#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET_CANDIDATE, TESTNET};

    use super::*;
```

with:

```rust
#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET, TESTNET};

    use super::*;
```

- [ ] **Step 2: Replace the MAINNET_CANDIDATE difficulty-bounds test**

Replace:

```rust
    #[test]
    fn mainnet_candidate_difficulty_bounds_are_clamped() {
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: MAINNET_CANDIDATE.max_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET_CANDIDATE, fast_sample),
            MAINNET_CANDIDATE.max_leading_zero_bits
        );

        let slow_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: u64::MAX,
            current_leading_zero_bits: MAINNET_CANDIDATE.min_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET_CANDIDATE, slow_sample),
            MAINNET_CANDIDATE.min_leading_zero_bits
        );
    }
```

with:

```rust
    #[test]
    fn mainnet_difficulty_bounds_are_clamped() {
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: MAINNET.max_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET, fast_sample),
            MAINNET.max_leading_zero_bits
        );

        let slow_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: u64::MAX,
            current_leading_zero_bits: MAINNET.min_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET, slow_sample),
            MAINNET.min_leading_zero_bits
        );
    }
```

- [ ] **Step 3: Add a test confirming retargeting is active from genesis for `MAINNET`**

Add this new test at the end of the `mod tests` block (after
`expected_difficulty_retargets_at_and_after_activation_height`):

```rust
    #[test]
    fn mainnet_retargets_starting_from_the_first_completed_window() {
        // MAINNET ships with difficulty_retarget_activation_height = 0, so
        // retargeting applies from the very first completed adjustment window
        // (no legacy fixed-difficulty period, unlike TESTNET/old MAINNET_CANDIDATE).
        assert_eq!(
            expected_leading_zero_bits(&MAINNET, 0, None),
            MAINNET.initial_leading_zero_bits
        );

        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: MAINNET.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&MAINNET, MAINNET.difficulty_adjustment_window, Some(fast_sample)),
            MAINNET.initial_leading_zero_bits + 1
        );

        let slow_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: MAINNET.target_block_seconds * MAINNET.difficulty_adjustment_window * 3,
            current_leading_zero_bits: MAINNET.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&MAINNET, MAINNET.difficulty_adjustment_window, Some(slow_sample)),
            MAINNET.initial_leading_zero_bits - 1
        );
    }
```

- [ ] **Step 4: Run the difficulty tests**

Run: `cargo test -p tensorium-core difficulty:: 2>&1 | tail -25`

Expected: All tests in `difficulty::tests` pass, including
`mainnet_difficulty_bounds_are_clamped` and
`mainnet_retargets_starting_from_the_first_completed_window`.

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/difficulty.rs
git commit -m "test(difficulty): update tests for MAINNET 42-bit difficulty and genesis retargeting"
```

---

### Task 4: `lib.rs` — update re-export

**Files:**
- Modify: `crates/tensorium-core/src/lib.rs`

- [ ] **Step 1: Update the chain re-export**

Replace:

```rust
pub use chain::{ChainNetwork, ConsensusParams, MAINNET_CANDIDATE, TESTNET};
```

with:

```rust
pub use chain::{ChainNetwork, ConsensusParams, MAINNET, TESTNET};
```

- [ ] **Step 2: Build `tensorium-core` to confirm the crate itself compiles cleanly**

Run: `cargo build -p tensorium-core 2>&1 | tail -20`

Expected: `Compiling tensorium-core ...` then `Finished` with no errors (warnings
about unused items are not expected at this point — every renamed item is used
within the crate's own tests).

- [ ] **Step 3: Run the full `tensorium-core` test suite**

Run: `cargo test -p tensorium-core 2>&1 | tail -10`

Expected: all tests pass (this includes `state.rs`, `validation.rs`, `pow.rs`,
etc., none of which reference `MAINNET`/`MAINNET_CANDIDATE` directly — they use
`TEST_PARAMS`/`TESTNET`/`Hash256::ZERO` — so they are unaffected by this rename
and should already be green).

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-core/src/lib.rs
git commit -m "feat(core): export MAINNET instead of MAINNET_CANDIDATE"
```

---

### Task 5: `tensorium-node/src/main.rs` — rename const usages, rename genesis placeholder constants

**Files:**
- Modify: `crates/tensorium-node/src/main.rs`

This file has ~20 usages of `MAINNET_CANDIDATE` plus the two genesis-placeholder
constants `MC_GENESIS_TIMESTAMP` / `MC_GENESIS_NONCE`. CLI subcommand names
(`mainnet-candidate`/`mc`), env var names, file path constants, and helper
function names (`mc_state_path_from_env`, `init_mainnet_candidate_state`, etc.)
are **not** renamed — see the plan's scope note.

- [ ] **Step 1: Update the import**

Replace:

```rust
use tensorium_core::{
    block::{merkle_root as compute_merkle_root, BlockHeader, Transaction},
    chain::{ConsensusParams, MAINNET_CANDIDATE},
    emission::reward_at_height,
    pow::header_meets_work,
    script::standard::{extract_address, p2pkh_from_address},
    Block, ChainState, Hash256, Mempool, StateError, UtxoSet,
};
```

with:

```rust
use tensorium_core::{
    block::{merkle_root as compute_merkle_root, BlockHeader, Transaction},
    chain::{ConsensusParams, MAINNET},
    emission::reward_at_height,
    pow::header_meets_work,
    script::standard::{extract_address, p2pkh_from_address},
    Block, ChainState, Hash256, Mempool, StateError, UtxoSet,
};
```

- [ ] **Step 2: Rename the genesis placeholder constants and update their doc comments**

Replace:

```rust
/// Genesis timestamp for the mainnet-candidate chain (2026-06-01 00:00:00 UTC).
/// All nodes MUST use this exact value to share the same genesis block.
const MC_GENESIS_TIMESTAMP: u64 = 1_780_272_000;
/// Genesis nonce for the mainnet-candidate chain (tokenomics v2, 2026-06-02).
/// Pre-mint: 8M TXM (founder 1M + liquidity 3M + bridge 2M + ecosystem 2M)
/// Mining: 25M TXM over 10 eras, initial reward 11.9027... TXM/block
/// Nonce will be updated after re-mine with new tokenomics.
const MC_GENESIS_NONCE: u64 = 798_243_452_272;
```

with:

```rust
/// Genesis timestamp for the MAINNET chain (TensorHash v1 relaunch, zero premine).
/// All nodes MUST use this exact value to share the same genesis block.
/// TODO(launch): placeholder — set to the actual mainnet genesis timestamp before launch.
const MAINNET_GENESIS_TIMESTAMP: u64 = 1_780_272_000;
/// Genesis nonce for the MAINNET chain (TensorHash v1, zero premine, 33M mining allocation).
/// TODO(launch): placeholder — TensorHash v1 invalidates any nonce found under the old
/// SHA256d algorithm. Re-mine at 42-bit difficulty (CPU brute-force across many cores,
/// or the Phase A2 GPU miner) before launch. `init_genesis_nonce` rejects an invalid
/// nonce with `StateError::MiningFailed`, so this placeholder fails loudly and safely.
const MAINNET_GENESIS_NONCE: u64 = 0;
```

- [ ] **Step 3: Replace every remaining `MAINNET_CANDIDATE` token with `MAINNET`, and every remaining `MC_GENESIS_TIMESTAMP`/`MC_GENESIS_NONCE` with `MAINNET_GENESIS_TIMESTAMP`/`MAINNET_GENESIS_NONCE`**

Use a scoped find-and-replace across `crates/tensorium-node/src/main.rs`:

```bash
sed -i \
  -e 's/MAINNET_CANDIDATE/MAINNET/g' \
  -e 's/MC_GENESIS_TIMESTAMP/MAINNET_GENESIS_TIMESTAMP/g' \
  -e 's/MC_GENESIS_NONCE/MAINNET_GENESIS_NONCE/g' \
  crates/tensorium-node/src/main.rs
```

This replaces all of:
- the `match nonce { ... None => MAINNET_GENESIS_NONCE }` default in the `mainnet-candidate init` subcommand
- `MAINNET.initial_leading_zero_bits` / `MAINNET.chain_id` / `MAINNET.target_block_seconds` /
  `MAINNET.halving_interval_blocks` in `print_status`, `print_help`, and `print_help_mc`
- `&MAINNET` arguments to `mine_next_block`, `serve_rpc`, `serve_p2p`, `connect_peer`,
  `sync_from_peer`, `init_genesis_nonce`
- the `params = &MAINNET` binding and `MAINNET.chain_id` / `MAINNET.initial_leading_zero_bits`
  in `mine_genesis_multithreaded`
- `&MAINNET` in `peers_for`'s `params.chain_id == MAINNET.chain_id` comparison
- the test `let mc_peers = peers_for(&MAINNET);` in
  `peers_for_mainnet_candidate_uses_mc_peer_list` (test name unchanged — see scope note)
- `init_genesis_nonce(&MAINNET, MAINNET_GENESIS_TIMESTAMP, nonce)` in `init_mainnet_candidate_state`

**Note:** `sed -i -e 's/MAINNET_CANDIDATE/MAINNET/g'` run *before* the
`MC_GENESIS_*` substitutions is safe and order-independent here because
`MAINNET_CANDIDATE` and `MC_GENESIS_TIMESTAMP`/`MC_GENESIS_NONCE` share no
common substring that the other pattern would match.

- [ ] **Step 4: Fix the doc comment on `mine_genesis_multithreaded` and the stale `genesis_hash` println**

The `sed` in Step 3 turns the doc comment

```rust
/// Multi-threaded CPU nonce search for the mainnet-candidate genesis block.
/// Returns the first nonce that satisfies MAINNET difficulty.
```

— this reads fine, no further edit needed for that comment.

However, `print_help_mc` has a hardcoded `genesis_hash` line left over from the
old SHA256d genesis, and the `genesis_nonce` line's comment is now stale.
Replace:

```rust
    println!("  genesis_ts     = {MAINNET_GENESIS_TIMESTAMP}  (2026-06-01 00:00:00 UTC)");
    println!("  genesis_nonce  = {MAINNET_GENESIS_NONCE}  (mined RTX 5090, 2026-05-31)");
    println!("  genesis_hash   = 0000000000d61e99b9e2530609632b399d0f0b538c2d54daa1dddbfe28ea08dc");
```

with:

```rust
    println!("  genesis_ts     = {MAINNET_GENESIS_TIMESTAMP}  (TBD — placeholder, set before launch)");
    println!("  genesis_nonce  = {MAINNET_GENESIS_NONCE}  (TBD — placeholder, re-mine before launch)");
```

- [ ] **Step 5: Build and run the `tensorium-node` test suite**

Run: `cargo build -p tensorium-node 2>&1 | tail -20`

Expected: `Finished` with no errors.

Run: `cargo test -p tensorium-node 2>&1 | tail -10`

Expected: all tests pass, including `peers_for_mainnet_candidate_uses_mc_peer_list`.

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): use renamed MAINNET chain config and genesis placeholder constants"
```

---

### Task 6: `txmwallet/src/main.rs` — rename const usages

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Replace all `MAINNET_CANDIDATE` tokens with `MAINNET`**

```bash
sed -i 's/MAINNET_CANDIDATE/MAINNET/g' crates/txmwallet/src/main.rs
```

This updates:
- the import `chain::MAINNET_CANDIDATE` → `chain::MAINNET`
- `.apply_block(&MAINNET, &block)` (two occurrences)
- `.saturating_add(MAINNET.coinbase_maturity_blocks)` (two occurrences)

- [ ] **Step 2: Build and test**

Run: `cargo build -p txmwallet 2>&1 | tail -20`

Expected: `Finished` with no errors.

Run: `cargo test -p txmwallet 2>&1 | tail -10`

Expected: all tests pass (5/5, matching the pre-rename baseline).

- [ ] **Step 3: Commit**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): use renamed MAINNET chain config"
```

---

### Task 7: `tensorium-indexer/src/rpc.rs` — update test fixture chain_id

**Files:**
- Modify: `crates/tensorium-indexer/src/rpc.rs`

- [ ] **Step 1: Update the hardcoded chain_id in the test fixture**

Replace:

```rust
            chain_id: "tensorium-mainnet-candidate-0".into(),
```

with:

```rust
            chain_id: "tensorium-mainnet".into(),
```

- [ ] **Step 2: Build and test**

Run: `cargo build -p tensorium-indexer 2>&1 | tail -20`

Expected: `Finished` with no errors.

Run: `cargo test -p tensorium-indexer 2>&1 | tail -10`

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/tensorium-indexer/src/rpc.rs
git commit -m "test(indexer): update sample block chain_id to tensorium-mainnet"
```

---

### Task 8: Final full-workspace verification

**Files:** none (verification only)

- [ ] **Step 1: Confirm no stray references to the old names remain**

Run:

```bash
grep -rn "MAINNET_CANDIDATE\|MainnetCandidate\|mainnet_candidate()\|tensorium-mainnet-candidate-0\|GENESIS_PRE_MINT_COINS\|FOUNDER_ALLOCATION_ATOMS\|MC_GENESIS_ALLOCATIONS\|MC_GENESIS_TIMESTAMP\|MC_GENESIS_NONCE" --include=*.rs | grep -v target
```

Expected: no output. (CLI subcommand strings `"mainnet-candidate"`, `"mc"`,
env vars like `TENSORIUM_MC_PEERS`, file paths like `tensorium-mc-state.json`,
and function names like `mc_state_path_from_env`/`init_mainnet_candidate_state`/
`peers_for_mainnet_candidate_uses_mc_peer_list`/`configured_mc_peers`/
`print_help_mc` are expected to remain — they are out of scope per this plan's
scope note.)

- [ ] **Step 2: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -30`

Expected: all crates report `test result: ok` with 0 failures.

- [ ] **Step 3: Run clippy across the workspace**

Run: `cargo clippy --workspace --all-targets 2>&1 | tail -30`

Expected: no new warnings introduced by this change (pre-existing warnings, if
any, are out of scope).

- [ ] **Step 4: No commit needed for this task** — it is verification-only. If
Steps 1-3 reveal any issue, fix it within the relevant task above and re-run
this task's checks before considering Phase B complete.
