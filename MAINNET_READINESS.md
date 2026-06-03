# Mainnet Readiness

Status: **MAINNET LIVE — 2026-06-02** · Chain running · Bridge live · SDK published · CEX outreach sent
Last updated: 2026-06-02

This document records what had to be true before launch and tracks the remaining post-launch ecosystem and operations follow-through.

## Current Position

**Phase 7 DONE.** All Phase 7 sprints (7A–7E) completed 2026-05-31.

- Mainnet-candidate code: `v0.3.1-mainnet-candidate` — genesis hardcoded, MC daemon complete
- MC genesis: nonce `1_936_263_118_035`, hash `0000000000269b71601aded6dda2991df6f88b67ac2bef13dff56f4f8a94dfae` (v3 — post-S1 script_pubkey serialisation)
- MC commands: `tensorium-node mainnet-candidate rpc/p2p-listen/sync/init`
- Public services: website, docs, whitepaper, explorer, and mainnet seed infrastructure

**Mainnet launched 2026-06-02. All Phase 8 gates passed. Phase 9A bridge live. Phase 9B/9C/9D done. CEX outreach sent.**

## Active Snapshot

Use this section first. The rest of the document preserves launch auditability and roadmap context.

- Chain status: mainnet live
- Public mining posture: GPU-first on mainnet
- Bridge status: live on Optimism with `wTXM`
- Explorer status: incremental indexer live with persisted snapshot reload
- Storage status: RocksDB migration complete
- Operations status: Phase 10A-10E complete on 2026-06-02
- Current follow-through: listings, liquidity, API docs, and longer-tail ecosystem work

## Post-Launch Execution Order

Immediate next phases are tracked in:

- `docs/superpowers/plans/2026-06-02-post-launch-mainnet-phases.md`
- `docs/operations/RESTORE_RUNBOOK.md` (Phase 10A artifact)

Recommended order:

1. Phase 10A — Recovery & Restore
2. Phase 10B — Explorer Durability
3. Phase 10C — Pool Custody & Payout Separation
4. Phase 10D — Public RPC & Ops Hardening
5. Phase 10E — Data Provider & Listing Readiness

Phase 10 status:

- **COMPLETE** on 2026-06-02
- Handoff prompt for next implementation pass: `docs/superpowers/prompts/2026-06-02-claude-code-phase11-handoff.md`

Phase 10D artifacts now live in:

- `docs/operations/PUBLIC_RPC_HARDENING_RUNBOOK.md`
- `docs/operations/PUBLIC_RPC_POSTURE.md`
- `templates/nginx-public-rpc.conf`

Phase 10E artifact:

- `docs/integrations/CANONICAL_ASSET_METADATA.md`

## Blocking Gates

| Gate | Status | Notes |
| --- | --- | --- |
| Consensus audit | DONE | Tokenomics, emission, difficulty, fork-choice/reorg, timestamp, coinbase, double-spend, RPC bind safety, P2P cap, ban-list fix, connection limit, TCP timeouts — 54 unit tests passing. Soak/integration test: Phase 8 item. |
| Founder wallet | DONE | Founder address `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d`, pool treasury `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9` generated 2026-05-31. |
| Founder lock policy | DONE | Social/manual 24-month lock documented; no L1 enforcement. Disclosure required in whitepaper before mainnet. |
| Mainnet genesis | DONE | Nonce `1_936_263_118_035` mined RTX 5090 (4.64 GH/s, 474s, 2026-06-02). Hash: `0000000000269b71601aded6dda2991df6f88b67ac2bef13dff56f4f8a94dfae`. Genesis v3 — post-S1 script_pubkey serialisation. Hardcoded in binary. |
| Storage migration decision | DONE | RocksDB migration shipped on 2026-06-02. Legacy `state.json` files now auto-migrate to `*.db/` on first open. |
| Peer discovery | DONE | Mainnet DNS seed live at `seed.tensoriumlabs.com:33333`; generic runtime defaults now point to mainnet DNS seeds. |
| Mining pool path | DONE | tensorium-pool reference pool implemented (HTTP proxy, 5% fee, payout ledger). |
| Pool fee policy | DONE | Pool treasury address generated (`txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`); payout accounting implemented; `pooltxm.tensoriumlabs.com` discloses 5% fee before miners connect. |
| Node/pool role boundaries | DONE | Documented in this file; mainnet scaling plan and separation posture are documented. |
| Monitoring | DONE | `/usr/local/bin/tensorium-monitor.sh` runs every 10 min via cron; checks RPC, P2P, explorer, disk, SSL expiry; logs to `/var/log/tensorium-monitor.log`. |
| Release reproducibility | DONE | v0.3.0-mainnet-candidate binaries built; SHA256 checksums in CHECKSUMS-v0.3.0-mainnet-candidate.txt. |
| Risk disclosure | DONE | `docs/project/RISK_DISCLOSURE.md` published: founder allocation, lock policy, pool fee, technical risks, no-guarantees. |

