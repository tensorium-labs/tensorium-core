# CEX Outreach Email Templates

Send from: dev@tensoriumlabs.com
Last updated: 2026-06-02

---

## Template A — General CEX (MEXC, CoinEx, Gate.io, XT.com)

**Subject:** Listing Application — Tensorium (TXM) | GPU PoW L1 | Open Source | Apache-2.0

---

Dear [Exchange] Listing Team,

I am writing to apply for listing Tensorium (TXM) on [Exchange].

**Project Summary**

Tensorium is a GPU-first Proof-of-Work Layer 1 blockchain — SHA256d consensus, UTXO model, 33M TXM hard cap. Mainnet launched on 2026-06-02. The project is fully open source (Apache-2.0) with no pre-sale and no VC allocation.

**Token Details**

| Field | Value |
|---|---|
| Name | Tensorium |
| Ticker | TXM |
| Type | Native L1 PoW (wTXM ERC-20 on Optimism for exchange listing) |
| wTXM Contract | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` (Optimism, chainId 10) |
| Max supply | 33,000,000 TXM |
| Circulating | ~1M TXM founder + mined supply (growing) |
| Decimals | 18 |
| Founder lock | Voluntary 24-month, max 10%/month |

**Key Differentiators**

- Pure GPU PoW mining (no ASIC advantage currently)
- No pre-sale, no VC, no hidden allocation
- Working bridge to Optimism (wTXM ERC-20 for DEX/CEX)
- Chrome wallet extension + CLI wallet + JS SDK
- 2-of-3 multisig bridge governance (Gnosis Safe)
- Full source code on GitHub: https://github.com/tensorium-labs/tensorium-core

**Ecosystem**

- Explorer: https://explorer.tensoriumlabs.com
- Bridge: https://bridge.tensoriumlabs.com
- Pool: https://pooltxm.tensoriumlabs.com
- Docs: https://docs.tensoriumlabs.com
- Whitepaper: https://whitepaper.tensoriumlabs.com
- Discord: https://discord.gg/KkgGSZKVZw

**Listing Contact**

Email: dev@tensoriumlabs.com
Website: https://tensoriumlabs.com

Full listing package is attached. Please let me know if you need additional information.

Best regards,
Tensorium Labs
dev@tensoriumlabs.com

---

## Template B — CoinGecko Application

**Submission URL:** https://www.coingecko.com/en/coins/new

**Fields to fill:**

- Coin name: Tensorium
- Ticker: wTXM (for Optimism tracking) or TXM
- Platform: Optimism
- Contract: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`
- Website: https://tensoriumlabs.com
- Description: GPU-first Proof-of-Work Layer 1 blockchain. SHA256d, UTXO model, 33M TXM hard cap, open mining, no pre-sale.
- GitHub: https://github.com/tensorium-labs/tensorium-core
- Categories: Layer 1, Proof of Work, GPU Mining
- Discord: https://discord.gg/KkgGSZKVZw
- Explorer: https://optimistic.etherscan.io/address/0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e

**Note:** CoinGecko requires active trading pairs. Submit after first Uniswap pool is live.

---

## Template C — CoinMarketCap Application

**Submission URL:** https://coinmarketcap.com/request/

**Fields:**

- Project name: Tensorium
- Ticker: TXM / wTXM
- Platform: Optimism (chainId 10)
- Contract: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`
- Max supply: 33000000
- Total supply: 33000000
- Circulating supply: (update at time of submission)
- Website: https://tensoriumlabs.com
- Source code: https://github.com/tensorium-labs/tensorium-core
- Whitepaper: https://whitepaper.tensoriumlabs.com
- Explorer: https://explorer.tensoriumlabs.com
- Tags: proof-of-work, gpu-mining, layer-1, utxo, open-source

**Note:** CMC requires active Uniswap or CEX trading pair. Submit after pool is live.

---

## Outreach Priority & Status

| Exchange | Type | Apply URL | Status | Min Requirements |
|---|---|---|---|---|
| CoinGecko | Price tracker | coingecko.com/en/coins/new | TODO | Active DEX pair |
| CoinMarketCap | Price tracker | coinmarketcap.com/request/ | TODO | Active DEX pair |
| MEXC Global | CEX Tier 2 | support.mexc.com listing form | TODO | Whitepaper + community |
| CoinEx | CEX Tier 2 | coinex.com/token-listing | TODO | PoW focus — good fit |
| Gate.io | CEX Tier 2 | gate.io/en/listing | TODO | Strong PoW support |
| Uniswap V3 OP | DEX | app.uniswap.org/pools/new | PENDING | First wTXM bridged |

---

## Checklist Before Sending

- [ ] Uniswap V3 pool live (needed for CMC/CoinGecko and most CEXes)
- [ ] At least 100 Discord members
- [ ] Attach: whitepaper PDF + tokenomics summary + risk disclosure
- [ ] Attach: audit report (if available — currently none)
- [ ] Include: GitHub link with recent commit activity
- [ ] Include: bridge contract verification on Optimism Etherscan
