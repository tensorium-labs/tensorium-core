# Mainnet Readiness

Status: **Phase 7 COMPLETE** — v0.3.1-mainnet-candidate released. MC daemon operational. Mainnet-candidate chain NOT yet publicly launched — pre-launch checklist (Phase 8) in progress.
Last updated: 2026-05-31

This document tracks what must be true before Tensorium mainnet-candidate chain launches publicly.

## Current Position

**Phase 7 DONE.** All Phase 7 sprints (7A–7E) completed 2026-05-31.

- Active public testnet: `tensorium-testnet-0`, diff 36 bits, release `v0.2.0-testnet`
- Mainnet-candidate code: `v0.3.1-mainnet-candidate` — genesis hardcoded, MC daemon complete
- MC genesis: nonce `56_167_663_277`, hash `0000000000d61e99b9e2530609632b399d0f0b538c2d54daa1dddbfe28ea08dc`
- MC commands: `tensorium-node mainnet-candidate rpc/p2p-listen/sync/init`
- Public services: website, docs, whitepaper, explorer, testnet seed node

**Mainnet-candidate chain launch requires completing Phase 8 (Pre-Launch) checklist below.**

## Blocking Gates

| Gate | Status | Notes |
| --- | --- | --- |
| Consensus audit | DONE | Tokenomics, emission, difficulty, fork-choice/reorg, timestamp, coinbase, double-spend, RPC bind safety, P2P cap, ban-list fix, connection limit, TCP timeouts — 54 unit tests passing. Soak/integration test: Phase 8 item. |
| Founder wallet | DONE | Founder address `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d`, pool treasury `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9` generated 2026-05-31. |
| Founder lock policy | DONE | Social/manual 24-month lock documented; no L1 enforcement. Disclosure required in whitepaper before mainnet. |
| Mainnet genesis | DONE | Nonce `56_167_663_277` mined RTX 5090 (2.28 GH/s, 24.6s, 2026-05-31). Hash: `0000000000d61e99b9e2530609632b399d0f0b538c2d54daa1dddbfe28ea08dc`. Hardcoded in binary. |
| Storage migration decision | DEFERRED | JSON state acceptable for mainnet-candidate. Binary/DB migration planned for future version. |
| Peer discovery | DONE | Built-in static seed list (`157.230.44.162:23333`) added to node binary; opt-out via `TENSORIUM_NO_DEFAULT_SEEDS=1`. DNS seed deferred to mainnet. |
| Mining pool path | DONE | tensorium-pool reference pool implemented (HTTP proxy, 5% fee, payout ledger). |
| Pool fee policy | DONE | Pool treasury address generated (`txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`); payout accounting implemented; `pooltxm.tensoriumlabs.com` discloses 5% fee before miners connect. |
| Node/pool role boundaries | DONE | Documented in this file; testnet colocates with isolation; mainnet-candidate scaling plan documented. |
| Monitoring | DONE | `/usr/local/bin/tensorium-monitor.sh` runs every 10 min via cron; checks RPC, P2P, explorer, disk, SSL expiry; logs to `/var/log/tensorium-monitor.log`. |
| Release reproducibility | DONE | v0.3.0-mainnet-candidate binaries built; SHA256 checksums in CHECKSUMS-v0.3.0-mainnet-candidate.txt. |
| Risk disclosure | DONE | RISK_DISCLOSURE.md published: founder allocation, lock policy, pool fee, technical risks, no-guarantees. |

## Consensus Checklist

- [x] Confirm max supply: 33,000,000 TXM.
- [x] Confirm founder allocation: 1,000,000 TXM.
- [x] Confirm mining allocation: 32,000,000 TXM.
- [x] Confirm initial reward: 15.23557865 TXM per block.
- [x] Confirm halving interval: 1,051,200 blocks.
- [x] Confirm max halving eras: 10.
- [x] Confirm coinbase maturity: 100 blocks.
- [x] Confirm mainnet candidate chain ID.
- [x] Confirm mainnet initial difficulty.
- [x] Confirm min/max difficulty bounds.
- [x] Confirm difficulty adjustment window.
- [x] Confirm max future timestamp tolerance.
- [x] Confirm max block size.
- [x] Add or review tests for reward sum and no over-minting.
- [x] Add or review tests for difficulty retarget clamp behavior.
- [x] Add or review tests for fork choice cumulative work.
- [x] Add or review tests for immature coinbase spend rejection.
- [x] Add or review tests for future timestamp rejection.
- [x] Add or review tests for wrong chain ID rejection.
- [x] Add or review tests for coinbase reward over-mint rejection.
- [x] Add or review tests for mempool pending double-spend rejection.

