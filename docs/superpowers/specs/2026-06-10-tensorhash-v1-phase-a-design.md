# TensorHash v1 — Phase A1 Design (Algorithm + Core Integration)

**Status:** Draft, pending review
**Context:** Part of the Tensorium Clean Relaunch (TensorHash v2.0 package, see
`/root/TensorHash/Tensorium_Relaunch_TensorHash_v2_0_Package.zip` for the source spec).
This document covers **Phase A1 only**: the TensorHash v1 PoW algorithm itself and
its integration into `tensorium-core` consensus. No real funds/OTC sales exist on
the current chain, so a clean genesis (Phase B) is acceptable.

## Goals

- Replace SHA256d PoW with **TensorHash v1**, a memory-hard, GPU-first algorithm
  adapted from the Autolykos v2 "sample-and-mix from a regenerable dataset" pattern,
  plus a custom **TensorMix** integer-matrix mixing layer.
- Minimum viable hardware: RTX 3090 (24GB VRAM). More VRAM/compute = better, no hard
  cap, but the dataset must comfortably fit a 24GB card.
- Verification (every full node, every block) must stay cheap — no GPU, no large
  memory required to validate a block.
- No CPU mining anywhere (drop `txmminer`); GPU mining via `tools/tensorium-miner`
  (existing CUDA C++ miner, currently SHA256d — kernel gets replaced).
- Ship with enough self-verification (KATs + GPU selftest + private devnet) to be
  confident before going straight to mainnet (no public testnet).

## Algorithm Parameters

| Parameter | Value | Notes |
|---|---|---|
| `ELEMENT_SIZE` | 32 bytes | one Blake2b-256 output |
| `N` (dataset element count) | ≈ 600,000,000 | ≈ 17.9 GiB dataset; fits 24GB w/ headroom |
| `EPOCH_LENGTH` | 8,192 blocks | ~5.7 days at 60s blocks (per relaunch spec) |
| `K` (indices sampled per attempt) | 32 | |

Exact `N` may be tuned (±) after Phase A's GPU benchmarks (step below), as long as
dataset size stays comfortably under 24GB.

## Algorithm Definition

### Dataset element function (memory-hard core)

```
dataset_element(epoch_seed, i) = Blake2b256(epoch_seed || i.to_le_bytes())
```

- `epoch_seed` = block-id hash of the last block of the previous epoch
  (epoch 0 uses a fixed genesis constant).
- Computable on demand — no storage required for verification. Miners
  materialize all `N` elements into VRAM because it's faster than recomputing
  per nonce attempt (this is the memory-hard property).

### Per-attempt PoW hash

```
tensorhash(header_bytes, nonce, epoch_seed):
  digest = Blake2b256(header_bytes || nonce.to_le_bytes())
  acc = [u64; 4] initialized from digest (4 x 8 bytes, LE)
  for j in 0..K:
    idx_seed = Blake2b256(digest || j.to_le_bytes())
    idx = u64::from_le_bytes(idx_seed[0..8]) % N
    elem = dataset_element(epoch_seed, idx)         # [u64; 4], LE
    for m in 0..4:
      acc[m] = acc[m].wrapping_mul(elem[m] | 1)
                     .wrapping_add(elem[(m+1) % 4].rotate_left(13))
  acc_bytes = concat(acc[0..4].to_le_bytes())
  return Blake2b256(header_bytes || nonce.to_le_bytes() || acc_bytes)
```

`pow_hash` is checked against `leading_zero_bits` exactly as the existing
`hash_meets_work` does today — no change to `difficulty.rs`.

### Why light-to-verify, heavy-to-mine

- **Verification**: 32 `dataset_element` computations + 32 mix steps + 2 extra
  Blake2b calls ≈ 34 hash operations total. Trivial CPU cost, runs on every
  full node for every block, including during initial sync.
- **Mining**: trying billions of nonces means recomputing `dataset_element`
  billions of times *unless* the ~18GB table is precomputed once per epoch and
  cached in VRAM. Without the cache, CPU mining is ~32x slower per attempt —
  this is what makes the algorithm GPU-favoring and effectively rules out CPU
  mining as competitive (hence dropping CPU mining entirely).

## Components

### 1. New crate: `tensorium-tensorhash` (pure Rust, CPU)

For **validation only**, not mining:

- `dataset_element(epoch_seed: Hash256, i: u64) -> [u64; 4]`
- `pow_hash(header_bytes: &[u8], nonce: u64, epoch_seed: Hash256) -> Hash256`
- Ships KAT (known-answer test) vectors as the consensus reference — the GPU
  miner's output must match this exactly, bit-for-bit.

### 2. `tensorium-core` changes

- `BlockHeader`: split into
  - `id_hash()` — existing `double_sha256(header_bytes)`, used for chain
    linkage, `previous_hash`, merkle roots, explorer display (unchanged).
  - `pow_hash(epoch_seed)` — new TensorHash-based hash, used **only** by
    `header_meets_work`.
- `pow.rs`: `header_meets_work(header, epoch_seed)` calls
  `tensorium_tensorhash::pow_hash(...)`.
- `validation.rs`: thread `epoch_seed` through `validate_block` —
  `epoch_seed` is derived from `ChainState` (id-hash of the last block of the
  previous epoch; epoch 0 uses the fixed genesis constant).
- `difficulty.rs`: **no changes** — leading-zero-bits target scheme is valid
  for any uniformly-distributed 256-bit hash output.

### 3. Remove `crates/txmminer`

- Delete the crate, remove from workspace `Cargo.toml` members. No CPU miner
  remains anywhere in the codebase.

### 4. `tools/tensorium-miner` (CUDA C++) — TensorHash kernel

Replace SHA256d with TensorHash; reuse existing infra (solo/stratum clients,
NVML monitoring, CLI):

- Remove `sha256d.cuh` and the SHA256d mining kernel.
- New `tensorhash.cuh` / mining kernel:
  - **Dataset generation kernel**: materializes all `N≈600M` elements
    (~18GB) into VRAM via parallel Blake2b. Regenerated whenever
    `epoch_seed` changes (every 8,192 blocks / ~5.7 days). Each element is
    independent — fully parallelizable, no cross-element dependencies.
  - **Mining kernel**: per nonce — compute digest, derive 32 indices, VRAM
    dataset lookup, TensorMix accumulate, final hash, compare to target.
  - **VRAM check at startup**: query device memory; require ≥ ~20GB free
    (18GB dataset + overhead). Refuse to mine below this (clear error
    message naming RTX 3090/24GB as the minimum). More VRAM/compute headroom
    naturally gives better/more stable hashrate — no artificial cap.
  - **`--selftest` mode** (see Testing section): runs KAT vectors through the
    real GPU kernel + generated dataset; must pass before mining starts.

## Testing & Benchmark Strategy

No public testnet exists, so correctness must be proven in layers before
mainnet genesis:

1. **Rust KATs** in `tensorium-tensorhash`: hardcoded
   `(epoch_seed, header_bytes, nonce) -> pow_hash` vectors, plus vectors for
   individual `dataset_element(seed, i)` values and intermediate TensorMix
   accumulator states. These pin the spec byte-for-byte.

2. **CUDA `--selftest`**: the GPU miner ships the same KAT vectors. On
   startup, generates the real ~18GB dataset and runs the vectors through the
   actual kernel, checking bit-for-bit match against the Rust reference.
   Mismatch = hard refuse to start. Catches endianness/overflow/index-derivation
   bugs before any block is mined.

3. **Private devnet**: 2-3 nodes, throwaway `chain_id`/genesis, shrunk
   `EPOCH_LENGTH` (reuse the existing `TESTNET` params pattern in `chain.rs`),
   real GPU mining. Validates: cross-node block validation, epoch transitions
   regenerate dataset correctly, difficulty retargeting, reorgs work with the
   new `pow_hash`. This is *private* validation (explicitly allowed/required
   by the relaunch spec), not a public testnet.

4. **GPU benchmarks** (RTX 3090/4090/5090/H100 via vast.ai rentals): measure
   real hashrate and dataset-generation time per epoch. Feeds Phase B's
   genesis difficulty constant (spec's "42-bit equivalent" is a placeholder
   pending real numbers).

## Dependencies & Out of Scope

- **Depends on**: nothing — this is the foundation of the relaunch.
- **Feeds into**: Phase A3 (pool share validation reuses
  `tensorium-tensorhash`), Phase B (genesis difficulty calibrated from
  benchmark results).
- **Out of scope**: explorer/wallet/bridge changes (Phase C), VPS redeploy
  (Phase D), docs/branding cleanup (Phase E), new genesis/chain
  ID/tokenomics (Phase B).
