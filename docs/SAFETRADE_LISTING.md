# SafeTrade Listing Package — Tensorium (TXM)

Submission материals for listing **native TXM** on SafeTrade (safetrade.com).

> SafeTrade does **not** list ERC-20 / former-ICO tokens, so we list the **native
> L1 coin (TXM)**, not wTXM. SafeTrade specializes in native PoW coins, which fits.

## Submission process
1. Suggest the coin via SafeTrade's coin suggestion form / voting portal
   (https://vote.safe.trade/ — read the requirements section there first).
2. Community voting: 1 vote = 1 SafeCoin; final listing at SafeTrade's discretion.
3. Provide the technical integration (node + wallet RPC) for deposits/withdrawals.

## Coin profile
| Field | Value |
|---|---|
| Name | Tensorium |
| Ticker | TXM |
| Type | Layer-1, Proof-of-Work, UTXO |
| Algorithm | SHA-256d (double SHA-256), GPU-first |
| Max supply | 33,000,000 TXM |
| Emission | 8M pre-mint + 25M mining over 10 halving eras |
| Target block time | 60 s |
| Decimals | 8 (1 TXM = 100,000,000 atoms) |
| Status | **Mainnet live** (chain_id `tensorium-mainnet-candidate-0`) |
| Address format | bech32, prefix `txm1…` |

## Links
- Website: https://tensoriumlabs.com
- Explorer: https://explorer.tensoriumlabs.com
- Docs: https://docs.tensoriumlabs.com
- Whitepaper: https://whitepaper.tensoriumlabs.com
- GitHub (core): https://github.com/tensorium-labs/tensorium-core
- Releases (node/wallet/miner): https://github.com/tensorium-labs/tensorium-core/releases
- Discord: https://discord.gg/KkgGSZKVZw
- Public RPC: https://mc-rpc.tensoriumlabs.com
- Logo: (attach TXM coin icon — see wallet-extension/icons)

## Network / integration details
- Node binary: `tensorium-node` (Linux x86_64), RPC `33332`, P2P `33333`.
- Wallet: `txmwallet` CLI (create / balance / send / broadcast; bech32 addresses;
  Argon2id + XChaCha20-Poly1305 encrypted wallet files).
- Node RPC is a **JSON-over-HTTP** API (`/getblockcount`, `/getblock/<h>`,
  `/getutxos/<addr|spk>`, `/sendrawtransaction`, `/getbalance` via UTXO scan),
  **not** Bitcoin-core JSON-RPC.

### ⚠️ Integration gap to resolve before listing
Most exchanges automate deposits/withdrawals via a Bitcoin-core-style RPC
(`getnewaddress`, `getbalance`, `sendtoaddress`, `listtransactions`, `validateaddress`).
TXM does not expose these natively. To list, we should provide a small
**Bitcoin-RPC-compatible adapter** that wraps `tensorium-node` + `txmwallet`
(HD/derived deposit addresses, balance, send, tx listing). This is the main
engineering task for any CEX integration and is reusable across exchanges.

## Supply transparency (for the application)
- Founder allocation: 1,000,000 TXM (intended 5-year lock — in progress).
- Ecosystem: 2,000,000 TXM. Liquidity: held for DEX pools (OP now; Arbitrum & Base later).
- Bridge custody backs wTXM 1:1 on Optimism (separate from CEX-listed native TXM).
- Live circulating/mined supply: query `https://explorer.tensoriumlabs.com`.
