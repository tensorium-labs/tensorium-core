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
[![Release](https://img.shields.io/badge/Release-v0.3.2--mainnet-orange)](https://github.com/tensorium-labs/tensorium-core/releases/tag/v0.3.2-mainnet)

## Install (Linux x86_64)

```bash
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
```

The installer is now mainnet-first: it downloads binaries, creates a wallet, initializes `mainnet-candidate`, syncs from the seed node, and optionally sets up systemd services.

| Binary | Role |
| --- | --- |
| `tensorium-node` | Full node (RPC + P2P) |
| `tensorium-miner` | **GPU miner v2** — NVIDIA CUDA, RTX 3000/4000/5000+, Stratum pool + solo |
| `txmminer-cuda` | Alias for `tensorium-miner` (backward-compatible) |
| `txmwallet` | Wallet CLI |

Or download directly from [Releases](https://github.com/tensorium-labs/tensorium-core/releases).

### Mining Topology

`tensorium-miner` (v2) is the recommended production miner for mainnet. Mainnet initial difficulty (40 leading zero bits) requires a GPU; CPU cannot mine at this difficulty. The binary is also available as `txmminer-cuda` for backward compatibility.

**Pool mining (Stratum — recommended for consistent payouts):**

```bash
tensorium-miner \
  --mode pool \
  --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 \
  --wallet YOUR_TXM_ADDRESS \
  --worker WORKER_NAME \
  --gpu all \
  --intensity auto
```

The pool charges a **5% fee** on block rewards. Pool stats and fee disclosure: https://pooltxm.tensoriumlabs.com

**Solo mining (0% fee — full reward to your address):**

```bash
tensorium-miner \
  --mode solo \
  --rpc http://127.0.0.1:33332 \
  --wallet YOUR_TXM_ADDRESS \
  --gpu all \
  --intensity auto
```

**Legacy pool mining (RPC mode — still supported):**

```bash
txmminer-cuda pooltxm.tensoriumlabs.com:23336 YOUR_ADDRESS
```

### GPU Mining

**Download GPU miner:**

```bash
# RTX 3060/3070/3080/3090 (sm_86):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm86
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/
sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda

# RTX 4060/4070/4080/4090 (sm_89):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm89
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/
sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda

# RTX 5090 / Blackwell (sm_120):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm120
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/
sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda
```

**Build from source for your GPU:**

```bash
cd tools/txmminer-cuda
make ARCH=sm_86    # RTX 3060/3070/3080/3090
make ARCH=sm_89    # RTX 4060/4070/4080/4090
make ARCH=sm_120   # RTX 5090 (Blackwell)
make ARCH=sm_90    # H100/H200
```

| GPU | Hashrate | Avg Block Time (diff 40, solo) |
| --- | --- | --- |
| RTX 3060 | ~380 MH/s | ~48 min |
| RTX 3080 | ~1.2 GH/s | ~15 min |
| RTX 4090 | ~2.5 GH/s | ~7 min |
| RTX 5090 | ~40 GH/s | ~23 sec |
| H100 SXM | ~2 GH/s | ~9 min |

Pool mining reduces variance — payouts are smoothed across all pool participants.

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

## Quick Start — Run a Mainnet Node

```bash
# 1. initialize mainnet chain state
tensorium-node mainnet-candidate init

# 2. start RPC (terminal 1)
tensorium-node mainnet-candidate rpc

# 3. start P2P — auto-connects to seed node (terminal 2)
tensorium-node mainnet-candidate p2p-listen

# 4. create a wallet
TENSORIUM_WALLET_PASSPHRASE=yourpass txmwallet create
txmwallet getnewaddress

# 5. start GPU miner (terminal 3) — pool mining via Stratum (recommended)
tensorium-miner --mode pool --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 --wallet YOUR_ADDRESS --worker $(hostname) --gpu all --intensity auto

# or solo mining (0% fee, needs your own node running)
tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet YOUR_ADDRESS --gpu all --intensity auto

# 6. check balance (coinbase matures after 100 confirmations)
txmwallet balance
```

> Sync from seed: if your node is behind, run `tensorium-node mainnet-candidate sync seed.tensoriumlabs.com:33333`

---

## Node Commands

All mainnet commands use the `mainnet-candidate` subcommand:

```
tensorium-node mainnet-candidate init                     initialize mainnet chain state
tensorium-node mainnet-candidate status                   show chain tip and height
tensorium-node mainnet-candidate rpc [bind]               start HTTP RPC (default 127.0.0.1:33332)
tensorium-node mainnet-candidate p2p-listen [bind]        start P2P server (default 0.0.0.0:33333)
tensorium-node mainnet-candidate sync [host:port]         sync blocks from a peer
tensorium-node mainnet-candidate p2p-connect <host:port>  test handshake to a peer
tensorium-node mainnet-candidate peers                    print known peers
tensorium-node mainnet-candidate banlist                  show peer ban list
tensorium-node mainnet-candidate unban <ip>               remove a ban
tensorium-node mainnet-candidate mine-once [addr]         mine one block (diagnostic only)
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
