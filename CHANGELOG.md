# Changelog

All notable changes to Tensorium are documented in this file.

## [v0.3.0-mainnet-candidate] — 2026-05-31

**Status: Mainnet-candidate release. Testnet remains active. Mainnet-candidate chain NOT yet launched — genesis nonce pending GPU mining.**

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
