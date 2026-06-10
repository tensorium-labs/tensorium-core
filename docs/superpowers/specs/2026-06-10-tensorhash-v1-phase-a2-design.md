# TensorHash v1 — Phase A2 Design (CUDA GPU Miner)

**Status:** Approved
**Context:** Part of the Tensorium Clean Relaunch. Phase A1 (TensorHash v1
algorithm + core integration, commit `22dac77`) and Phase B (MAINNET rename,
zero-premine tokenomics, 42-bit initial difficulty, commit `6461eb4`) are
merged to `main`. This document covers **Phase A2**: replacing the SHA256d
CUDA kernel in `tools/tensorium-miner` with TensorHash v1, plus the minimal
node-side support a TensorHash miner needs. Phase A2 gates the genesis
re-mine: `MAINNET_GENESIS_NONCE` is a placeholder `0` that must be re-mined
at 42-bit difficulty under TensorHash v1.

Reference implementation: `crates/tensorium-tensorhash/src/lib.rs` (pure
Rust, consensus-pinned by KAT vectors). Algorithm parameters:
`DATASET_N = 600_000_000` (×32 B = 19.2 GB ≈ 17.9 GiB), `EPOCH_LENGTH = 8_192`,
`K = 32`, Blake2b-256 throughout.

## Goals

- Full miner replacement: the existing `tools/tensorium-miner` (solo +
  stratum clients, NVML monitoring, multi-GPU nonce partitioning) keeps its
  architecture; only the hashing core changes from SHA256d to TensorHash v1.
- Bit-for-bit agreement with the Rust reference, proven by a layered
  `--selftest` before any mining starts.
- Genesis-mine capability (no node required) + a verified end-to-end dry-run
  at the placeholder timestamp on a rented GPU.
- Node `/getblocktemplate` exposes `epoch_seed` so the miner never needs to
  derive it from chain history.

### Decisions made during brainstorming

- **Scope:** full miner replacement (not genesis-tool-only).
- **GPU access:** no GPU on the dev box — local validation is compile-only
  (CUDA toolkit installs and compiles without a GPU); runtime testing on a
  rented vast.ai RTX 3090/4090.
- **Genesis dry-run:** in scope. The *real* genesis re-mine is a launch-time
  step because the nonce depends on the final genesis timestamp, which is
  chosen at launch.
- **Difficulty policy:** keep 42 bits as shipped in Phase B; recalibrate
  `initial_leading_zero_bits` in a small follow-up commit only if benchmarks
  show a single 3090/4090 finds blocks >4× faster or slower than the 60 s
  target. (Retargeting is active from block 0, so miscalibration self-corrects
  within hours either way.)
- **Approach:** drop-in kernel swap inside the existing miner (rejected:
  fresh C++ rewrite, Rust-host/cudarc miner — both increase the amount of
  code that can only be debugged on rented GPU time).

## 1. Miner Changes (`tools/tensorium-miner/`)

### New files

- **`blake2b.cuh`** — Blake2b-256 implementation written once and compiled
  for both targets (`__host__ __device__` functions). Input sizes are small
  and fixed-shape (40 B for dataset elements/index seeds, ~110 B for the
  attempt digest, ~142 B for the final hash), so only 1–2 compression calls
  per invocation; no streaming API needed.
- **`tensorhash.cuh`** — `__device__` per-attempt TensorHash, mirroring
  `tensorium_tensorhash::pow_hash` exactly:
  1. `digest = Blake2b256(prefix || nonce_le)`
  2. accumulator `[u64;4]` from digest (LE)
  3. for `j in 0..32`: `idx = LE_u64(Blake2b256(digest || j_le)[0..8]) % N`,
     load 32 B element from the VRAM dataset, TensorMix fold
     (`acc[m] = acc[m] * (elem[m]|1) + rotl13(elem[(m+1)%4])`, wrapping)
  4. `Blake2b256(prefix || nonce_le || acc_bytes)`, compare leading zero bits
     against the target.
- **`tensorhash_kernel.cu`** — replaces `mining_kernel.cu`. Exposes a C
  interface in the same style as the current `mining_ctx_*` functions:
  - `tensorhash_ctx_create(header_len)` / `tensorhash_ctx_destroy` — also
    owns the dataset device buffer (19.2 GB `cudaMalloc`).
  - `tensorhash_generate_dataset(ctx, epoch_seed)` — grid-stride kernel,
    each thread writes `Blake2b256(seed || i_le)` for its indices.
    Expected runtime: seconds to ~1 min per epoch change (~5.7 days apart).
  - `tensorhash_dataset_spotcheck(ctx, count, out_elems, indices)` — copies
    selected elements back to host for selftest comparison.
  - `launch_tensorhash_kernel_ctx(ctx, header_template, difficulty_bits,
    start_nonce, blocks, threads, iters_per_thread, nonce_out)` — mining
    kernel; one nonce per thread per iteration, grid-stride over the nonce
    range, atomic found-flag early exit. Per attempt ≈ 35 Blake2b
    compressions + 32 random 32 B global loads (`__ldg`).
