# TensorHash v1 Phase A2 — CUDA GPU Miner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the SHA256d CUDA kernel in `tools/tensorium-miner` with TensorHash v1 (bit-for-bit equal to `crates/tensorium-tensorhash`), add selftest/benchmark/genesis modes, and add the node-side support (`epoch_seed` in templates, genesis prefix/verify subcommands, devnet runmode) needed to mine and verify the new chain.

**Architecture:** Drop-in kernel swap — the miner's job/share/thread architecture, solo+stratum clients, and NVML monitoring stay; only the hashing core changes. A single-source Blake2b-256 (`blake2b.cuh`) compiles for both host and device; the host reference (`host_tensorhash.cpp`) recomputes only the K=32 touched dataset elements per verification, while the GPU materializes the full 19.2 GB dataset in VRAM once per epoch. Consensus safety is pinned by hardcoded KAT vectors cross-derived from the Rust reference.

**Tech Stack:** CUDA C (nvcc, sm_86+), C/C++ (g++), Rust (node-side), pthreads. No new Rust dependencies.

**Spec:** `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a2-design.md`

**Repo:** `/root/.openclaw/workspace/tensorium-core` (work on `main`; this box has NO GPU — CUDA code is compile-gated locally with nvcc, runtime-tested later on a vast.ai rental per Task 13).

**Reference KAT vectors** (all verified against the Rust crate and an independent Python `hashlib.blake2b` implementation; hex, Blake2b-256 unless noted):

| # | Input | Expected |
|---|---|---|
| V1 | `blake2b256(b"")` | `0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8` |
| V2 | `blake2b256(b"abc")` | `bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319` |
| V3 | `blake2b256(142 × b"a")` (two-block path) | `b318961b001b73c05a5cd3c224fa1468772a46b039ca9ad84ff1788a321bf49e` |
| V4 | `dataset_element([0;32], 0)` | `4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800` |
| V5 | `dataset_element([0;32], 599_999_999)` | `b7bc37d22421db9279c262ef23d75a606372411972b589410f32b9ca22b82e81` |
| V6 | `dataset_element([0;32], 123_456_789)` | `6cb58c6796255d9e11b3db3237571be55114bc5cc3b11dc137eae82547fde646` |
| V7 | `pow_hash(b"tensorhash-v1-kat-vector", 12345, [0;32])` | `9eddf122dc2f33d206ef3bb7f2e32fbd049fa00f9be7cb9a98f6f7055666e47f` |
| V8 | `pow_hash(102 × b"x", 777, [1;32])` (real prefix length, non-zero seed) | `cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491` |

---

### Task 1: Core — make `BlockHeader::pow_prefix_bytes` public

**Files:**
- Modify: `crates/tensorium-core/src/block.rs:164` (visibility) and the `pow_hash_tests` module in the same file

- [ ] **Step 1: Write the failing test**

Add to the existing `mod pow_hash_tests` in `crates/tensorium-core/src/block.rs`:

```rust
    #[test]
    fn pow_prefix_is_nonce_independent_and_102_bytes_for_mainnet() {
        let mut header = BlockHeader {
            version: 1,
            chain_id: "tensorium-mainnet".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1,
            leading_zero_bits: 42,
            nonce: 99,
        };
        let prefix = header.pow_prefix_bytes();
        // version(4) + chain_id(17) + height(8) + prev(32) + merkle(32)
        // + timestamp(8) + bits(1) = 102 — this is the byte count the CUDA
        // miner's genesis mode receives via print-genesis-prefix.
        assert_eq!(prefix.len(), 102);
        header.nonce = 12345;
        assert_eq!(prefix, header.pow_prefix_bytes(), "prefix must not depend on nonce");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core pow_prefix_is_nonce_independent 2>&1 | tail -5`
Expected: compile error — `pow_prefix_bytes` is private (the test module is inside the same file so it may actually compile; if it compiles and passes, the visibility change below is still required for tensorium-node — proceed).

- [ ] **Step 3: Make it public**

In `crates/tensorium-core/src/block.rs` change:

```rust
    fn pow_prefix_bytes(&self) -> Vec<u8> {
```

to:

```rust
    pub fn pow_prefix_bytes(&self) -> Vec<u8> {
```

(keep the existing doc comment).

- [ ] **Step 4: Run tests**

Run: `cargo test -p tensorium-core pow_ 2>&1 | tail -5`
Expected: PASS (all `pow_hash_tests` including the new one)

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/block.rs
git commit -m "feat(core): make BlockHeader::pow_prefix_bytes public for the GPU miner genesis flow"
```

---

### Task 2: Node — `epoch_seed` in `/getblocktemplate`

**Files:**
- Modify: `crates/tensorium-node/src/main.rs` (`getblocktemplate` handler, ~line 1712)

No practical unit test exists for the monolithic RPC handler (it writes to a TcpStream); this one-line JSON addition is covered end-to-end by the devnet live-path check in Task 12/13.

- [ ] **Step 1: Add the field**

In the `("GET", path) if path.starts_with("/getblocktemplate/")` arm, the response currently is:

```rust
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": params.chain_id,
                    "height": block.header.height,
                    "previous_hash": block.header.previous_hash,
                    "leading_zero_bits": block.header.leading_zero_bits,
                    "tx_count": block.transactions.len(),
                    "template": block,
                }),
            )
```

Add one entry (`Hash256` serializes as a JSON array of 32 bytes, same as `previous_hash`):

```rust
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": params.chain_id,
                    "height": block.header.height,
                    "previous_hash": block.header.previous_hash,
                    "leading_zero_bits": block.header.leading_zero_bits,
                    "epoch_seed": state.epoch_seed_for_height(block.header.height),
                    "tx_count": block.transactions.len(),
                    "template": block,
                }),
            )
```

- [ ] **Step 2: Build + run node tests**

Run: `cargo test -p tensorium-node 2>&1 | tail -3`
Expected: all existing tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): include epoch_seed in getblocktemplate response for TensorHash miners"
```

---

### Task 3: Node — `print-genesis-prefix` + `verify-genesis` subcommands

**Files:**
- Modify: `crates/tensorium-node/src/main.rs` (extract `genesis_header_template`, add two CLI arms, add tests, update `print_help`)

- [ ] **Step 1: Write the failing tests**

Add to the node's `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn genesis_header_template_matches_consensus_shape() {
        let header = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
        assert_eq!(header.height, 0);
        assert_eq!(header.nonce, 0);
        assert_eq!(header.chain_id, MAINNET.chain_id);
        assert_eq!(header.leading_zero_bits, MAINNET.initial_leading_zero_bits);
        assert_eq!(header.previous_hash, Hash256::ZERO);
        // pow prefix is what print-genesis-prefix emits for the CUDA miner
        assert_eq!(header.pow_prefix_bytes().len(), 102);
    }

    #[test]
    fn genesis_header_template_depends_on_timestamp() {
        let a = genesis_header_template(1_780_272_000);
        let b = genesis_header_template(1_780_272_001);
        assert_ne!(a.pow_prefix_bytes(), b.pow_prefix_bytes());
    }

    #[test]
    fn unmined_genesis_nonce_zero_fails_work_check() {
        // The placeholder nonce 0 must not satisfy 42-bit difficulty —
        // verify-genesis relies on header_meets_work for its pass/fail.
        let header = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
        assert!(!header_meets_work(&header, Hash256::ZERO));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-node genesis_header_template 2>&1 | tail -5`
Expected: FAIL — `genesis_header_template` not found.

- [ ] **Step 3: Extract `genesis_header_template` from `mine_genesis_multithreaded`**

In `crates/tensorium-node/src/main.rs`, directly above `mine_genesis_multithreaded` (~line 316), add:

```rust
/// Genesis block header (nonce = 0) for the given launch timestamp.
/// Must construct exactly what `init_genesis_nonce` validates via
/// `candidate_block` — same coinbase, same merkle root.
fn genesis_header_template(timestamp_seconds: u64) -> BlockHeader {
    let params = &MAINNET;
    let reward = reward_at_height(params, 0);
    let coinbase = Transaction::genesis_coinbase(
        reward,
        "genesis",
        params.founder_allocation_atoms,
        params.founder_address,
        params.genesis_allocations,
    );
    let real_merkle = compute_merkle_root(&[coinbase]);
    BlockHeader {
        version: 1,
        chain_id: params.chain_id.to_owned(),
        height: 0,
        previous_hash: Hash256::ZERO,
        merkle_root: real_merkle,
        timestamp_seconds,
        leading_zero_bits: params.initial_leading_zero_bits,
        nonce: 0,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
```

Then replace the `let header_template = { ... };` block inside `mine_genesis_multithreaded` with:

```rust
    let header_template = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
```

(The replaced block is the `{ let params = &MAINNET; ... BlockHeader { ... } }` expression — its body is now the function above, byte-identical.)

- [ ] **Step 4: Add the CLI arms**

In `run()`'s top-level `match command`, after the `"unban"` arm, add:

```rust
        "print-genesis-prefix" => {
            let timestamp: u64 = match args.get(2) {
                Some(s) => s.parse().map_err(|_| format!("invalid timestamp: {s}"))?,
                None => MAINNET_GENESIS_TIMESTAMP,
            };
            let header = genesis_header_template(timestamp);
            println!("chain_id    = {}", MAINNET.chain_id);
            println!("timestamp   = {timestamp}");
            println!("bits        = {}", header.leading_zero_bits);
            println!("merkle_root = {}", header.merkle_root);
            println!("prefix_hex  = {}", hex_lower(&header.pow_prefix_bytes()));
            println!();
            println!("mine with:  tensorium-miner --mode genesis --prefix <prefix_hex> --bits {}",
                header.leading_zero_bits);
        }
        "verify-genesis" => {
            let usage = "usage: tensorium-node verify-genesis <timestamp> <nonce>";
            let timestamp: u64 = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| usage.to_owned())?;
            let nonce: u64 = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| usage.to_owned())?;
            let mut header = genesis_header_template(timestamp);
            header.nonce = nonce;
            // Genesis is height 0 → epoch 0 → fixed zero seed.
            let pow = header.pow_hash(Hash256::ZERO);
            if header_meets_work(&header, Hash256::ZERO) {
                println!("VALID    pow_hash = {pow}");
                println!("paste into crates/tensorium-node/src/main.rs:");
                println!("  const MAINNET_GENESIS_TIMESTAMP: u64 = {timestamp};");
                println!("  const MAINNET_GENESIS_NONCE: u64 = {nonce};");
            } else {
                println!(
                    "INVALID  pow_hash = {pow}  (needs {} leading zero bits)",
                    header.leading_zero_bits
                );
                std::process::exit(1);
            }
        }
```

Also add both commands to `print_help()` output (one line each, mirroring the existing style):

```rust
    println!("  print-genesis-prefix [ts]   print MAINNET genesis pow-prefix hex for GPU mining");
    println!("  verify-genesis <ts> <nonce> check a mined genesis nonce against MAINNET difficulty");
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p tensorium-node genesis 2>&1 | tail -5`
Expected: PASS (3 new tests + any pre-existing `genesis`-named tests).

- [ ] **Step 6: Smoke-run the subcommands**

```bash
cargo run -p tensorium-node -- print-genesis-prefix | head -8
cargo run -p tensorium-node -- verify-genesis 1780272000 0; echo "exit=$?"
```
Expected: prefix output with 204-char `prefix_hex`; verify prints `INVALID` and `exit=1`.

- [ ] **Step 7: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): print-genesis-prefix and verify-genesis subcommands for the GPU genesis re-mine"
```

---

### Task 4: Node — `devnet` runmode (TESTNET params)

**Files:**
- Modify: `crates/tensorium-node/src/main.rs` (env-path helpers, CLI arm, help)

The node CLI today only serves `MAINNET`, whose genesis can't initialize until the re-mine. The live-path test (template → GPU mine → submit) needs a runnable low-difficulty chain: TESTNET (20 bits, CPU-mineable genesis).

- [ ] **Step 1: Add path helpers**

Next to `mc_state_path_from_env` (~line 299), add:

```rust
fn devnet_state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_DEVNET_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tensorium-devnet.json"))
}

fn devnet_mempool_path_from_env() -> PathBuf {
    env::var("TENSORIUM_DEVNET_MEMPOOL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tensorium-devnet-mempool.json"))
}
```

- [ ] **Step 2: Add the CLI arm**

In `run()`'s top-level `match command`, after the new `"verify-genesis"` arm, add (import `TESTNET` alongside `MAINNET` in the existing `use tensorium_core::...` list if not already imported):

```rust
        "devnet" => {
            let subcmd = args.get(2).map(String::as_str).unwrap_or("help");
            match subcmd {
                "init" => {
                    let mut state = ChainState::open_db(&devnet_state_path_from_env())?;
                    state
                        .init_genesis(&TESTNET, now_seconds(), u64::MAX)
                        .map_err(|err| err.to_string())?;
                    println!("devnet (TESTNET params, {} bits) genesis initialized",
                        TESTNET.initial_leading_zero_bits);
                    print_status(&state, &TESTNET);
                }
                "rpc" => {
                    let bind = args.get(3).map(String::as_str).unwrap_or("127.0.0.1:43332");
                    serve_rpc(
                        bind,
                        devnet_state_path_from_env(),
                        devnet_mempool_path_from_env(),
                        &TESTNET,
                    )?;
                }
                "status" => {
                    let state = load_state(&devnet_state_path_from_env())?;
                    print_status(&state, &TESTNET);
                }
                _ => {
                    println!("usage: tensorium-node devnet init|rpc [bind]|status");
                    println!("  low-difficulty TESTNET-params chain for miner live-path testing");
                    println!("  env: TENSORIUM_DEVNET_STATE, TENSORIUM_DEVNET_MEMPOOL");
                }
            }
        }
```

Note: `ChainState::open_db` — match the call pattern of `init_mainnet_candidate_state` (line ~431). If `serve_rpc`/`load_state` take `&Path` vs `PathBuf` differently than shown, follow the existing `"rpc"`/`"status"` arms' exact calling convention.

Add to `print_help()`:

```rust
    println!("  devnet init|rpc|status      low-difficulty TESTNET chain for miner testing");
```

- [ ] **Step 3: Build + test**

Run: `cargo test -p tensorium-node 2>&1 | tail -3 && cargo clippy -p tensorium-node 2>&1 | tail -3`
Expected: tests PASS, clippy clean (warnings only).

No unit test for the arm itself: `devnet init` CPU-mines a 20-bit genesis (~1M attempts × 34 Blake2b), too slow under the dev test profile; it runs for real in Task 13's runbook with a release binary.

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): devnet runmode on TESTNET params for GPU miner live-path testing"
```

---

### Task 5: Install CUDA toolkit (compile-only, no GPU needed)

**Files:** none (environment)

- [ ] **Step 1: Install**

```bash
apt-get update && apt-get install -y nvidia-cuda-toolkit
nvcc --version
```
Expected: nvcc 12.x version banner. nvcc compiles `.cu` without any GPU present.

- [ ] **Step 2: Fallback if the apt package is unavailable**

If apt can't find it, fetch NVIDIA's installer (network required, ~4 GB):

```bash
wget -q https://developer.download.nvidia.com/compute/cuda/12.4.1/local_installers/cuda_12.4.1_550.54.15_linux.run
sh cuda_12.4.1_550.54.15_linux.run --silent --toolkit --override
export PATH=/usr/local/cuda/bin:$PATH
nvcc --version
```

If neither works (no network), mark every later `nvcc` compile gate in Tasks 8–10 as **deferred to the GPU box** and continue — the g++-only `make test-host` gate (Tasks 6–7) still runs locally.

---

### Task 6: Miner — `tensorhash_params.h` + `blake2b.cuh` + host KAT harness

**Files:**
- Create: `tools/tensorium-miner/tensorhash_params.h`
- Create: `tools/tensorium-miner/blake2b.cuh`
- Create: `tools/tensorium-miner/test_host_tensorhash.cpp` (Blake2b vectors first; extended in Task 7)
- Modify: `tools/tensorium-miner/Makefile` (add `test-host` target only — full rewrite happens in Task 10)

- [ ] **Step 1: Create `tensorhash_params.h`**

```c
// tools/tensorium-miner/tensorhash_params.h
// TensorHash v1 consensus parameters — MUST match crates/tensorium-tensorhash.
#pragma once
#include <stdint.h>

