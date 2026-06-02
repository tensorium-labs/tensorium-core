# Changelog

All notable changes to Tensorium are documented in this file.

## [Phase 10 — RocksDB Storage Migration Complete] — 2026-06-02

### Added
- `ChainState` now persists blocks in RocksDB with column families for blocks, canonical height mapping, and metadata
- Automatic one-time migration from legacy `state.json` to `*.db/` on first open
- Persistent init coverage in `tensorium-node` test suite to ensure `init` produces a reusable on-disk chain state
- `PUBLIC_RPC_HARDENING_RUNBOOK.md` with alert thresholds and incident response for chain stall, peer isolation, explorer divergence, RPC abuse, backup failure, and disk pressure
- `templates/nginx-public-rpc.conf` so public RPC reverse-proxy policy lives in repo instead of tribal knowledge
- `CANONICAL_ASSET_METADATA.md` as the concise single-source packet for wallets, listing forms, and data providers
- `docs/superpowers/prompts/2026-06-02-claude-code-phase11-handoff.md` as the next-worker handoff prompt after Phase 10 closure

### Changed
- `tensorium-node init` now creates persistent RocksDB-backed testnet state instead of building genesis in a tempdir
- `tensorium-node mainnet-candidate init` and `mainnet-candidate mine-genesis` now persist MC genesis to the configured state path
- `txmwallet` now reads chain state through the RocksDB loader and rebuilds wallet UTXOs from `canonical_blocks_iter()`
- `tensorium-pool` now exposes treasury / payout-hot-wallet custody metadata via CLI and HTTP, and pool payout operations are documented in `POOL_PAYOUT_RUNBOOK.md`
- `install.sh` systemd RPC service now binds to `127.0.0.1` by default, matching the node's public-bind guard and intended reverse-proxy posture

### Verified
- `cargo test --workspace` passing after storage migration
- Local smoke run: `tensorium-node init` created `tensorium-testnet-state.db/`
- Local RPC smoke run: `/getblock/0` responded in ~22.56 ms

---

## [Phase 9C/9D Complete + CEX Outreach] — 2026-06-02

### Phase 9C — Python SDK (DONE)
- `tensorium-sdk==0.1.1` published to PyPI via GitHub Actions OIDC
- `pip install tensorium-sdk`
- License corrected: MIT → Apache-2.0
- 7/7 tests passing
- https://pypi.org/project/tensorium-sdk/

### Phase 9D — CEX Outreach (DONE)
- Listing applications sent to 14 exchanges from dev@tensoriumlabs.com
- Tier 1–2: MEXC Global, Gate.io, CoinEx, OKX, Bybit, BingX, BitMart, XT.com
- Tier 3: LBank, CoinW, DigiFinex, Hotcoin, BTCC, SafeTrade
- `CEX_LISTING_PACKAGE.md` added to repo with full token info + templates
- Next step: submit to CoinGecko + CMC after Uniswap V3 pool is live

### Deferred
- Telegram: Discord-first strategy, Telegram later
- Twitter/X: Discord-first strategy

---

## [Mainnet Launch] — 2026-06-02

**Tensorium mainnet (`tensorium-mainnet-candidate-0`) declared live.**

Infrastructure stable since 2026-06-01 genesis. Soak test gate removed. TXM mining is open. Bridge live.

---

## [Phase 9A — Bridge & Ecosystem] — 2026-06-02

**Status: Bridge live. Safe 2-of-3. Explorer indexer. SDK published. Discord open.**

### Phase 9A — Bridge
- **Gnosis Safe 2-of-3** created on OP Mainnet: `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9`
  - Owners: deployer + signer B (`0x50B0...`) + signer C (`0x950f...`) | threshold: 2
- **wTXM + Controller ownership** transferred from deployer EOA to Safe (Ownable2Step)
- **Bridge publicly open** — soak test gate removed 2026-06-02
- Bridge website status updated: "Bridge Live"
- Discord: bridge open announcement pinned in #announcements

