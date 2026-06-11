# Tensorium (TXM) — CEX/CMC/CoinGecko Listing Package

Last updated: 2026-06-11

Canonical source for integrator-facing fields:

- `docs/integrations/CANONICAL_ASSET_METADATA.md`

---

## Token Information

| Field | Value |
|---|---|
| **Project name** | Tensorium |
| **Token ticker** | TXM |
| **Token type** | Native L1 PoW (not ERC-20 on original chain) |
| **Wrapped token** | wTXM — ERC-20 on Optimism (for DEX/CEX listing) |
| **wTXM contract** | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` (Optimism mainnet, chainId 10) |
| **Decimals** | 18 |
| **Max supply** | 33,000,000 TXM |
| **Circulating supply** | Mined from genesis (public mining, zero premine) |
| **Founder allocation** | 0 TXM in genesis |
| **Mining allocation** | 33,000,000 TXM — block rewards over 10 halving eras (~20 years) |
| **Block reward** | 11.90279581 TXM/block (Era 1) |
| **Halving interval** | 1,051,200 blocks (~2 years) |
| **Mining algorithm** | TensorHash v1 Proof-of-Work |
| **Consensus** | Nakamoto PoW, UTXO model |
| **Launch date** | 2026-06-11 (mainnet v1 relaunch) |
| **License** | Apache-2.0 |

---

## Project Description

**Short (100 chars):**
Tensorium (TXM) is a GPU-first Proof-of-Work Layer 1 blockchain with transparent tokenomics and open mining.

**Medium (300 chars):**
Tensorium is a GPU-first Proof-of-Work Layer 1 blockchain — TensorHash v1 consensus, UTXO model, 33M TXM fixed supply. Open to any miner. No pre-sale. No VC allocation. Zero premine on mainnet v1. wTXM bridged to Optimism for DEX access.

**Full:**
Tensorium is a Proof-of-Work Layer 1 blockchain focused on GPU-first mining, transparent tokenomics, and open infrastructure. Built in Rust, it uses TensorHash v1 hashing, a UTXO transaction model, and a fixed supply of 33,000,000 TXM distributed entirely through mining on the relaunched mainnet v1 chain.

Key characteristics:
- GPU-first: mainnet v1 difficulty starts at 42 bits — requires a modern high-VRAM NVIDIA GPU to mine practically
- Open mining: no mining pool required — solo mining is fee-free at the protocol level
- Transparent supply: all emission is on-chain, no hidden minting
- Bridge: wTXM ERC-20 on Optimism — 2-of-3 multisig operated bridge, enables DEX liquidity
- Developer tools: JS SDK, Chrome wallet extension, block explorer, public RPC

---

## Links

| Resource | URL |
|---|---|
| Website | https://tensoriumlabs.com |
| Whitepaper | https://whitepaper.tensoriumlabs.com |
| Documentation | https://docs.tensoriumlabs.com |
| Block Explorer | https://explorer.tensoriumlabs.com |
| Source Code | https://github.com/tensorium-labs/tensorium-core |
| Mining Pool | https://pooltxm.tensoriumlabs.com |
| Bridge | https://bridge.tensoriumlabs.com |
| Chrome Wallet | https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest |
| Discord | https://discord.gg/KkgGSZKVZw |
| wTXM on OP Explorer | https://optimistic.etherscan.io/address/0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e |

For wallets, indexers, and listing forms that need one concise reference packet, use `CANONICAL_ASSET_METADATA.md`.

---

## Technical Details

| Field | Value |
|---|---|
| Mainnet chain ID | `tensorium-mainnet` |
| RPC endpoint | `https://rpc.tensoriumlabs.com` |
| P2P port | 33333 |
| Seed node | `seed.tensoriumlabs.com:33333` |
| Backup seed | same public seed policy; canonical entrypoint is `seed.tensoriumlabs.com:33333` |
| Node software | `tensorium-node` (Rust, open source) |
| Wallet | `txmwallet` CLI + Chrome extension |
| Mining software | `txmminer` (CPU diagnostic/dev), `tensorium-miner` (NVIDIA CUDA, practical mainnet mining) |

**wTXM Bridge (Optimism mainnet):**

| Field | Value |
|---|---|
| wTXM contract | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` |
| Bridge controller | `0x4b31C557AD64609B975610812273BF82F1475384` |
| Multisig (Safe) | `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9` (2-of-3 Gnosis Safe) |
| Bridge URL | https://bridge.tensoriumlabs.com |
| Max per tx | 10,000 TXM |
| Daily mint cap | 50,000 TXM |
| Custody address | `txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq` |

---

## Tokenomics Summary

```
Max supply:           33,000,000 TXM (hard cap — no inflation beyond this)
Founder allocation:            0 TXM (zero premine)
Mining allocation:    33,000,000 TXM (open PoW mining)

Block reward schedule (TensorHash v1, 42-bit launch difficulty):
  Era 1:  11.90279581 TXM/block  (blocks       0 – 1,051,200)
  Era 2:   5.95139790 TXM/block  (blocks 1,051,201 – 2,102,400)
  Era 3:   2.97569895 TXM/block  ...
  (halving every 1,051,200 blocks ≈ 2 years, 10 eras total)
```

---

## Contact

- **Project email:** dev@tensoriumlabs.com
- **Discord:** https://discord.gg/KkgGSZKVZw
- **GitHub:** https://github.com/tensorium-labs/tensorium-core

---

## CMC / CoinGecko Application Notes

**CoinMarketCap:**
- Submit at: https://coinmarketcap.com/request/
- Category: Proof of Work
- Platform: Optimism (for wTXM tracking)
- Tags: pow, gpu-mining, layer-1, utxo

**CoinGecko:**
- Submit at: https://www.coingecko.com/en/coins/new
- Category: Layer 1, Proof of Work
- Note: Use wTXM contract on Optimism for price tracking until native DEX exists

---

## Target Exchanges (Priority Order)

### Tier 0 — Price Tracking (no trading required)
| Exchange | Status | Notes |
|---|---|---|
| CoinGecko | TODO | Apply at coingecko.com/en/coins/new — needs pool/DEX pair |
| CoinMarketCap | TODO | Apply at coinmarketcap.com/request/ — needs Uniswap pool |

### Tier 1 — DEX (needs wTXM liquidity on Optimism)
| Exchange | Status | Notes |
|---|---|---|
| Uniswap V3 (Optimism) | PENDING | Create wTXM/WETH pool when first wTXM bridged |
| Velodrome (Optimism) | TODO | After Uniswap pool is live |

### Tier 2 — Accessible CEX
| Exchange | Status | Notes |
|---|---|---|
| MEXC Global | SENT 2026-06-02 | `https://support.mexc.com/hc/en-001/articles/360059604091` — new listing application |
| CoinEx | TODO | `https://www.coinex.com/token-listing` — focus on PoW chains |
| Gate.io | TODO | `https://www.gate.io/en/listing` — strong PoW support |
| XT.com | TODO | Accessible Tier 3, active PoW support |
| LBank | TODO | Tier 3, active listing program |

### Tier 3 — Future (after traction)
| Exchange | Status | Notes |
|---|---|---|
| KuCoin | Future | Needs trading volume + community size |
| Huobi / HTX | Future | Needs volume + docs |
| OKX | Future | Needs strong metrics |
| Binance | Long-term | Needs significant traction |
