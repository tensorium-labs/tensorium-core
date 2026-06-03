# Tensorium Miner v2 — Design Spec

**Date:** 2026-06-03  
**Scope:** Full rewrite of `txmminer-cuda` into `tensorium-miner` with multi-GPU, Stratum pool server (Rust) + Stratum client (C++), NVML monitoring, clean CLI.  
**Approach:** Big Bang — all components built in one sprint.

---

## 1. Goals

1. Replace single-GPU HTTP-only `txmminer-cuda` with `tensorium-miner` — a multi-GPU miner supporting both solo and pool mining modes.
2. Add a Tensorium Stratum protocol server to `tensorium-pool` (port 3333) so miners can connect via `stratum+tcp://`.
3. Publish the Tensorium Stratum protocol as a documented spec so the community can build compatible miners and pools.
4. Maintain full backward compatibility — `txmminer-cuda` symlinked to `tensorium-miner`, solo HTTP mode unchanged.

---

## 2. Non-Goals

- Multi-pool failover (out of scope for v2)
- Windows support (Linux x86_64 only)
- Stratum v2 (NOISE protocol encrypted transport)
- Dynamic difficulty per-worker (all workers share one pool difficulty)
- Web dashboard / API for miner stats

---

## 3. Architecture

### 3.1 Miner (`tools/txmminer-cuda/`)

```
tensorium-miner (binary)
├── main.cpp              — CLI parsing, mode dispatch, signal handling, stats aggregator
├── common.h              — shared types: MinerConfig, JobDesc, ShareResult, GpuStats
├── gpu_worker.cu         — per-GPU thread: kernel loop, nonce range, job update, share queue
├── mining_kernel.cu      — SHA256d CUDA kernel (existing, no functional changes)
├── sha256d.cuh           — SHA256d device helpers (unchanged)
├── solo_client.cpp/h     — HTTP RPC: getblocktemplate, submitblock, reconnect
├── stratum_client.cpp/h  — Stratum TCP: subscribe, authorize, notify, submit, reconnect
├── nvml_monitor.cpp/h    — NVML polling: temp, power, fan (optional, graceful fallback)
└── Makefile              — multi-file build, WITH_NVML=1 flag, sm_* target
```

`txmminer-cuda` → symlink to `tensorium-miner` (backward compat, existing scripts unchanged).

### 3.2 Pool (`crates/tensorium-pool/src/`)

```
main.rs        — add Stratum listener thread on port 3333 (alongside existing HTTP port 23336)
stratum.rs     — NEW: Stratum server, job broadcaster, share validator, worker registry
accounting.rs  — EXISTING: reused for pool fee split and payout ledger
```

---

## 4. Tensorium Stratum Protocol v1

### 4.1 Transport

- TCP, port 3333
- Newline-delimited JSON (`\n` terminated)
- No encryption (v1 — cleartext)
- Server keeps connection alive; client reconnects on drop

### 4.2 Message Flow

```
Client                              Server
  |--- mining.subscribe ----------->|
  |<-- subscribe response ----------|
  |--- mining.authorize ----------->|
  |<-- authorize response ----------|
  |<-- mining.set_difficulty -------|
  |<-- mining.notify (first job) ---|
  |
  |--- mining.submit (share) ------>|
  |<-- submit response (accepted) --|
  |
  |<-- mining.notify (new block) ---| ← broadcast to all workers
  |--- mining.submit (share) ------>|
  |<-- submit response -------------|
```

### 4.3 Message Definitions

#### `mining.subscribe` (client → server)
```json
{
  "id": 1,
  "method": "mining.subscribe",
  "params": ["tensorium-miner/2.0"]
}
```

**Response:**
```json
{
  "id": 1,
  "result": {
    "session_id": "a1b2c3d4",
    "protocol": "tensorium-stratum/1",
    "nonce_bits": 64
  },
  "error": null
}
```

#### `mining.authorize` (client → server)
```json
{
  "id": 2,
  "method": "mining.authorize",
  "params": ["txm1xxx.rig01", "x"]
}
```
`params[0]` = `wallet.worker_name`. Password (`params[1]`) ignored in v1.

**Response (two messages):**
```json
{"id": 2, "result": true, "error": null}
{"id": null, "method": "mining.set_difficulty", "params": [1048576]}
```

#### `mining.notify` (server → client, broadcast)
```json
{
  "id": null,
  "method": "mining.notify",
  "params": {
    "job_id": "h302-a1b2c3",
    "chain_id": "tensorium-mainnet-candidate-0",
    "height": 302,
    "previous_hash": "0000000000abc123...",
    "merkle_root": "deadbeefcafe0000...",
    "timestamp": 1780300000,
    "difficulty_bits": 40,
    "share_difficulty": 1048576,
    "clean_jobs": true
  }
}
```

`clean_jobs: true` means discard all pending work and start on this job immediately.  
`difficulty_bits` = network difficulty (consensus, from node).  
`share_difficulty` = pool share threshold (set by pool, ≤ network diff).