## Consensus Checklist

- [x] Confirm max supply: 33,000,000 TXM total (8,000,000 pre-mint + 25,000,000 mining).
- [x] Confirm pre-mint allocation: 8,000,000 TXM (founder + liquidity + ecosystem reserves).
- [x] Confirm mining allocation: 25,000,000 TXM.
- [x] Confirm initial reward: 11.9027 TXM per block (1,190,279,581 atoms).
- [x] Confirm halving interval: 1,051,200 blocks.
- [x] Confirm max halving eras: 10.
- [x] Confirm mainnet-candidate coinbase maturity: 100 blocks.
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

- Added tokenomics tests for chain supply split and founder allocation safety.
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

## Liquidity Wallet Lock Policy (2026-06-02)

The 3,000,000 TXM liquidity pre-mint is managed in three tranches:

| Tranche | Amount | Purpose | Lock |
|---------|--------|---------|------|
| Initial pool | 500,000 TXM | Uniswap V3 wTXM/ETH seed at $0.005/TXM | None — deployed at launch |
| Reserve A | 1,000,000 TXM | Liquidity reserve | **2-year voluntary social lock** |
| Reserve B | 1,500,000 TXM | Long-term protocol liquidity | **5-year voluntary social lock** |

Lock type: **social/manual, not L1-enforced.** Same model as the founder lock.
L1 timelock (OP_CLTV) is planned for Scripting Layer S3.

All movements from the liquidity wallet are visible on-chain at:
`txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p`

## Official Pool Fee Policy Draft

Draft decision:

- Official/reference pool fee: 5%.
- Fee destination: a new pool treasury or founder/development treasury wallet.
- Scope: pool-level payout accounting only.
- Not a protocol-level miner tax.
- Solo mining must remain fee-free at the protocol level.

Required safety rules:

- [x] Publish the pool fee before miners connect. *(live on `pooltxm.tensoriumlabs.com` before miner connect flow)*
- [x] Pool treasury address generated: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`
- [x] Show gross reward, pool fee, and net miner payout in pool accounting.
- [x] Pool treasury private key is separate from founder cold wallet — different keypair.
- [x] Do not hide the fee in miner code, payout scripts, or explorer output.
- [x] Document that miners can avoid pool fee by solo mining.

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

- [x] Early single-host phase: one VPS can run node, pool, and explorer with isolated roles.
- [x] Backup node phase: add at least one independent backup/seed node.
- [ ] Mature mainnet phase: split high-risk services further as traffic and funds increase.

Mainnet candidate recommendation:

- [x] Seed node: `tensorium-node`, no private payout keys.
- [x] Backup seed node: independent node for redundancy.
- [x] Pool service: pool API/stratum, share accounting, payout scheduler.
- [x] Explorer service: indexer and web UI with read-only RPC access.
- [ ] Cold storage: founder wallet and treasury reserve.

Wallet separation:

- [x] Founder cold wallet is separate from pool treasury.
- [x] Pool treasury wallet receives fee revenue and has a published address.
- [x] Pool payout hot wallet is limited and operational only. *(policy + runbook documented; runtime metadata exposed in pool service)*
- [ ] Explorer/docs infrastructure has no private keys.

## Infrastructure Checklist

Phase 7C update (2026-05-31):

- [x] Mainnet seed node prepared separately from earlier pre-launch assumptions. *(generic runtime now points to mainnet defaults; dedicated backup seed also live.)*
- [x] Backup seed node prepared. *(Vultr `txm-mc-seed-1`, `139.180.137.144`, deployed 2026-06-01 with MC RPC/P2P, sync, firewall, monitoring, and soak cron.)*
- [x] Node, pool, explorer, and treasury roles isolated or explicitly documented for mainnet operations.
- [x] Backup node plan documented. *(primary + secondary seed topology documented and deployed)*
- [x] RPC bound to localhost only. *(mainnet default `127.0.0.1:33332`, enforced by default)*
- [x] P2P public port documented. *(mainnet default `0.0.0.0:33333`, firewall allows `33333/tcp`)*
- [x] Firewall allowlist documented. *(UFW: SSH/22, HTTP/80, HTTPS/443, P2P/33333)*
- [x] Log rotation configured. *(journald: max 500M / 50M per file / 30 days; explorer logrotate: 14 days)*
- [x] Chain state backup plan documented. *(daily cron 03:00 UTC → /root/backups/, 14 rolling backups)*
- [x] Explorer deployed for mainnet candidate. *(explorer.tensoriumlabs.com, pm2, nginx, SSL)*
- [x] Docs and whitepaper updated for mainnet candidate. *(docs.tensoriumlabs.com, whitepaper.tensoriumlabs.com)*
- [x] SSL renewal verified. *(certbot auto-renew active; monitor shows 89 days remaining)*
- [x] External monitoring configured. *(tensorium-monitor.sh every 10 min; logs /var/log/tensorium-monitor.log)*
- [x] Public RPC fronting policy documented. *(localhost-only node RPC + nginx template + incident runbook committed in Phase 10D)*

### Peer Discovery

- [x] Built-in mainnet seed list: `DEFAULT_SEEDS = ["seed.tensoriumlabs.com:33333", "seed2.tensoriumlabs.com:33333"]` in `tensorium-node`.
- [x] New nodes connect without manual configuration; generic runtime now falls back to mainnet DNS seeds automatically.
- [x] Seed node itself runs with `TENSORIUM_NO_DEFAULT_SEEDS=1` to avoid self-connection.
- [x] DNS seed (`seed.tensoriumlabs.com` → seed IP) active for mainnet-candidate stage.

### Backup and Monitoring

- Backup: `/usr/local/bin/tensorium-backup.sh` — tarballs the chain state directory (`*.db/`) plus `mempool.json`, `banlist.json`, and any `*.json.migrated` rollback backup; cron `0 3 * * *`; keeps 14 rolling backups under `/root/backups/`.
- Monitor: `/usr/local/bin/tensorium-monitor.sh` — checks RPC health, P2P port, explorer, disk %, SSL expiry; cron `*/10 * * * *`; logs to `/var/log/tensorium-monitor.log`.

## Mining Checklist

Phase 7D update (2026-05-31):

- [x] CUDA miner tested from release binary. *(pre-launch GPU validation completed; current mainnet path uses the patched CUDA miner flow.)*
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

- Pool miners point `txmminer-cuda` at the pool bind address instead of the node RPC. (`txmminer` CPU cannot mine at mainnet difficulty and is dev/diagnostic only.)
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
TENSORIUM_NODE_RPC=127.0.0.1:33332                 # default
TENSORIUM_POOL_BIND=0.0.0.0:23336                  # default
TENSORIUM_POOL_LEDGER=pool-ledger.json             # default
```