---

## [Phase 9 Ecosystem] — 2026-06-02

**Status: Explorer indexer live. SDK published. Discord community open. Phase 9A (bridge) in progress.**

### Phase 9B — Explorer Indexer (DONE)
- **Explorer in-process indexer** now builds `address→history` + `txid→record` in memory from RPC-backed block fetches, then serves `/api/address/:addr`, `/api/tx/:txid`, and `/api/indexer/status` without rescanning the chain on every request
- **Explorer index snapshot persistence** added via `txindex.json`, allowing restart-time reload instead of guaranteed full cold rebuild
- **`/api/address/:addr`** now returns full tx history (received, sent, mined), live UTXO balance from RPC, pending balance, indexer status
- **`/api/tx/:txid`** fast path via index: O(1) txid→height lookup replaces 200-block scan
- **`/api/search?q=`** global search: block height / 64-hex txid / `txm1…` address
- **`/api/indexer/status`** endpoint for monitoring
- **`address.html`** rewritten: balance stats grid, paginated tx history (25/page), Received/Sent/Mined badges, time-ago timestamps

### Phase 9C — SDK (DONE)
- **`@tensorium/sdk@0.1.1`** published to npm — `npm install @tensorium/sdk`
- Fixed v0.1.0 bug: ESM output was `index.mjs` (missing) → corrected to `index.js`
- License corrected MIT → Apache-2.0
- 13 tests passing; ESM + CJS + TypeScript types
- https://www.npmjs.com/package/@tensorium/sdk

### Phase 9D — Community / Discord (DONE)
- **Discord server** live: `discord.gg/KkgGSZKVZw`
- 7 categories, 20 channels, 9 roles (Founder / Core Dev / Moderator / Top Miner / Miner / Community / Verified Miner / Early Adopter / Bot)
- **Auto-role bot** (`txm-discord-bot.service`) on VPS: assigns ⭐ Early Adopter + 🌟 Community to every new member; sends DM welcome with key links
- Channel guides posted: GPU mining, pool mining, node operator, testnet, mainnet-candidate, GPU benchmarks, FAQ, rules, announcements
- **Discord CTA section** added to `tensoriumlabs.com`
- Bot renamed to **TXM Bot** globally
- **Mainnet-candidate launch announcement** pinned in #announcements

---

## [v0.3.1-mainnet-candidate] — 2026-06-01

**Status: Genesis hardcoded. MC daemon operational. Phase 8 infrastructure complete. Soak test running.**

### Added
- **MC genesis hardcoded** — nonce `114_103_168_481` mined on RTX 5090 (2.28 GH/s, 24.6s); hash `000000000063ab6f057a16376b1712e709719126ad977a3d4be23f83b89f0392`; includes 1M TXM founder allocation in genesis output. `tensorium-node mainnet-candidate init` requires no argument.
- **MC RPC/P2P daemon** — `ConsensusParams` threaded through all node functions; subcommands `mainnet-candidate rpc [bind]`, `mainnet-candidate p2p-listen [bind]`, `mainnet-candidate sync [peer]`; env vars `TENSORIUM_MC_MEMPOOL`, `TENSORIUM_MC_BANS` (commit `9286304`)
- **Backup seed node** — Vultr `txm-mc-seed-1` (`139.180.137.144`) deployed as second-provider MC seed; runbook at `BACKUP_SEED_NODE_RUNBOOK.md`
- **DNS seed** — `MC_DEFAULT_SEEDS = ["seed.tensoriumlabs.com:33333"]` in node binary; `seed.tensoriumlabs.com` A→`157.230.44.162` (commit `40f723d`)
- **Public RPC proxies** — `rpc.tensoriumlabs.com` (testnet →23332) and `mc-rpc.tensoriumlabs.com` (MC →33332) via nginx with CORS + rate-limit (`10r/s`) and Let's Encrypt TLS
- **Chrome wallet extension** — `tensorium-wallet-extension` v0.1.1; TypeScript + React MV3; secp256k1+SHA256d; create/import/send/history/settings; 20/20 tests; Apache-2.0; GitHub release live with manual install ZIP
- **Testnet faucet** — `https://faucet.tensoriumlabs.com`; 10 TXM/request, 24h cooldown; testnet reset to 20-bit diff / 10-block maturity for CPU mining
- **`tensorium-sdk-js`** — JS/TS SDK for TXM chain; code complete, tests/build/pack pass; npm publish pending token/2FA policy resolution
- **Pool website** — `https://pooltxm.tensoriumlabs.com` deployed; Next.js frontend with stats, miner lookup, payout history, 5% fee disclosure
- **Community subdomains** — `bridge.tensoriumlabs.com`, `otc.tensoriumlabs.com`, `status.tensoriumlabs.com`, `faucet.tensoriumlabs.com` all live with HTTPS
- **Project email** — `dev@tensoriumlabs.com` with Postfix/Dovecot TLS; MX/SPF/DMARC/DKIM verified
- **GitHub migration** — repos live under `tensorium-labs` namespace; legacy `rygroup-dev` repos set to private

