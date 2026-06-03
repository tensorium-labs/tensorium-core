# Tensorium Miner v2 — Handoff Prompt

Date: 2026-06-03. Resume dari state ini di sesi berikutnya.

---

## Baca Dulu

1. `/root/.claude/projects/-root/memory/project_tensorium.md` — full history
2. `tools/txmminer-cuda/solo_client.cpp` — bug fixes terbaru
3. `git log --oneline -8` — semua commit sesi ini

---

## State Saat Ini

**Chain:** `tensorium-mainnet-candidate-0`, height 357 (VPS), miner sedang mine ke block 358  
**GPU miner:** Vast.ai RTX 5090 `ssh -p 2602 root@64.31.38.214`  
- tmux session `txmminer` — solo mining ke MC node via SSH tunnel  
- ~8.3 GH/s real throughput (~132 detik/block expected)  
- Binary: `~/tensorium-core/tools/txmminer-cuda/tensorium-miner` (sm_120)

**Tunnel MC node (dari Vast.ai ke VPS):**
```bash
pgrep -f "ssh.*33332" > /dev/null || sshpass -p [REDACTED] ssh -fN \
  -o StrictHostKeyChecking=no -L 33332:127.0.0.1:33332 root@157.230.44.162
```

---

## Bug Fixes yang Sudah Selesai Sesi Ini

### tensorium-miner v2 (tools/txmminer-cuda/)

**Bug 1:** Solo mode pakai `share_bits=20` (pool threshold) bukan `difficulty_bits=40`  
→ Kernel tidak pernah nemu real block (hanya 20-bit hash)  
→ Fix: `job.share_bits = job.difficulty_bits` di solo_client.cpp

**Bug 2:** `submit_block` kirim `"transactions":[]` tanpa coinbase  
→ Node reject: "block merkle root is invalid"  
→ Fix: cache raw `template` JSON dari response, replace `"nonce"` value saja

**Bug 3:** `intensity auto` hardcoded ke level 7 (8192×128 threads) bukan level 8 (8192×256)  
→ Hashrate 50% rendah  
→ Fix: `use_intensity=0` di main.cpp → `intensity_to_launch` default ke level 8

---

## Langkah Pertama Sesi Berikutnya

### 1. Verify solo mining bekerja

```bash
# Check apakah block sudah ditemukan sejak handoff
ssh -o StrictHostKeyChecking=no -p 2602 root@64.31.38.214 \
  'grep -E "BLOCK|submit|height=" ~/miner.log | tail -10'

# Check chain height di VPS
sshpass -p '[REDACTED]' ssh -o StrictHostKeyChecking=no root@157.230.44.162 \
  'curl -s http://127.0.0.1:33332/getblockcount'
```

Kalau block ditemukan dan height naik dari 357 → berarti solo mining working end-to-end.

### 2. Kalau miner mati (restart)

```bash
ssh -o StrictHostKeyChecking=no -p 2602 root@64.31.38.214 '
# Ensure tunnel
pgrep -f "ssh.*33332" > /dev/null || sshpass -p [REDACTED] ssh -fN \
  -o StrictHostKeyChecking=no -L 33332:127.0.0.1:33332 root@157.230.44.162

# Restart miner
tmux kill-session -t txmminer 2>/dev/null
tmux new-session -d -s txmminer
tmux send-keys -t txmminer "cd ~/tensorium-core/tools/txmminer-cuda && \
  ./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck \
  --gpu all --intensity auto 2>&1 | tee ~/miner.log" Enter
'
```

### 3. Kalau ada code fix (rebuild)

```bash
ssh -o StrictHostKeyChecking=no -p 2602 root@64.31.38.214 '
cd ~/tensorium-core && git pull origin main
cd tools/txmminer-cuda && make clean && make ARCH=sm_120
'
```

---

## Prioritas Lanjutan

1. **Verify solo mining end-to-end** — block ditemukan + submitted + height naik
2. **Top up Uniswap V4 liquidity** — kirim ETH ke `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB` di OP Mainnet, tambah 490K wTXM
3. **CMC + CoinGecko submission** — setelah pool punya ≥1 trade (data di `docs/integrations/CANONICAL_ASSET_METADATA.md`)
4. **CEX follow-up** — check `dev@tensoriumlabs.com` (14 exchanges contacted 2026-06-02)
5. **S3 scripting** — OP_CLTV, HTLC, atomic swap

---

## Commits Sesi Ini (chronological)

```
163262a chore: update Cargo.lock
d3bace8 fix(miner): submit_block — use cached template JSON with coinbase
791dca9 fix(miner): intensity auto — pass 0 to intensity_to_launch
6902e17 fix(miner): intensity auto → level 8 (8192×256 threads)
4ebdc8b fix(miner): submit_block — build Block JSON from JobDesc + refresh after block
d2923be fix(miner): solo mode — mine at full difficulty_bits, not share_bits
7bc189d release: tensorium-miner v2 — multi-GPU Stratum pool mode, NVML
```

---

## Infrastructure Quick Reference

| Service | Details |
|---------|---------|
| VPS | `157.230.44.162`, password `[REDACTED]` |
| MC RPC (VPS) | `http://127.0.0.1:33332` (local) / `https://mc-rpc.tensoriumlabs.com` (public) |
| GPU Vast.ai | `ssh -p 2602 root@64.31.38.214` |
| Pool Stratum | `pooltxm.tensoriumlabs.com:3333` |
| Pool HTTP | `pooltxm.tensoriumlabs.com:23336` |
| Gnosis Safe | `https://app.safe.global/OP:0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9` |
| wTXM | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` (OP Mainnet) |
| Uniswap V4 LP | tokenId #25829, 10K wTXM seeded |