Pool treasury address: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`

Payout flow (operator responsibility):

1. Block found → gross reward on-chain to treasury, ledger entry created.
2. Operator reviews `tensorium-pool accounting`.
3. Operator signs and broadcasts payment transaction from treasury wallet to miner address.
4. Operator runs `tensorium-pool mark-paid <miner_addr>`.

Solo mining (fee-free): solo miners point `txmminer-cuda` directly at their own `tensorium-node` RPC endpoint — no pool fee. (`txmminer` CPU is dev/diagnostic only and cannot mine at mainnet difficulty.)

## Release Checklist

Historical release-prep checklist retained for auditability.

- [ ] Version tag chosen.
- [ ] Release notes written.
- [ ] Linux binaries built.
- [ ] CUDA miner binaries built for supported architectures.
- [ ] SHA256 checksums generated.
- [ ] Install script points to correct release.
- [ ] Upgrade instructions written.
- [ ] Rollback or emergency communication plan written.

## Current Decision

Tensorium v0.3.1-mainnet-candidate is the current documented mainnet-candidate baseline. Phase 7 (7A–7E) is complete.

## Launch Blockers

Historical note: the launch blockers are now closed.

- Final public launch announcement: DONE on 2026-06-02
- Phase 8 launch gates: PASSED
- Everything below Phase 9 and Phase 10 is post-launch execution, not a blocker to chain liveness

## Phase 8 — Pre-Launch Checklist

Historical section preserved for launch auditability. Phase 8 is complete and the chain is already live.

### 8A — Infrastructure

| Item | Status | Notes |
|---|---|---|
| MC RPC/P2P daemon | DONE | `mainnet-candidate rpc/p2p-listen/sync` operational (commit 9286304) |
| Mainnet-candidate seed VPS | TEMP DECISION | Use existing DigitalOcean VPS (157.230.44.162) as the temporary MC/mainnet-candidate host. Dedicated VPS migration remains planned after launch pressure is lower. |
| MC seed node deployed | DONE | `tensorium-node mainnet-candidate init` + systemd `tensorium-mc-rpc` (127.0.0.1:33332) + `tensorium-mc-p2p` (0.0.0.0:33333) live on VPS 157.230.44.162 since 2026-06-01. |
| DNS seed | DONE | `seed.tensoriumlabs.com` A → 157.230.44.162 (user confirmed 2026-06-01). `MC_DEFAULT_SEEDS=["seed.tensoriumlabs.com:33333"]` hardcoded in node binary (commit `40f723d`). |
| MC P2P sync test | DONE | 2026-06-01: second MC node initialized with isolated state file, synced from `seed.tensoriumlabs.com:33333`, matched genesis tip/height (`0`), and served P2P on `:33334` for verification. Repeat after non-genesis MC activity during soak if chain height increases. |
| Backup seed node | DONE | Vultr `txm-mc-seed-1` (`139.180.137.144`) deployed 2026-06-01 as a second-provider MC seed node. `tensorium-mc-rpc` + `tensorium-mc-p2p` active, sync matches primary seed at height `0`, firewall open for `33333/tcp`, monitoring + soak cron installed. Runbook: `docs/operations/BACKUP_SEED_NODE_RUNBOOK.md`. |
| Firewall + SSL on MC VPS | DONE | UFW 33333/tcp open; `rpc.tensoriumlabs.com` + `mc-rpc.tensoriumlabs.com` nginx HTTPS proxies live with Let's Encrypt certs (2026-06-01). |
| Monitor for MC node | DONE | `tensorium-monitor.sh` checks mc_rpc (height), mc_p2p, pub_rpc (https), mc_pub_rpc (https), faucet; all green 2026-06-01. Hourly soak log `/var/log/tensorium-soak.log`. |

### 8B — Wallet & UX

| Item | Status | Notes |
|---|---|---|
| CLI wallet works on MC chain | DONE | `txmwallet` unchanged; works with any address format |
| Chrome extension wallet | DONE | `tensorium-wallet-extension` v0.1.1 — TypeScript+React MV3, `chrome.storage.local`, secp256k1+SHA256d, 20/20 tests, Apache-2.0. GitHub release live with manual install ZIP. Chrome Web Store submission under review. |
| Mobile wallet | DEFERRED | iOS/Android — post-launch |
| Web wallet | DEFERRED | In-browser without extension — post-launch |

Chrome extension wallet stack: TypeScript + React, separate repo `tensorium-wallet-extension`, store encrypted key in `chrome.storage.local`, reuse secp256k1+SHA256d from txmwallet (port to JS or WASM).

### 8C — Pool Website & Faucet

| Item | Status | Notes |
|---|---|---|
| Pool website | DONE | `https://pooltxm.tensoriumlabs.com` deployed on the current main operations VPS — Next.js + TypeScript frontend for `tensorium-pool`: stats, miner lookup, payout history, connect guide |
| Pool fee disclosure | DONE | Shows 5% fee, treasury address, gross reward, pool fee, and net payout before miners connect |
| Bridge landing | DONE | `https://bridge.tensoriumlabs.com` — landing + roadmap page. Functional bridge (wTXM on an EVM L2, current direction: Optimism) planned Phase 9A. |
| OTC board | DONE | `https://otc.tensoriumlabs.com` — peer-to-peer trading board, community-managed via Telegram. |
| Status page | DONE | `https://status.tensoriumlabs.com` — live service health, auto-refresh 60s, pulls from RPC API. |

### 8D — Docs & Community

