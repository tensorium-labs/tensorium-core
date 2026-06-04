# Tensorium — Next Session Handoff Prompt

Date written: 2026-06-03. Resume from this state.

---

## Baca Dulu (urutan ini)

1. `/root/.claude/projects/-root/memory/project_tensorium.md` — full project history & status
2. `README.md` — current public-facing commands (solo/pool mining, install)
3. `MAINNET_READINESS.md` — gate checklist

Lalu jalankan di VPS sebelum lanjut:
```bash
curl -s http://127.0.0.1:33332/getblockcount    # MC chain height
pm2 list                                          # semua services
cat /root/tensorium-bridge-relayer/relayer-state.json  # bridge state
tail -20 /root/tensorium-bridge-relayer/logs/error.log # bridge errors
```

---

## State Saat Ini (2026-06-03)

**Chain:** `tensorium-mainnet-candidate-0`, height ~302, diff=40 bits  
**GPU miner:** Vast.ai RTX 5090 `ssh -p 2602 root@64.31.38.214` → tmux `txmminer`, ~7.8 GH/s  
**Commit terakhir:** `a0679bc` — release v0.3.2-mainnet  

**Release v0.3.2-mainnet** live di GitHub:
- `tensorium-node-linux-x86_64` ✓
- `txmwallet-linux-x86_64` ✓
- `tensorium-miner-linux-x86_64-sm86/sm89/sm120` ✓

**Bridge:** LIVE dan otomatis — relayer auto-mint wTXM untuk setiap deposit baru  
- Controller: `0x4b31C557AD64609B975610812273BF82F1475384` (OP Mainnet)  
- wTXM: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` (OP Mainnet)  
- Gnosis Safe: `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9`  
- maxPerTx: 1,000,000 wTXM  

**Uniswap V4 pool:** LIVE di OP Mainnet  
- LP NFT #25829, seed 10K wTXM  
- Butuh top up: 490K wTXM + ~$2500 ETH ke `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB`

**Pool mining:** LIVE dan fixed  
- `tensorium-miner pooltxm.tensoriumlabs.com:23336 YOUR_ADDRESS`
- Fee 5%, treasury `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`

---

## BUGFIX yang sudah selesai sesi ini

- Pool `TENSORIUM_NODE_RPC` fixed dari 23332 (testnet) → 33332 (MC) ✓  
- tensorium-miner: constant memory optimization → 7.8 GH/s ✓
- GitHub release dibuat dari 0 (tidak ada sebelumnya) ✓  
- Docs bersih: no testnet/CPU miner refs ✓  

---

## Prioritas Next Session (urutan)

### 1. Top up Uniswap Liquidity
- Kirim ETH ke `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB` di OP Mainnet
- Tambah 490K wTXM + ETH ke pool V4 #25829 di https://app.uniswap.org/positions/v4/optimism/25829
- Target: $5K–10K TVL sebelum submit CMC/CoinGecko

### 2. CMC + CoinGecko Submission
- Submit SETELAH pool punya ≥1 trade dan minimal liquidity
- Data di `docs/integrations/CANONICAL_ASSET_METADATA.md`
- CoinGecko: https://www.coingecko.com/en/coins/add
- CMC: https://coinmarketcap.com/request/

### 3. Bridge Relayer Monitoring
- Cek apakah ada deposit user yang masuk ke custody address
- Cek `relayer-state.json` untuk processedUtxos terbaru
- Pastikan tidak ada `failedMints` baru

### 4. CEX Follow-up
- Cek email `dev@tensoriumlabs.com` — 14 exchanges dicontact 2026-06-02
- Exchanges: MEXC, Gate.io, CoinEx, OKX, Bybit, SafeTrade, LBank, XT.com, BitMart, CoinW, DigiFinex, Hotcoin, BingX, BTCC

### 5. S3 Scripting Layer (OP_CLTV / HTLC)
- Timelock transactions (OP_CHECKLOCKTIMEVERIFY)
- HTLC → atomic swap foundation
- Resume dari scripting codebase di `crates/tensorium-core/src/script/`

### 6. Chrome Web Store
- v0.1.0 submitted, tunggu approval
- v0.1.1 ZIP ready untuk upload setelah approved

---

## VPS Quick Reference

| Service | Bind | Command |
|---------|------|---------|
| MC RPC | 127.0.0.1:33332 | `tensorium-node mainnet-candidate rpc` |
| MC P2P | 0.0.0.0:33333 | `tensorium-node mainnet-candidate p2p-listen` |
| Pool | 0.0.0.0:23336 | `tensorium-pool serve` |
| Bridge relayer | 127.0.0.1:3004 | pm2 `tensorium-bridge-relayer` |
| Explorer | 0.0.0.0:3000 | pm2 `tensorium-explorer` |
| Faucet | 0.0.0.0:3003 | pm2 `tensorium-faucet` |
| Pool website | 0.0.0.0:3002 | pm2 `tensorium-pool-website` |

VPS: `157.230.44.162` — password di session context (jangan disimpan di file)  
GPU Vast.ai: `ssh -p 2602 root@64.31.38.214` — key auth, tmux `txmminer` = miner aktif

---

## Wallet Inventory

| Wallet | Address | Balance |
|--------|---------|---------|
| Founder | `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d` | 1,000,000 TXM |
| Liquidity | `txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p` | ~2,500,000 TXM |
| Bridge reserve | `txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq` | 2,000,000 TXM |
| Ecosystem | `txm1jwz2nvfajy84kyypzxp0pq8n5vrwahu6yny9hf` | 2,000,000 TXM |
| Miner rewards | `txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck` | growing |
| EVM liquidity | `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB` | 490K wTXM ready |

---

## Security Reminders

- GitHub token dari session ini harus sudah direvoke
- Jangan commit .env, private key, atau webhook URL ke repo
- Bridge relayer state: `/root/tensorium-bridge-relayer/relayer-state.json` — jangan hapus