Phase 7A test update:

- Added tokenomics tests for testnet and mainnet candidate supply split.
- Added emission tests for 10-era mining allocation and rounding dust.
- Added mainnet candidate emission schedule comparison.
- Added difficulty tests for upward, downward, flat, and clamped retarget behavior.
- Verification on VPS: `cargo test --workspace` passed with 32 tests.

Phase 7A safety test update:

- Added block validation tests for wrong chain ID, future timestamp, and coinbase reward above schedule.
- Added fork-choice reorg test that keeps equal-work first-seen blocks but reorgs to the branch with higher cumulative work.
- Added mempool pending double-spend rejection so conflicting unconfirmed transactions do not accumulate in the pool.
- Reviewed existing wallet restore/sign/verify tests and immature coinbase rejection tests.
- Verification on VPS: `cargo test --workspace` passed with 37 tests.

Phase 7A node safety update:

- RPC now refuses non-loopback binds by default unless `TENSORIUM_RPC_ALLOW_PUBLIC=1` is explicitly set.
- Added tests for RPC bind guard.
- P2P newline message reads now have a 1 MiB cap to avoid unbounded memory growth from malformed peers.
- Ban list cleanup now prunes expired bans when recording new violations.
- CPU miner submit output now prints the accepted block hash, removing the remaining unused response field warning.
- Verification on VPS: `cargo test --workspace` passed with 39 total unit tests and no warnings.

Phase 7A extended hardening (2026-05-31):

- **Bug fix:** `prune_expired` used `map_or(false, …)` which wiped sub-threshold score entries before they could accumulate to the ban threshold. Fixed to `map_or(true, …)` so only expired bans are removed; score-only entries persist across calls.
- **P2P connection limit:** Added `MAX_INBOUND_PEERS = 64` with an `AtomicUsize` counter. New connections above the limit are refused immediately, preventing thread-exhaustion DoS.
- **TCP I/O timeouts:** P2P connections have a 30-second read/write timeout; RPC connections have a 10-second read timeout. A slow or dead peer/client no longer holds a thread indefinitely.
- **RPC rate-limit strategy documented:** RPC is single-threaded and loopback-only by default. Public RPC (`TENSORIUM_RPC_ALLOW_PUBLIC=1`) requires nginx with `limit_req` and `limit_conn` in front; this is documented in the source.
- **HTTP 400 status text fixed:** `write_json_response` now returns `Bad Request` for 400 codes.
- **6 new BanList unit tests:** sub-threshold persistence, threshold activation, instant-ban on bad handshake, expiry/prune behaviour, active-ban survival, and manual unban.
- VPS verified: `cargo fmt` OK; `cargo test --workspace` → **45 tests passed** (37 core + 8 node), 0 failed, no warnings.

## Founder Wallet Policy

Phase 7B completed (2026-05-31).

- [x] Generate founder wallet address.
- [x] Generate pool treasury wallet address if the official/reference pool charges fees.
- [x] Store founder private key outside public VPS infrastructure.
- [x] Publish founder address before genesis.
- [x] Publish founder allocation amount.
- [x] Publish lock/vesting policy.
- [x] Explain whether lock is protocol-enforced or policy/manual.
- [x] Publish pool fee policy and pool treasury address before opening an official pool. *(pool website disclosure)*

### Founder Cold Wallet

- Address: `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d`
- Allocation: `1,000,000 TXM` (genesis allocation, pre-mined at block 0)
- Wallet file: stored on local machine only (`/root/cold-wallets/founder/founder-cold.json`), encrypted with passphrase.
- Private key must NOT be copied to VPS seed node, explorer server, docs server, or CI.
- This address must appear in mainnet genesis block output.