| Item | Status | Notes |
|---|---|---|
| Whitepaper update | DONE | Added pool fee, founder lock, MC genesis, Phase 8 roadmap |
| Docs: pool guide | DONE | Added official pool endpoint, fee disclosure, miner commands, payout lookup |
| Docs: MC node guide | DONE | Added `mainnet-candidate rpc/p2p-listen/sync` commands and MC genesis metadata |
| Project identity email | DONE | `dev@tensoriumlabs.com` mailbox created on VPS with Postfix/Dovecot TLS; DNS MX/SPF/DMARC verified publicly |
| DKIM email signing | DONE | OpenDKIM installed, Postfix signing verified locally, and public DNS selector `txm20260531` verified with `opendkim-testkey` |
| GitHub project identity | DONE | `tensorium-labs` GitHub user namespace created; repos created under Tensorium namespace; local remotes and public links migrated from `rygroup-dev` |
| Legacy GitHub repos | DONE | Old `rygroup-dev/tensorium-core` and `rygroup-dev/tensorium-pool-website` set back to private after migration |
| Working order | DONE | Future flow: local edit -> local checks -> push `tensorium-labs` -> VPS deploy/sync -> smoke checks |
| Temporary mainnet-candidate host | DECIDED | Use current DigitalOcean VPS first; local + GitHub remain source of truth so migration to Hetzner/dedicated VPS is straightforward later. |
| Docs: Chrome extension guide | DONE | `https://docs.tensoriumlabs.com/chrome-wallet.html` deployed 2026-06-01. Covers install, create/import, send, network selector, security model, FAQ. |
| Public RPC endpoints | DONE | `https://mc-rpc.tensoriumlabs.com` is the main public chain RPC. Current posture: DO remains primary public RPC host; Vultr backup seed stays seed-only until public RPC split/failover is explicitly activated. See `docs/operations/PUBLIC_RPC_POSTURE.md`. |
| Risk disclosure on website | DONE | Root site and docs link to `docs/project/RISK_DISCLOSURE.md` |
| Announce mainnet-candidate launch | **DONE** | 2026-06-02: Bridge opened, Discord announcement pinned, bridge website live, ecosystem complete. Mainnet declared live. |

### 8E — Security & Legal

| Item | Status | Notes |
|---|---|---|
| Source code license | DONE | Apache-2.0 added with `LICENSE` and `NOTICE`; workspace package license updated |
| Soak test (ongoing runtime watch) | **DONE** | MC chain ran from 2026-06-01. Soak gate removed 2026-06-02 — infrastructure stable, monitoring green, mainnet declared live. Soak logging continues passively. |
| Security audit | DEFERRED | External audit recommended before economic value. Can defer to post-launch. |

---

---

## Phase 9A — Bridge & Ecosystem (DONE — 2026-06-02)