#define TH_ELEMENT_SIZE   32
#define TH_DATASET_N      600000000ULL
#define TH_DATASET_BYTES  (TH_DATASET_N * (uint64_t)TH_ELEMENT_SIZE)  /* 19.2 GB */
#define TH_EPOCH_LENGTH   8192ULL
#define TH_K              32
#define TH_PREFIX_MAX     184   /* HEADER_MAX(192) - 8 nonce bytes */
```

- [ ] **Step 2: Write the failing test harness (Blake2b vectors V1–V3)**

```cpp
// tools/tensorium-miner/test_host_tensorhash.cpp
// Host-side KAT harness — runs on any x86 box, no GPU/CUDA required.
// Build+run: make test-host
#include "blake2b.cuh"
#include <stdio.h>
#include <string.h>

static int hex_eq(const uint8_t h[32], const char *hex) {
    char got[65];
    for (int i = 0; i < 32; i++) sprintf(got + i * 2, "%02x", h[i]);
    got[64] = '\0';
    if (strcmp(got, hex) == 0) return 1;
    fprintf(stderr, "  got      %s\n  expected %s\n", got, hex);
    return 0;
}

static int g_failures = 0;
#define CHECK(name, cond) do { \
    if (cond) printf("PASS  %s\n", name); \
    else { printf("FAIL  %s\n", name); g_failures++; } \
} while (0)

static void blake2b_vectors(void) {
    uint8_t out[32];
    th_blake2b256((const uint8_t *)"", 0, out);
    CHECK("V1 blake2b256(empty)", hex_eq(out,
        "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"));

    th_blake2b256((const uint8_t *)"abc", 3, out);
    CHECK("V2 blake2b256(abc)", hex_eq(out,
        "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"));

    uint8_t a142[142];
    memset(a142, 'a', sizeof(a142));
    th_blake2b256(a142, 142, out);  /* exercises the two-block path */
    CHECK("V3 blake2b256(142*'a')", hex_eq(out,
        "b318961b001b73c05a5cd3c224fa1468772a46b039ca9ad84ff1788a321bf49e"));
}

int main(void) {
    blake2b_vectors();
    if (g_failures) { printf("\n%d FAILURE(S)\n", g_failures); return 1; }
    printf("\nall host KATs pass\n");
    return 0;
}
```

Append to `tools/tensorium-miner/Makefile` (leave everything else untouched for now):

```make
# Host-only KAT harness — no GPU/CUDA needed (g++ only).
test-host: test_host_tensorhash.cpp blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -o test_host_tensorhash test_host_tensorhash.cpp
	./test_host_tensorhash
```

(After Task 7 this target gains `host_tensorhash.cpp` — see that task.)

- [ ] **Step 3: Run to verify it fails**

Run: `cd tools/tensorium-miner && make test-host`
Expected: FAIL — `blake2b.cuh: No such file or directory`.

- [ ] **Step 4: Implement `blake2b.cuh`**

```c
// tools/tensorium-miner/blake2b.cuh
// Blake2b-256 (RFC 7693, sequential, unkeyed) — single source compiled for
// both host (g++) and device (nvcc). CONSENSUS-CRITICAL: must match the
// `blake2` Rust crate used by crates/tensorium-tensorhash bit-for-bit;
// pinned by the KAT vectors in test_host_tensorhash.cpp and --selftest.
#pragma once
#include <stdint.h>

#ifdef __CUDACC__
#define TH_HD __host__ __device__ __forceinline__
#else
#define TH_HD static inline
#endif

#define TH_B2B_IV_INIT { \
    0x6a09e667f3bcc908ULL, 0xbb67ae8584caa73bULL, \
    0x3c6ef372fe94f82bULL, 0xa54ff53a5f1d36f1ULL, \
    0x510e527fade682d1ULL, 0x9b05688c2b3e6c1fULL, \
    0x1f83d9abfb41bd6bULL, 0x5be0cd19137e2179ULL }

#define TH_B2B_SIGMA_INIT { \
    { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15}, \
    {14,10, 4, 8, 9,15,13, 6, 1,12, 0, 2,11, 7, 5, 3}, \
    {11, 8,12, 0, 5, 2,15,13,10,14, 3, 6, 7, 1, 9, 4}, \
    { 7, 9, 3, 1,13,12,11,14, 2, 6, 5,10, 4, 0,15, 8}, \
    { 9, 0, 5, 7, 2, 4,10,15,14, 1,11,12, 6, 8, 3,13}, \
    { 2,12, 6,10, 0,11, 8, 3, 4,13, 7, 5,15,14, 1, 9}, \
    {12, 5, 1,15,14,13, 4,10, 0, 7, 6, 3, 9, 2, 8,11}, \
    {13,11, 7,14,12, 1, 3, 9, 5, 0,15, 4, 8, 6, 2,10}, \
    { 6,15,14, 9,11, 3, 0, 8,12, 2,13, 7, 1, 4,10, 5}, \
    {10, 2, 8, 4, 7, 6, 1, 5,15,11, 9,14, 3,12,13, 0}, \
    { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15}, \
    {14,10, 4, 8, 9,15,13, 6, 1,12, 0, 2,11, 7, 5, 3} }

/* Two physical copies so the device pass reads constant memory while host
   code (same translation unit) reads ordinary statics. The literal tables
   come from one macro each, so they cannot diverge. */
#ifdef __CUDACC__
__constant__ static const uint64_t TH_B2B_IV_D[8]        = TH_B2B_IV_INIT;
__constant__ static const uint8_t  TH_B2B_SIGMA_D[12][16] = TH_B2B_SIGMA_INIT;
#endif
static const uint64_t TH_B2B_IV_H[8]        = TH_B2B_IV_INIT;
static const uint8_t  TH_B2B_SIGMA_H[12][16] = TH_B2B_SIGMA_INIT;

#ifdef __CUDA_ARCH__
#define TH_B2B_IV    TH_B2B_IV_D
#define TH_B2B_SIGMA TH_B2B_SIGMA_D
#else
#define TH_B2B_IV    TH_B2B_IV_H
#define TH_B2B_SIGMA TH_B2B_SIGMA_H
#endif

TH_HD uint64_t th_rotr64(uint64_t x, int n) { return (x >> n) | (x << (64 - n)); }

#define TH_G(v, a, b, c, d, x, y) do { \
    v[a] = v[a] + v[b] + (x); v[d] = th_rotr64(v[d] ^ v[a], 32); \
    v[c] = v[c] + v[d];       v[b] = th_rotr64(v[b] ^ v[c], 24); \
    v[a] = v[a] + v[b] + (y); v[d] = th_rotr64(v[d] ^ v[a], 16); \
    v[c] = v[c] + v[d];       v[b] = th_rotr64(v[b] ^ v[c], 63); \
} while (0)

