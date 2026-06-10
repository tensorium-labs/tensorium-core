# TensorHash v1 — Phase B Design (Genesis & Tokenomics v2)

**Status:** Draft, pending review
**Context:** Part of the Tensorium Clean Relaunch (TensorHash v2.0 package, see
`/root/TensorHash/Tensorium_Relaunch_TensorHash_v2_0_Package.zip` for the source spec).
Phase A1 (TensorHash v1 PoW algorithm + core integration) is complete and merged
to `main` (commit `22dac77`). This document covers **Phase B**: reconfiguring the
mainnet consensus parameters in `tensorium-core` for a clean, zero-premine,
mining-only TXM relaunch, and renaming the chain identity from
`MAINNET_CANDIDATE` to `MAINNET`.

No real funds or OTC sales exist on the current `MAINNET_CANDIDATE` chain, so a
clean genesis is acceptable (per the Phase A1 design doc).

## Goals

- Replace `MAINNET_CANDIDATE` ("tensorium-mainnet-candidate-0") with `MAINNET`
  ("tensorium-mainnet") as the definitive launch configuration.
- Zero premine: no founder/seed/ICO/liquidity genesis allocations. 100% of the
  33,000,000 TXM max supply is mined, starting from block 1.
- Adopt the relaunch spec's emission schedule (4-year halving era).
- Adopt the relaunch spec's initial difficulty (42-bit equivalent) with
  difficulty retargeting active from genesis (no backward-compat constraint —
  this is a fresh chain).
- Leave genesis timestamp/nonce as explicit placeholders to be re-mined offline
  before actual mainnet launch (out of scope for this phase).

## 1. Chain Identity Rename

Rename throughout `tensorium-core` and all dependent crates:

| Old | New |
|---|---|
| `ChainNetwork::MainnetCandidate` | `ChainNetwork::Mainnet` |
| `ConsensusParams::mainnet_candidate()` | `ConsensusParams::mainnet()` |
| `MAINNET_CANDIDATE` (const) | `MAINNET` (const) |
| `chain_id: "tensorium-mainnet-candidate-0"` | `chain_id: "tensorium-mainnet"` |

### Affected files (mechanical rename + parameter changes)

- `crates/tensorium-core/src/chain.rs` — enum variant, struct constructor fn,
  const, chain_id string, allocation/emission/difficulty constants (see
  sections 2-4), doc comments, tests.
- `crates/tensorium-core/src/emission.rs` — test references to
  `MAINNET_CANDIDATE` → `MAINNET`, updated expected reward/supply numbers.
- `crates/tensorium-core/src/difficulty.rs` — test references to
  `MAINNET_CANDIDATE` → `MAINNET`, updated expected difficulty bounds.
- `crates/tensorium-core/src/lib.rs` — re-export
  `MAINNET_CANDIDATE` → `MAINNET`.
- `crates/tensorium-node/src/main.rs` — all `MAINNET_CANDIDATE` usages →
  `MAINNET`; helper function/identifier renames:
  - `init_mainnet_candidate_state` → `init_mainnet_state`
  - `mc_state_path_from_env` → `mainnet_state_path_from_env`
  - `mc_mempool_path_from_env` → `mainnet_mempool_path_from_env`
  - `MC_GENESIS_TIMESTAMP` → `MAINNET_GENESIS_TIMESTAMP`
  - `MC_GENESIS_NONCE` → `MAINNET_GENESIS_NONCE`
  - test `peers_for_mainnet_candidate_uses_mc_peer_list` →
    `peers_for_mainnet_uses_mainnet_peer_list`
  - doc comments referencing "MAINNET_CANDIDATE difficulty" updated.
- `crates/tensorium-pool/src/main.rs` — any `MAINNET_CANDIDATE` usages →
  `MAINNET`.
- `crates/txmwallet/src/main.rs` — `MAINNET_CANDIDATE` → `MAINNET` (test
  helpers using `apply_block(&MAINNET_CANDIDATE, ...)` and
  `MAINNET_CANDIDATE.coinbase_maturity_blocks`).