| Item | Status | Notes |
|---|---|---|
| wTXM ERC-20 (OP Mainnet) | DONE | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` |
| TensoriumBridgeController (OP Mainnet) | DONE | `0x4b31C557AD64609B975610812273BF82F1475384` |
| Gnosis Safe 2-of-3 | DONE | `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9` — 3 owners, threshold 2 |
| Contract ownership → Safe | DONE | Both wTXM and Controller owned by Safe |
| Bridge relayer (VPS) | DONE | `tensorium-bridge-relayer` pm2 process, 6-block confirm, ~15 min avg |
| Bridge website | DONE | https://bridge.tensoriumlabs.com — status "Bridge Live" |
| Bridge publicly open | DONE | 2026-06-02 — soak test gate removed |
| Explorer indexer | DONE | In-process indexer, O(1) address history, `/api/search`, paginated UI |
| SDK JS | DONE | `@tensorium/sdk@0.1.1` on npm |
| Discord community | DONE | `discord.gg/KkgGSZKVZw` — 7 categories, 20 channels, auto-role bot |
| Uniswap V3 pool | PENDING | Create via https://app.uniswap.org/pools/new?chain=optimism when first wTXM is bridged |

---

## Phase 9 — Ecosystem (Post-Launch)

This section tracks ecosystem work after launch. The chain is already live; remaining items here are adoption and integration follow-through.

### 9A — DEX / Swap Platform

TXM needs a way to be bought and sold. Three options by complexity:

| Option | Complexity | Timeline |
|---|---|---|
| OTC/P2P trading board (`swap.tensoriumlabs.com`) | Low | 1-2 weeks |
| Bridge to Optimism + OP DEX listing (wTXM) | High | 2-3 months |
| Native atomic swap (HTLC) | Very high | Requires Phase 10B |

**Recommended sequence:**
1. OTC trading board first (fast, P2P listing, no smart contracts)
2. Bridge to Optimism → wTXM → OP DEX liquidity (wide exposure)
3. Native atomic swap after scripting layer (Phase 10)

Execution roadmap: see `docs/bridge/phase9a/PHASE9A_SWAP_ROADMAP.md`.
Bridge trust model decision: see `docs/bridge/phase9a/PHASE9A_BRIDGE_MODEL_DECISION.md`.
Bridge policy: see `docs/bridge/phase9a/PHASE9A_BRIDGE_POLICY.md`.
`wTXM` contract spec: see `docs/bridge/phase9a/PHASE9A_WTXM_CONTRACT_SPEC.md`.
Bridge controller spec: see `docs/bridge/phase9a/PHASE9A_BRIDGE_CONTROLLER_SPEC.md`.
Contracts implementation plan: see `contracts/PHASE9A_CONTRACTS_IMPLEMENTATION_PLAN.md`.
Initial Hardhat workspace and local tests live under `contracts/`.
Signer/custody layout: see `docs/bridge/phase9a/PHASE9A_SIGNER_CUSTODY_LAYOUT.md`.
Bridge ledger format: see `docs/bridge/phase9a/PHASE9A_BRIDGE_LEDGER_FORMAT.md`.
Bridge operator runbook: see `docs/bridge/phase9a/PHASE9A_OPERATOR_RUNBOOK.md`.
Execution checklist: see `docs/bridge/phase9a/PHASE9A_EXECUTION_CHECKLIST.md`.

### 9B — Explorer Improvements

| Feature | Status | Notes |
|---|---|---|
| Address page — balance + TX history | **DONE** | In-process explorer indexer now builds address and tx lookup tables from RPC-backed block fetches, persists `txindex.json`, and no longer rescans the chain on every request. |
| TX detail page fast path | **DONE** | `/api/tx/:txid` uses index for instant height lookup instead of scanning 200 blocks |
| Global search | **DONE** | `/api/search?q=` handles block height, 64-hex txid, `txm1…` address |
| Public REST API | **DONE** | `/api/address/:addr` returns balance+history; `/api/indexer/status`; `/api/search` |
| `address.html` UI rewrite | **DONE** | Shows spendable balance, pending balance, UTXO count, paginated tx history (25/page) with Received/Sent/Mined badges and time-ago |
| Network stats / difficulty chart | EXISTING | `/api/charts` endpoint + chart UI on main explorer page |
| Mempool viewer | EXISTING | `/mempool` page, `/api/mempool` endpoint |

**Indexer architecture:** explorer now keeps an in-process incremental index sourced from RPC block fetches. It tracks tx records and address appearances in memory, refreshes forward from the last indexed tip, and exposes `/api/indexer/status` for health checks.

### 9C — SDK & Developer Tools

- `tensorium-sdk-js` — **DONE** — published as `@tensorium/sdk@0.1.1` on npm. `npm install @tensorium/sdk`. Fixed ESM output path (`index.js` not `index.mjs`), license Apache-2.0. 13 tests passing. https://www.npmjs.com/package/@tensorium/sdk
- `tensorium-sdk-py` — **DONE** — `pip install tensorium-sdk` (v0.1.1, PyPI, Apache-2.0, 7 tests)
- RPC API reference docs — TODO — `docs.tensoriumlabs.com/api`
- Example dApp using SDK — TODO

### 9D — Listing & Community

| Item | Status | Notes |
|---|---|---|
| Discord server | **DONE** | Full setup: 7 categories, 20 channels, 9 roles, invite `discord.gg/KkgGSZKVZw` |
| Discord auto-role bot | **DONE** | `txm-discord-bot.service` running on VPS — assigns ⭐ Early Adopter + 🌟 Community on join; DMs welcome message |
| Discord guides | **DONE** | GPU mining guide, pool guide, and node operator guide all posted |
| Discord announcement | **DONE** | Mainnet-candidate launch announcement posted and pinned in #announcements |
| Website Discord CTA | **DONE** | Discord section added to `tensoriumlabs.com` before footer |
| CEX outreach | **DONE** | 14 exchanges contacted 2026-06-02: MEXC, Gate.io, CoinEx, OKX, Bybit, SafeTrade, LBank, XT.com, BitMart, CoinW, DigiFinex, Hotcoin, BingX, BTCC |
| Open Telegram | DEFERRED | User decision: Discord-first strategy, Telegram later |
| Twitter/X | DEFERRED | Discord-first strategy |
| Mining competition | TODO | post-launch — after exchange listing confirmed |

---

## Phase 10 — Advanced Protocol (Long-term)

- **Bridge EVM formal**: multi-sig relayer, audited smart contract, `bridge.tensoriumlabs.com`
- **Scripting layer**: OP codes (multisig, timelock, HTLC) → enables native atomic swap + DEX
- **Governance**: TIP process (Tensorium Improvement Proposal), on-chain or off-chain signaling
- **Storage migration**: JSON state -> RocksDB complete; automatic migration from legacy `state.json` now ships in the node and wallet paths
- **Mobile wallet**: iOS + Android (React Native or Flutter)

---

## Ecosystem Checklist (Full)

**Protocol — DONE:**
- [x] tensorium-node, txmwallet, txmminer, txmminer-cuda, tensorium-pool
- [x] Genesis block mined, MC params frozen, MC daemon complete

**Infrastructure — historical Phase 8 / ongoing follow-through:**
- [x] Temporary MC seed VPS decision: use existing DigitalOcean VPS first
- [ ] Dedicated MC VPS migration after temporary launch
- [x] DNS seed
- [x] MC P2P sync test
- [x] Backup seed node
- [x] Block explorer, monitoring, backup

**Wallet & UX — post-launch follow-through:**
- [x] Chrome extension wallet
- [ ] Mobile wallet (Phase 10)

**Mining Ecosystem — post-launch follow-through:**
- [x] Pool website (pooltxm.tensoriumlabs.com)
- [x] Testnet faucet
- [x] Mining guide, pool reference implementation

**Trading & Liquidity — Phase 9+:**
- [x] OTC trading board
- [x] Bridge landing / roadmap
- [x] Functional bridge to Optimism + wTXM
- [ ] DEX listing (Optimism DEX)
- [ ] CEX listing

**Developer — Phase 9:**
- [x] SDK JS — `@tensorium/sdk@0.1.1` live on npm (`npm install @tensorium/sdk`)
- [x] SDK Python — `pip install tensorium-sdk` v0.1.1
- [ ] Public REST API docs
- [ ] Developer onboarding guide

**Advanced Protocol — Phase 10:**
- [ ] Scripting layer (OP codes, HTLC, atomic swap)
- [ ] Governance mechanism
- [x] Storage migration (RocksDB)

**Community & Legal:**
- [x] Open source license (Apache-2.0)
- [x] Community infrastructure runbook + announcement template prepared
- [x] Discord server live — `discord.gg/KkgGSZKVZw` — 7 categories, 20 channels, auto-role bot
- [ ] Telegram public (deferred)
- [ ] Twitter/X (deferred)
- [ ] Security audit external

---

### Mainnet-Candidate Genesis (Reference)

- **Nonce:** `1_936_263_118_035`
- **Hash:** `0000000000269b71601aded6dda2991df6f88b67ac2bef13dff56f4f8a94dfae`
- **Timestamp:** `1_780_272_000` (2026-06-01 00:00:00 UTC)
- **Mined:** RTX 5090, CUDA, 4.64 GH/s, 474 seconds (2026-06-02)
- **Format:** v3 — post-S1 script_pubkey coinbase serialisation
- **Verified:** local `tensorium-node mainnet-candidate init` → identical hash
- **Command:** `tensorium-node mainnet-candidate init` (no args needed)

### VPS Plan for Mainnet-Candidate Seed Node

Temporary decision: use the existing DigitalOcean VPS (`157.230.44.162`) as the
first mainnet-candidate host. Local Git and the `tensorium-labs` GitHub
repositories are the source of truth, so a later migration is operationally
simple: clone from GitHub, copy env/secret files, rebuild services, sync
state/backups if required, then switch DNS.

A later dedicated MC seed node VPS should be:

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

Historical execution order retained for reference.

1. **Current VPS MC node** (8A) — deploy mainnet-candidate services on the existing DO VPS first
2. **DNS seed** (8A) — point to the current VPS during the temporary phase
3. **MC P2P sync test** (8A) — confirm chain works before public announcement
4. **Chrome extension wallet** (8B) — DONE: `tensorium-wallet-extension` published, Apache-2.0, 20/20 tests
5. **User onboarding infrastructure** (8C)
6. **Soak test** (8E) — keep MC chain running and monitored before wider announcement
7. **Dedicated VPS migration** — move to Hetzner/dedicated host when ready without changing source control flow
