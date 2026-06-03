# Tensorium Core

A Proof-of-Work blockchain built in Rust — live mainnet and CUDA mining.

> **Status:** Mainnet live (declared 2026-06-02). GPU mining active on `tensorium-mainnet-candidate-0`.
> Mainnet chain: `tensorium-mainnet-candidate-0` | Ticker: `TXM` | P2P: `33333` | RPC: `33332`

[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/KkgGSZKVZw)
[![npm](https://img.shields.io/badge/npm-%40tensorium%2Fsdk-red?logo=npm)](https://www.npmjs.com/package/@tensorium/sdk)
[![PyPI](https://img.shields.io/badge/PyPI-tensorium--sdk-blue?logo=python)](https://pypi.org/project/tensorium-sdk/)
[![Website](https://img.shields.io/badge/Website-tensoriumlabs.com-black)](https://tensoriumlabs.com)
[![Docs](https://img.shields.io/badge/Docs-docs.tensoriumlabs.com-7c3aed)](https://docs.tensoriumlabs.com)
[![Explorer](https://img.shields.io/badge/Explorer-Live-green)](https://explorer.tensoriumlabs.com)
[![Release](https://img.shields.io/badge/Release-v0.3.1--mainnet--candidate-orange)](https://github.com/tensorium-labs/tensorium-core/releases/tag/v0.3.1-mainnet-candidate)

## Install (Linux x86_64)

```bash
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
```

The installer is now mainnet-first: it downloads binaries, creates a wallet, initializes `mainnet-candidate`, syncs from the seed node, and optionally sets up systemd services.

| Binary | Role |
| --- | --- |
| `tensorium-node` | Full node (RPC + P2P) |
| `txmminer` | CPU miner (dev/test only — diff 36 requires GPU) |
| `txmminer-cuda` | **GPU miner** — NVIDIA CUDA, RTX 3000/4000/5000+ |
| `txmwallet` | Wallet CLI |

Or download directly from [Releases](https://github.com/tensorium-labs/tensorium-core/releases).

### Mining Topology

`txmminer-cuda` is the only supported production miner for mainnet. Mainnet initial difficulty (40 leading zero bits) requires a GPU; CPU cannot mine at this difficulty.

**Solo mining** — miner connects directly to your own node RPC. 0% pool fee. Full block reward goes to your address.

```bash
txmminer-cuda 127.0.0.1:33332 YOUR_ADDRESS
```

**Pool mining** — miner connects to the official pool bind address (`pooltxm.tensoriumlabs.com:23336`). The pool charges a **5% fee** on block rewards, disclosed before you connect. Solo miners who run their own node pay no pool fee.

```bash
txmminer-cuda pooltxm.tensoriumlabs.com:23336 YOUR_ADDRESS
```

Pool stats and fee disclosure: https://pooltxm.tensoriumlabs.com

### GPU Mining

```bash
# Pre-built binary (sm_86 = RTX 3000/4000 series)
chmod +x txmminer-cuda-linux-x86_64-sm86
sudo mv txmminer-cuda-linux-x86_64-sm86 /usr/local/bin/txmminer-cuda

# Solo: mine directly against your own node RPC
txmminer-cuda 127.0.0.1:33332 YOUR_ADDRESS

# Build from source for your GPU
cd tools/txmminer-cuda
make ARCH=sm_86    # RTX 3000/4000
make ARCH=sm_89    # RTX 4000 Ada
make ARCH=sm_90    # H100/H200
```

`txmminer` (CPU) is a development/diagnostic tool only — it cannot mine at mainnet difficulty and must not be used as a production miner.

| GPU | Hashrate | Avg Block Time (diff 36) |
| --- | --- | --- |
| RTX 3060 | ~380 MH/s | varies with live mainnet difficulty |
| RTX 3080 | ~1.2 GH/s | varies with live mainnet difficulty |
| RTX 4090 | ~2.5 GH/s | ~27 seconds |
| H100 SXM | ~2 GH/s | ~34 seconds |

---

## What Is Tensorium

Tensorium is a Proof-of-Work Layer 1 blockchain focused on open mining, transparent tokenomics, and a GPU-first live mainnet direction.

- Max supply: 33,000,000 TXM total (8,000,000 pre-mint + 25,000,000 mining)
- Block time: 60 seconds
- Initial reward: 11.9027 TXM/block (1,190,279,581 atoms)
- Halving: every 1,051,200 blocks (~2 years), 10 eras over 20 years
- Mainnet PoW: SHA256d, GPU-first (40-bit initial difficulty)
- Current phase: post-launch operations; mainnet is live and Phase 10 operational hardening is complete

### Pool Fee Policy Draft

Tensorium consensus does not include a hidden miner tax.

The current operating policy allows an official/reference mining pool to charge a transparent `5%` pool fee. This fee is handled by pool payout accounting, sent to a published pool treasury/development wallet, and shown before miners connect. Solo miners who submit blocks directly to their own node are not charged this pool fee by the protocol.

Pool operations now distinguish between:

- pool treasury wallet: receives block rewards / fee revenue
- payout hot wallet: operational wallet used to pay miners

See `docs/operations/POOL_PAYOUT_RUNBOOK.md` for the refill and payout procedure.

For safety, the node and pool should be separate trust boundaries. The temporary mainnet-candidate setup may colocate services on the current VPS to keep operations moving, as long as processes, folders, env files, logs, and wallet files are isolated. As the network grows, adding more nodes is good for redundancy, sync health, and decentralization; mainnet-candidate infrastructure should add backup seed nodes and split high-risk services when needed.

---

## Chrome Wallet Extension

A self-custody browser wallet for TXM is available as a Chrome extension.

- Repo: [tensorium-labs/tensorium-wallet-extension](https://github.com/tensorium-labs/tensorium-wallet-extension)
- Install (manual, while Chrome Web Store review is pending): download ZIP from the [latest release](https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest), unzip, open `chrome://extensions`, enable Developer mode, click Load unpacked
- Features: create/import wallet, balance, send, history (last 200 blocks, sent + received), network selector (mainnet / custom RPC), export backup, lock

---

## Crates

| Crate | Type | Role |
| --- | --- | --- |
| `tensorium-core` | library | Block, transaction, UTXO, mempool, wallet, consensus, fork-choice |
| `tensorium-node` | binary | Full node: HTTP RPC + P2P server |
| `txmminer` | binary | CPU miner (dev/diagnostic only — cannot mine at mainnet difficulty) |
| `txmminer-cuda` | binary | NVIDIA CUDA GPU miner |
| `txmwallet` | binary | CLI wallet |

---

## Build

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git
cd tensorium-core
cargo build --release
cargo test
```

---

## Quick Start — Single Node

> **Mainnet:** use `mainnet-candidate init` / `mainnet-candidate rpc` to connect to the live chain.
> The bare `init`/`rpc` commands target the local dev chain and are for development only.

```bash
# 1. create genesis (mainnet-candidate)
cargo run -p tensorium-node -- mainnet-candidate init

# 2. start RPC server (terminal 1)
cargo run -p tensorium-node -- mainnet-candidate rpc

# 3. create a wallet (terminal 2)
TENSORIUM_WALLET_PASSPHRASE=yourpass cargo run -p txmwallet -- create
export MINER_ADDR=$(cargo run -p txmwallet -- getnewaddress 2>/dev/null)

# 4. start local miner
# for real mainnet mining prefer txmminer-cuda; txmminer is diagnostic-only here
cargo run -p txmminer -- 127.0.0.1:33332 "$MINER_ADDR"

# 5. check balance (after enough local dev blocks mature)
cargo run -p txmwallet -- balance
```

---

## Quick Start — Two Nodes (P2P)

```bash
# --- Node A (seed) ---
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- init
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- rpc 127.0.0.1:33332 &
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- p2p-listen 127.0.0.1:33333 &
# dev/diagnostic only — CPU cannot mine at mainnet difficulty
cargo run -p txmminer -- 127.0.0.1:33332 miner-a

# --- Node B (syncs from A) ---
TENSORIUM_STATE=state-b.json cargo run -p tensorium-node -- init
# sync all blocks from A
TENSORIUM_STATE=state-b.json cargo run -p tensorium-node -- sync 127.0.0.1:33333
# listen for new blocks broadcast by A
TENSORIUM_STATE=state-b.json TENSORIUM_NODE_ID=node-b \
  cargo run -p tensorium-node -- p2p-listen 127.0.0.1:33334 &

# tell A to broadcast to B
TENSORIUM_PEERS=127.0.0.1:33334 TENSORIUM_STATE=state-a.json \
  cargo run -p tensorium-node -- rpc 127.0.0.1:33332
```

---

## Node Commands

```
tensorium-node init                      create mainnet genesis block
tensorium-node status                    show chain tip and height
tensorium-node mine-once [addr]          mine one block (diagnostic only)
tensorium-node rpc [bind]                start HTTP RPC (default 127.0.0.1:33332)
tensorium-node p2p-listen [bind]         start P2P server (default 0.0.0.0:33333)
tensorium-node p2p-connect <host:port>   diagnostic handshake to a peer
tensorium-node sync [host:port]          download missing blocks from a peer
tensorium-node peers                     print TENSORIUM_PEERS list
tensorium-node banlist                   show peer ban list
tensorium-node unban <ip>                remove a ban
tensorium-node mainnet-candidate ...     explicit alias for the same mainnet chain
```

## RPC Endpoints

```
GET  /health
GET  /getblockcount
GET  /getdifficulty
GET  /getblock/<height>
GET  /getblocktemplate/<miner_address>    includes pending mempool transactions
POST /submitblock                         accepts mined block; broadcasts to peers; cleans mempool
POST /sendrawtransaction                  validates signed tx; adds to mempool; broadcasts to peers
GET  /getmempoolinfo
GET  /getbanlist
GET  /unban/<ip>
```

## Wallet Commands

```
txmwallet create                                 generate keypair and save encrypted wallet
txmwallet getnewaddress                          print wallet address (txm1...)
txmwallet show                                   print public wallet info
txmwallet balance                                scan chain state for wallet UTXOs
txmwallet send <to_addr> <atoms> [file]          build and sign a transaction file
txmwallet broadcast [tx_file] [rpc_addr]         submit signed tx to a node
txmwallet unlock-check                           verify passphrase can decrypt wallet
```

## Environment Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `TENSORIUM_STATE` | `tensorium-mainnet-state.json` | Chain state path; JSON paths auto-migrate to `*.db/` RocksDB |
| `TENSORIUM_MEMPOOL` | `tensorium-mainnet-mempool.json` | Mempool file |
| `TENSORIUM_BANS` | `tensorium-mainnet-banlist.json` | Peer ban list file |
| `TENSORIUM_PEERS` | `""` | Comma-separated peers for block/tx broadcast |
| `TENSORIUM_NODE_ID` | `node-<timestamp>` | Identity in P2P handshake |
| `TENSORIUM_WALLET` | `tensorium-wallet.json` | Wallet file |
| `TENSORIUM_WALLET_PASSPHRASE` | required | Passphrase to decrypt wallet |

---

## Ops Scripts

- `tensorium-backup.sh` — creates rolling tarball backups of RocksDB state directories, mempool/banlist JSON files, and any `*.json.migrated` rollback backups. Deploy to `/usr/local/bin/tensorium-backup.sh` on operators' hosts if you use the runbook defaults.
- `docs/operations/PUBLIC_RPC_HARDENING_RUNBOOK.md` — public RPC thresholds, incident checklists, and service ownership rules for mainnet operations.
- `templates/nginx-public-rpc.conf` — nginx reverse-proxy template that keeps node RPC on localhost and applies request/concurrency limits before public exposure.

## Canonical Metadata

- `docs/integrations/CANONICAL_ASSET_METADATA.md` — single-source packet for chain metadata, RPC/explorer URLs, bridge data, tokenomics, and support contact used by wallets, data providers, and listing forms.

## Documentation Layout

- `docs/operations/` — runbooks for backup/restore, pool payouts, public RPC posture, and other operator workflows
- `docs/integrations/` — canonical listing/integrator metadata packets
- `docs/bridge/phase9a/` — bridge-specific Phase 9A specifications, policy, and custody documentation
- `docs/project/` — project-level references such as risk disclosure, migration notes, and known issues

---

## P2P Protocol

All messages are newline-delimited JSON over TCP.

**Handshake** (both sides send first):
```json
{"protocol":"tensorium-p2p","version":1,"chain_id":"tensorium-mainnet-candidate-0",
 "node_id":"node-1","height":100,"tip_hash":"..."}
```

**Post-handshake messages:**

| Type | Direction | Description |
| --- | --- | --- |
| `NewBlock` | push | Broadcast a newly mined block |
| `Ack` | response | Block accepted at given height |
| `Reject` | response | Block rejected with reason |
| `NewTx` | push | Broadcast an unconfirmed transaction |
| `TxAck` | response | Transaction accepted into mempool |
| `TxReject` | response | Transaction rejected with reason |
| `GetBlocks` | request | Ask for blocks starting at `from_height` |
| `Blocks` | response | Batch of up to 50 blocks (empty = no more) |

---

## Fork Choice

Canonical chain = chain with greatest cumulative work, where block work = `2^leading_zero_bits`.

When a competing chain has more work, the node reorganizes:
1. Detects common ancestor
2. Replaces canonical chain Vec with the new best chain
3. Logs reorg depth to stderr

All validated blocks (canonical + stale) are kept in `block_map` for future fork comparisons.

---

## Peer Ban Policy

| Offense | Score | Threshold to ban |
| --- | ---: | --- |
| Wrong chain_id / protocol / version in handshake | 100 | Instant ban |
| Invalid block (bad PoW, tampered) | 20 | 5 bad blocks |
| Invalid transaction (bad signature) | 10 | 10 bad txs |
| Unparseable message | 2 | 50 bad messages |

Ban duration: 1 hour. Persisted to `tensorium-mainnet-banlist.json`.

---

## Consensus Parameters (Mainnet Default)

| Parameter | Value |
| --- | --- |
| Chain ID | `tensorium-mainnet-candidate-0` |
| Target block time | 60 seconds |
| Initial PoW difficulty | 40 leading zero bits |
| Difficulty window | 60 blocks |
| Max adjustment per window | ±1 bit |
| Coinbase maturity | 100 blocks |
| Max future timestamp | 2 hours |
| P2P port | 33333 |
| RPC port | 33332 |

---

## Community

| | |
|---|---|
| 🌐 Website | [tensoriumlabs.com](https://tensoriumlabs.com) — project homepage |
| 💬 Telegram | [t.me/+QOsnpSdhDGZkZGQ1](https://t.me/+QOsnpSdhDGZkZGQ1) — chat, mining help, announcements |
| 🐛 Issues | [github.com/tensorium-labs/tensorium-core/issues](https://github.com/tensorium-labs/tensorium-core/issues) — bug reports and feature requests |
| 📖 Docs | [docs.tensoriumlabs.com](https://docs.tensoriumlabs.com) — node setup, mining guide, RPC reference |
| 📄 Whitepaper | [whitepaper.tensoriumlabs.com](https://whitepaper.tensoriumlabs.com) — technical design and tokenomics |
| 🔍 Explorer | [explorer.tensoriumlabs.com](https://explorer.tensoriumlabs.com) — live chain data |

---

## License

Tensorium Core is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

---

## Safety

- Keep RPC bound to `127.0.0.1` — never expose it directly to the internet.
- Mainnet is live, but public RPC should remain reverse-proxied and rate-limited.
- Founder, treasury, and payout custody should follow the documented runbooks in `docs/operations/` and `docs/project/`.
