# Tensorium Core

A Proof-of-Work blockchain built in Rust — live mainnet and CUDA mining.

> **Status:** TensorHash v1 mainnet live. GPU mining active on `tensorium-mainnet`.
> Mainnet chain: `tensorium-mainnet` | Ticker: `TXM` | P2P: `33333` | RPC: `33332`

[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/KkgGSZKVZw)
[![npm](https://img.shields.io/badge/npm-%40tensorium%2Fsdk-red?logo=npm)](https://www.npmjs.com/package/@tensorium/sdk)
[![PyPI](https://img.shields.io/badge/PyPI-tensorium--sdk-blue?logo=python)](https://pypi.org/project/tensorium-sdk/)
[![Website](https://img.shields.io/badge/Website-tensoriumlabs.com-black)](https://tensoriumlabs.com)
[![Docs](https://img.shields.io/badge/Docs-docs.tensoriumlabs.com-7c3aed)](https://docs.tensoriumlabs.com)
[![Explorer](https://img.shields.io/badge/Explorer-Live-green)](https://explorer.tensoriumlabs.com)
[![Release](https://img.shields.io/badge/Release-Mainnet%20v1-orange)](https://github.com/tensorium-labs/tensorium-core/releases/latest)

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
```

The installer is mainnet-first: it downloads `tensorium-node` + `txmwallet`,
creates a wallet, initializes `tensorium-mainnet`, syncs from
`seed.tensoriumlabs.com:33333`, and can install systemd services.

Supported targets:

- Linux `x86_64`
- Linux `aarch64`
- macOS `x86_64`
- macOS `aarch64`

### GitHub Install Paths

**1. One-liner node + wallet install**

```bash
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
```

After the installer finishes, the essential checks are:

```bash
curl -s http://127.0.0.1:33332/getblockcount
TENSORIUM_WALLET="$HOME/tensorium-mainnet-node/wallet.json" \
TENSORIUM_WALLET_PASSPHRASE='<your-passphrase>' \
txmwallet getnewaddress
```

**2. Build node + wallet manually from GitHub**

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git
cd tensorium-core
cargo build --release

# binaries:
# target/release/tensorium-node
# target/release/txmwallet
```

**3. Build the GPU miner on a CUDA box**

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git && \
cd tensorium-core/tools/tensorium-miner && \
make && \
./tensorium-miner --selftest
```

| Binary | Role |
| --- | --- |
| `tensorium-node` | Full node (RPC + P2P) |
| `tensorium-miner` | **GPU miner** — NVIDIA CUDA, RTX 3090+/A100/H100 class, Stratum pool + solo |
| `txmwallet` | Wallet CLI |

Or download directly from [Releases](https://github.com/tensorium-labs/tensorium-core/releases).

### Mining Topology

`tensorium-miner` is the recommended production miner for mainnet. Mainnet v1 initial difficulty (42 leading zero bits) requires a GPU; CPU cannot mine at this difficulty.

**Pool mining (recommended)**

Use this when you want the fastest working path. No local node required.

```bash
tensorium-miner \
  --mode pool \
  --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 \
  --wallet YOUR_TXM_ADDRESS \
  --worker WORKER_NAME \
  --gpu all
```

Pool stats and payout history: https://pooltxm.tensoriumlabs.com  
Fee: **5%** of block reward. Reward method: **PPLNS** (last 4096 shares, difficulty-weighted).  
Treasury: `txm1px2nmtp087mz8dv3lplqadwzxawk0c5kg0mt24`

**Solo mining**

For public solo mining, use the miner-compatible HTTP endpoint:

```bash
tensorium-miner \
  --mode solo \
  --rpc http://mc-rpc.tensoriumlabs.com \
  --wallet YOUR_TXM_ADDRESS \
  --gpu all
```

`rpc.tensoriumlabs.com` remains the canonical public HTTPS endpoint for wallets,
SDKs, and integrations. `tensorium-miner` currently uses plain HTTP and does
not follow HTTPS redirects, so use `http://mc-rpc.tensoriumlabs.com` for solo
mining until miner-side HTTPS/redirect support is added.

Or point to your own local node if you run one:
```bash
tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet YOUR_TXM_ADDRESS --gpu all
```

### GPU Miner — Architecture Guide

**Build from source (recommended)**

```bash
# Prerequisites: CUDA toolkit + gcc (standard on Vast.ai, RunPod, etc.)
git clone https://github.com/tensorium-labs/tensorium-core.git
cd tensorium-core/tools/tensorium-miner

# Auto-detect your GPU architecture:
make

# Or specify manually:
make ARCH=sm_86    # RTX 3090 / 3090 Ti
make ARCH=sm_89    # RTX 4090
make ARCH=sm_120   # RTX 5090
make ARCH=sm_90    # H100 / H200
make ARCH=sm_80    # A100

# Install:
sudo make install
# Or manually: sudo mv tensorium-miner /usr/local/bin/

# Self-check before mining:
./tensorium-miner --selftest

# Run (pool):
tensorium-miner --mode pool --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 --wallet YOUR_ADDRESS --worker rig1 --gpu all

# Run (solo, public endpoint):
tensorium-miner --mode solo --rpc http://mc-rpc.tensoriumlabs.com --wallet YOUR_ADDRESS --gpu all
```

**Prebuilt miner binaries**

```bash
# Find your GPU architecture:
nvidia-smi --query-gpu=compute_cap --format=csv,noheader
# Output like "8.6" → sm_86, "8.9" → sm_89, "12.0" → sm_120

# RTX 3090 / 3090 Ti (sm_86):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm86
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/

# RTX 4090 (sm_89):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm89
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/

# RTX 5090 (sm_120):
curl -fsSL -o tensorium-miner \
  https://github.com/tensorium-labs/tensorium-core/releases/latest/download/tensorium-miner-linux-x86_64-sm120
chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/

# Always selftest after install:
tensorium-miner --selftest
```

| GPU | Arch | Hashrate | Avg Block Time (diff 40) |
| --- | --- | --- | --- |
| RTX 3090 | sm_86 | TBD | TBD |
| RTX 4090 | sm_89 | ~2.5 GH/s | ~7 min |
| RTX 5090 | sm_120 | ~220.31 MH/s (measured) | hardware/network dependent |
| H100 SXM | sm_90 | TBD | TBD |

Pool mining (PPLNS) smooths payouts across participants — rewards are split proportionally by share contribution in the last 4096 shares. Recommended for GPUs with longer solo block times.

---

## What Is Tensorium

Tensorium is a Proof-of-Work Layer 1 blockchain focused on open mining, transparent tokenomics, and a GPU-first live mainnet direction.

- Max supply: 33,000,000 TXM total (zero premine, 33,000,000 mining allocation)
- Block time: 60 seconds
- Initial reward: 7.85584523 TXM/block (785,584,523 atoms)
- Halving: every 2,102,400 blocks (~4 years), 10 eras over ~40 years
- Mainnet PoW: TensorHash v1, GPU-first (42-bit initial difficulty)
- Current phase: Mainnet v1 live on `tensorium-mainnet`

### Pool Fee Policy Draft

Tensorium consensus does not include a hidden miner tax.

The current operating policy allows an official/reference mining pool to charge a transparent `5%` pool fee. This fee is handled by pool payout accounting, sent to a published pool treasury/development wallet, and shown before miners connect. Solo miners who submit blocks directly to their own node are not charged this pool fee by the protocol.

Pool operations now distinguish between:

- pool treasury wallet: receives block rewards / fee revenue
- payout hot wallet: operational wallet used to pay miners

See `docs/operations/POOL_PAYOUT_RUNBOOK.md` for the refill and payout procedure.

For safety, the node and pool should be separate trust boundaries.

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
| `tensorium-core` | library | Block, transaction, UTXO, mempool, wallet, consensus, fork-choice, script VM |
| `tensorium-node` | binary | Full node: HTTP RPC + P2P server |
| `tensorium-pool` | binary | Stratum + RPC mining pool (5% fee accounting, payout ledger) |
| `txmwallet` | binary | CLI wallet (P2PKH, multisig, HTLC) |
| `tensorium-miner` | tool (`tools/tensorium-miner`) | NVIDIA CUDA GPU miner v2 — Stratum pool + solo, multi-GPU |

---

## Build

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git
cd tensorium-core
cargo build --release
cargo test
```

---

## Quick Start

### Option A — Mine without running a node (fastest)

```bash
# 1. Build and install the miner
git clone https://github.com/tensorium-labs/tensorium-core.git
cd tensorium-core/tools/tensorium-miner && make && sudo mv tensorium-miner /usr/local/bin/

# 2. Create a wallet
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
# (this installs txmwallet and creates $HOME/tensorium-mainnet-node/wallet.json)

# 3. Check the wallet address
TENSORIUM_WALLET="$HOME/tensorium-mainnet-node/wallet.json" \
TENSORIUM_WALLET_PASSPHRASE='<your-passphrase>' \
txmwallet getnewaddress

# 4. Run the miner selftest
tensorium-miner --selftest

# 5. Pool mining — Stratum, 5% fee, no node required
tensorium-miner --mode pool \
  --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 \
  --wallet YOUR_TXM_ADDRESS \
  --worker rig1 \
  --gpu all

# Or: Solo mining — 0% fee, uses public node (no local node required)
tensorium-miner --mode solo \
  --rpc http://mc-rpc.tensoriumlabs.com \
  --wallet YOUR_TXM_ADDRESS \
  --gpu all
```

### Option B — Run your own full node + mine

```bash
# 1. Install all binaries
curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash

# 2. Start the node (RPC + P2P in one process)
tensorium-node daemon          # default: RPC 127.0.0.1:33332, P2P 0.0.0.0:33333

# 3. Show the wallet address created by install.sh
TENSORIUM_WALLET="$HOME/tensorium-mainnet-node/wallet.json" \
TENSORIUM_WALLET_PASSPHRASE='<your-passphrase>' \
txmwallet getnewaddress

# 4. Mine against your own node (0% fee)
tensorium-miner --mode solo --rpc http://127.0.0.1:33332 --wallet YOUR_ADDRESS --gpu all

# Check balance (coinbase matures after 10 confirmations)
TENSORIUM_WALLET="$HOME/tensorium-mainnet-node/wallet.json" \
TENSORIUM_WALLET_PASSPHRASE='<your-passphrase>' \
txmwallet balance
```

> **Sync from seed:** `tensorium-node sync seed.tensoriumlabs.com:33333`

---

## Node Commands

All mainnet commands use the top-level command set:

```
tensorium-node init                     initialize mainnet chain state
tensorium-node status                   show chain tip and height
tensorium-node daemon [rpc_bind] [p2p_bind]  start RPC + P2P in one process (recommended)
tensorium-node rpc [bind]               start HTTP RPC only (default 127.0.0.1:33332)
tensorium-node p2p-listen [bind]        start P2P server only (default 0.0.0.0:33333)
tensorium-node sync [host:port]         sync blocks from a peer
tensorium-node p2p-connect <host:port>  test handshake to a peer
tensorium-node peers                    print known peers
tensorium-node banlist                  show peer ban list
tensorium-node unban <ip>               remove a ban
tensorium-node mine-once [addr]         mine one block (diagnostic only)
```

## RPC Endpoints

```
GET  /health
GET  /getblockcount
GET  /getdifficulty
GET  /getblock/<height>
GET  /getblocktemplate/<miner_address>    includes pending mempool transactions (sorted by fee)
POST /submitblock                         accepts mined block; broadcasts to peers; cleans mempool
POST /sendrawtransaction                  validates signed tx; adds to mempool; broadcasts to peers
GET  /getmempoolinfo                      pending tx count + fee stats (min/max/median)
GET  /estimatefee                         recommended fee in atoms and TXM
GET  /getutxos/<address_or_spk_hex>       list unspent outputs for address/scriptPubKey
GET  /getbanlist
GET  /unban/<ip>
```

## Wallet Commands

```
txmwallet create                                 generate keypair and save encrypted wallet
txmwallet getnewaddress                          print wallet address (txm1...)
txmwallet show                                   print public wallet info
txmwallet balance                                scan chain state for wallet UTXOs
txmwallet send <to_addr> <atoms>                 build and sign a transaction (default fee)
txmwallet send <to_addr> <atoms> --priority      use priority fee (10× faster inclusion)
txmwallet send <to_addr> <atoms> --fee <atoms>   custom fee in atoms
txmwallet broadcast [tx_file] [rpc_addr]         submit signed tx to a node
txmwallet unlock-check                           verify passphrase can decrypt wallet
```

### Transaction Fees

Tensorium uses **implicit fees** (Bitcoin-style): `fee = sum(inputs) − sum(outputs)`.

| Fee tier | Atoms | TXM |
| --- | --- | --- |
| Minimum relay fee | 10,000 | 0.0001 TXM |
| Priority fee (`--priority`) | 100,000 | 0.001 TXM |
| Custom (`--fee <n>`) | user-defined | — |

The node **rejects transactions below the minimum relay fee**. The fee is automatically deducted from the change output; you only need to ensure your balance covers `amount + fee`.

Check current fee recommendations:
```bash
curl https://rpc.tensoriumlabs.com/estimatefee
```

**Multisig (m-of-n) — see Scripting below:**

```
txmwallet multisig-script <m> <pubkey_hex...>    print an m-of-n scriptPubKey hex
txmwallet send-from-script <spk_hex> <dest> <atoms> [file] [rpc]
                                                 build an unsigned spend from a script UTXO
txmwallet multisig-sign <tx_file>                sign an input with this wallet (offline)
txmwallet multisig-combine <tx_file> <sig...>    combine signatures into a broadcast-ready tx
```

**HTLC (Hash Time Locked Contracts — atomic swaps & timelocks):**

```
txmwallet htlc-secret                            generate a 32-byte preimage + its sha256 hash
txmwallet htlc-script <hash_hex> <recipient_addr> <refund_addr> <locktime_height>
                                                 print an HTLC scriptPubKey hex
txmwallet htlc-claim <spk_hex> <dest> <preimage_hex> [rpc]
                                                 spend an HTLC by revealing the preimage (fee deducted from HTLC value)
txmwallet htlc-refund <spk_hex> <dest> [rpc]     reclaim an HTLC after its locktime height (fee deducted from HTLC value)
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

## Ops References

- `docs/operations/PUBLIC_RPC_HARDENING_RUNBOOK.md` — public RPC thresholds, incident checklists, and service ownership rules for mainnet operations.
- `docs/operations/BACKUP_SEED_NODE_RUNBOOK.md` — backup seed / backup pool operational procedure for private operator environments.
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
{"protocol":"tensorium-p2p","version":1,"chain_id":"tensorium-mainnet",
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
| Chain ID | `tensorium-mainnet` |
| Target block time | 60 seconds |
| Initial PoW difficulty | 42 leading zero bits |
| Min difficulty | 34 leading zero bits |
| Max difficulty | 58 leading zero bits |
| Difficulty window | 60 blocks |
| Max adjustment per window | ±1 bit |
| Coinbase maturity | 10 blocks |
| Max future timestamp | 2 hours |
| P2P port | 33333 |
| RPC port | 33332 |

---

## Script System

Tensorium uses a Bitcoin-style stack script VM for transaction outputs. Supported standard scripts:

| Script | Purpose |
| --- | --- |
| **P2PKH** | Pay-to-Public-Key-Hash — the default single-key address (`txm1...`) |
| **Multisig** | Bare m-of-n (`OP_CHECKMULTISIG`) — treasury / shared custody |
| **HTLC** | Hash Time Locked Contract — trustless atomic swaps and timelocked escrow |

Opcodes include `OP_DUP`, `OP_HASH160` (`SHA256(x)[0..20]`), `OP_CHECKSIG`, `OP_CHECKMULTISIG`,
`OP_SHA256`, `OP_IF/ELSE/ENDIF`, and `OP_CHECKLOCKTIMEVERIFY` (absolute block-height timelock).

**HTLC** has two spend paths: a *claim* branch (reveal a `SHA256` preimage + recipient signature)
and a *refund* branch (after a block-height `locktime`, the sender reclaims with the refund key).
Because the hashlock is `SHA256` — also an EVM precompile — the same secret can unlock matching
HTLCs across chains, enabling **trustless atomic swaps** (e.g. TXM ⇄ wTXM on Optimism). See
[`docs/integrations/ATOMIC_SWAP_HTLC.md`](docs/integrations/ATOMIC_SWAP_HTLC.md) for a full walkthrough.

---

## Network Infrastructure

| Service | Primary | Backup |
|---|---|---|
| P2P seed | `seed.tensoriumlabs.com:33333` | — |
| Public RPC | `https://rpc.tensoriumlabs.com` | Miner-compatible HTTP: `http://mc-rpc.tensoriumlabs.com` |
| Stratum pool | `pooltxm.tensoriumlabs.com:3333` | — |
| Explorer | [explorer.tensoriumlabs.com](https://explorer.tensoriumlabs.com) | — |
| Pool stats | [pooltxm.tensoriumlabs.com](https://pooltxm.tensoriumlabs.com) | — |

---

## Community

| | |
|---|---|
| 🌐 Website | [tensoriumlabs.com](https://tensoriumlabs.com) — project homepage |
| 💬 Discord | [discord.gg/KkgGSZKVZw](https://discord.gg/KkgGSZKVZw) — chat, mining help, announcements |
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