#### `mining.submit` (client → server)
```json
{
  "id": 3,
  "method": "mining.submit",
  "params": {
    "job_id": "h302-a1b2c3",
    "worker": "txm1xxx.rig01",
    "nonce": "0000deadbeef1234"
  }
}
```
`nonce` = 16 hex chars (8 bytes little-endian, matches Tensorium header format).

**Response:**
```json
{"id": 3, "result": "accepted", "error": null}
```
or
```json
{"id": 3, "result": "rejected", "error": "stale"}
```

Error values: `"stale"` | `"invalid"` | `"duplicate"`.

#### `mining.ping` / `mining.pong` (keepalive)
```json
{"id": null, "method": "mining.ping", "params": []}
{"id": null, "method": "mining.pong", "params": []}
```
Server sends ping every 30s. Client must respond within 10s or connection is dropped.

### 4.4 Difficulty Separation

```
Network difficulty  = difficulty_bits field in mining.notify (e.g. 40 bits = 2^40 hashes expected)
Share difficulty    = share_difficulty field in mining.notify (default 1,048,576 ≈ 2^20)

Miner logic:
  sha256d(header) → count leading zero bits
  if leading_zeros >= share_difficulty_bits: submit share to pool
  (pool also checks if leading_zeros >= difficulty_bits → real block found)

Pool logic on mining.submit:
  1. Reconstruct header from job_id + nonce
  2. SHA256d(header) → leading_zeros
  3. if leading_zeros < share_difficulty_bits: reject("invalid")
  4. if job_id not current or previous: reject("stale")
  5. accept share → record in accounting
  6. if leading_zeros >= network difficulty_bits: submit block to node RPC
```

`share_difficulty` range: 524,288–1,048,576 (pool operator configurable, default 1,048,576).  
`share_difficulty_bits` = floor(log2(share_difficulty)) = 19–20 bits.

---

## 5. Multi-GPU Architecture

### 5.1 Thread Model

```
main thread
  ├── stratum_client thread  (pool mode) — TCP conn, recv notify, send submit
  │     └── job_channel (broadcast) ─────┐
  ├── solo_client thread    (solo mode) ──┤
  │     └── job_channel (broadcast) ─────┤
  │                                       ↓
  ├── gpu_worker[0] thread  ← job_channel, → share_queue
  ├── gpu_worker[1] thread  ← job_channel, → share_queue
  ├── gpu_worker[N] thread  ← job_channel, → share_queue
  │
  ├── share_dispatcher thread — reads share_queue → calls stratum/solo submit
  ├── nvml_monitor thread    — polls NVML every 30s, updates GpuStats[]
  └── stats_printer thread   — prints aggregate hashrate + per-GPU stats every 5s
```

### 5.2 Nonce Space Split

Nonce is 64-bit. Split evenly across N GPUs:

```
GPU 0: [0,                   UINT64_MAX/N)
GPU 1: [UINT64_MAX/N,        2*UINT64_MAX/N)
GPU N: [(N-1)*UINT64_MAX/N,  UINT64_MAX)
```

Within each GPU, nonce advances by `stride = blocks × threads` per kernel iteration, same as current implementation.

### 5.3 Job Update

When a new job arrives:
1. `job_channel` publishes `JobDesc` (atomic pointer swap)
2. Each `gpu_worker` checks `job_channel` after each kernel launch
3. On new job: reset nonce to GPU's range start, upload new midstate + W2 to constant memory
4. Stale shares (from previous job) are silently discarded at `share_dispatcher`

---

## 6. CLI Interface

### 6.1 Solo Mode

```bash
tensorium-miner \
  --mode solo \
  --rpc http://127.0.0.1:33332 \
  --wallet txm1xxxxxxxxxxxxxxxxxxxxxxxx \
  --gpu all \
  --intensity auto
```

### 6.2 Pool Mode

```bash
tensorium-miner \
  --mode pool \
  --pool stratum+tcp://pool.tensoriumlabs.com:3333 \
  --wallet txm1xxxxxxxxxxxxxxxxxxxxxxxx \
  --worker rig01 \
  --gpu all \
  --intensity auto \
  --share-diff 1048576
```

### 6.3 Flag Reference

| Flag | Default | Description |
|------|---------|-------------|
| `--mode` | `solo` | `solo` or `pool` |
| `--rpc` | `http://127.0.0.1:33332` | Solo: node RPC URL |
| `--pool` | — | Pool: `stratum+tcp://host:port` |
| `--wallet` | required | TXM reward/payout address |
| `--worker` | hostname | Worker name shown in pool stats |
| `--gpu` | `all` | `all`, `0`, `0,1,2` (comma-separated) |
| `--intensity` | `auto` | `auto` or `1`–`10` |
| `--share-diff` | `1048576` | Pool share difficulty (pool mode only) |
| `--nvml` | auto-detect | `on` / `off` to force |

### 6.4 Intensity Mapping

| Level | CUDA Blocks | Threads | Notes |
|-------|------------|---------|-------|
| `auto` | 8192 | 256 | Default, optimal for RTX 3000+ |
| 1 | 1024 | 128 | Low power / older GPU |
| 3 | 2048 | 256 | Mid-range |
| 5 | 4096 | 256 | Standard |
| 7 | 8192 | 256 | High performance |
| 10 | 16384 | 256 | Maximum |

