# tensorium-miner — TensorHash v1 GPU Miner

CUDA miner for Tensorium's memory-hard, GPU-first mainnet. The miner
materializes a **17.9 GiB dataset in VRAM** (regenerated every 8,192 blocks
≈ 5.7 days, when the epoch seed changes) and samples 32 random elements per
hash attempt. Verification on full nodes stays cheap — only miners need the
dataset.

## Requirements

| Component | Requirement |
|-----------|-------------|
| NVIDIA GPU | **24 GB+ VRAM — RTX 3090 minimum** |
| CUDA Toolkit | 11.0 or newer |
| GCC/Clang | Any recent version |
| OS | Linux x86_64 (Windows supported via WSL2) |

> **Cards below 24 GB cannot mine TensorHash v1.** The miner checks free VRAM
> at startup and refuses to run with less than ~20 GiB free (17.9 GiB dataset
> + working headroom).

## Build

```bash
cd tools/tensorium-miner

# Auto-detect GPU and build
make

# Or specify GPU architecture explicitly
make ARCH=sm_86    # RTX 3090 (Ampere)
make ARCH=sm_89    # RTX 4090 (Ada)
make ARCH=sm_80    # A100 80GB
make ARCH=sm_90    # H100 / H200
make ARCH=sm_100   # RTX 5090 (Blackwell, needs CUDA 12.8+)
```

| GPU | VRAM | `ARCH` |
|-----|------|--------|
| RTX 3090 / 3090 Ti | 24 GB | `sm_86` |
| RTX 4090 | 24 GB | `sm_89` |
| A100 | 80 GB | `sm_80` |
| H100 / H200 | 80/141 GB | `sm_90` |
| RTX 5090 | 32 GB | `sm_100` |

## First run — selftest

The GPU implementation must match the consensus reference bit-for-bit. After
building, run the selftest once:

```bash
./tensorium-miner --selftest
```

It verifies the host reference against hardcoded Rust KAT vectors, generates
the full dataset, spot-checks thousands of elements, and pushes known-answer
and randomized attempts through the real mining kernel. **Any mismatch and
the miner refuses to mine.** A lighter spot-check also runs automatically
after every dataset (re)generation during normal mining.

The host-only KAT vectors can be checked on any machine (no GPU needed):

```bash
make test-host
```

## Usage

```bash
# Solo mining (full network difficulty)
./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet txm1youraddress

# Pool mining
./tensorium-miner --mode pool --pool stratum+tcp://pool.tensoriumlabs.com:3333 \
                  --wallet txm1youraddress --worker rig1

# Standalone modes
./tensorium-miner --selftest                 # consensus KAT verification
./tensorium-miner --benchmark 120            # dataset-gen time + sustained MH/s
./tensorium-miner --mode genesis --prefix <hex> --bits 42 [--start-nonce N]

# Options:
#   --rpc               Node RPC address (keep RPC on localhost)
#   --wallet            Your txm1... wallet address for block rewards
#   --gpu all|0,1,2     GPUs to use (default: all)
#   --intensity auto|N  1-10 kernel size preset (default: auto)
#   --share-diff N      pool share difficulty
#   --start-nonce N     genesis mode: nonce search start offset
```

## Genesis mining workflow

The mainnet genesis nonce is mined offline with this miner (no node/RPC
needed — genesis is epoch 0, fixed zero seed):

```bash
# 1. On any box with the repo: get the canonical prefix for a launch timestamp
tensorium-node print-genesis-prefix <unix_timestamp>

# 2. On the GPU box: mine it
./tensorium-miner --mode genesis --prefix <prefix_hex> --bits 42

# 3. Verify the found nonce against the Rust consensus reference
tensorium-node verify-genesis <unix_timestamp> <nonce>

# 4. Commit the nonce into MAINNET_GENESIS_NONCE (launch-time step)
```

See `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a2-design.md`.

## Performance

TensorHash hashrates are measured in **MH/s** (not GH/s — this is not
SHA256d). Throughput is bound by VRAM random-access bandwidth plus Blake2b
compute: each attempt does ~35 Blake2b-256 compressions and 32 random 32-byte
dataset reads. AI-class cards with HBM (H100/H200) benefit from the higher
memory bandwidth, unlike with SHA256d.

Measured results (fill in via `--benchmark` on real hardware):

| GPU | Dataset gen | Sustained hashrate |
|-----|-------------|--------------------|
| RTX 5090 (measured 2026-06-10) | 0.14 s | 220.31 MH/s |
| RTX 3090 | TBD (run `--benchmark`) | TBD (run `--benchmark`) |
| RTX 4090 | TBD (run `--benchmark`) | TBD (run `--benchmark`) |
| H100 | TBD (run `--benchmark`) | TBD (run `--benchmark`) |

Expected attempts for the 42-bit genesis: 2^42 ≈ 4.4×10¹². Network block
times after launch depend on live difficulty (retargeting is active from
block 0).

## Technical notes

### Header format

Tensorium block headers serialize with a variable-length `chain_id`; mainnet
(`tensorium-mainnet`) gives a **102-byte pow prefix + 8 nonce bytes**:

```
bytes [0..3]      version         (u32 LE)
bytes [4..N]      chain_id        (variable-length ASCII)
bytes [...]       height          (u64 LE)
bytes [...]       previous_hash   (32 bytes)
bytes [...]       merkle_root     (32 bytes)
bytes [...]       timestamp       (u64 LE)
bytes [...]       difficulty_bits (u8)
bytes [...]       nonce           (u64 LE)  ← varied per thread
```

The pow hash is `tensorhash_v1(prefix, nonce, epoch_seed)` — see
`crates/tensorium-tensorhash` for the consensus reference.

### Node integration

- `GET /getblocktemplate/<address>` — candidate block; the response includes
  `epoch_seed`, which the miner needs to (re)generate its dataset. Nodes
  older than v0.5 don't send it and are rejected by the miner.
- `POST /submitblock` — submit mined block.

Keep the RPC bound to `127.0.0.1` — never expose it publicly.

## Multi-GPU

Run one instance with `--gpu all` (default): each GPU gets its own dataset
copy and a disjoint nonce range. One instance per GPU also works:

```bash
./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet txm1you --gpu 0 &
./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet txm1you --gpu 1 &
```