- `crates/tensorium-indexer/src/rpc.rs` — hardcoded
  `chain_id: "tensorium-mainnet-candidate-0".into()` → `"tensorium-mainnet".into()`.

`TESTNET` and `TEST_PARAMS` are **unchanged** by this rename — they remain the
low-difficulty CPU dev/test network and are out of scope for this phase.

## 2. Tokenomics v2 — Zero Premine

Current `chain.rs` constants and their new values:

```rust
pub const COIN: u64 = 100_000_000;                    // unchanged
pub const MAX_HALVING_ERAS: u32 = 10;                 // unchanged
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;       // unchanged
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN; // unchanged

// REMOVED: GENESIS_PRE_MINT_COINS (was 8_000_000)
// REMOVED: FOUNDER_ALLOCATION_ATOMS (was GENESIS_PRE_MINT_COINS * COIN)
// REMOVED: MC_GENESIS_ALLOCATIONS array entirely

// MINING_ALLOCATION_COINS now equals the full supply (0 premine).
pub const MINING_ALLOCATION_COINS: u64 = TOTAL_SUPPLY_COINS; // 33_000_000
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;
```

`ConsensusParams::mainnet()` (renamed from `mainnet_candidate()`):

```rust
founder_allocation_atoms: 0,
mining_allocation_atoms: MINING_ALLOCATION_ATOMS, // = TOTAL_SUPPLY_ATOMS
genesis_allocations: &[],   // no genesis pre-mint buckets
founder_address: "",
```

The `assert_supply_split` test helper (`founder_allocation_atoms +
mining_allocation_atoms == total_supply_atoms`) continues to hold trivially
(`0 + 33M == 33M`).

### Removed / updated reference comment block

The existing comment block above `MAINNET_CANDIDATE`:

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

becomes:

```rust
// MAINNET — TensorHash v1 clean relaunch, tokenomics v2 (2026-06-10)
// chain_id:        tensorium-mainnet
// Algorithm:       TensorHash v1 (memory-hard, GPU-first)
// Initial diff:    42 bits equivalent, retargeting active from genesis
// Genesis ts:      TBD — set at actual launch time
// Genesis nonce:   TBD — re-mine offline (CPU brute-force or GPU miner) before launch
// Pre-mint:        0 (zero premine, mining-only issuance)
// Mining (33M):    ~7.8558 TXM/block, halving every ~4 years, 10 eras, ~40 years
pub const MAINNET: ConsensusParams = ConsensusParams::mainnet();
```

## 3. Emission Schedule — 4-Year Era

```rust
halving_interval_blocks: 2_102_400, // ~4 years at 60s blocks (was 1_051_200, 2 years)
max_halving_eras: MAX_HALVING_ERAS, // 10, unchanged
initial_reward_atoms: 785_584_523,  // ≈ 7.85584523 TXM/block
```

`initial_reward_atoms` is computed using the **same dust-minimizing formula**
already used for `TESTNET` (32M mining, 1_523_557_865) and the old MC (25M
mining, 1_190_279_581):

```text
initial_reward_atoms = floor(mining_allocation_atoms * 512 / (1023 * halving_interval_blocks))
                      = floor(33_000_000 * 100_000_000 * 512 / (1023 * 2_102_400))
                      = 785_584_523
```

This formula accounts for the 10-era cap directly (sum of `reward >> era` for
`era` in `0..10` is `reward * 1023/512`), so the unminted "dust" after era 10
stays on the order of tens of TXM — not the ~32,226 TXM the relaunch spec's
literal "infinite halving" formula (`max_supply / (2 * era_blocks) =
7.848173516 TXM`) would leave unminted. Public-facing docs (Phase E) should
describe the reward as "~7.8558 TXM per block, halving roughly every 4 years"
rather than quoting the spec's draft 7.848173516 figure verbatim.

### Test updates (`emission.rs`)

