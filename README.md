# Tensorium Core

Experimental Rust implementation of the Tensorium proof-of-work blockchain.

> **Status:** Private testnet development — do not use for mainnet, real funds, or public mining yet.
> Chain: `tensorium-testnet-0` | Ticker: `TXM` | P2P port: `23333` | RPC port: `23332`

---

## What Is Tensorium

Tensorium is a Proof-of-Work Layer 1 blockchain focused on open mining, transparent tokenomics, and a GPU-first mainnet direction.

- Max supply: 33,000,000 TXM (1,000,000 founder + 32,000,000 mining)
- Block time: 60 seconds
- Initial reward: 15.23557865 TXM/block
- Halving: every 1,051,200 blocks (~2 years), 10 eras over 20 years
- Testnet PoW: SHA256d, CPU-friendly for bootstrap
- Mainnet direction: GPU-first (after stable testnet)

---

## Crates

| Crate | Type | Role |
| --- | --- | --- |
| `tensorium-core` | library | Block, transaction, UTXO, mempool, wallet, consensus, fork-choice |
| `tensorium-node` | binary | Full node: HTTP RPC + P2P server |
| `txmminer` | binary | CPU miner |
| `txmwallet` | binary | CLI wallet |

---

## Build

```bash
git clone https://github.com/rygroup-dev/tensorium-core.git
cd tensorium-core
cargo build --release
cargo test
```

---

## Quick Start — Single Node

```bash
# 1. create genesis
cargo run -p tensorium-node -- init

# 2. start RPC server (terminal 1)
cargo run -p tensorium-node -- rpc

# 3. create a wallet (terminal 2)
TENSORIUM_WALLET_PASSPHRASE=yourpass cargo run -p txmwallet -- create
export MINER_ADDR=$(cargo run -p txmwallet -- getnewaddress 2>/dev/null)

# 4. start CPU miner
cargo run -p txmminer -- 127.0.0.1:23332 "$MINER_ADDR"

# 5. check balance (after 101+ blocks for coinbase maturity)
cargo run -p txmwallet -- balance
```

---

## Quick Start — Two Nodes (P2P)

```bash
# --- Node A (seed) ---
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- init
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- rpc 127.0.0.1:23332 &
TENSORIUM_STATE=state-a.json cargo run -p tensorium-node -- p2p-listen 127.0.0.1:23333 &
cargo run -p txmminer -- 127.0.0.1:23332 miner-a

# --- Node B (syncs from A) ---
TENSORIUM_STATE=state-b.json cargo run -p tensorium-node -- init
# sync all blocks from A
TENSORIUM_STATE=state-b.json cargo run -p tensorium-node -- sync 127.0.0.1:23333
# listen for new blocks broadcast by A
TENSORIUM_STATE=state-b.json TENSORIUM_NODE_ID=node-b \
  cargo run -p tensorium-node -- p2p-listen 127.0.0.1:23334 &

# tell A to broadcast to B
TENSORIUM_PEERS=127.0.0.1:23334 TENSORIUM_STATE=state-a.json \
  cargo run -p tensorium-node -- rpc 127.0.0.1:23332
```

---

## Node Commands

```
tensorium-node init                      create genesis block
tensorium-node status                    show chain tip and height
tensorium-node mine-once [addr]          mine one block (dev/test only)
tensorium-node rpc [bind]                start HTTP RPC (default 127.0.0.1:23332)
tensorium-node p2p-listen [bind]         start P2P server (default 127.0.0.1:23333)
tensorium-node p2p-connect <host:port>   diagnostic handshake to a peer
tensorium-node sync [host:port]          download missing blocks from a peer
tensorium-node peers                     print TENSORIUM_PEERS list
tensorium-node banlist                   show peer ban list
tensorium-node unban <ip>                remove a ban
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
| `TENSORIUM_STATE` | `tensorium-testnet-state.json` | Chain state file |
| `TENSORIUM_MEMPOOL` | `tensorium-testnet-mempool.json` | Mempool file |
| `TENSORIUM_BANS` | `tensorium-testnet-banlist.json` | Peer ban list file |
| `TENSORIUM_PEERS` | `""` | Comma-separated peers for block/tx broadcast |
| `TENSORIUM_NODE_ID` | `node-<timestamp>` | Identity in P2P handshake |
| `TENSORIUM_WALLET` | `tensorium-wallet.json` | Wallet file |
| `TENSORIUM_WALLET_PASSPHRASE` | required | Passphrase to decrypt wallet |

---

## P2P Protocol

All messages are newline-delimited JSON over TCP.

**Handshake** (both sides send first):
```json
{"protocol":"tensorium-p2p","version":1,"chain_id":"tensorium-testnet-0",
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

Ban duration: 1 hour. Persisted to `tensorium-testnet-banlist.json`.

---

## Consensus Parameters (Testnet)

| Parameter | Value |
| --- | --- |
| Chain ID | `tensorium-testnet-0` |
| Target block time | 60 seconds |
| Initial PoW difficulty | 12 leading zero bits |
| Difficulty window | 60 blocks |
| Max adjustment per window | ±1 bit |
| Coinbase maturity | 100 blocks |
| Max future timestamp | 2 hours |
| P2P port | 23333 |
| RPC port | 23332 |

---

## Safety

- Keep RPC bound to `127.0.0.1` — never expose it directly to the internet.
- This is testnet software. Testnet TXM has no monetary value.
- Do not use this code for mainnet, real funds, or production mining.
- Mainnet requires: stable testnet, GPU-first mining tested, security review, whitepaper, risk disclosure.
