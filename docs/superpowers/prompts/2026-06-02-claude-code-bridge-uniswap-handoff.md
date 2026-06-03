# Tensorium — Bridge & Uniswap Handoff Prompt

Date: 2026-06-02. Resume at next session from this exact state.

Read these files first:
1. `README.md`
2. `MAINNET_READINESS.md`
3. `CHANGELOG.md`
4. `/root/uniswap-pool-instructions.md`

Then check `git log --oneline -5` and `curl -s http://127.0.0.1:33332/getblockcount` on VPS before touching anything.

---

## Verified Current State

**Chain:** `tensorium-mainnet-candidate-0` live, height ~193 at session end.

**Tokenomics v2 (2026-06-02):**
- Genesis nonce: `798_243_452_272`
- Hash: `0000000000007076b8daa7e605fcbdbeec5ad8f4dcedbfec762ae47a19ae18431b`
- Pre-mint (8M): founder 1M + liquidity 3M + bridge 2M + ecosystem 2M
- Mining (25M): 11.9027 TXM/block, 10 eras, ~20 years

**GPU miner:** RTX 5090 on Vast.ai `64.31.38.214:2602`, running at ~7.5 GH/s (~2.4 min/block).
- tmux session `txmminer` — mining to `txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck`
- tmux session `tunnel` — SSH tunnel to VPS port 33332

**VPS `157.230.44.162`:** all services active (tensorium-mc-rpc, tensorium-mc-p2p, tensorium-pool, txm-discord-bot, bridge-relayer, explorer, faucet, pool-website).

**Code state (tensorium-core main):**
- S1 (P2PKH scripting): DONE
- S2 (bare multisig): DONE (spec + plan + implementation, 83 tests pass)
- All 9 GitHub repos cleaned of testnet references

---

## The ONE Thing Blocking Uniswap Pool

**Bridge deposit is stuck at the Safe approval step.**

What happened:
1. 500,000 TXM was sent from liquidity wallet to bridge custody ✓
2. TXM txid: `ad2d81543ca0dce77e7c9f30eec2a7337c5cb7986b2da1e0150357a81cd1568d`
3. 6+ confirmations reached ✓
4. Bridge relayer tried to call `mintWTXM` on the controller ✗
5. **Failed because:** `TensoriumBridgeController` is owned by Gnosis Safe (2-of-3), not the operator EOA.

**What the relayer tried to call (on OP Mainnet):**
```
Contract: 0x4b31C557AD64609B975610812273BF82F1475384 (TensoriumBridgeController)
Function: mintWTXM
txid:      0xad2d81543ca0dce77e7c9f30eec2a7337c5cb7986b2da1e0150357a81cd1568d
recipient: 0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB
amount:    500000000000000000000000  (500K × 10^18)
```

**Error:** `execution reverted` — caller is not the Safe owner.

---

## Immediate Next Steps (in order)

### Step 1 — Mint wTXM via Gnosis Safe (FIRST PRIORITY)

Open Safe UI:
`https://app.safe.global/OP:0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9/transactions/queue`

Create New Transaction → Contract Interaction:
- Contract address: `0x4b31C557AD64609B975610812273BF82F1475384`
- Load ABI or use raw calldata
- Function: `mintWTXM(bytes32 txid, address recipient, uint256 amount)`
- txid: `0xad2d81543ca0dce77e7c9f30eec2a7337c5cb7986b2da1e0150357a81cd1568d`
- recipient: `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB`
- amount: `500000000000000000000000`

Needs 2 of 3 Safe signers. After minting, verify at:
`https://optimistic.etherscan.io/address/0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB`

Alternatively, **fix the relayer long-term** by calling `setRelayer(relayerAddress)` via Safe (if this function exists on the controller), then the relayer can mint autonomously.

### Step 2 — Create Uniswap V3 Pool

Full instructions: `/root/uniswap-pool-instructions.md`