- `mainnet_candidate_emission_matches_reference_schedule` →
  `mainnet_emission_matches_reference_schedule`, updated to assert:
  - `MAINNET.initial_reward_atoms == 785_584_523`
  - `emitted_supply_until_height(&MAINNET, MAINNET.halving_interval_blocks * 10)`
    is `<= 33_000_000 * COIN` and within one block reward of it (same pattern
    as the existing `ten_era_emission_matches_mining_allocation_with_rounding_dust`
    test for TESTNET).
- `reward_is_zero_after_final_halving_era` test loop `[TESTNET, MAINNET_CANDIDATE]`
  → `[TESTNET, MAINNET]` (no other change needed — logic is parameter-driven).

## 4. Difficulty — 42-Bit Initial, Retargeting Active From Genesis

```rust
initial_leading_zero_bits: 42,           // was 40
min_leading_zero_bits: 34,               // was 32 (initial - 8, same spread as before)
max_leading_zero_bits: 58,               // was 56 (initial + 16, same spread as before)
difficulty_adjustment_window: 60,        // was 120 (per relaunch spec)
difficulty_retarget_activation_height: 0, // was u64::MAX (was disabled)
```

Retargeting can be active from block 0 because this is a brand-new chain with
no existing blocks mined under fixed difficulty — there is no backward-compat
concern (unlike a hard-fork on a live chain). `difficulty::expected_leading_zero_bits`
already implements the ±1-bit-per-window adjustment with clamping; no changes
to `difficulty.rs` logic are needed, only the `MAINNET` parameter values and
renamed test references.

### Test updates (`difficulty.rs`)

- `mainnet_candidate_difficulty_bounds_are_clamped` →
  `mainnet_difficulty_bounds_are_clamped`, updated to use `MAINNET.max_leading_zero_bits`
  (58) / `MAINNET.min_leading_zero_bits` (34).
- Add a case (or extend an existing parametrized test) confirming
  `expected_leading_zero_bits(&MAINNET, 0, None) == 42` and that retargeting
  applies starting from the first completed window (height ==
  `difficulty_adjustment_window`), exercising
  `difficulty_retarget_activation_height: 0`.

## 5. Genesis Timestamp & Nonce — Placeholders

In `tensorium-node/src/main.rs`:

```rust
// TODO(launch): set to the actual mainnet genesis timestamp before launch.
const MAINNET_GENESIS_TIMESTAMP: u64 = 1_780_272_000; // placeholder, currently 2026-06-01 00:00:00 UTC

// TODO(launch): re-mine before launch — TensorHash v1 invalidates this nonce
// (it was found under the old SHA256d algorithm). Mining at 42-bit difficulty
// requires either a parallel CPU brute-force run or the Phase A2 GPU miner.
const MAINNET_GENESIS_NONCE: u64 = 0; // placeholder — will fail header_meets_work until re-mined
```

`init_genesis_nonce` already returns `StateError::MiningFailed` if the supplied
nonce doesn't satisfy `header_meets_work`, so the placeholder `0` fails loudly
and safely if someone attempts to start a `MAINNET` node before re-mining —
it cannot silently produce an invalid genesis.

Re-mining the genesis nonce, choosing the final genesis timestamp, and
launching the chain are explicitly **out of scope** for this phase (tracked as
a follow-up before Phase D deploy).

## Dependencies & Out of Scope

- **Depends on:** Phase A1 (TensorHash v1 algorithm + core integration, merged).
- **Out of scope:**
  - Phase A2 (CUDA TensorHash kernel) — needed before genesis can realistically
    be mined at 42-bit difficulty.
  - Phase A3 (pool share validation against TensorHash).
  - Re-mining the actual genesis nonce / choosing the final genesis timestamp.
  - Phase C (explorer/wallet/bridge updates beyond the mechanical
    `MAINNET_CANDIDATE` → `MAINNET` rename already covered here).
  - Phase D (VPS redeploy).
  - Phase E (docs/branding cleanup — TOKENOMICS.md, MINING.md, etc.).
  - `TESTNET` / `TEST_PARAMS` changes — these remain the low-difficulty CPU
    dev/test network, untouched by this phase.