### Pool Treasury Wallet

- Address: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`
- Purpose: receives 5% official/reference pool fee revenue.
- Wallet file: stored on local machine only (`/root/cold-wallets/pool-treasury/pool-treasury.json`), encrypted with passphrase.
- Separate from founder cold wallet — different keypair, different passphrase.
- Pool treasury address will be disclosed on the official pool page before miners connect.

### Lock and Vesting Policy

Lock type: **social/manual, not L1-enforced.**

Tensorium does not currently implement a native timelock or vesting contract at the consensus layer.

The founder lock policy is:

1. The founder address (`txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d`) will receive `1,000,000 TXM` in the mainnet genesis block.
2. The founder commits to a **24-month voluntary lock** starting from mainnet genesis: no more than 10% of the allocation (100,000 TXM) may be moved in any single calendar month for the first 24 months.
3. After month 24, the remaining balance is fully unlocked and moveable at founder discretion.
4. All movements from the founder address will be visible on-chain and on the public explorer.
5. This policy is social/reputational only — L1 consensus does not enforce it. Miners and community members must decide whether they accept this trust model.

This disclosure must appear in the whitepaper and risk disclosure before mainnet launch.

### Recommended default:

- Generate founder wallet offline or on a trusted local machine. ✓ done
- Do not store founder private key on the seed node, explorer server, docs server, or CI. ✓ documented
- If native lock is not implemented, disclose that the lock is social/manual, not enforced by L1 consensus. ✓ documented above

## Official Pool Fee Policy Draft

Draft decision:

- Official/reference pool fee: 5%.
- Fee destination: a new pool treasury or founder/development treasury wallet.
- Scope: pool-level payout accounting only.
- Not a protocol-level miner tax.
- Solo mining must remain fee-free at the protocol level.

Required safety rules:

- [ ] Publish the pool fee before miners connect. *(deferred to pool launch)*
- [x] Pool treasury address generated: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`
- [ ] Show gross reward, pool fee, and net miner payout in pool accounting.
- [x] Pool treasury private key is separate from founder cold wallet — different keypair.
- [ ] Do not hide the fee in miner code, payout scripts, or explorer output.
- [ ] Document that miners can avoid pool fee by solo mining.

Rationale:

- A transparent pool fee is easier to reason about than a hidden consensus tax.
- It does not change max supply or block reward rules.
- Miners can choose between solo mining and official pool mining.
- The fee can fund infrastructure, development, explorer, docs, and operations.

## Node and Pool Role Boundaries

Do not treat the full node and the mining pool as the same trust boundary.

This does not mean every service must run on a different VPS from day one. The first requirement is role isolation. Physical/VPS separation can happen gradually as the network grows.

The node is a consensus component:

- validates blocks and transactions,
- stores chain state,
- exposes RPC locally,
- connects to peers,
- must not hold pool treasury or payout private keys.

The pool is an operational component:

- accepts miner shares,
- tracks miner accounting,
- controls payout scheduling,
- charges the official 5% pool fee,
- may need a limited hot wallet.

Testnet rule:

- Node and pool may run on the same VPS for early testing to keep operations light.
- If colocated, they must use separate process names, folders, environment files, logs, and wallet files.
- RPC should stay on localhost.
- Pool hot wallet balance should be limited.

Scaling recommendation:

- [ ] Stage 1 testnet: one VPS can run node, pool, and explorer with isolated roles.
- [ ] Stage 2 public testnet: add at least one backup node.
- [ ] Stage 3 mainnet candidate: split high-risk services as traffic and funds increase.

Mainnet candidate recommendation:

- [ ] Seed node: `tensorium-node`, no private payout keys.
- [ ] Backup seed node: independent node for redundancy.
- [ ] Pool service: pool API/stratum, share accounting, payout scheduler.
- [ ] Explorer service: indexer and web UI with read-only RPC access.
- [ ] Cold storage: founder wallet and treasury reserve.

Wallet separation:

- [ ] Founder cold wallet is separate from pool treasury.
- [ ] Pool treasury wallet receives fee revenue and has a published address.
- [ ] Pool payout hot wallet is limited and operational only.
- [ ] Explorer/docs infrastructure has no private keys.

## Infrastructure Checklist

Phase 7C update (2026-05-31):

- [ ] Mainnet seed node prepared separately from testnet. *(deferred — requires new VPS decision)*
- [ ] Backup seed node prepared. *(deferred — to be added as traffic grows)*
- [x] Node, pool, explorer, and treasury roles isolated or explicitly documented for testnet.
- [x] Backup node plan documented. *(Stage 1 testnet single VPS acceptable; Stage 2 adds backup node)*
- [x] RPC bound to localhost only. *(127.0.0.1:23332, enforced by default)*
- [x] P2P public port documented. *(0.0.0.0:23333, UFW allows 23333)*
- [x] Firewall allowlist documented. *(UFW: SSH/22, HTTP/80, HTTPS/443, P2P/23333)*
- [x] Log rotation configured. *(journald: max 500M / 50M per file / 30 days; explorer logrotate: 14 days)*
- [x] Chain state backup plan documented. *(daily cron 03:00 UTC → /root/backups/, 14 rolling backups)*
- [x] Explorer deployed for mainnet candidate. *(explorer.tensoriumlabs.com, pm2, nginx, SSL)*
- [x] Docs and whitepaper updated for mainnet candidate. *(docs.tensoriumlabs.com, whitepaper.tensoriumlabs.com)*
- [x] SSL renewal verified. *(certbot auto-renew active; monitor shows 89 days remaining)*
- [x] External monitoring configured. *(tensorium-monitor.sh every 10 min; logs /var/log/tensorium-monitor.log)*

### Peer Discovery

- [x] Built-in static seed list: `DEFAULT_SEEDS = ["157.230.44.162:23333"]` in `tensorium-node`.
- [x] New nodes connect without manual configuration; seed falls back automatically.
- [x] Seed node itself runs with `TENSORIUM_NO_DEFAULT_SEEDS=1` to avoid self-connection.
- [ ] DNS seed (`seed.tensoriumlabs.com` → seed IP) deferred to mainnet candidate stage.

### Backup and Monitoring

- Backup: `/usr/local/bin/tensorium-backup.sh` — tarballs `state.json`, `mempool.json`, `banlist.json`; cron `0 3 * * *`; keeps 14 rolling backups under `/root/backups/`.
- Monitor: `/usr/local/bin/tensorium-monitor.sh` — checks RPC health, P2P port, explorer, disk %, SSL expiry; cron `*/10 * * * *`; logs to `/var/log/tensorium-monitor.log`.

## Mining Checklist

Phase 7D update (2026-05-31):

- [x] CUDA miner tested from release binary. *(v0.2.0-testnet, RTX 3060 mined 5 blocks at diff 36)*
- [x] CUDA miner tested from source build. *(sm86, compiled and tested Phase 6)*
- [x] RTX 3000/4000 benchmark published. *(RTX 3060 ~410 MH/s, avg block time ~167s at diff 36)*
- [ ] At least one high-end GPU benchmark published. *(RTX 4090 tested via Vast AI; formal publish deferred)*
- [x] Multi-GPU behavior tested or explicitly deferred. *(deferred: txmminer-cuda is single-GPU per process; multi-GPU via multiple processes documented)*
- [x] Pool mining path decided. *(reference pool: tensorium-pool crate, HTTP proxy model)*
- [x] Pool payout accounting supports 5% official pool fee. *(split_fee() in accounting.rs, 9 unit tests)*
- [x] Pool fee disclosure added to docs/UI. *(`pooltxm.tensoriumlabs.com` shows 5% fee, treasury address, gross/net payout)*
- [x] Solo mining guide updated. *(README and docs.tensoriumlabs.com cover solo mining)*

### Pool: tensorium-pool Reference Implementation

Pool binary: `tensorium-pool` (new crate in workspace, commit 2ed0104).

Architecture:

- Miners point `txmminer` / `txmminer-cuda` at the pool bind address instead of the node RPC.
- Pool proxies `GET /getblocktemplate/<miner_addr>` → node using **pool treasury address** as coinbase recipient.
- Pool proxies `POST /submitblock` → node; on acceptance records payout accounting.
- Payout ledger: `pool-ledger.json` (JSON, appended per accepted block).

Fee model:

- `POOL_FEE_BPS = 500` (5.00 %).
- `split_fee(gross, 500)` → `(net = gross × 0.95, fee = gross × 0.05)`, fee rounds down.
- Gross reward credited to pool treasury on-chain; pool operator owes `net` to miner.

Pool endpoints:

| Endpoint | Purpose |
|---|---|
| `GET /health` | liveness check |
| `GET /getblocktemplate/<miner>` | work distribution (coinbase → treasury) |
| `POST /submitblock` | block submission + accounting |
| `GET /pool/stats` | blocks found, fees collected, pending net |
| `GET /pool/accounting` | full payout ledger |
| `GET /pool/pending/<addr>` | per-miner pending payout |

Required env vars:

```
TENSORIUM_POOL_TREASURY=<pool_treasury_address>   # required
TENSORIUM_NODE_RPC=127.0.0.1:23332                 # default
TENSORIUM_POOL_BIND=0.0.0.0:23336                  # default
TENSORIUM_POOL_LEDGER=pool-ledger.json             # default
```

Pool treasury address: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`

Payout flow (operator responsibility):

1. Block found → gross reward on-chain to treasury, ledger entry created.
2. Operator reviews `tensorium-pool accounting`.
3. Operator signs and broadcasts payment transaction from treasury wallet to miner address.
4. Operator runs `tensorium-pool mark-paid <miner_addr>`.

Solo mining (fee-free): miners point `txmminer` directly at `tensorium-node` RPC — no pool fee.

## Release Checklist

- [ ] Version tag chosen.
- [ ] Release notes written.
- [ ] Linux binaries built.
- [ ] CUDA miner binaries built for supported architectures.
- [ ] SHA256 checksums generated.
- [ ] Install script points to correct release.
- [ ] Upgrade instructions written.
- [ ] Rollback or emergency communication plan written.

## Current Decision

Tensorium v0.3.0-mainnet-candidate is released. Phase 7 (7A–7E) is complete.

## Phase 8 — Pre-Launch Checklist

Phase 7 is complete. Phase 8 covers everything required before the mainnet-candidate chain goes live and TXM has real economic value.

### 8A — Infrastructure

| Item | Status | Notes |
|---|---|---|
| MC RPC/P2P daemon | DONE | `mainnet-candidate rpc/p2p-listen/sync` operational (commit 9286304) |
| Mainnet-candidate seed VPS | TODO | Current testnet VPS (157.230.44.162) must NOT be reused as mainnet seed. Need dedicated VPS: minimum 4 vCPU, 8 GB RAM, 100 GB NVMe SSD. |
| MC seed node deployed | TODO | Deploy `tensorium-node mainnet-candidate init` + `rpc` + `p2p-listen` on new VPS |
| DNS seed | TODO | `seed.tensoriumlabs.com` A record → MC seed VPS IP; implement DNS seed support in node or document static seed list update |
| MC P2P sync test | TODO | Run 2+ independent MC nodes and verify they sync to same chain tip |
| Backup seed node | TODO | At least one additional MC seed node (different provider/region) |
| Firewall + SSL on MC VPS | TODO | UFW: SSH, 80, 443, P2P 33333; certbot for any web endpoint on new VPS |
| Monitor for MC node | TODO | Extend `tensorium-monitor.sh` to also check MC RPC (127.0.0.1:33332) and P2P (33333) |

### 8B — Wallet & UX

| Item | Status | Notes |
|---|---|---|
| CLI wallet works on MC chain | DONE | `txmwallet` unchanged; works with any address format |
| Chrome extension wallet | TODO | Browser wallet — TypeScript+React, `chrome.storage.local`, connect to node RPC, network selector |
| Mobile wallet | DEFERRED | iOS/Android — post-launch |
| Web wallet | DEFERRED | In-browser without extension — post-launch |

Chrome extension wallet stack: TypeScript + React, separate repo `tensorium-wallet-extension`, store encrypted key in `chrome.storage.local`, reuse secp256k1+SHA256d from txmwallet (port to JS or WASM).

### 8C — Pool Website & Faucet

| Item | Status | Notes |
|---|---|---|
| Pool website | DONE | `https://pooltxm.tensoriumlabs.com` deployed on testnet VPS — Next.js + TypeScript frontend for `tensorium-pool`: stats, miner lookup, payout history, connect guide |
| Pool fee disclosure | DONE | Shows 5% fee, treasury address, gross reward, pool fee, and net payout before miners connect |
| Testnet faucet | TODO | `faucet.tensoriumlabs.com` — 1x per address per 24h, send small testnet TXM for testing |