Summary:
- URL: `https://app.uniswap.org/pools/new?chain=optimism`
- Token: wTXM `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`
- Pair: wTXM/ETH
- Fee: 0.3%
- Initial price: **$0.005/TXM** → at ETH $3000 = **600,000 wTXM per ETH**
- Amount: **500,000 wTXM** + ~0.833 ETH (~$5,000 total)
- Range: Full range (0 to ∞)

### Step 3 — CMC + CoinGecko Submission

After pool has at least 1 trade, submit using data from `docs/integrations/CANONICAL_ASSET_METADATA.md`:
- CoinGecko: `https://www.coingecko.com/en/coins/add`
- CMC: `https://coinmarketcap.com/request/`

---

## Wallet Inventory (all at height 193+, all mature)

| Wallet | Address | Balance | File |
|--------|---------|---------|------|
| Founder | `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d` | 1,000,000 TXM | `/root/cold-wallets/founder/founder-cold.json` |
| Liquidity | `txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p` | ~2,500,000 TXM (after 500K bridged) | `/root/cold-wallets/liquidity/liquidity-cold.json` |
| Bridge reserve | `txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq` | 2,000,000 TXM + 500K deposit | VPS bridge relayer custody |
| Ecosystem | `txm1jwz2nvfajy84kyypzxp0pq8n5vrwahu6yny9hf` | 2,000,000 TXM | `/root/cold-wallets/ecosystem/ecosystem-cold.json` |
| Miner rewards | `txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck` | Growing (mining ongoing) | GPU rental wallet |

Liquidity lock policy (in MAINNET_READINESS.md):
- 500K → Uniswap initial pool (immediate)
- 1M → 2-year voluntary lock
- 1.5M → 5-year voluntary lock

---

## Infrastructure Reference

| Service | Host | Details |
|---------|------|---------|
| VPS | `157.230.44.162` | DigitalOcean, password `[REDACTED]` |
| Mainnet RPC | `https://mc-rpc.tensoriumlabs.com` | nginx → 127.0.0.1:33332 |
| GPU rental | `ssh -p 2602 root@64.31.38.214` | Vast.ai RTX 5090, key auth |
| OP Safe | `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9` | 2-of-3, OP Mainnet |
| wTXM contract | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` | OP Mainnet |
| Controller | `0x4b31C557AD64609B975610812273BF82F1475384` | OP Mainnet, owned by Safe |
| Bridge relayer | pm2 `tensorium-bridge-relayer` on VPS | `/root/tensorium-bridge-relayer/` |
| Bridge API key | stored in `/root/tensorium-bridge-relayer/.env` on VPS | Internal use only |

---

## Security Actions Required (DO THIS BEFORE ANYTHING ELSE)

1. **Revoke GitHub token** — the token used this session was exposed in chat multiple times. Go to GitHub → Settings → Developer settings → Personal access tokens → Revoke all active tokens from this session. Generate a new one.

2. **Rotate OP deployer private key** — the deployer key for `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB` was exposed in chat. Transfer any ETH off it to a fresh wallet and rotate before using it for Safe transactions or contract calls.

3. **Check bridge custody passphrase** — `bridge-custody-phase9a` appeared in `.env.example` as a possible real passphrase. Check the actual VPS `.env` at `/root/tensorium-bridge-relayer/.env` and change if it matches.

---

## Longer-Term Queue

After bridge + Uniswap are done:
- **S3 scripting layer** — OP_CHECKLOCKTIMEVERIFY (timelock, HTLC) → enables native atomic swap
- **Uniswap V3 pool** → submit to CMC + CoinGecko
- **Bridge relayer fix** — add `setRelayer()` to controller via Safe so minting is automatic
- **CEX follow-up** — check `dev@tensoriumlabs.com` for responses (14 exchanges contacted)
- **Chrome Web Store** — upload v0.1.1 after v0.1.0 review approved
- **Lock disclosures** — publish 2yr/5yr liquidity lock formally in whitepaper + risk disclosure
