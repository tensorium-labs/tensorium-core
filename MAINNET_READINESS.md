# Mainnet Readiness

Status: Phase 7 started, mainnet not ready.
Last updated: 2026-05-31

This document tracks what must be true before Tensorium can move from public GPU-first testnet to a mainnet candidate release.

## Current Position

- Phase 6 GPU-first testnet is complete.
- Public testnet release: `v0.2.0-testnet`.
- Testnet chain ID: `tensorium-testnet-0`.
- Testnet difficulty: 36 leading zero bits.
- CUDA miner: `txmminer-cuda`.
- Public services: website, docs, whitepaper, explorer, seed/test node.

Mainnet launch is not approved until every blocking item below is resolved.

## Blocking Gates

| Gate | Status | Notes |
| --- | --- | --- |
| Consensus audit | DOING | Tokenomics, emission, difficulty, fork-choice/reorg, timestamp, coinbase over-mint, pending double-spend, RPC bind safety, P2P message-size guard, ban-list fix, connection limit, and TCP timeouts added; soak/integration testing and storage scalability remain. |
| Founder wallet | TODO | Address must be generated and published before genesis. |
| Founder lock policy | TODO | Manual policy or native lock mechanism must be documented. |
| Mainnet genesis | TODO | Must be generated after final params and founder address are frozen. |
| Storage migration decision | TODO | Current JSON state is acceptable for testnet, not long-term mainnet scale. |
| Peer discovery | TODO | DNS seed, static seed list, or peer exchange plan needed. |
| Mining pool path | TODO | Decide whether to ship a reference pool or document solo mining only. |
| Pool fee policy | TODO | Official/reference pool fee is drafted as 5%, but treasury address and payout accounting are not final. |
| Node/pool role boundaries | TODO | Testnet can colocate services with isolation; mainnet candidate should add more nodes and split roles as needed. |
| Monitoring | TODO | Node, disk, RPC, P2P, explorer, and SSL monitoring needed. |
| Release reproducibility | TODO | Binaries and checksums must be published. |
| Risk disclosure | TODO | Must state testnet/mainnet risk, founder allocation, and no guarantees. |

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

Required before mainnet genesis:

- [ ] Generate founder wallet address.
- [ ] Generate pool treasury wallet address if the official/reference pool charges fees.
- [ ] Store founder private key outside public VPS infrastructure.
- [ ] Publish founder address before genesis.
- [ ] Publish founder allocation amount.
- [ ] Publish lock/vesting policy.
- [ ] Explain whether lock is protocol-enforced or policy/manual.
- [ ] Publish pool fee policy and pool treasury address before opening an official pool.

Recommended default:

- Generate founder wallet offline or on a trusted local machine.
- Do not store founder private key on the seed node, explorer server, docs server, or CI.
- If native lock is not implemented, disclose that the lock is social/manual, not enforced by L1 consensus.

## Official Pool Fee Policy Draft

Draft decision:

- Official/reference pool fee: 5%.
- Fee destination: a new pool treasury or founder/development treasury wallet.
- Scope: pool-level payout accounting only.
- Not a protocol-level miner tax.
- Solo mining must remain fee-free at the protocol level.

Required safety rules:

- [ ] Publish the pool fee before miners connect.
- [ ] Publish the pool treasury address.
- [ ] Show gross reward, pool fee, and net miner payout in pool accounting.
- [ ] Keep pool treasury private key separate from founder cold wallet.
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

- [ ] Mainnet seed node prepared separately from testnet.
- [ ] Backup seed node prepared.
- [ ] Node, pool, explorer, and treasury roles isolated or explicitly documented for testnet.
- [ ] Backup node plan documented.
- [ ] RPC bound to localhost only.
- [ ] P2P public port documented.
- [ ] Firewall allowlist documented.
- [ ] Log rotation configured.
- [ ] Chain state backup plan documented.
- [ ] Explorer deployed for mainnet candidate.
- [ ] Docs and whitepaper updated for mainnet candidate.
- [ ] SSL renewal verified.
- [ ] External monitoring configured.

## Mining Checklist

- [ ] CUDA miner tested from release binary.
- [ ] CUDA miner tested from source build.
- [ ] RTX 3000/4000 benchmark published.
- [ ] At least one high-end GPU benchmark published.
- [ ] Multi-GPU behavior tested or explicitly deferred.
- [ ] Pool mining path decided.
- [ ] Pool payout accounting supports 5% official pool fee.
- [ ] Pool fee disclosure added to docs/UI.
- [ ] Solo mining guide updated.

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

Tensorium is in Phase 7 preparation, not mainnet launch.

The next concrete work is:

1. Audit consensus parameters and tokenomics tests.
2. Decide founder wallet and lock policy.
3. Add monitoring and backup for the public testnet VPS.
4. Decide peer discovery and pool mining path.