TH_HD void th_blake2b_compress(uint64_t h[8], const uint8_t block[128],
                               uint64_t t, int last) {
    uint64_t m[16];
    for (int i = 0; i < 16; i++) {
        uint64_t w = 0;
        for (int k = 7; k >= 0; k--) w = (w << 8) | block[i * 8 + k];
        m[i] = w;  /* little-endian load */
    }
    uint64_t v[16];
    for (int i = 0; i < 8; i++) v[i] = h[i];
    for (int i = 0; i < 8; i++) v[8 + i] = TH_B2B_IV[i];
    v[12] ^= t;                 /* t_hi is always 0 for our input sizes */
    if (last) v[14] = ~v[14];
    for (int r = 0; r < 12; r++) {
        const uint8_t *s = TH_B2B_SIGMA[r];
        TH_G(v, 0, 4,  8, 12, m[s[0]],  m[s[1]]);
        TH_G(v, 1, 5,  9, 13, m[s[2]],  m[s[3]]);
        TH_G(v, 2, 6, 10, 14, m[s[4]],  m[s[5]]);
        TH_G(v, 3, 7, 11, 15, m[s[6]],  m[s[7]]);
        TH_G(v, 0, 5, 10, 15, m[s[8]],  m[s[9]]);
        TH_G(v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        TH_G(v, 2, 7,  8, 13, m[s[12]], m[s[13]]);
        TH_G(v, 3, 4,  9, 14, m[s[14]], m[s[15]]);
    }
    for (int i = 0; i < 8; i++) h[i] ^= v[i] ^ v[8 + i];
}

/* One-shot Blake2b-256 of `len` bytes (len <= a few hundred in TensorHash). */
TH_HD void th_blake2b256(const uint8_t *data, uint32_t len, uint8_t out[32]) {
    uint64_t h[8] = TH_B2B_IV_INIT;
    h[0] ^= 0x01010000ULL ^ 32ULL;  /* digest_length=32, fanout=1, depth=1 */

    uint32_t off = 0;
    while (len - off > 128) {       /* full non-final blocks */
        th_blake2b_compress(h, data + off, (uint64_t)off + 128, 0);
        off += 128;
    }
    uint8_t block[128];
    uint32_t rem = len - off;       /* 0..128 — final block, zero-padded */
    for (uint32_t i = 0; i < 128; i++) block[i] = (i < rem) ? data[off + i] : 0;
    th_blake2b_compress(h, block, (uint64_t)len, 1);

    for (int i = 0; i < 32; i++) out[i] = (uint8_t)(h[i / 8] >> (8 * (i % 8)));
}
```

- [ ] **Step 5: Run the harness**

Run: `cd tools/tensorium-miner && make test-host`
Expected: `PASS V1/V2/V3`, `all host KATs pass`, exit 0. If any vector fails, fix `blake2b.cuh` before proceeding — everything downstream depends on it.

- [ ] **Step 6: Commit**

```bash
git add tools/tensorium-miner/tensorhash_params.h tools/tensorium-miner/blake2b.cuh \
        tools/tensorium-miner/test_host_tensorhash.cpp tools/tensorium-miner/Makefile
git commit -m "feat(miner): host+device Blake2b-256 with KAT harness (TensorHash v1 groundwork)"
```

---

### Task 7: Miner — host TensorHash reference (`host_tensorhash.{h,cpp}`)

**Files:**
- Create: `tools/tensorium-miner/host_tensorhash.h`
- Create: `tools/tensorium-miner/host_tensorhash.cpp`
- Modify: `tools/tensorium-miner/test_host_tensorhash.cpp` (vectors V4–V8)
- Modify: `tools/tensorium-miner/Makefile` (`test-host` gains the new .cpp)

- [ ] **Step 1: Extend the failing test harness**

In `test_host_tensorhash.cpp`, add `#include "host_tensorhash.h"` below the existing include, add this function, and call `tensorhash_vectors();` from `main` after `blake2b_vectors();`:

```cpp
static void tensorhash_vectors(void) {
    uint8_t out[32];
    uint8_t zero_seed[32] = {0};

    host_dataset_element(zero_seed, 0, out);
    CHECK("V4 elem(0)", hex_eq(out,
        "4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800"));

    host_dataset_element(zero_seed, 599999999ULL, out);
    CHECK("V5 elem(N-1)", hex_eq(out,
        "b7bc37d22421db9279c262ef23d75a606372411972b589410f32b9ca22b82e81"));

    host_dataset_element(zero_seed, 123456789ULL, out);
    CHECK("V6 elem(123456789)", hex_eq(out,
        "6cb58c6796255d9e11b3db3237571be55114bc5cc3b11dc137eae82547fde646"));

    host_pow_hash((const uint8_t *)TH_KAT_POW_HEADER,
                  (uint32_t)strlen(TH_KAT_POW_HEADER),
                  TH_KAT_POW_NONCE, zero_seed, out);
    CHECK("V7 pow_hash KAT", hex_eq(out, TH_KAT_POW_HEX));

    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    host_pow_hash(xprefix, 102, 777, one_seed, out);
    CHECK("V8 pow_hash non-zero seed", hex_eq(out,
        "cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491"));

    CHECK("kat_check() aggregate", host_tensorhash_kat_check() == 0);
}
```

Update the Makefile `test-host` target:

```make
test-host: test_host_tensorhash.cpp host_tensorhash.cpp host_tensorhash.h blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -o test_host_tensorhash test_host_tensorhash.cpp host_tensorhash.cpp
	./test_host_tensorhash
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd tools/tensorium-miner && make test-host`
Expected: FAIL — `host_tensorhash.h: No such file or directory`.

- [ ] **Step 3: Implement the header**

```c
// tools/tensorium-miner/host_tensorhash.h
// Host CPU reference for TensorHash v1 — cheap verification (recomputes only
// the K=32 touched dataset elements per attempt; the full dataset lives only
// in GPU VRAM). Used for share/block pre-verification and selftest oracles.
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

void host_dataset_element(const uint8_t seed[32], uint64_t index, uint8_t out[32]);

/* prefix = serialized header bytes WITHOUT the trailing 8 nonce bytes.
   prefix_len <= TH_PREFIX_MAX. */
void host_pow_hash(const uint8_t *prefix, uint32_t prefix_len, uint64_t nonce,
                   const uint8_t seed[32], uint8_t out[32]);

int host_leading_zero_bits(const uint8_t hash[32]);

/* Runs every hardcoded KAT vector; returns 0 on full pass, else the 1-based
   index of the first failing vector. Selftest layer 1. */
int host_tensorhash_kat_check(void);

/* The full-pipeline KAT (Rust: pow_hash_known_answer_vector) — shared with
   the GPU selftest so the kernel path checks the same vector. */
extern const char    *TH_KAT_POW_HEADER;   /* "tensorhash-v1-kat-vector" */
extern const uint64_t TH_KAT_POW_NONCE;    /* 12345 */
extern const char    *TH_KAT_POW_HEX;      /* expected pow-hash hex */

/* 64-char hex -> 32 bytes; returns 1 on success. */
int th_hex32_to_bytes(const char *hex, uint8_t out[32]);

#ifdef __cplusplus
}
#endif
```

- [ ] **Step 4: Implement `host_tensorhash.cpp`**

```cpp
// tools/tensorium-miner/host_tensorhash.cpp
#include "host_tensorhash.h"
#include "tensorhash_params.h"
#include "blake2b.cuh"
#include <string.h>

static void le64_store(uint8_t *b, uint64_t v) {
    for (int i = 0; i < 8; i++) { b[i] = (uint8_t)v; v >>= 8; }
}
static uint64_t le64_load(const uint8_t *b) {
    uint64_t v = 0;
    for (int i = 7; i >= 0; i--) v = (v << 8) | b[i];
    return v;
}
static uint64_t rotl64(uint64_t x, int n) { return (x << n) | (x >> (64 - n)); }

void host_dataset_element(const uint8_t seed[32], uint64_t index, uint8_t out[32]) {
    uint8_t buf[40];
    memcpy(buf, seed, 32);
    le64_store(buf + 32, index);
    th_blake2b256(buf, 40, out);
}

void host_pow_hash(const uint8_t *prefix, uint32_t prefix_len, uint64_t nonce,
                   const uint8_t seed[32], uint8_t out[32]) {
    uint8_t buf[TH_PREFIX_MAX + 8 + 32];
    memcpy(buf, prefix, prefix_len);
    le64_store(buf + prefix_len, nonce);

    uint8_t digest[32];
    th_blake2b256(buf, prefix_len + 8, digest);

    uint64_t acc[4];
    for (int m = 0; m < 4; m++) acc[m] = le64_load(digest + m * 8);

    uint8_t ibuf[40], iseed[32], elem_bytes[32];
    memcpy(ibuf, digest, 32);
    for (uint64_t j = 0; j < TH_K; j++) {
        le64_store(ibuf + 32, j);
        th_blake2b256(ibuf, 40, iseed);
        uint64_t idx = le64_load(iseed) % TH_DATASET_N;

        host_dataset_element(seed, idx, elem_bytes);
        uint64_t elem[4];
        for (int m = 0; m < 4; m++) elem[m] = le64_load(elem_bytes + m * 8);

        uint64_t next[4];
        for (int m = 0; m < 4; m++)
            next[m] = acc[m] * (elem[m] | 1ULL) + rotl64(elem[(m + 1) & 3], 13);
        for (int m = 0; m < 4; m++) acc[m] = next[m];
    }

    /* final hash input: prefix || nonce_le || acc_bytes (buf already holds
       prefix||nonce — append the accumulator) */
    for (int m = 0; m < 4; m++) le64_store(buf + prefix_len + 8 + m * 8, acc[m]);
    th_blake2b256(buf, prefix_len + 8 + 32, out);
}

int host_leading_zero_bits(const uint8_t hash[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (hash[i] == 0) { bits += 8; continue; }
        unsigned x = hash[i];
        while (!(x & 0x80)) { bits++; x <<= 1; }
        break;
    }
    return bits;
}

const char    *TH_KAT_POW_HEADER = "tensorhash-v1-kat-vector";
const uint64_t TH_KAT_POW_NONCE  = 12345;
const char    *TH_KAT_POW_HEX =
    "9eddf122dc2f33d206ef3bb7f2e32fbd049fa00f9be7cb9a98f6f7055666e47f";

int th_hex32_to_bytes(const char *hex, uint8_t out[32]) {
    for (int i = 0; i < 32; i++) {
        int hi = hex[i * 2], lo = hex[i * 2 + 1];
        hi = (hi >= 'a') ? hi - 'a' + 10 : (hi >= 'A') ? hi - 'A' + 10 : hi - '0';
        lo = (lo >= 'a') ? lo - 'a' + 10 : (lo >= 'A') ? lo - 'A' + 10 : lo - '0';
        if (hi < 0 || hi > 15 || lo < 0 || lo > 15) return 0;
        out[i] = (uint8_t)((hi << 4) | lo);
    }
    return 1;
}

int host_tensorhash_kat_check(void) {
    uint8_t out[32], expect[32];
    uint8_t zero_seed[32] = {0};

    /* 1: blake2b two-block path */
    uint8_t a142[142];
    memset(a142, 'a', sizeof(a142));
    th_blake2b256(a142, 142, out);
    th_hex32_to_bytes("b318961b001b73c05a5cd3c224fa1468772a46b039ca9ad84ff1788a321bf49e", expect);
    if (memcmp(out, expect, 32) != 0) return 1;

    /* 2: dataset element 0 */
    host_dataset_element(zero_seed, 0, out);
    th_hex32_to_bytes("4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800", expect);
    if (memcmp(out, expect, 32) != 0) return 2;

    /* 3: dataset element N-1 */
    host_dataset_element(zero_seed, TH_DATASET_N - 1, out);
    th_hex32_to_bytes("b7bc37d22421db9279c262ef23d75a606372411972b589410f32b9ca22b82e81", expect);
    if (memcmp(out, expect, 32) != 0) return 3;

    /* 4: full pow_hash KAT (zero seed) */
    host_pow_hash((const uint8_t *)TH_KAT_POW_HEADER,
                  (uint32_t)strlen(TH_KAT_POW_HEADER),
                  TH_KAT_POW_NONCE, zero_seed, out);
    th_hex32_to_bytes(TH_KAT_POW_HEX, expect);
    if (memcmp(out, expect, 32) != 0) return 4;

    /* 5: pow_hash with non-zero seed + real 102-byte prefix length */
    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    host_pow_hash(xprefix, 102, 777, one_seed, out);
    th_hex32_to_bytes("cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491", expect);
    if (memcmp(out, expect, 32) != 0) return 5;

    return 0;
}
```

- [ ] **Step 5: Run the harness**

Run: `cd tools/tensorium-miner && make test-host`
Expected: V1–V8 + aggregate all PASS, exit 0.

- [ ] **Step 6: Commit**

```bash
git add tools/tensorium-miner/host_tensorhash.h tools/tensorium-miner/host_tensorhash.cpp \
        tools/tensorium-miner/test_host_tensorhash.cpp tools/tensorium-miner/Makefile
git commit -m "feat(miner): host TensorHash v1 reference implementation with full KAT coverage"
```

---

### Task 8: Miner — device kernels (`tensorhash.cuh`, `tensorhash_kernel.cu`)

**Files:**
- Create: `tools/tensorium-miner/tensorhash.cuh`
- Create: `tools/tensorium-miner/tensorhash_kernel.cu`

Compile gate only on this box (no GPU); runtime proof is `--selftest` in Task 13.

- [ ] **Step 1: Create `tensorhash.cuh`** (device per-attempt hash)

```c
// tools/tensorium-miner/tensorhash.cuh
// Device-side TensorHash v1 — mirrors host_tensorhash.cpp / the Rust
// reference exactly. Dataset element loads come from the VRAM dataset
// instead of being recomputed.
#pragma once
#include "blake2b.cuh"
#include "tensorhash_params.h"

__device__ __forceinline__ void th_le64_store_dev(uint8_t *b, uint64_t v) {
    #pragma unroll
    for (int i = 0; i < 8; i++) { b[i] = (uint8_t)v; v >>= 8; }
}

__device__ __forceinline__ uint64_t th_le64_load_dev(const uint8_t *b) {
    uint64_t v = 0;
    #pragma unroll
    for (int i = 7; i >= 0; i--) v = (v << 8) | b[i];
    return v;
}

__device__ __forceinline__ uint64_t th_rotl64_dev(uint64_t x, int n) {
    return (x << n) | (x >> (64 - n));
}

/* Full TensorHash v1 pow hash for one (prefix, nonce) attempt.
   prefix points at the nonce-less header bytes (constant or global mem),
   dataset is the 19.2 GB VRAM element table (32-byte aligned rows). */
__device__ void th_pow_hash_device(const uint8_t *prefix, uint32_t prefix_len,
                                   uint64_t nonce, const uint8_t *dataset,
                                   uint8_t out[32]) {
    uint8_t buf[TH_PREFIX_MAX + 8 + 32];
    for (uint32_t i = 0; i < prefix_len; i++) buf[i] = prefix[i];
    th_le64_store_dev(buf + prefix_len, nonce);

    uint8_t digest[32];
    th_blake2b256(buf, prefix_len + 8, digest);

    uint64_t acc[4];
    #pragma unroll
    for (int m = 0; m < 4; m++) acc[m] = th_le64_load_dev(digest + m * 8);

    uint8_t ibuf[40], iseed[32];
    #pragma unroll
    for (int i = 0; i < 32; i++) ibuf[i] = digest[i];

    for (uint64_t j = 0; j < TH_K; j++) {
        th_le64_store_dev(ibuf + 32, j);
        th_blake2b256(ibuf, 40, iseed);
        uint64_t idx = th_le64_load_dev(iseed) % TH_DATASET_N;

        /* rows are 32-byte aligned; little-endian arch => direct u64 loads
           match from_le_bytes. __ldg routes through the read-only cache. */
        const uint64_t *e = (const uint64_t *)(dataset + idx * 32ULL);
        uint64_t elem[4];
        elem[0] = __ldg(e + 0); elem[1] = __ldg(e + 1);
        elem[2] = __ldg(e + 2); elem[3] = __ldg(e + 3);

        uint64_t next[4];
        #pragma unroll
        for (int m = 0; m < 4; m++)
            next[m] = acc[m] * (elem[m] | 1ULL)
                    + th_rotl64_dev(elem[(m + 1) & 3], 13);
        #pragma unroll
        for (int m = 0; m < 4; m++) acc[m] = next[m];
    }

    #pragma unroll
    for (int m = 0; m < 4; m++) th_le64_store_dev(buf + prefix_len + 8 + m * 8, acc[m]);
    th_blake2b256(buf, prefix_len + 8 + 32, out);
}

__device__ __forceinline__ int th_leading_zero_bits_dev(const uint8_t h[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (h[i] == 0) { bits += 8; continue; }
        bits += __clz((unsigned)h[i]) - 24;
        break;
    }
    return bits;
}
```

- [ ] **Step 2: Create `tensorhash_kernel.cu`** (kernels + C API)

```c
// tools/tensorium-miner/tensorhash_kernel.cu
// Dataset generation + mining kernels and the C interface used by
// gpu_worker.cu and modes.cpp. Replaces the SHA256d mining_kernel.cu.
#include "tensorhash.cuh"
#include "host_tensorhash.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* Headroom beyond the dataset for context buffers + driver overhead. */
#define TH_VRAM_HEADROOM (768ULL << 20)

__constant__ static uint8_t  c_prefix[TH_PREFIX_MAX];
__constant__ static uint32_t c_prefix_len;
__constant__ static uint8_t  c_gen_seed[32];

// ── Kernels ──────────────────────────────────────────────────────────────────

__global__ void th_dataset_gen_kernel(uint8_t *dataset) {
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint8_t buf[40];
    #pragma unroll
    for (int i = 0; i < 32; i++) buf[i] = c_gen_seed[i];

    for (uint64_t i = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
         i < TH_DATASET_N; i += stride) {
        th_le64_store_dev(buf + 32, i);
        th_blake2b256(buf, 40, dataset + i * 32ULL);
    }
}

__global__ void th_mine_kernel(const uint8_t *dataset, uint8_t difficulty_bits,
                               uint64_t start_nonce, uint32_t iters,
                               int *found, uint64_t *result_nonce) {
    if (__ldg(found)) return;

    uint64_t gid    = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint64_t nonce  = start_nonce + gid;
    uint8_t hash[32];

    for (uint32_t i = 0; i < iters; i++) {
        if (__ldg(found)) return;
        th_pow_hash_device(c_prefix, c_prefix_len, nonce, dataset, hash);
        if (th_leading_zero_bits_dev(hash) >= (int)difficulty_bits) {
            if (atomicCAS(found, 0, 1) == 0) *result_nonce = nonce;
            return;
        }
        nonce += stride;
    }
}

/* Computes the pow hash of exactly one nonce — selftest layers 3/4. */
__global__ void th_hash_one_kernel(const uint8_t *dataset, uint64_t nonce,
                                   uint8_t *hash_out) {
    if (blockIdx.x == 0 && threadIdx.x == 0)
        th_pow_hash_device(c_prefix, c_prefix_len, nonce, dataset, hash_out);
}

// ── C interface ──────────────────────────────────────────────────────────────

struct TensorHashCtx {
    uint8_t  *d_dataset;
    int      *d_found;
    uint64_t *d_result_nonce;
    uint8_t  *d_hash_out;
    uint8_t   current_seed[32];
    int       seed_valid;
    double    last_gen_seconds;
};

extern "C" {

/* Error codes for th_ctx_create. */
#define TH_ERR_NONE        0
#define TH_ERR_VRAM        1   /* not enough free VRAM (needs ~20 GB) */
#define TH_ERR_ALLOC       2   /* cudaMalloc failed */

TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out) {
    *err = TH_ERR_NONE;
    size_t free_b = 0, total_b = 0;
    cudaMemGetInfo(&free_b, &total_b);
    if (free_bytes_out) *free_bytes_out = free_b;
    if (free_b < TH_DATASET_BYTES + TH_VRAM_HEADROOM) {
        *err = TH_ERR_VRAM;
        return NULL;
    }
    TensorHashCtx *ctx = (TensorHashCtx *)calloc(1, sizeof(TensorHashCtx));
    if (cudaMalloc(&ctx->d_dataset, TH_DATASET_BYTES) != cudaSuccess ||
        cudaMalloc(&ctx->d_found, sizeof(int)) != cudaSuccess ||
        cudaMalloc(&ctx->d_result_nonce, sizeof(uint64_t)) != cudaSuccess ||
        cudaMalloc(&ctx->d_hash_out, 32) != cudaSuccess) {
        *err = TH_ERR_ALLOC;
        if (ctx->d_dataset)      cudaFree(ctx->d_dataset);
        if (ctx->d_found)        cudaFree(ctx->d_found);
        if (ctx->d_result_nonce) cudaFree(ctx->d_result_nonce);
        if (ctx->d_hash_out)     cudaFree(ctx->d_hash_out);
        free(ctx);
        return NULL;
    }
    ctx->seed_valid = 0;
    return ctx;
}

void th_ctx_destroy(TensorHashCtx *ctx) {
    if (!ctx) return;
    cudaFree(ctx->d_dataset);
    cudaFree(ctx->d_found);
    cudaFree(ctx->d_result_nonce);
    cudaFree(ctx->d_hash_out);
    free(ctx);
}

int th_ctx_seed_matches(TensorHashCtx *ctx, const uint8_t seed[32]) {
    return ctx->seed_valid && memcmp(ctx->current_seed, seed, 32) == 0;
}

double th_last_dataset_gen_seconds(TensorHashCtx *ctx) {
    return ctx->last_gen_seconds;
}

/* Generates the full dataset for `seed`, then spot-checks element 0,
   element N-1 and `spot_count` deterministic pseudo-random indices against
   the host reference (selftest layer 2 — runs on EVERY generation).
   Returns 0 on success, -1 on CUDA error, index+1 of a mismatching spot
   check otherwise. */
int th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32],
                            int spot_count) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    cudaMemcpyToSymbol(c_gen_seed, seed, 32);
    th_dataset_gen_kernel<<<4096, 256>>>(ctx->d_dataset);
    if (cudaDeviceSynchronize() != cudaSuccess) return -1;

    clock_gettime(CLOCK_MONOTONIC, &t1);
    ctx->last_gen_seconds =
        (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) * 1e-9;

    /* spot checks: fixed boundary indices + xorshift64 sequence (fixed seed
       => deterministic, host and device check identical indices) */
    uint64_t rng = 0x9e3779b97f4a7c15ULL;
    for (int s = 0; s < spot_count + 2; s++) {
        uint64_t idx;
        if (s == 0)      idx = 0;
        else if (s == 1) idx = TH_DATASET_N - 1;
        else {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            idx = rng % TH_DATASET_N;
        }
        uint8_t got[32], expect[32];
        if (cudaMemcpy(got, ctx->d_dataset + idx * 32ULL, 32,
                       cudaMemcpyDeviceToHost) != cudaSuccess) return -1;
        host_dataset_element(seed, idx, expect);
        if (memcmp(got, expect, 32) != 0) {
            fprintf(stderr,
                "[tensorhash] DATASET SPOT-CHECK FAILED at index %llu — "
                "GPU output does not match reference. Refusing to mine.\n",
                (unsigned long long)idx);
            return s + 1;
        }
    }

    memcpy(ctx->current_seed, seed, 32);
    ctx->seed_valid = 1;
    return 0;
}

/* header_template = full header bytes INCLUDING the trailing 8 nonce bytes
   (same convention as the old SHA256d kernel); the prefix is everything but
   those 8 bytes. Returns 1 + *nonce_out when a nonce meeting
   difficulty_bits is found. */
int th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                     uint16_t header_len, uint8_t difficulty_bits,
                     uint64_t start_nonce, int blocks, int threads,
                     uint32_t iters_per_thread, uint64_t *nonce_out) {
    if (!ctx->seed_valid || header_len <= 8 ||
        (uint32_t)(header_len - 8) > TH_PREFIX_MAX) return 0;

    uint32_t prefix_len = (uint32_t)header_len - 8;
    cudaMemcpyToSymbol(c_prefix, header_template, prefix_len);
    cudaMemcpyToSymbol(c_prefix_len, &prefix_len, sizeof(uint32_t));

    int      h_found = 0;
    uint64_t h_nonce = UINT64_MAX;
    cudaMemcpy(ctx->d_found, &h_found, sizeof(int), cudaMemcpyHostToDevice);
    cudaMemcpy(ctx->d_result_nonce, &h_nonce, sizeof(uint64_t), cudaMemcpyHostToDevice);

    th_mine_kernel<<<blocks, threads>>>(ctx->d_dataset, difficulty_bits,
                                        start_nonce, iters_per_thread,
                                        ctx->d_found, ctx->d_result_nonce);
    cudaDeviceSynchronize();

    cudaMemcpy(&h_found, ctx->d_found, sizeof(int), cudaMemcpyDeviceToHost);
    cudaMemcpy(&h_nonce, ctx->d_result_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);
    if (h_found) { *nonce_out = h_nonce; return 1; }
    return 0;
}

/* Computes the pow hash of a single (prefix, nonce) through the REAL device
   code path. prefix here EXCLUDES nonce bytes. Returns 1 on success. */
int th_hash_one(TensorHashCtx *ctx, const uint8_t *prefix, uint16_t prefix_len,
                uint64_t nonce, uint8_t out_hash[32]) {
    if (!ctx->seed_valid || prefix_len > TH_PREFIX_MAX) return 0;
    uint32_t plen = prefix_len;
    cudaMemcpyToSymbol(c_prefix, prefix, plen);
    cudaMemcpyToSymbol(c_prefix_len, &plen, sizeof(uint32_t));
    th_hash_one_kernel<<<1, 1>>>(ctx->d_dataset, nonce, ctx->d_hash_out);
    if (cudaDeviceSynchronize() != cudaSuccess) return 0;
    cudaMemcpy(out_hash, ctx->d_hash_out, 32, cudaMemcpyDeviceToHost);
    return 1;
}

} // extern "C"
```

- [ ] **Step 3: Compile gate**

Run: `cd tools/tensorium-miner && nvcc -arch=sm_86 -O3 -c -o /tmp/thk.o tensorhash_kernel.cu && echo COMPILE-OK`
Expected: `COMPILE-OK` (warnings acceptable, errors not). If nvcc is unavailable (Task 5 fallback), defer and note it.

- [ ] **Step 4: Commit**

```bash
git add tools/tensorium-miner/tensorhash.cuh tools/tensorium-miner/tensorhash_kernel.cu
git commit -m "feat(miner): TensorHash v1 CUDA kernels — dataset generation, mining, hash-one, spot-check"
```

---

### Task 9: Miner — plumbing (`common.h`, solo + stratum `epoch_seed`)

**Files:**
- Modify: `tools/tensorium-miner/common.h`
- Modify: `tools/tensorium-miner/solo_client.cpp`
- Modify: `tools/tensorium-miner/stratum_client.cpp`

- [ ] **Step 1: `common.h`** — in `JobDesc`, after `uint8_t merkle_root[32];` add:

```c
    uint8_t  epoch_seed[32];     /* TensorHash dataset seed for this height */
```

and bump the version:

```c
#define TENSORIUM_MINER_VERSION "3.0.0"
```

- [ ] **Step 2: `solo_client.cpp`** — in `fetch_template`, after the two `extract_byte_array` calls for `previous_hash`/`merkle_root`, add (note: `epoch_seed` lives at the response root, so search `s_rpc_buf`, not `hdr`):

```c
    if (!extract_byte_array(s_rpc_buf, "epoch_seed", job->epoch_seed, 32)) {
        fprintf(stderr,
            "[solo] template has no epoch_seed — tensorium-node is too old "
            "for TensorHash v1, upgrade the node\n");
        return 0;
    }
```

- [ ] **Step 3: `stratum_client.cpp`** — in `parse_notify`, after the `merkle_root` parse, add:

```c
    memset(hex, 0, sizeof(hex));
    if (jstr(params, "epoch_seed", hex, sizeof(hex)) && strlen(hex) == 64) {
        hex64_to_bytes(hex, job->epoch_seed);
    } else {
        static int warned = 0;
        if (!warned) {
            fprintf(stderr,
                "[pool] job has no epoch_seed — assuming epoch 0 (zero seed); "
                "the pool must send epoch_seed once the chain passes height 8191\n");
            warned = 1;
        }
        memset(job->epoch_seed, 0, 32);
    }
```

Also update the stale chain_id fallback in the same function:

```c
        strncpy(job->chain_id, "tensorium-mainnet", CHAIN_ID_LEN - 1);
```

(replacing `"tensorium-mainnet-candidate-0"`).

- [ ] **Step 4: Compile gate (host-compilable files)**

Run: `cd tools/tensorium-miner && g++ -O2 -Wall -std=c++11 -pthread -fsyntax-only solo_client.cpp stratum_client.cpp && echo SYNTAX-OK`
Expected: `SYNTAX-OK`.

- [ ] **Step 5: Commit**

```bash
git add tools/tensorium-miner/common.h tools/tensorium-miner/solo_client.cpp \
        tools/tensorium-miner/stratum_client.cpp
git commit -m "feat(miner): thread epoch_seed from solo/stratum jobs into JobDesc"
```

---

### Task 10: Miner — `gpu_worker.cu` rewrite, modes, `main.cpp`, Makefile, deletions

**Files:**
- Modify: `tools/tensorium-miner/gpu_worker.cu` (full rewrite below)
- Create: `tools/tensorium-miner/modes.h`, `tools/tensorium-miner/modes.cpp`
- Modify: `tools/tensorium-miner/main.cpp`
- Modify: `tools/tensorium-miner/common.h` (MODE_GENESIS enum value)
- Modify: `tools/tensorium-miner/solo_client.cpp` (one stale comment)
- Modify: `tools/tensorium-miner/Makefile` (full rewrite below)
- Delete: `tools/tensorium-miner/sha256d.cuh`, `mining_kernel.cu`, `mine_genesis.cu`, `main.cu`

- [ ] **Step 1: Rewrite `gpu_worker.cu`**

Replace the entire file with:

```c
// tools/tensorium-miner/gpu_worker.cu
#include "gpu_worker.h"
#include "solo_client.h"       /* build_header */
#include "host_tensorhash.h"   /* host verification of found nonces */
#include "tensorhash_params.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

/* TensorHash kernel C interface (tensorhash_kernel.cu) */
struct TensorHashCtx;
extern "C" {
TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out);
void   th_ctx_destroy(TensorHashCtx *ctx);
int    th_ctx_seed_matches(TensorHashCtx *ctx, const uint8_t seed[32]);
int    th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32], int spot_count);
double th_last_dataset_gen_seconds(TensorHashCtx *ctx);
int    th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                        uint16_t header_len, uint8_t difficulty_bits,
                        uint64_t start_nonce, int blocks, int threads,
                        uint32_t iters_per_thread, uint64_t *nonce_out);
}

/* Verify a found nonce on the CPU before submitting (cheap: K=32 elements
   recomputed on demand). Returns the leading-zero-bit count. */
static int verify_share(const JobDesc *job, uint64_t nonce) {
    uint8_t header[HEADER_MAX];
    int hlen = build_header(job, nonce, header);
    if (hlen <= 8) return 0;
    uint8_t hash[32];
    host_pow_hash(header, (uint32_t)(hlen - 8), nonce, job->epoch_seed, hash);
    return host_leading_zero_bits(hash);
}

/* (Re)generate the dataset for the job's epoch seed if it changed.
   Returns 0 on success. */
static int ensure_dataset(TensorHashCtx *ctx, int gpu_id, const JobDesc *job) {
    if (th_ctx_seed_matches(ctx, job->epoch_seed)) return 0;
    printf("[GPU %d] generating %.1f GiB TensorHash dataset (epoch seed changed)...\n",
           gpu_id, (double)TH_DATASET_BYTES / (1024.0 * 1024.0 * 1024.0));
    fflush(stdout);
    int rc = th_ctx_generate_dataset(ctx, job->epoch_seed, 4096);
    if (rc != 0) {
        fprintf(stderr, "[GPU %d] dataset generation/spot-check failed (rc=%d)\n",
                gpu_id, rc);
        return rc;
    }
    printf("[GPU %d] dataset ready in %.1fs (spot-check passed)\n",
           gpu_id, th_last_dataset_gen_seconds(ctx));
    fflush(stdout);
    return 0;
}

void *gpu_worker_thread(void *arg) {
    GpuWorkerArgs *a = (GpuWorkerArgs *)arg;
    SharedState   *s = a->state;

    if (a->gpu_id < 0 || a->gpu_id >= MAX_GPUS) {
        fprintf(stderr, "[GPU ?] invalid gpu_id=%d (MAX_GPUS=%d)\n", a->gpu_id, MAX_GPUS);
        return NULL;
    }

    cudaError_t err = cudaSetDevice(a->gpu_id);
    if (err != cudaSuccess) {
        fprintf(stderr, "[GPU %d] cudaSetDevice failed: %s\n",
                a->gpu_id, cudaGetErrorString(err));
        return NULL;
    }

    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, a->gpu_id);

    /* VRAM gate: TensorHash needs the full dataset resident. */
    int th_err = 0;
    size_t free_b = 0;
    TensorHashCtx *ctx = th_ctx_create(&th_err, &free_b);
    if (!ctx) {
        if (th_err == 1) {
            fprintf(stderr,
                "[GPU %d] %s has only %.1f GiB free VRAM — TensorHash v1 needs "
                "~20 GiB (dataset 17.9 GiB + headroom).\n"
                "[GPU %d] Minimum supported card: RTX 3090 / 24 GB.\n",
                a->gpu_id, prop.name, (double)free_b / (1024.0 * 1024.0 * 1024.0),
                a->gpu_id);
        } else {
            fprintf(stderr, "[GPU %d] TensorHash context allocation failed\n", a->gpu_id);
        }
        return NULL;
    }

    pthread_mutex_lock(&s->stats_mutex);
    GpuStats *gs = &s->gpu_stats[a->gpu_id];
    gs->gpu_id  = a->gpu_id;
    snprintf(gs->name, sizeof(gs->name), "%s", prop.name);
    gs->temp_c  = -1;
    gs->power_w = -1;
    gs->fan_pct = -1;
    pthread_mutex_unlock(&s->stats_mutex);

    printf("[GPU %d] %s  blocks=%d  threads=%d  (TensorHash v1)\n",
           a->gpu_id, prop.name, a->cuda_blocks, a->cuda_threads);
    fflush(stdout);

    JobDesc job;
    job_wait(s, &job);
    if (!s->running) { th_ctx_destroy(ctx); return NULL; }

    int last_gen = s->job_generation;
    if (ensure_dataset(ctx, a->gpu_id, &job) != 0) { th_ctx_destroy(ctx); return NULL; }

    /* ~16M nonces per launch: at TensorHash rates (tens of MH/s) that is a
       sub-second launch, keeping job switchover latency low. */
    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)a->cuda_blocks * (uint64_t)a->cuda_threads));
    if (iters < 1) iters = 1;
    uint64_t nonces_per_launch = (uint64_t)a->cuda_blocks * a->cuda_threads * iters;

    uint64_t nonce = a->nonce_start;
    uint64_t hashes_since_reset = 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    while (s->running) {
        pthread_mutex_lock(&s->job_mutex);
        if (s->job_generation != last_gen) {
            job      = s->current_job;
            last_gen = s->job_generation;
            nonce = a->nonce_start;
            hashes_since_reset = 0;
            clock_gettime(CLOCK_MONOTONIC, &t0);
        }
        pthread_mutex_unlock(&s->job_mutex);

        if (ensure_dataset(ctx, a->gpu_id, &job) != 0) break;

        uint8_t header_tmpl[HEADER_MAX];
        int hlen = build_header(&job, nonce, header_tmpl);
        if (hlen <= 8) { usleep(100000); continue; }

        uint64_t found_nonce = 0;
        int found = th_launch_mining(ctx, header_tmpl, (uint16_t)hlen,
                                     job.share_bits, nonce,
                                     a->cuda_blocks, a->cuda_threads, iters,
                                     &found_nonce);

        hashes_since_reset += nonces_per_launch;
        nonce += nonces_per_launch;
        if (nonce >= a->nonce_end || nonce < a->nonce_start)
            nonce = a->nonce_start;

        clock_gettime(CLOCK_MONOTONIC, &t1);
        double elapsed = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) * 1e-9;
        if (elapsed > 0.0) {
            pthread_mutex_lock(&s->stats_mutex);
            gs->hashrate_ghs = (double)hashes_since_reset / elapsed / 1e9;
            gs->hashes_total += nonces_per_launch;
            pthread_mutex_unlock(&s->stats_mutex);
        }

        if (found) {
            int zeros = verify_share(&job, found_nonce);
            int is_share = (zeros >= (int)job.share_bits);
            int is_block = (zeros >= (int)job.difficulty_bits);

            if (!is_share && !is_block) {
                fprintf(stderr,
                    "[GPU %d] kernel result FAILED host verification "
                    "(nonce=%llu zeros=%d) — possible GPU memory fault\n",
                    a->gpu_id, (unsigned long long)found_nonce, zeros);
            }

            if (is_share || is_block) {
                pthread_mutex_lock(&s->stats_mutex);
                gs->shares_found++;
                pthread_mutex_unlock(&s->stats_mutex);

                ShareResult sr;
                memset(&sr, 0, sizeof(sr));
                strncpy(sr.job_id, job.job_id, JOB_ID_LEN - 1);
                strncpy(sr.worker, a->cfg->worker, WORKER_LEN - 1);
                sr.nonce    = found_nonce;
                sr.gpu_id   = a->gpu_id;
                sr.is_block = is_block;
                share_push(s, &sr);

                nonce = found_nonce + nonces_per_launch;
                if (nonce >= a->nonce_end || nonce < a->nonce_start)
                    nonce = a->nonce_start;
                hashes_since_reset = 0;
                clock_gettime(CLOCK_MONOTONIC, &t0);
            }
        }
    }

    th_ctx_destroy(ctx);
    return NULL;
}
```

- [ ] **Step 2: Create `modes.h`**

```c
// tools/tensorium-miner/modes.h
// Standalone run modes: --selftest, --benchmark, --mode genesis.
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/* All return a process exit code (0 = success). */
int run_selftest(int gpu_id);
int run_benchmark(int gpu_id, int seconds, int cuda_blocks, int cuda_threads);
int run_genesis(const char *prefix_hex, int bits, uint64_t start_nonce,
                int gpu_count, const int *gpu_ids,
                int cuda_blocks, int cuda_threads);

#ifdef __cplusplus
}
#endif
```

- [ ] **Step 3: Create `modes.cpp`**

```cpp
// tools/tensorium-miner/modes.cpp
#include "modes.h"
#include "host_tensorhash.h"
#include "tensorhash_params.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <pthread.h>

struct TensorHashCtx;
extern "C" {
TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out);
void   th_ctx_destroy(TensorHashCtx *ctx);
int    th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32], int spot_count);
double th_last_dataset_gen_seconds(TensorHashCtx *ctx);
int    th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                        uint16_t header_len, uint8_t difficulty_bits,
                        uint64_t start_nonce, int blocks, int threads,
                        uint32_t iters_per_thread, uint64_t *nonce_out);
int    th_hash_one(TensorHashCtx *ctx, const uint8_t *prefix, uint16_t prefix_len,
                   uint64_t nonce, uint8_t out_hash[32]);
}

static double now_mono(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

static TensorHashCtx *mode_ctx_create(int gpu_id) {
    if (cudaSetDevice(gpu_id) != cudaSuccess) {
        fprintf(stderr, "cudaSetDevice(%d) failed\n", gpu_id);
        return NULL;
    }
    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, gpu_id);
    int err = 0;
    size_t free_b = 0;
    TensorHashCtx *ctx = th_ctx_create(&err, &free_b);
    if (!ctx) {
        if (err == 1)
            fprintf(stderr,
                "GPU %d (%s): %.1f GiB free VRAM — TensorHash needs ~20 GiB. "
                "Minimum supported card: RTX 3090 / 24 GB.\n",
                gpu_id, prop.name, free_b / (1024.0 * 1024.0 * 1024.0));
        else
            fprintf(stderr, "GPU %d: context allocation failed\n", gpu_id);
        return NULL;
    }
    printf("GPU %d: %s\n", gpu_id, prop.name);
    return ctx;
}

// ── --selftest ────────────────────────────────────────────────────────────────

int run_selftest(int gpu_id) {
    printf("=== TensorHash v1 selftest ===\n");

    /* Layer 1: host reference vs hardcoded Rust KATs */
    int rc = host_tensorhash_kat_check();
    if (rc != 0) {
        fprintf(stderr, "FAIL layer 1: host KAT vector %d\n", rc);
        return 1;
    }
    printf("layer 1 PASS  host reference matches Rust KATs\n");

    TensorHashCtx *ctx = mode_ctx_create(gpu_id);
    if (!ctx) return 1;

    /* Layer 2: dataset generation + spot-check, zero seed */
    uint8_t zero_seed[32] = {0};
    printf("generating dataset (zero seed)...\n");
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        fprintf(stderr, "FAIL layer 2: dataset spot-check (zero seed)\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("layer 2 PASS  dataset spot-check (4098 elements) in %.1fs\n",
           th_last_dataset_gen_seconds(ctx));

    /* Layer 3: the Rust pow_hash KAT through the real kernel path */
    uint8_t got[32], expect[32];
    th_hex32_to_bytes(TH_KAT_POW_HEX, expect);
    if (!th_hash_one(ctx, (const uint8_t *)TH_KAT_POW_HEADER,
                     (uint16_t)strlen(TH_KAT_POW_HEADER),
                     TH_KAT_POW_NONCE, got) ||
        memcmp(got, expect, 32) != 0) {
        fprintf(stderr, "FAIL layer 3: kernel pow_hash KAT mismatch\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("layer 3 PASS  kernel reproduces the Rust pow_hash KAT\n");

    /* Layer 4a: 1024 random (prefix, nonce) GPU-vs-host, zero seed */
    uint64_t rng = 0xdeadbeefcafef00dULL;
    uint8_t prefix[102], host_hash[32];
    for (int t = 0; t < 1024; t++) {
        for (int i = 0; i < 102; i++) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            prefix[i] = (uint8_t)rng;
        }
        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
        uint64_t nonce = rng;
        if (!th_hash_one(ctx, prefix, 102, nonce, got)) {
            fprintf(stderr, "FAIL layer 4a: th_hash_one error at trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
        host_pow_hash(prefix, 102, nonce, zero_seed, host_hash);
        if (memcmp(got, host_hash, 32) != 0) {
            fprintf(stderr, "FAIL layer 4a: GPU/host mismatch at trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
    }
    printf("layer 4a PASS  1024 random attempts match host (zero seed)\n");

    /* Layer 4b: regenerate with seed=[1;32], verify the V8 vector + 16 randoms */
    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    printf("regenerating dataset (seed = [1;32])...\n");
    if (th_ctx_generate_dataset(ctx, one_seed, 1024) != 0) {
        fprintf(stderr, "FAIL layer 4b: dataset spot-check (one seed)\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    th_hex32_to_bytes("cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491", expect);
    if (!th_hash_one(ctx, xprefix, 102, 777, got) || memcmp(got, expect, 32) != 0) {
        fprintf(stderr, "FAIL layer 4b: V8 vector mismatch on kernel path\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    for (int t = 0; t < 16; t++) {
        for (int i = 0; i < 102; i++) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            prefix[i] = (uint8_t)rng;
        }
        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
        uint64_t nonce = rng;
        th_hash_one(ctx, prefix, 102, nonce, got);
        host_pow_hash(prefix, 102, nonce, one_seed, host_hash);
        if (memcmp(got, host_hash, 32) != 0) {
            fprintf(stderr, "FAIL layer 4b: GPU/host mismatch (one seed) trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
    }
    printf("layer 4b PASS  non-zero seed: V8 + 16 random attempts match\n");

    th_ctx_destroy(ctx);
    printf("\n=== ALL SELFTEST LAYERS PASS — GPU implementation is consensus-equivalent ===\n");
    return 0;
}

// ── --benchmark ───────────────────────────────────────────────────────────────

int run_benchmark(int gpu_id, int seconds, int cuda_blocks, int cuda_threads) {
    if (seconds <= 0) seconds = 60;
    printf("=== TensorHash v1 benchmark (%ds) ===\n", seconds);

    TensorHashCtx *ctx = mode_ctx_create(gpu_id);
    if (!ctx) return 1;

    uint8_t zero_seed[32] = {0};
    printf("generating dataset...\n");
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("dataset generation: %.2fs (regenerates every %llu blocks / ~5.7 days)\n",
           th_last_dataset_gen_seconds(ctx), (unsigned long long)TH_EPOCH_LENGTH);

    /* random fake 102-byte header (incl. 8 nonce bytes => 110 total) at
       impossible difficulty 64 so the loop never exits early */
    uint8_t header[110];
    for (int i = 0; i < 110; i++) header[i] = (uint8_t)(i * 37 + 11);

    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)cuda_blocks * (uint64_t)cuda_threads));
    if (iters < 1) iters = 1;
    uint64_t per_launch = (uint64_t)cuda_blocks * cuda_threads * iters;

    double t_start = now_mono();
    uint64_t total = 0, nonce = 0, dummy;
    while (now_mono() - t_start < (double)seconds) {
        th_launch_mining(ctx, header, 110, 64, nonce,
                         cuda_blocks, cuda_threads, iters, &dummy);
        total += per_launch;
        nonce += per_launch;
    }
    double elapsed = now_mono() - t_start;
    double mhs = total / elapsed / 1e6;
    printf("\nhashrate: %.2f MH/s  (%llu hashes in %.1fs, blocks=%d threads=%d)\n",
           mhs, (unsigned long long)total, elapsed, cuda_blocks, cuda_threads);
    printf("expected time to 42-bit genesis on this GPU alone: %.1f hours\n",
           4398046511104.0 /* 2^42 */ / (mhs * 1e6) / 3600.0);

    th_ctx_destroy(ctx);
    return 0;
}

// ── --mode genesis ────────────────────────────────────────────────────────────

typedef struct {
    int      gpu_id;
    const uint8_t *header;   /* prefix + 8 zero nonce bytes */
    int      header_len;
    int      bits;
    uint64_t nonce_start, nonce_end;
    int      blocks, threads;
} GenesisArgs;

static volatile int      g_gen_found = 0;
static volatile uint64_t g_gen_nonce = 0;
static pthread_mutex_t   g_gen_mutex = PTHREAD_MUTEX_INITIALIZER;

static void *genesis_thread(void *p) {
    GenesisArgs *a = (GenesisArgs *)p;
    if (cudaSetDevice(a->gpu_id) != cudaSuccess) return NULL;
    TensorHashCtx *ctx = mode_ctx_create(a->gpu_id);
    if (!ctx) return NULL;

    uint8_t zero_seed[32] = {0};   /* genesis is epoch 0 */
    printf("[GPU %d] generating dataset...\n", a->gpu_id);
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        th_ctx_destroy(ctx);
        return NULL;
    }

    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)a->blocks * (uint64_t)a->threads));
    if (iters < 1) iters = 1;
    uint64_t per_launch = (uint64_t)a->blocks * a->threads * iters;

    uint64_t nonce = a->nonce_start, done = 0;
    double t0 = now_mono(), last_print = t0;

    while (!g_gen_found && nonce < a->nonce_end) {
        uint64_t found_nonce = 0;
        if (th_launch_mining(ctx, a->header, (uint16_t)a->header_len,
                             (uint8_t)a->bits, nonce,
                             a->blocks, a->threads, iters, &found_nonce)) {
            pthread_mutex_lock(&g_gen_mutex);
            if (!g_gen_found) { g_gen_found = 1; g_gen_nonce = found_nonce; }
            pthread_mutex_unlock(&g_gen_mutex);
            break;
        }
        nonce += per_launch;
        done  += per_launch;
        double now = now_mono();
        if (now - last_print >= 10.0) {
            double mhs = done / (now - t0) / 1e6;
            double expect_h = (double)(1ULL << a->bits) / (mhs * 1e6) / 3600.0;
            printf("[GPU %d] %.2f MH/s  %llu MH done  (E[total] ≈ %.1f GPU-hours at %d bits)\n",
                   a->gpu_id, mhs, (unsigned long long)(done / 1000000ULL),
                   expect_h, a->bits);
            fflush(stdout);
            last_print = now;
        }
    }

    th_ctx_destroy(ctx);
    return NULL;
}

int run_genesis(const char *prefix_hex, int bits, uint64_t start_nonce,
                int gpu_count, const int *gpu_ids,
                int cuda_blocks, int cuda_threads) {
    size_t hexlen = strlen(prefix_hex);
    if (hexlen % 2 != 0 || hexlen / 2 > TH_PREFIX_MAX) {
        fprintf(stderr, "--prefix: bad hex length %zu (max %d bytes)\n",
                hexlen, TH_PREFIX_MAX);
        return 1;
    }
    int prefix_len = (int)(hexlen / 2);
    uint8_t header[HEADER_MAX] = {0};
    for (int i = 0; i < prefix_len; i++) {
        unsigned b;
        if (sscanf(prefix_hex + i * 2, "%2x", &b) != 1) {
            fprintf(stderr, "--prefix: invalid hex at byte %d\n", i);
            return 1;
        }
        header[i] = (uint8_t)b;
    }
    int header_len = prefix_len + 8;   /* trailing 8 nonce bytes, kernel-filled */

    printf("=== TensorHash v1 genesis mine ===\n");
    printf("prefix=%d bytes  bits=%d  start_nonce=%llu  gpus=%d\n",
           prefix_len, bits, (unsigned long long)start_nonce, gpu_count);
    printf("expected attempts: 2^%d ≈ %.2e\n", bits, (double)(1ULL << bits));

    pthread_t   threads_arr[MAX_GPUS];
    GenesisArgs args[MAX_GPUS];
    uint64_t span = (UINT64_MAX - start_nonce) / (uint64_t)gpu_count;
    for (int i = 0; i < gpu_count; i++) {
        args[i].gpu_id      = gpu_ids[i];
        args[i].header      = header;
        args[i].header_len  = header_len;
        args[i].bits        = bits;
        args[i].nonce_start = start_nonce + (uint64_t)i * span;
        args[i].nonce_end   = (i == gpu_count - 1) ? UINT64_MAX
                                                   : start_nonce + (uint64_t)(i + 1) * span;
        args[i].blocks      = cuda_blocks;
        args[i].threads     = cuda_threads;
        pthread_create(&threads_arr[i], NULL, genesis_thread, &args[i]);
    }
    for (int i = 0; i < gpu_count; i++) pthread_join(threads_arr[i], NULL);

    if (!g_gen_found) {
        fprintf(stderr, "no nonce found (interrupted or nonce space exhausted)\n");
        return 1;
    }

    /* Host-verify before reporting. */
    uint8_t zero_seed[32] = {0}, hash[32];
    host_pow_hash(header, (uint32_t)prefix_len, g_gen_nonce, zero_seed, hash);
    int zeros = host_leading_zero_bits(hash);
    if (zeros < bits) {
        fprintf(stderr, "FOUND NONCE FAILED HOST VERIFICATION (zeros=%d < %d)\n",
                zeros, bits);
        return 1;
    }
    printf("\nGENESIS NONCE: %llu\n", (unsigned long long)g_gen_nonce);
    printf("verified on host: %d leading zero bits (need %d)\n", zeros, bits);
    printf("next: tensorium-node verify-genesis <timestamp> %llu\n",
           (unsigned long long)g_gen_nonce);
    return 0;
}
```

- [ ] **Step 4: Update `main.cpp`**

(a) Add `#include "modes.h"` after the `nvml_monitor.h` include.

(b) Extend `MiningMode` in `common.h`:

```c
typedef enum { MODE_SOLO = 0, MODE_POOL = 1, MODE_GENESIS = 2 } MiningMode;
```

(c) In the flag parser, extend the `--mode` handler:

```c
        else if (strcmp(argv[i], "--mode") == 0) {
            const char *m = NEXTARG();
            if      (strcmp(m, "pool") == 0)    cfg.mode = MODE_POOL;
            else if (strcmp(m, "genesis") == 0) cfg.mode = MODE_GENESIS;
            else                                cfg.mode = MODE_SOLO;
        }
```

(d) Add locals near `int use_intensity = 0;`:

```c
    int do_selftest = 0;
    int do_benchmark = 0, bench_secs = 60;
    const char *genesis_prefix = NULL;
    int genesis_bits = 42;
    uint64_t genesis_start = 0;
```

(e) Add flag handlers before the final `else` in the parser loop:

```c
        else if (strcmp(argv[i], "--selftest") == 0) do_selftest = 1;
        else if (strcmp(argv[i], "--benchmark") == 0) {
            do_benchmark = 1;
            if (i + 1 < argc && argv[i + 1][0] != '-') bench_secs = atoi(argv[++i]);
        }
        else if (strcmp(argv[i], "--prefix") == 0) genesis_prefix = NEXTARG();
        else if (strcmp(argv[i], "--bits") == 0)   genesis_bits = atoi(NEXTARG());
        else if (strcmp(argv[i], "--start-nonce") == 0)
            genesis_start = (uint64_t)strtoull(NEXTARG(), NULL, 10);
```

(f) Replace the `/* Validate required args */` block with mode dispatch first:

```c
    /* Compute cuda_blocks/threads from intensity (needed by all modes) */
    if (cfg.cuda_blocks == 0)
        intensity_to_launch(use_intensity, &cfg.cuda_blocks, &cfg.cuda_threads);

    /* ── Standalone modes (no wallet/RPC needed) ── */
    if (do_selftest)
        return run_selftest(cfg.gpu_count > 0 ? cfg.gpu_ids[0] : 0);
    if (do_benchmark)
        return run_benchmark(cfg.gpu_count > 0 ? cfg.gpu_ids[0] : 0,
                             bench_secs, cfg.cuda_blocks, cfg.cuda_threads);
    if (cfg.mode == MODE_GENESIS) {
        if (!genesis_prefix) {
            fprintf(stderr, "error: --mode genesis requires --prefix <hex> "
                            "(from: tensorium-node print-genesis-prefix)\n");
            return 1;
        }
        int total = 0;
        cudaGetDeviceCount(&total);
        if (total == 0) { fprintf(stderr, "error: no CUDA GPUs found\n"); return 1; }
        int ids[MAX_GPUS], n;
        if (cfg.gpu_count == 0) {
            n = (total > MAX_GPUS) ? MAX_GPUS : total;
            for (int i = 0; i < n; i++) ids[i] = i;
        } else {
            n = cfg.gpu_count;
            memcpy(ids, cfg.gpu_ids, n * sizeof(int));
        }
        return run_genesis(genesis_prefix, genesis_bits, genesis_start,
                           n, ids, cfg.cuda_blocks, cfg.cuda_threads);
    }

    /* Validate required args (solo/pool only) */
    if (cfg.wallet[0] == '\0') {
        fprintf(stderr, "error: --wallet is required\n");
        print_usage(argv[0]); return 1;
    }
    if (cfg.mode == MODE_POOL && cfg.pool_host[0] == '\0') {
        fprintf(stderr, "error: --pool is required in pool mode\n");
        return 1;
    }
```

(Delete the now-duplicated `if (cfg.cuda_blocks == 0) intensity_to_launch(...)` that previously sat after validation.)

(g) Update `print_usage` to document the new modes:

```c
        "Standalone modes:\n"
        "  %s --selftest                 verify GPU kernel against consensus KATs\n"
        "  %s --benchmark [secs]         dataset-gen time + sustained hashrate\n"
        "  %s --mode genesis --prefix HEX --bits N [--start-nonce N]\n"
        "                                mine a genesis nonce (no node needed)\n\n"
```

(add the three extra `prog` arguments to the corresponding `fprintf` call).

(h) In `stats_thread`, change the two hashrate printfs from GH/s to MH/s:

```c
                printf("[GPU %d] %8.2f MH/s  temp=%d°C  power=%dW  fan=%d%%  shares=%llu\n",
                       g->gpu_id, g->hashrate_ghs * 1000.0, g->temp_c, g->power_w, g->fan_pct,
                       (unsigned long long)g->shares_found);
```

```c
                printf("[GPU %d] %8.2f MH/s  shares=%llu\n",
                       g->gpu_id, g->hashrate_ghs * 1000.0,
                       (unsigned long long)g->shares_found);
```

and the total line:

```c
            printf("[total] %8.2f MH/s  shares=%llu\n\n",
                   total_ghs * 1000.0, (unsigned long long)total_shares);
```

Also update the solo-mode comment in `solo_client.cpp` (`/* In solo mode the kernel must mine at full 40-bit network difficulty. */` → `/* In solo mode the kernel mines at full network difficulty. */`).

- [ ] **Step 5: Delete the SHA256d-era files**

```bash
cd tools/tensorium-miner
git rm sha256d.cuh mining_kernel.cu mine_genesis.cu main.cu
```

- [ ] **Step 6: Rewrite the Makefile**

```make
# tools/tensorium-miner/Makefile
NVCC    ?= nvcc
CXX     ?= g++
TARGET  ?= tensorium-miner

ifndef ARCH
  DETECTED := $(shell nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>/dev/null | head -1 | tr -d .)
  ifneq ($(DETECTED),)
    ARCH := sm_$(DETECTED)
  else
    ARCH := sm_86
    $(warning nvidia-smi not found — defaulting to $(ARCH))
  endif
endif

NVCCFLAGS := -arch=$(ARCH) -O3 -Xcompiler "-O3 -pthread" \
             -Xptxas -O3,--warn-on-spills
CXXFLAGS  := -O3 -Wall -std=c++11 -pthread

SRCS_CU  := gpu_worker.cu tensorhash_kernel.cu
SRCS_CPP := main.cpp solo_client.cpp stratum_client.cpp modes.cpp host_tensorhash.cpp

ifdef WITH_NVML
  SRCS_CPP  += nvml_monitor.cpp
  NVCCFLAGS += -DWITH_NVML
  CXXFLAGS  += -DWITH_NVML
  LDLIBS    += -lnvidia-ml
endif

OBJS_CU  := $(SRCS_CU:.cu=.o)
OBJS_CPP := $(SRCS_CPP:.cpp=.o)

all: $(TARGET)

$(TARGET): $(OBJS_CU) $(OBJS_CPP)
	$(NVCC) -arch=$(ARCH) -Xcompiler -pthread -o $@ $^ $(LDLIBS)
	@echo ""
	@echo "Built: $(TARGET) ($(ARCH))"

gpu_worker.o: gpu_worker.cu gpu_worker.h common.h solo_client.h host_tensorhash.h tensorhash_params.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

tensorhash_kernel.o: tensorhash_kernel.cu tensorhash.cuh blake2b.cuh tensorhash_params.h host_tensorhash.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

# main.cpp and modes.cpp use cuda_runtime.h — compile via nvcc -x c++
main.o: main.cpp common.h solo_client.h stratum_client.h gpu_worker.h nvml_monitor.h modes.h
	$(NVCC) $(NVCCFLAGS) -x c++ -c -o $@ $<

modes.o: modes.cpp modes.h common.h host_tensorhash.h tensorhash_params.h
	$(NVCC) $(NVCCFLAGS) -x c++ -c -o $@ $<

host_tensorhash.o: host_tensorhash.cpp host_tensorhash.h blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

solo_client.o: solo_client.cpp solo_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

stratum_client.o: stratum_client.cpp stratum_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

nvml_monitor.o: nvml_monitor.cpp nvml_monitor.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

# Host-only KAT harness — no GPU/CUDA needed (g++ only).
test-host: test_host_tensorhash.cpp host_tensorhash.cpp host_tensorhash.h blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -o test_host_tensorhash test_host_tensorhash.cpp host_tensorhash.cpp
	./test_host_tensorhash

install: $(TARGET)
	sudo cp $(TARGET) /usr/local/bin/tensorium-miner
	@echo "Installed tensorium-miner"

clean:
	rm -f $(TARGET) test_host_tensorhash *.o

info:
	@echo "ARCH=$(ARCH)"
	@nvcc --version 2>/dev/null | head -1
	@nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader 2>/dev/null || echo "no GPU detected"
```

(Note: `--use_fast_math` was dropped — irrelevant to integer hashing, and risky habits near consensus code.)

- [ ] **Step 7: Full compile gate**

```bash
cd tools/tensorium-miner
make clean && make test-host && make ARCH=sm_86 && echo BUILD-OK
```
Expected: KATs pass, `BUILD-OK` (link succeeds without a GPU; running needs one).

- [ ] **Step 8: Commit**

```bash
git add -A tools/tensorium-miner
git commit -m "feat(miner): TensorHash v1 miner — dataset lifecycle, selftest/benchmark/genesis modes, SHA256d removed"
```

---

### Task 11: Miner — README rewrite

**Files:**
- Modify: `tools/tensorium-miner/README.md`

- [ ] **Step 1: Replace the README content**

Rewrite `README.md` with these sections (keep the existing tone/format; full replacement):

1. **Title/intro:** "tensorium-miner — TensorHash v1 GPU Miner. CUDA miner for Tensorium's memory-hard, GPU-first mainnet. The miner materializes a 17.9 GiB dataset in VRAM (regenerated every 8,192 blocks ≈ 5.7 days) and samples 32 random elements per hash attempt."
2. **Requirements table:** NVIDIA GPU with **24 GB+ VRAM (RTX 3090 minimum)** — the miner refuses to start below ~20 GiB free; CUDA Toolkit 11.0+; Linux x86_64. State explicitly: cards below 24 GB cannot mine TensorHash v1.
3. **Build:** unchanged `make` / `make ARCH=sm_XX` instructions and the existing GPU-family table (drop GTX 10xx/RTX 20xx rows — insufficient VRAM; keep 3090, 4090, A100 80GB, H100, 5090).
4. **First run / selftest:** document that the miner spot-checks the dataset on every generation, and recommend a one-time `./tensorium-miner --selftest` after building; mismatch = refuse to mine.
5. **Usage:** solo/pool examples (same as before) plus the three new modes (`--selftest`, `--benchmark [secs]`, `--mode genesis --prefix <hex> --bits 42`), and the `--start-nonce` flag.
6. **Genesis workflow:** the five-step flow (`print-genesis-prefix` → `--mode genesis` → `verify-genesis` → commit nonce), referencing the spec.
7. **Performance notes:** delete all SHA256d GH/s tables; state hashrates are in MH/s, dominated by VRAM random-access bandwidth + Blake2b throughput, and that a benchmark table will be filled in from real vast.ai measurements (leave a placeholder table with "TBD (run `--benchmark`)" cells for RTX 3090/4090/H100 — these are *measurement* placeholders for Task 13, not spec gaps).
8. **Header format + RPC integration:** keep, updating chain_id example to `tensorium-mainnet` (102-byte pow prefix + 8 nonce bytes), and note the template now carries `epoch_seed`.
9. **Multi-GPU:** keep as-is.

- [ ] **Step 2: Commit**

```bash
git add tools/tensorium-miner/README.md
git commit -m "docs(miner): rewrite README for TensorHash v1 (24GB VRAM minimum, new modes)"
```

---

### Task 12: Local validation sweep

**Files:** none (verification)

- [ ] **Step 1: Rust workspace**

```bash
cargo test --workspace 2>&1 | tail -5
cargo clippy --workspace --all-targets 2>&1 | tail -3
```
Expected: all tests pass (219+ — 216 pre-existing + 4 new node/core tests, minus any counting drift); clippy finishes with warnings only.

- [ ] **Step 2: Miner matrix**

```bash
cd tools/tensorium-miner
make clean && make test-host
for arch in sm_86 sm_89 sm_90; do make clean >/dev/null; make ARCH=$arch >/dev/null && echo "$arch OK" || echo "$arch FAIL"; done
```
Expected: KATs pass; all three arches build OK.

- [ ] **Step 3: Push**

```bash
git push origin main
```

---

### Task 13: GPU runtime validation (vast.ai runbook — needs user-provided rental)

**Files:**
- Create: `docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md` (results log, filled during the session)

This task cannot run on this box. Coordinate with the user to rent a vast.ai instance (RTX 3090 or 4090, 24 GB, CUDA 12.x image, ≥25 GB disk). Then on the rental:

- [ ] **Step 1: Setup**

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git && cd tensorium-core
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env"
cargo build --release -p tensorium-node
cd tools/tensorium-miner && make && make test-host
```

- [ ] **Step 2: Selftest (the consensus gate)**

```bash
./tensorium-miner --selftest
```
Expected: all layers (1, 2, 3, 4a, 4b) PASS. **Any failure stops Phase A2 — debug before anything else.**

- [ ] **Step 3: Benchmark**

```bash
./tensorium-miner --benchmark 120
```
Record: dataset-generation seconds + sustained MH/s into the validation-notes doc and the README table. Evaluate the difficulty policy: if a single 3090/4090's expected solo block time at 42 bits is >4× off the 60 s target in either direction, flag for the recalibration follow-up commit (decided with the user; not automatic).

- [ ] **Step 4: Live-path test (devnet)**

```bash
cd ../..
./target/release/tensorium-node devnet init        # CPU-mines 20-bit TESTNET genesis (~seconds)
./target/release/tensorium-node devnet rpc 127.0.0.1:43332 &
curl -s localhost:43332/getblocktemplate/txm1qtestaddr | head -c 400   # must contain "epoch_seed"
cd tools/tensorium-miner
./tensorium-miner --mode solo --rpc http://127.0.0.1:43332 --wallet txm1qtestaddr
```
Expected: miner parses `epoch_seed`, generates dataset, finds 20-bit blocks within seconds each, `submitblock response: ...accepted...`, devnet height advances. Let it mine ≥10 blocks.

- [ ] **Step 5: Genesis dry-run**

```bash
cd ../.. && ./target/release/tensorium-node print-genesis-prefix   # placeholder timestamp
cd tools/tensorium-miner
./tensorium-miner --mode genesis --prefix <prefix_hex_from_above> --bits 42
# ... runs for the benchmark-predicted hours; on success:
cd ../.. && ./target/release/tensorium-node verify-genesis 1780272000 <nonce>
```
Expected: `VALID pow_hash = ...`. Record the nonce + wall-clock time in the notes doc. **Do NOT commit this nonce as `MAINNET_GENESIS_NONCE`** — the real one is mined at launch with the final timestamp. (If rental budget is tight, the dry-run may run at `--bits 36` (~1/64 the work) to validate the pipeline, with the 42-bit estimate extrapolated from the benchmark — note which variant was run.)

- [ ] **Step 6: Write up + commit the notes doc, update README benchmark table**

```bash
git add docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md tools/tensorium-miner/README.md
git commit -m "docs: Phase A2 GPU validation results (selftest, benchmark, devnet live-path, genesis dry-run)"
git push origin main
```

---

## Self-Review Notes

- **Spec coverage:** miner file layout (Tasks 6–11), CLI modes (10), node epoch_seed (2), genesis subcommands (3), devnet (4), selftest layers 1–4 (7, 8, 10), validation+benchmark plan (12, 13), genesis dry-run (13). Out-of-scope items untouched.
- **Consensus pinning:** every hash-bearing file is gated by V1–V8 vectors cross-derived from the Rust crate and an independent Python implementation before this plan was written.
- **Known deferred risks (acceptable):** device kernels are compile-gated only until Task 13 (no local GPU — inherent to the environment); `th_mine_kernel`'s per-thread `buf` may spill to local memory (perf, not correctness — `--warn-on-spills` will surface it; optimization is post-benchmark work, not this plan).