---

## 7. Output Format

```
tensorium-miner v2.0.0
GPUs detected: 2

  [0] NVIDIA GeForce RTX 5090   sm_120   blocks=8192  threads=256
  [1] NVIDIA GeForce RTX 4090   sm_89    blocks=8192  threads=256

mode=pool   pool=pool.tensoriumlabs.com:3333   worker=rig01
[pool] connected
[pool] authorized: txm1xxx.rig01
[pool] share_diff=1048576 (~20 bits)   network_diff=40 bits
[pool] mining height=302   job=h302-a1b2c3

[GPU 0]  7.82 GH/s   temp=68°C   power=445W   fan=72%
[GPU 1]  2.51 GH/s   temp=71°C   power=290W   fan=68%
[total] 10.33 GH/s   shares=14 accepted / 0 rejected

[pool] ✓ share accepted   height=302   nonce=0000deadbeef1234   GPU=0
[pool] ✓ share accepted   height=302   nonce=00001234abcd5678   GPU=1
[pool] ⛏ BLOCK FOUND!     height=303   nonce=000000cafe001234   submitted
[pool] mining height=303   job=h303-x9y8z7
```

---

## 8. Pool Server Changes (`tensorium-pool`)

### 8.1 New Port Layout

| Port | Protocol | Purpose |
|------|----------|---------|
| 23336 | HTTP | Existing RPC proxy (unchanged) |
| 3333 | TCP Stratum | New: miner connections |

### 8.2 `stratum.rs` Responsibilities

- Accept TCP connections on port 3333
- Handle subscribe / authorize per connection
- Maintain worker registry: `worker_name → wallet_address → connection`
- Broadcast new jobs to all connected workers when node produces new block template
- Validate submitted shares: reconstruct header, SHA256d, check share_difficulty
- On valid block: call node `/submitblock`, broadcast new job immediately
- Track: shares accepted, shares rejected, blocks found per worker
- Ping/pong keepalive every 30s

### 8.3 Share Difficulty Config

```
TENSORIUM_POOL_SHARE_DIFF=1048576   # env var, default 1048576 (range: 524288–1048576)
```

### 8.4 Stratum Stats in Pool API

Extend existing `/api/pool` HTTP endpoint with:
```json
{
  "stratum_workers": 3,
  "stratum_port": 3333,
  "share_difficulty": 1048576,
  "shares_accepted_1h": 142,
  "shares_rejected_1h": 2
}
```

---

## 9. NVML Monitoring

- Compile with `make WITH_NVML=1` (links `libnvml`)
- If `nvml.h` not found: compile without, NVML fields show `--`
- Polls every 30 seconds per GPU: temperature, power draw, fan speed
- No crash, no warning spam if NVML unavailable
- Vast.ai compatible (NVML available in most Vast.ai containers)

---

## 10. Error Handling & Reconnect

| Scenario | Behavior |
|----------|----------|
| Pool TCP disconnect | Retry after 5s, backoff ×2 up to 60s max |
| Node RPC timeout (solo) | Retry after 3s |
| Stale job (new block during kernel) | GPU abandons kernel, loads new job immediately |
| Duplicate nonce submit | Pool rejects as `"duplicate"`, miner logs, continues |
| CUDA error on GPU N | Log error, disable that GPU, continue with remaining |
| All GPUs failed | Exit with error code 1 |
| SIGINT / SIGTERM | Clean shutdown: flush pending shares, close connections |

---

## 11. Build

```bash
# Solo mode only (no Stratum)
make ARCH=sm_120

# With NVML monitoring
make ARCH=sm_120 WITH_NVML=1

# All architectures (release)
make ARCH=sm_86   && cp tensorium-miner tensorium-miner-sm86
make ARCH=sm_89   && cp tensorium-miner tensorium-miner-sm89
make ARCH=sm_120  && cp tensorium-miner tensorium-miner-sm120

# Install
sudo cp tensorium-miner /usr/local/bin/
sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda
```

Pool (Rust):
```bash
cargo build -p tensorium-pool --release
# New env var: TENSORIUM_POOL_SHARE_DIFF=1048576
# New env var: TENSORIUM_STRATUM_BIND=0.0.0.0:3333
```

---

## 12. Testing Checklist

- [ ] Solo mode: single GPU, mine 3 blocks on testnet
- [ ] Solo mode: multi-GPU (2+), verify nonce ranges don't overlap
- [ ] Pool mode: connect to Stratum server, receive job, submit shares
- [ ] Pool mode: share accepted/rejected correctly based on share_diff
- [ ] Pool mode: real block found via Stratum → pool submits to node → block confirmed
- [ ] Job update: new block → all GPUs switch job within 1 kernel cycle
- [ ] Reconnect: kill pool → miner reconnects within 10s
- [ ] NVML: stats show with `WITH_NVML=1`, graceful fallback without
- [ ] Multi-GPU: 2 GPUs total hashrate ≈ sum of individual hashrates
- [ ] Backward compat: `txmminer-cuda host:port address` still works via symlink