- **`host_tensorhash.h` / `host_tensorhash.cpp`** — host CPU reference:
  `host_dataset_element(seed, i)` and `host_pow_hash(prefix, len, nonce,
  seed)`. Verification is cheap on CPU because only the K=32 touched elements
  are recomputed on demand (same property the Rust node verifier relies on).
  Used for (a) share/block pre-verification in `gpu_worker.cu` before
  submission — replacing the host SHA256d code — and (b) selftest expected
  values for arbitrary inputs.

### Deleted files

- `sha256d.cuh`, `mining_kernel.cu` — replaced.
- `mine_genesis.cu` — genesis mining folds into the main binary
  (`--mode genesis`), reusing the dataset context and selftest.
- `main.cu` — dead legacy single-file miner; not referenced by the Makefile.

### Modified files

- **`common.h`** — `JobDesc` gains `uint8_t epoch_seed[32]`. `HEADER_MAX`
  (192) already fits the ~102 B MAINNET pow prefix + 8 B nonce.
- **`gpu_worker.cu`** —
  - **VRAM check at startup:** query free device memory; require ≥ 20 GB
    free (19.2 GB dataset + working headroom). Refuse to start below that
    with an error naming RTX 3090 / 24 GB as the minimum supported card.
  - **Dataset lifecycle:** track the seed the dataset was generated from;
    when a published job carries a different `epoch_seed`, regenerate
    (log the stall; mining pauses on that GPU until done) and re-run the
    auto spot-check before resuming.
  - Share/block pre-verification switches from host SHA256d to
    `host_pow_hash`.
- **`solo_client.cpp`** — parse `epoch_seed` (64-char hex) from the
  `/getblocktemplate` response into `JobDesc`. Missing field = hard error
  (node too old).
- **`stratum_client.cpp`** — parse `epoch_seed` from the pool job if present;
  if absent, warn once and use the zero seed (correct for epoch 0; pool-side
  forwarding is Phase A3).
- **`main.cpp`** — new flags (below), wire selftest/benchmark/genesis modes.
- **`Makefile`** — source list updated; `WITH_NVML` and ARCH handling
  unchanged.
- **`README.md`** — rewrite for TensorHash: hardware requirements (24 GB+
  VRAM, RTX 3090 minimum), dataset/epoch explanation, new modes, updated
  tuning guidance; drop the SHA256d hashrate tables.

### New CLI modes

- `--selftest` — run the full KAT verification (section 3) and exit.
- `--benchmark [secs]` — generate the dataset (timed), then measure
  sustained hashrate for `secs` (default 60) and report; feeds the
  difficulty-recalibration decision.
- `--mode genesis --prefix <hex> --bits <n>` — mine an arbitrary header
  prefix at `n` leading zero bits with the fixed zero epoch seed, print the
  winning nonce, and exit. No RPC/node required. Multi-GPU capable via the
  existing per-device nonce partitioning.

In every mode, dataset generation is followed automatically by the cheap
spot-check layer (section 3, layers 1–2); a mismatch refuses to mine. The
full `--selftest` additionally runs layers 3–4.

## 2. Node-Side Support (`crates/tensorium-node`)

- **`/getblocktemplate` response** gains an `"epoch_seed"` field, computed via
  `state.epoch_seed_for_height(block.header.height)` and serialized like every
  other `Hash256` in the response (JSON array of 32 byte values, same as
  `previous_hash`) — the miner parses it with its existing
  `extract_byte_array` helper. (Stratum, by contrast, uses 64-char hex strings
  per the existing pool convention — relevant in Phase A3.)
- **`devnet` subcommand family** (`devnet init|rpc|status`) — runs a node on
  the existing low-difficulty `TESTNET` params (20 bits, CPU-mineable
  genesis). Today the node CLI only serves `MAINNET`, whose genesis cannot be
  initialized until the re-mine — so the end-to-end live-path test
  (template → GPU mine → submitblock) needs a runnable devnet mode. Uses
  separate state/mempool paths (`TENSORIUM_DEVNET_STATE` /
  `TENSORIUM_DEVNET_MEMPOOL` env overrides).
- **`print-genesis-prefix --timestamp <unix>`** (new CLI subcommand) —
  builds the MAINNET genesis block exactly as `init_mainnet_state` would for
  that timestamp and prints the header's `pow_prefix_bytes` as hex plus the
  difficulty bits (42). Requires making `BlockHeader::pow_prefix_bytes`
  `pub` in `tensorium-core` (currently private). This keeps consensus
  serialization out of the C code entirely — the CUDA genesis miner consumes
  opaque prefix bytes.