### 8D — Docs & Community

| Item | Status | Notes |
|---|---|---|
| Whitepaper update | DONE | Added pool fee, founder lock, MC genesis, Phase 8 roadmap |
| Docs: pool guide | DONE | Added official pool endpoint, fee disclosure, miner commands, payout lookup |
| Docs: MC node guide | DONE | Added `mainnet-candidate rpc/p2p-listen/sync` commands and MC genesis metadata |
| Project identity email | DONE | `dev@tensoriumlabs.com` mailbox created on VPS with Postfix/Dovecot TLS; DNS MX/SPF/DMARC verified publicly |
| DKIM email signing | DONE | OpenDKIM installed, Postfix signing verified locally, and public DNS selector `txm20260531` verified with `opendkim-testkey` |
| GitHub organization identity | TODO | Create Tensorium-owned GitHub org/account before license/final public launch; migrate links from `rygroup-dev` after ready |
| Docs: Chrome extension guide | TODO | After wallet extension built |
| Risk disclosure on website | DONE | Root site and docs link to `RISK_DISCLOSURE.md` |
| Announce mainnet-candidate launch | TODO | LAST STEP only: after 8A infrastructure, 8E license/security, soak test, monitoring, and final checks pass |

### 8E — Security & Legal

| Item | Status | Notes |
|---|---|---|
| Source code license | TODO | Currently UNLICENSED. Choose MIT or Apache 2.0 before launch. |
| Soak test (2+ weeks MC chain) | TODO | Run MC chain with 2+ nodes for 2+ weeks before public announcement. |
| Security audit | DEFERRED | External audit recommended before economic value. Can defer to post-launch. |

---

## Phase 9 — Ecosystem (Post-Launch)

Starts after MC chain is live and stable (minimum 2 weeks soak test). Builds the ecosystem on top of the chain.

### 9A — DEX / Swap Platform

TXM needs a way to be bought and sold. Three options by complexity:

| Option | Complexity | Timeline |
|---|---|---|
| OTC/P2P trading board (`swap.tensoriumlabs.com`) | Low | 1-2 weeks |
| Bridge to BSC + PancakeSwap listing (wTXM) | High | 2-3 months |
| Native atomic swap (HTLC) | Very high | Requires Phase 10B |

**Recommended sequence:**
1. OTC trading board first (fast, P2P listing, no smart contracts)
2. Bridge to BSC → wTXM → PancakeSwap (wide exposure)
3. Native atomic swap after scripting layer (Phase 10)

### 9B — Explorer Improvements

| Feature | Priority |
|---|---|
| Address page (`/address/<addr>`) — balance + TX history | High |
| TX detail page (`/tx/<txid>`) — inputs, outputs, confirmations | High |
| Global search (block / txid / address) | High |
| Network stats (hashrate, supply minted, difficulty chart) | Medium |
| Mempool viewer | Medium |
| Public REST API (`/api/v1/address/<addr>`, `/api/v1/stats`) | Medium |

### 9C — SDK & Developer Tools

- `tensorium-sdk-js` — npm package: query balance, sign+broadcast TX, Node.js + browser
- `tensorium-sdk-py` — pip package: same as JS, for scripting/automation
- RPC API reference docs at `docs.tensoriumlabs.com/api`
- Example dApp using SDK

### 9D — Listing & Community

