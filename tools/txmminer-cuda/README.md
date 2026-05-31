# txmminer-cuda — Tensorium GPU Miner

CUDA-accelerated SHA256d miner for the Tensorium testnet and mainnet.

## Requirements

| Component | Version |
|-----------|---------|
| NVIDIA GPU | Compute Capability 6.1+ (GTX 1060 or newer) |
| CUDA Toolkit | 11.0 or newer |
| GCC/Clang | Any recent version |
| OS | Linux x86_64 (Windows supported via WSL2) |

> **Note:** This miner is for Tensorium's CPU-bootstrap testnet and the upcoming GPU-first testnet (Phase 6). At testnet difficulty 26, a single RTX 3060 will mine blocks in milliseconds. Difficulty will be raised to 36+ bits during Phase 6 for meaningful GPU mining.

## Build

```bash
cd tools/txmminer-cuda

# Auto-detect GPU and build
make

# Or specify GPU architecture explicitly
make ARCH=sm_89    # RTX 4000 series
make ARCH=sm_86    # RTX 3000 series
make ARCH=sm_80    # A100 / RTX 30 (Ampere)
make ARCH=sm_75    # RTX 2000 series (Turing)
make ARCH=sm_61    # GTX 1000 series (Pascal)
make ARCH=sm_90    # H100
```

Common architectures:

| GPU Family | `ARCH` |
|------------|--------|
| GTX 1060/1070/1080 | `sm_61` |
| RTX 2060/2070/2080 | `sm_75` |
| RTX 3060/3070/3080/3090 | `sm_86` |
| RTX 4060/4070/4080/4090 | `sm_89` |
| H100 SXM / PCIe | `sm_90` |
| H200 | `sm_90` |
| RTX 5090 (Blackwell) | `sm_100` |

## Usage

```bash
# Basic (auto-select GPU 0, default blocks/threads)
./txmminer-cuda 127.0.0.1:23332 txm1youraddress

# Specify GPU device, CUDA blocks, and threads per block
./txmminer-cuda 127.0.0.1:23332 txm1youraddress 0 2048 256

# Arguments:
#   rpc_host:port       Node RPC address (keep RPC on localhost)
#   miner_address       Your txm1... wallet address for block rewards
#   device_id           CUDA device index (default: 0)
#   cuda_blocks         Grid blocks (default: 2048)
#   cuda_threads        Threads per block (default: 256)
```

## Performance Guide

### Tuning `cuda_blocks` and `cuda_threads`

The total parallel hashrate = `cuda_blocks × cuda_threads × iters_per_thread / elapsed`.

| GPU | Recommended | Expected Hashrate |
|-----|-------------|-------------------|
| RTX 3060 | `2048 256` | ~500 MH/s – 1 GH/s |
| RTX 3080 | `4096 256` | ~1.2 – 2 GH/s |
| RTX 4090 | `8192 256` | ~2.5 – 4 GH/s |
| H100 SXM | `8192 512` | ~2 – 3.5 GH/s |

> SHA256d is a compute-intensive workload. H100/H200 are optimized for AI matrix
> operations and do NOT significantly outperform gaming GPUs (RTX 4090) for SHA256d.

### Expected Block Times at Difficulty 26

| Hardware | ~Hashrate | Expected Block Time |
|----------|-----------|---------------------|
| RTX 3060 | 600 MH/s | ~0.1 seconds |
| RTX 4090 | 3 GH/s | ~0.02 seconds |

Phase 6 will raise difficulty to 36+ bits (68 billion hashes expected per block).
At that difficulty, RTX 3060 (~600 MH/s) will take ~113 seconds per block on average.

## Technical Notes

### SHA256d Midstate Optimisation

The Tensorium block header is 112 bytes. SHA256d requires 2 compression rounds for the 112-byte input + 1 for the second hash = 3 rounds total.

This miner uses the **midstate optimisation**:
1. CPU precomputes the SHA256 state after the first 64 bytes of the header (constant for all nonces)
2. GPU receives only the midstate + the remaining 48 bytes
3. GPU completes: 1 compression round (second block) + 1 full SHA256 (second hash) = 2 rounds

This reduces GPU work from 3 → 2 compression rounds per nonce (~33% faster).

### Header Format

```
bytes [0..3]    version       (u32 LE)
bytes [4..22]   chain_id      ("tensorium-testnet-0", 19 bytes)
bytes [23..30]  height        (u64 LE)
bytes [31..62]  previous_hash (32 bytes)
bytes [63..94]  merkle_root   (32 bytes)
bytes [95..102] timestamp     (u64 LE)
bytes [103]     difficulty_bits (u8)
bytes [104..111] nonce        (u64 LE)  ← varied per thread
```

## Integration with Node

The CUDA miner uses the same RPC endpoints as `txmminer`:
- `GET /getblocktemplate/<address>` — fetch candidate block
- `POST /submitblock` — submit mined block

Keep the RPC bound to `127.0.0.1:23332` — never expose it publicly.

## Multi-GPU Setup

Run one instance per GPU:

```bash
./txmminer-cuda 127.0.0.1:23332 txm1youraddress 0 &  # GPU 0
./txmminer-cuda 127.0.0.1:23332 txm1youraddress 1 &  # GPU 1
./txmminer-cuda 127.0.0.1:23332 txm1youraddress 2 &  # GPU 2
```

Each instance uses a different `start_nonce` region by default (based on device ID).