- **`verify-genesis --timestamp <unix> --nonce <n>`** (new CLI subcommand) —
  reconstructs the genesis header with that nonce and checks
  `header_meets_work` via the Rust reference, printing pass/fail and the
  pow-hash. Run before pasting the mined nonce into `MAINNET_GENESIS_NONCE`.

Pool (`tensorium-pool`) job format changes are **out of scope** (Phase A3).

## 3. Selftest — Consensus Safety Layers

The GPU implementation must match `tensorium-tensorhash` bit-for-bit. The
Rust KATs are the root of trust; their hex values are hardcoded in the miner:

1. **Host reference vs Rust KATs:** `host_dataset_element([0;32], 0)` must
   equal `4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800`;
   `host_pow_hash("tensorhash-v1-kat-vector", 12345, [0;32])` must equal
   `9eddf122dc2f33d206ef3bb7f2e32fbd049fa00f9be7cb9a98f6f7055666e47f`.
   Catches host Blake2b/endianness bugs with zero GPU involvement.
2. **GPU dataset spot-check** (auto-runs after every dataset generation):
   element 0, element N−1, and 4096 random indices read back from VRAM and
   compared against the host reference for the active seed.
3. **KAT through the real kernel path:** mine the KAT header with the nonce
   range pinned to exactly 12345 and compare the kernel's computed pow-hash
   to the Rust KAT value (kernel gets a debug output slot for this).
4. **Randomized cross-check:** a batch of 1024 random
   (header, nonce) attempts computed by the GPU kernel and compared to
   `host_pow_hash`, under both the zero seed and one non-zero seed.

Any mismatch at any layer → print diagnostics and refuse to mine.

## 4. Validation & Benchmark Plan

1. **Local (no GPU):** install the CUDA toolkit on the dev box (nvcc
   compiles without a GPU). Gate: clean `make` for `sm_86`, `sm_89`,
   `sm_90`; clean workspace `cargo test` + `cargo clippy` for the node-side
   changes.
2. **Rented GPU (vast.ai, RTX 3090 or 4090, 24 GB):**
   - `--selftest` passes (all four layers).
   - `--benchmark`: record dataset-generation time and sustained hashrate;
     write results into `tools/tensorium-miner/README.md` and evaluate the
     42-bit calibration (expected attempts for 42 bits ≈ 2^42 ≈ 4.4×10¹²;
     at an assumed 50–150 MH/s that is roughly 8–24 GPU-hours per block —
     the benchmark replaces these assumptions with measurements).
   - **Live-path test:** run a `tensorium-node devnet` (TESTNET-params) node
     on the rental box, solo-mine against it — validates template parsing (incl.
     `epoch_seed`), mining, and `submitblock` acceptance end-to-end.
   - **Genesis dry-run:** `print-genesis-prefix` at the placeholder
     timestamp (`1_780_272_000`) → `--mode genesis --prefix … --bits 42` →
     `verify-genesis` confirms. The resulting nonce is recorded in the
     benchmark notes but **not** committed as `MAINNET_GENESIS_NONCE` —
     the real nonce is mined at launch once the final timestamp is chosen.
   - *Stretch (not a gate):* drive a TESTNET chain across an epoch boundary
     (8192 blocks at low difficulty) to observe a live dataset regeneration.

## 5. Genesis Re-Mine Workflow (launch-time, enabled by this phase)

1. Choose the final launch timestamp → update `MAINNET_GENESIS_TIMESTAMP`.
2. `tensorium-node print-genesis-prefix --timestamp <t>` → prefix hex.
3. On GPU box: `tensorium-miner --mode genesis --prefix <hex> --bits 42`
   (rent something large if the benchmark says a 3090 is too slow).
4. `tensorium-node verify-genesis --timestamp <t> --nonce <n>` → pass.
5. Commit the nonce into `MAINNET_GENESIS_NONCE`. (`init_genesis_nonce`
   still fails loudly with `StateError::MiningFailed` on any bad nonce.)

## Dependencies & Out of Scope

- **Depends on:** Phase A1 (algorithm + Rust reference, merged), Phase B
  (MAINNET params, merged).
- **Out of scope:**
  - Phase A3 — pool share validation against TensorHash and pool→stratum
    `epoch_seed` forwarding.
  - The real genesis re-mine and final timestamp choice (launch-time step;
    workflow above).
  - Changing `DATASET_N` / `EPOCH_LENGTH` / `K` — consensus-pinned since
    Phase A1; any retuning would be a deliberate consensus change with new
    KATs, not part of this phase.
  - Phases C (explorer/wallet/bridge), D (VPS redeploy), E (docs/branding).
