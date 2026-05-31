# Tensorium Core

Experimental Rust implementation of the Tensorium proof-of-work chain.

Status: private early scaffold. Do not use this for mainnet, funds, or public mining yet.

## What Exists Now

- Workspace layout for `tensorium-core` and `tensorium-node`.
- Consensus parameter structs for testnet and mainnet-candidate profiles.
- 20-year emission schedule with 10 halving eras.
- Basic block, transaction, merkle root, double-SHA256 hash, and PoW helpers.
- Difficulty adjustment skeleton with bounded step changes.
- Block validation skeleton for chain id, height, parent hash, time, merkle root, PoW, and coinbase reward.
- Local dev node binary that mines a testnet genesis-style block.

## Run Locally

```bash
cargo test
cargo run -p tensorium-node -- init
cargo run -p tensorium-node -- status
cargo run -p tensorium-node -- mine-once local-dev-miner
cargo run -p tensorium-node -- rpc
curl http://127.0.0.1:23332/getblockcount
curl http://127.0.0.1:23332/getdifficulty
curl http://127.0.0.1:23332/getblock/0
curl http://127.0.0.1:23332/getblocktemplate/local-dev-miner
curl -X POST --data-binary @mined-block.json http://127.0.0.1:23332/submitblock
cargo run -p txmminer -- 127.0.0.1:23332 local-dev-miner
```

The RPC server binds to `127.0.0.1:23332` by default. Keep it localhost-only until authentication, rate limits, and public-node hardening are implemented.

## Current Consensus Defaults

- Target block time: 60 seconds.
- Halving interval: 1,051,200 blocks, about 2 years.
- Halving eras: 10, about 20 years total.
- Supply cap: 33,000,000 TXM.
- Founder allocation: 1,000,000 TXM.
- Mining allocation: 32,000,000 TXM.
- Initial mining reward: 15.23557865 TXM per block.
- Testnet PoW starts easier so the first local node can mine.
- Mainnet-candidate PoW starts harder and is intended to become GPU-first before launch.

## Safety Rules

- Mainnet must not launch until consensus, difficulty adjustment, networking, wallet handling, and mining/pool behavior have passed review.
- CPU mining is acceptable only for bootstrap testnet work.
- GPU-first mining must be proven in a dedicated testnet before any mainnet candidate.
- RPC must default to localhost and safe methods only.
- Any change to emission, halving, block time, difficulty, or hashing is a hard-fork-level change.