- CEX outreach (whitepaper, tokenomics, source code ready)
- Open Telegram to public (currently private invite)
- Discord server (mining support, dev, governance channels)
- Twitter/X: announce milestones, mining stats
- Mining event / competition for hashrate bootstrap

---

## Phase 10 — Advanced Protocol (Long-term)

- **Bridge EVM formal**: multi-sig relayer, audited smart contract, `bridge.tensoriumlabs.com`
- **Scripting layer**: OP codes (multisig, timelock, HTLC) → enables native atomic swap + DEX
- **Governance**: TIP process (Tensorium Improvement Proposal), on-chain or off-chain signaling
- **Storage migration**: JSON state → RocksDB/LMDB for scalability at millions of TXs
- **Mobile wallet**: iOS + Android (React Native or Flutter)

---

## Ecosystem Checklist (Full)

**Protocol — DONE:**
- [x] tensorium-node, txmwallet, txmminer, txmminer-cuda, tensorium-pool
- [x] Genesis block mined, MC params frozen, MC daemon complete

**Infrastructure — Phase 8:**
- [ ] MC seed VPS (new, dedicated)
- [ ] DNS seed
- [ ] MC P2P sync test
- [ ] Backup seed node
- [x] Block explorer, monitoring, backup

**Wallet & UX — Phase 8-9:**
- [ ] Chrome extension wallet
- [ ] Mobile wallet (Phase 10)

**Mining Ecosystem — Phase 8-9:**
- [x] Pool website (pooltxm.tensoriumlabs.com)
- [ ] Testnet faucet
- [x] Mining guide, pool reference implementation

**Trading & Liquidity — Phase 9:**
- [ ] OTC trading board
- [ ] Bridge to BSC + wTXM
- [ ] DEX listing (PancakeSwap)
- [ ] CEX listing

**Developer — Phase 9:**
- [ ] SDK JS/Python
- [ ] Public REST API docs
- [ ] Developer onboarding guide

**Advanced Protocol — Phase 10:**
- [ ] Scripting layer (OP codes, HTLC, atomic swap)
- [ ] Governance mechanism
- [ ] Storage migration (RocksDB)

**Community & Legal:**
- [ ] Open source license (MIT/Apache 2.0)
- [ ] Telegram public, Discord, Twitter/X
- [ ] Security audit external

---

### Mainnet-Candidate Genesis (Reference)

- **Nonce:** `56_167_663_277`
- **Hash:** `0000000000d61e99b9e2530609632b399d0f0b538c2d54daa1dddbfe28ea08dc`
- **Timestamp:** `1_780_272_000` (2026-06-01 00:00:00 UTC)
- **Mined:** RTX 5090, CUDA, 2.28 GH/s, 24.6 seconds (2026-05-31)
- **Verified:** GPU server (142.188.39.36) + VPS (157.230.44.162) → identical hash
- **Command:** `tensorium-node mainnet-candidate init` (no args needed)

### VPS Recommendation for Mainnet-Candidate Seed Node

Current testnet VPS (157.230.44.162) should stay as testnet-only. A dedicated MC seed node VPS should be:

| Spec | Minimum | Recommended |
|---|---|---|
| CPU | 2 cores | 4 cores |
| RAM | 4 GB | 8 GB |
| Disk | 50 GB SSD | 100 GB NVMe |
| Network | 100 Mbps | 1 Gbps |
| Provider | Any | Hetzner / DigitalOcean / Vultr |
| Cost | ~$10–15/mo | ~$20–48/mo |

Ports to open: SSH (22), HTTP (80), HTTPS (443), MC P2P (33333). RPC stays on localhost.

### Priority Order for Phase 8

1. **New VPS + MC node** (8A) — cannot launch without a stable seed node
2. **DNS seed** (8A) — enables auto-discovery for new nodes
3. **MC P2P sync test** (8A) — confirm chain works before public
4. **Docs update** (8C) — whitepaper + pool guide + MC node guide
5. **Chrome extension wallet** (8B) — high impact for community onboarding
6. **License** (8D) — must be decided before public launch
7. **Soak test** (8D) — 2+ weeks MC chain running before announcement