### Fixed
- Testnet reset 2026-06-01: difficulty 36→20 bits, coinbase maturity 100→10 blocks for CPU-minable onboarding
- `txmwallet` binary updated with new maturity setting and deployed to VPS

---

## [v0.3.0-mainnet-candidate] — 2026-05-31

**Status: Mainnet-candidate code complete. Genesis mining pending at time of release; hardcoded in v0.3.1.**

### Added
- **`tensorium-pool`** reference mining pool (`crates/tensorium-pool`)
  - HTTP RPC proxy: miners point at pool instead of node; pool uses pool treasury as coinbase
  - 5% official pool fee (`POOL_FEE_BPS = 500`), calculated with `split_fee(gross, bps)`
  - Persistent payout ledger (`pool-ledger.json`): one entry per accepted block
  - Endpoints: `/health`, `/getblocktemplate/<miner>`, `/submitblock`, `/pool/stats`, `/pool/accounting`, `/pool/pending/<addr>`
  - CLI: `stats`, `accounting`, `pending <addr>`, `mark-paid <addr>`
  - 9 unit tests covering fee split, ledger, stats, pending, and mark-paid
  - Pool treasury address: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9`
- **Built-in static seed list** in `tensorium-node`
  - `DEFAULT_SEEDS = ["157.230.44.162:23333"]` — new nodes auto-connect without manual config
  - Opt-out: `TENSORIUM_NO_DEFAULT_SEEDS=1`
  - Seed node runs with `TENSORIUM_NO_DEFAULT_SEEDS=1` to avoid self-connection
- **`tensorium-node mainnet-candidate` subcommands**
  - `mainnet-candidate init <genesis_nonce>` — initialize mainnet-candidate state
  - `mainnet-candidate mine-genesis [threads]` — multi-threaded CPU genesis mining
  - `mainnet-candidate status` — show mc chain status
  - MC defaults: state `tensorium-mc-state.json`, RPC `127.0.0.1:33332`, P2P `0.0.0.0:33333`
  - `TENSORIUM_MC_STATE` env var for custom mc state path
- **Founder wallet and pool treasury** generated and published (Phase 7B)
  - Founder cold wallet: `txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d` — 1,000,000 TXM genesis allocation
  - Pool treasury: `txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9` — receives 5% pool fee
  - Social/manual lock policy: max 10%/month for first 24 months, fully unlocked after month 24
- **Infrastructure and monitoring** (Phase 7C)
  - `tensorium-monitor.sh`: cron every 10 min; checks RPC, P2P, explorer, disk %, SSL expiry
  - `tensorium-backup.sh`: daily cron 03:00 UTC; 14 rolling tarballs of chain state
  - `journald` capped at 500M / 50M-per-file / 30-day retention
  - Explorer log rotation: 14-day retention

### Changed
- **`MAINNET_CANDIDATE` params frozen** as of Phase 7E
  - `chain_id`: `tensorium-mainnet-candidate-0`
  - `initial_leading_zero_bits`: 40 (requires RTX 3060+ to mine genesis)
  - `min_leading_zero_bits`: 32, `max_leading_zero_bits`: 56
  - `difficulty_adjustment_window`: 120 blocks
  - Genesis timestamp: `1_780_272_000` (2026-06-01 00:00:00 UTC, TBD until launch)
  - Genesis nonce: TBD — must be GPU-mined before mainnet-candidate chain launch
- **Seed node service files** updated with `TENSORIUM_NO_DEFAULT_SEEDS=1`
- **`tensorium-node` help** updated with new env vars (`TENSORIUM_PEERS` override note, `TENSORIUM_NO_DEFAULT_SEEDS`, `TENSORIUM_MC_STATE`)

### Fixed (Phase 7A)
- **`prune_expired` bug**: `map_or(false, …)` wiped sub-threshold score entries before they could reach ban threshold; fixed to `map_or(true, …)` — only expired bans are removed, accumulated score persists
- **RPC non-loopback bind guard**: node refuses non-loopback RPC binds by default unless `TENSORIUM_RPC_ALLOW_PUBLIC=1` is explicitly set
- **P2P 1 MiB message cap**: newline-delimited P2P reads capped at 1 MiB to prevent unbounded memory growth from malformed peers
- **P2P connection limit**: `MAX_INBOUND_PEERS = 64` with `AtomicUsize` counter; connections above limit refused before thread spawn
- **TCP I/O timeouts**: P2P 30s read/write timeout, RPC 10s read timeout
- **HTTP 400 status text**: `write_json_response` now returns `Bad Request` for 400 codes
- **Mempool double-spend rejection**: mempool now rejects transactions that conflict with already-pending UTXOs (`MempoolError::PendingConflict`) rather than only filtering at block template time

### Security (Phase 7A)
- Added BanList tests: sub-threshold persistence, threshold activation, instant-ban on bad handshake, expiry pruning, active-ban survival, manual unban
- Added fork-choice test: equal-work first-seen blocks kept; higher-cumulative-work branch triggers reorg
- Added coinbase over-mint rejection test
- Added wrong chain ID rejection test
- Added future timestamp rejection test
- Added immature coinbase spend rejection test

---

## [v0.2.0-testnet] — 2025-05-31

- GPU-first testnet at 36-bit difficulty
- Genesis nonce: `64_092_008_986` (pre-mined via CUDA, RTX 3060, 369 MH/s, 173.5s)
- Genesis hash: `00000000095752e05ed8dca9041a3a2ca34ae99ce43dfe711ef8d8df3e302c9e`
- `txmminer-cuda`: NVIDIA CUDA miner (sm86), tested on RTX 3060 ~410 MH/s
- Explorer: `explorer.tensoriumlabs.com`
- Docs: `docs.tensoriumlabs.com`
- Whitepaper: `whitepaper.tensoriumlabs.com`
- Install script: `curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash`

---

## [v0.1.1-testnet] — 2025-05-30

- P2P genesis fix: hardcode genesis timestamp `1_748_649_600` to ensure all nodes share the same genesis block
- Chain ID: `tensorium-testnet-0`
- Difficulty: 26 bits

---

## [v0.1.0-testnet] — 2025-05-29

- Initial public testnet release
- Chain: `tensorium-testnet-0`, SHA256d PoW, UTXO model
- Wallet CLI: `txmwallet create`, `getnewaddress`, `balance`, `send`
- CPU miner: `txmminer`
- Node: `tensorium-node rpc`, `p2p-listen`, `sync`, `init`
- P2P handshake and block sync
- Explorer (early version)
