# TXM Asset Protocol — Layer 2 (indexer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `txm-asset-indexer`, a standalone Rust service that scans canonical TXM blocks via the node RPC, reconstructs all TXM20/NFT balances and ownership deterministically using the `tensorium-core::assets` codec + state machine (Layer 1), persists a JSON snapshot, handles reorgs, and serves the data over a read-only REST API.

**Architecture:** A new workspace crate `crates/tensorium-indexer` (binary `txm-asset-indexer`). It depends on `tensorium-core` for the asset codec/state and on the node's HTTP RPC for block data. I/O is kept at the edges (a raw-`TcpStream` RPC client and a raw-`TcpStream` REST server, matching the node/pool convention — no new HTTP deps); all parsing, source-resolution, apply, snapshot, reorg-decision, and API-routing logic are pure functions unit-tested TDD-style. Asset state is held in memory as `tensorium_core::AssetState` plus an `outpoint→address` index; a serializable `Snapshot` (hex-encoded keys) mirrors it for atomic persistence. Reorg below the last-scanned tip triggers a full rebuild from genesis (the chain is young; the spec permits snapshot-at-intervals, and a full rebuild is the simplest correct form).

**Tech Stack:** Rust, `tensorium-core` crate (assets + block + script::standard + Hash256), `serde`/`serde_json`, `std::net::{TcpStream,TcpListener}`, `cargo test`.

**Key decisions (locked here):**
- Indexer is a Rust crate (not a module on the JS explorer) so it shares the Layer-1 codec in `tensorium-core`.
- `ACTIVATION_HEIGHT = 0` for the MVP: scan the whole chain. No asset ops exist before activation, and the `outpoint→address` index must cover genesis to resolve sources, so scanning from 0 is both correct and simplest.
- Source of an asset op = address of the output spent by `inputs[0]` (resolved via the `outpoint→address` index built from all prior outputs, in canonical order).
- Persistence = a single JSON snapshot written atomically (tmp + rename). Reorg (block hash at `last_height` changed) ⇒ rebuild from 0.
- REST server binds `127.0.0.1` by default; public exposure goes behind nginx (same posture as the node RPC).

## File Structure

- `crates/tensorium-indexer/Cargo.toml` — crate manifest (deps: tensorium-core path, serde, serde_json).
- `crates/tensorium-indexer/src/main.rs` — arg/env parsing, load snapshot, spawn scanner thread, run REST server.
- `crates/tensorium-indexer/src/rpc.rs` — `NodeRpc` HTTP client + pure `parse_block_response` / `parse_count_response`.
- `crates/tensorium-indexer/src/index.rs` — `Indexer` (AssetState + outpoint index + history), `record_outputs`, `resolve_source`, `apply_block`.
- `crates/tensorium-indexer/src/store.rs` — `Snapshot` serializable mirror, `from_indexer`/`apply_to`, atomic `save`/`load`.
- `crates/tensorium-indexer/src/sync.rs` — `needs_rebuild` decision + `scan` driver (block-fetch injected for testability).
- `crates/tensorium-indexer/src/api.rs` — pure `route(&Indexer, method, path) -> (u16, Value)` + thin `serve` socket wrapper.
- Modify: `Cargo.toml` (workspace `members`).

---

### Task 1: Crate scaffold + workspace registration

**Files:**
- Create: `crates/tensorium-indexer/Cargo.toml`
- Create: `crates/tensorium-indexer/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create the crate manifest**

`crates/tensorium-indexer/Cargo.toml`:
```toml
[package]
name = "tensorium-indexer"
version = "0.1.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true
description = "TXM asset overlay indexer — scans blocks, reconstructs TXM20/NFT balances, serves a REST API"

[[bin]]
name = "txm-asset-indexer"
path = "src/main.rs"

[dependencies]
tensorium-core = { path = "../tensorium-core" }
serde = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 2: Create a placeholder main + module decls**

`crates/tensorium-indexer/src/main.rs`:
```rust
//! txm-asset-indexer — Layer 2 of the TXM asset marketplace.
mod api;
mod index;
mod rpc;
mod store;
mod sync;

fn main() {
    println!("txm-asset-indexer (scaffold)");
}
```

Create empty module files so it compiles:

`crates/tensorium-indexer/src/rpc.rs`:
```rust
use tensorium_core::block::Block;
```
`crates/tensorium-indexer/src/index.rs`:
```rust
use std::collections::HashMap;
use tensorium_core::assets::AssetState;
use tensorium_core::block::{Block, Transaction};
```
`crates/tensorium-indexer/src/store.rs`:
```rust
use serde::{Deserialize, Serialize};
```
`crates/tensorium-indexer/src/sync.rs`:
```rust
```
`crates/tensorium-indexer/src/api.rs`:
```rust
use serde_json::Value;
```

- [ ] **Step 3: Register the crate in the workspace**

In the root `Cargo.toml`, add to `members` (after `"crates/txm-rpc-adapter",`):
```toml
    "crates/tensorium-indexer",
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p tensorium-indexer`
Expected: builds (unused-import warnings are fine for now).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-indexer Cargo.toml Cargo.lock
git commit -m "feat(indexer): scaffold txm-asset-indexer crate"
```

---

### Task 2: RPC client — parse block + count responses

**Files:**
- Modify: `crates/tensorium-indexer/src/rpc.rs`

- [ ] **Step 1: Write the failing tests**

Replace `crates/tensorium-indexer/src/rpc.rs` with:
```rust
use serde::Deserialize;
use tensorium_core::block::Block;
use tensorium_core::hash::Hash256;

/// Shape of `GET /getblock/<h>` → `{ "hash": <hash>, "block": <Block> }`.
#[derive(Deserialize)]
struct BlockResponse {
    hash: Hash256,
    block: Block,
}

/// Shape of `GET /getblockcount` → `{ "height": <u64|null>, ... }`.
#[derive(Deserialize)]
struct CountResponse {
    height: Option<u64>,
}

/// Pure: parse a `/getblock` JSON body into (block hash, block).
pub fn parse_block_response(body: &str) -> Result<(Hash256, Block), String> {
    let r: BlockResponse =
        serde_json::from_str(body).map_err(|e| format!("getblock parse: {e}"))?;
    Ok((r.hash, r.block))
}

/// Pure: parse a `/getblockcount` JSON body into the tip height (None = empty chain).
pub fn parse_count_response(body: &str) -> Result<Option<u64>, String> {
    let r: CountResponse =
        serde_json::from_str(body).map_err(|e| format!("getblockcount parse: {e}"))?;
    Ok(r.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tensorium_core::block::Transaction;

    #[test]
    fn parses_block_response_roundtrip() {
        let block = sample_block();
        let hash = Hash256([5u8; 32]);
        let body = json!({ "hash": hash, "block": block }).to_string();
        let (got_hash, got_block) = parse_block_response(&body).unwrap();
        assert_eq!(got_hash, hash);
        assert_eq!(got_block.header.height, block.header.height);
        assert_eq!(got_block.transactions.len(), block.transactions.len());
    }

    #[test]
    fn parses_count_response() {
        assert_eq!(parse_count_response(r#"{"height":1911,"blocks":1912}"#).unwrap(), Some(1911));
        assert_eq!(parse_count_response(r#"{"height":null}"#).unwrap(), None);
        assert!(parse_count_response("not json").is_err());
    }

    fn sample_block() -> Block {
        let coinbase = Transaction::coinbase(10, 1190, "txm1miner");
        let header = tensorium_core::block::BlockHeader {
            version: 1,
            chain_id: "tensorium-mainnet-candidate-0".into(),
            height: 10,
            previous_hash: Hash256([1u8; 32]),
            merkle_root: Hash256([2u8; 32]),
            timestamp_seconds: 1_780_000_000,
            leading_zero_bits: 40,
            nonce: 42,
        };
        Block::new(header, vec![coinbase])
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tensorium-indexer rpc::tests`
Expected: PASS. (If `BlockHeader` has additional required fields, add them to `sample_block` to match `crates/tensorium-core/src/block.rs`.)

- [ ] **Step 3: Add the socket RPC client (thin I/O wrapper)**

Append to `crates/tensorium-indexer/src/rpc.rs` (above `#[cfg(test)]`):
```rust
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// HTTP RPC client for a Tensorium node. `addr` is `host:port` (e.g. `127.0.0.1:33332`).
pub struct NodeRpc {
    pub addr: String,
}

impl NodeRpc {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    fn get(&self, path: &str) -> Result<String, String> {
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: {}\r\nconnection: close\r\n\r\n", self.addr);
        let mut stream = TcpStream::connect(&self.addr)
            .map_err(|e| format!("RPC connect {}: {e}", self.addr))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("RPC timeout: {e}"))?;
        stream.write_all(request.as_bytes()).map_err(|e| format!("RPC write: {e}"))?;
        let mut response = String::new();
        stream.read_to_string(&mut response).map_err(|e| format!("RPC read: {e}"))?;
        let (head, body) =
            response.split_once("\r\n\r\n").ok_or_else(|| "invalid HTTP response".to_owned())?;
        if !head.starts_with("HTTP/1.1 200") {
            return Err(format!("RPC error ({path}): {body}"));
        }
        Ok(body.to_owned())
    }

    /// Tip height, or None if the chain is empty.
    pub fn block_count(&self) -> Result<Option<u64>, String> {
        parse_count_response(&self.get("/getblockcount")?)
    }

    /// Fetch block at `height` → (block hash, block).
    pub fn block_at(&self, height: u64) -> Result<(Hash256, Block), String> {
        parse_block_response(&self.get(&format!("/getblock/{height}"))?)
    }
}
```

- [ ] **Step 4: Verify it still builds + tests pass**

Run: `cargo test -p tensorium-indexer rpc::tests`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-indexer/src/rpc.rs
git commit -m "feat(indexer): node RPC client + block/count response parsers"
```

---

### Task 3: Indexer state — outpoint index + source resolution

**Files:**
- Modify: `crates/tensorium-indexer/src/index.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/tensorium-indexer/src/index.rs` with:
```rust
use std::collections::HashMap;
use tensorium_core::assets::AssetState;
use tensorium_core::block::Transaction;
use tensorium_core::hash::Hash256;
use tensorium_core::script::standard::extract_address;

/// One recorded asset event, served by `/history/<address>`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub height: u64,
    pub txid: String,
    pub op: String,       // "issue" | "nft_mint" | "transfer"
    pub asset_id: String, // hex
    pub from: String,
    pub to: String,       // dest (transfer) or "" (issue/mint)
    pub amount: u64,
}

/// In-memory indexer state. Deterministically reconstructable from the chain.
#[derive(Default)]
pub struct Indexer {
    pub state: AssetState,
    /// "<txid_hex>:<vout>" -> address of that P2PKH/P2SH output.
    pub outpoints: HashMap<String, String>,
    /// address -> chronological asset events.
    pub history: HashMap<String, Vec<HistoryEntry>>,
    pub last_height: u64,
    pub last_hash: String,
    pub scanned_any: bool,
}

/// Key for the outpoint index.
pub fn outpoint_key(txid: &Hash256, vout: u32) -> String {
    format!("{}:{}", txid.to_hex(), vout)
}

impl Indexer {
    /// Record every output of `tx` into the outpoint index (address-bearing only).
    pub fn record_outputs(&mut self, tx: &Transaction) {
        for (vout, out) in tx.outputs.iter().enumerate() {
            if let Some(addr) = extract_address(&out.script_pubkey) {
                self.outpoints.insert(outpoint_key(&tx.id, vout as u32), addr);
            }
        }
    }

    /// Resolve the source address of `tx` = address of the output spent by `inputs[0]`.
    pub fn resolve_source(&self, tx: &Transaction) -> Option<String> {
        let first = tx.inputs.first()?;
        let key = outpoint_key(&first.previous_output.txid, first.previous_output.output_index);
        self.outpoints.get(&key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tensorium_core::block::{OutPoint, TxInput, TxOutput};
    use tensorium_core::script::standard::p2pkh_from_address;
    use tensorium_core::WalletKeypair;

    fn addr() -> String {
        WalletKeypair::generate().address.as_str().to_string()
    }

    #[test]
    fn records_outputs_and_resolves_source_from_first_input() {
        let alice = addr();
        // tx A: creates an output paying alice at vout 0.
        let tx_a = Transaction::payment(
            vec![],
            vec![TxOutput { value_atoms: 100, script_pubkey: p2pkh_from_address(&alice).unwrap() }],
        );
        let mut idx = Indexer::default();
        idx.record_outputs(&tx_a);

        // tx B spends A:0 as inputs[0] → source must resolve to alice.
        let tx_b = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint { txid: tx_a.id, output_index: 0 },
                signature_script: vec![],
            }],
            vec![],
        );
        assert_eq!(idx.resolve_source(&tx_b), Some(alice));

        // Unknown prev-output → None.
        let tx_c = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint { txid: Hash256([9u8; 32]), output_index: 7 },
                signature_script: vec![],
            }],
            vec![],
        );
        assert_eq!(idx.resolve_source(&tx_c), None);

        // No inputs (coinbase-like) → None.
        let tx_d = Transaction::payment(vec![], vec![]);
        assert_eq!(idx.resolve_source(&tx_d), None);
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p tensorium-indexer index::tests::records_outputs_and_resolves_source_from_first_input`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add crates/tensorium-indexer/src/index.rs
git commit -m "feat(indexer): outpoint index + inputs[0] source resolution"
```

---

### Task 4: `apply_block` — extract + apply asset ops per block

**Files:**
- Modify: `crates/tensorium-indexer/src/index.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `index.rs`:
```rust
    use tensorium_core::assets::{encode_op, AssetOp, IssueData, TransferData};
    use tensorium_core::block::{Block, BlockHeader};
    use tensorium_core::script::OP_RETURN;

    fn op_return_spk(op: &AssetOp) -> Vec<u8> {
        let data = encode_op(op);
        let mut spk = vec![OP_RETURN, 0x4c, data.len() as u8];
        spk.extend_from_slice(&data);
        spk
    }

    fn block_with(height: u64, txs: Vec<Transaction>) -> Block {
        let header = BlockHeader {
            version: 1,
            chain_id: "test".into(),
            height,
            previous_hash: Hash256([0u8; 32]),
            merkle_root: Hash256([0u8; 32]),
            timestamp_seconds: 0,
            leading_zero_bits: 0,
            nonce: 0,
        };
        Block::new(header, txs)
    }

    #[test]
    fn apply_block_indexes_issue_then_transfer() {
        let alice = addr();
        let bob = addr();
        let mut idx = Indexer::default();

        // Block 1: alice funds herself (so a UTXO she owns exists), then ISSUEs.
        // Funding tx pays alice at vout 0; issue tx spends it as inputs[0].
        let fund = Transaction::payment(
            vec![],
            vec![TxOutput { value_atoms: 1000, script_pubkey: p2pkh_from_address(&alice).unwrap() }],
        );
        let issue_op = AssetOp::Issue(IssueData {
            ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
        });
        let issue_tx = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint { txid: fund.id, output_index: 0 },
                signature_script: vec![],
            }],
            vec![
                TxOutput { value_atoms: 1, script_pubkey: p2pkh_from_address(&alice).unwrap() },
                TxOutput { value_atoms: 0, script_pubkey: op_return_spk(&issue_op) },
            ],
        );
        let asset_id = issue_tx.id.0;
        idx.apply_block(&block_with(1, vec![fund, issue_tx.clone()]), 1);
        assert_eq!(idx.state.ft_balance(&alice, &asset_id), 1000);

        // Block 2: alice transfers 250 GOLD to bob. inputs[0] spends issue_tx:0 (alice).
        let xfer_op = AssetOp::Transfer(TransferData { asset_id, amount: 250, dest_output_index: 0 });
        let xfer_tx = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint { txid: issue_tx.id, output_index: 0 },
                signature_script: vec![],
            }],
            vec![
                TxOutput { value_atoms: 1, script_pubkey: p2pkh_from_address(&bob).unwrap() },
                TxOutput { value_atoms: 0, script_pubkey: op_return_spk(&xfer_op) },
            ],
        );
        idx.apply_block(&block_with(2, vec![xfer_tx]), 2);

        assert_eq!(idx.state.ft_balance(&alice, &asset_id), 750);
        assert_eq!(idx.state.ft_balance(&bob, &asset_id), 250);
        assert_eq!(idx.last_height, 2);
        // history recorded for both parties.
        assert_eq!(idx.history.get(&alice).map(|v| v.len()), Some(2)); // issue + transfer-from
        assert_eq!(idx.history.get(&bob).map(|v| v.len()), Some(1));   // transfer-to
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-indexer index::tests::apply_block_indexes_issue_then_transfer`
Expected: FAIL — `apply_block` not found.

- [ ] **Step 3: Implement `apply_block`**

Add to `impl Indexer` in `index.rs` (after `resolve_source`):
```rust
    /// Apply every tx in `block` in canonical order: record outputs, and for the
    /// first valid `TXMA` op, resolve source + dest and apply it to the asset state.
    pub fn apply_block(&mut self, block: &Block, height: u64) {
        use tensorium_core::assets::{extract_asset_op, ApplyResult, AssetOp};

        for tx in &block.transactions {
            // Resolve source BEFORE recording this tx's own outputs (a tx never
            // spends its own outputs; sources come from prior txs).
            let source = self.resolve_source(tx);

            if let Some(op) = extract_asset_op(tx) {
                if let Some(src) = source.as_deref() {
                    let dest = match &op {
                        AssetOp::Transfer(d) => tx
                            .outputs
                            .get(d.dest_output_index as usize)
                            .and_then(|o| extract_address(&o.script_pubkey)),
                        _ => None,
                    };
                    let result = self.state.apply(tx.id.0, height, src, dest.as_deref(), &op);
                    if result == ApplyResult::Applied {
                        self.record_history(height, tx.id.0, src, dest.as_deref(), &op);
                    }
                }
            }

            self.record_outputs(tx);
        }

        self.last_height = height;
        self.scanned_any = true;
    }

    fn record_history(
        &mut self,
        height: u64,
        txid: [u8; 32],
        source: &str,
        dest: Option<&str>,
        op: &tensorium_core::assets::AssetOp,
    ) {
        use tensorium_core::assets::AssetOp;
        let txid_hex = Hash256(txid).to_hex();
        let (kind, asset_id, to, amount) = match op {
            AssetOp::Issue(d) => ("issue", txid, String::new(), d.supply),
            AssetOp::NftMint(_) => ("nft_mint", txid, String::new(), 1),
            AssetOp::Transfer(d) => (
                "transfer",
                d.asset_id,
                dest.unwrap_or("").to_string(),
                d.amount,
            ),
        };
        let entry = HistoryEntry {
            height,
            txid: txid_hex,
            op: kind.to_string(),
            asset_id: Hash256(asset_id).to_hex(),
            from: source.to_string(),
            to: to.clone(),
            amount,
        };
        self.history.entry(source.to_string()).or_default().push(entry.clone());
        if !to.is_empty() && to != source {
            self.history.entry(to).or_default().push(entry);
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-indexer index::tests::apply_block_indexes_issue_then_transfer`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-indexer/src/index.rs
git commit -m "feat(indexer): apply_block extract+apply asset ops + history"
```

---

### Task 5: Snapshot persistence (atomic JSON)

**Files:**
- Modify: `crates/tensorium-indexer/src/store.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/tensorium-indexer/src/store.rs` with:
```rust
use crate::index::{HistoryEntry, Indexer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tensorium_core::assets::{AssetInfo, AssetKind};
use tensorium_core::hash::Hash256;

/// Serializable mirror of `Indexer` with hex-encoded keys (JSON-safe).
#[derive(Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub last_height: u64,
    pub last_hash: String,
    pub scanned_any: bool,
    /// asset_id hex -> info
    pub assets: HashMap<String, AssetInfoJson>,
    /// "<addr>|<asset_hex>" -> balance
    pub ft_balances: HashMap<String, u64>,
    /// asset_id hex -> owner address
    pub nft_owner: HashMap<String, String>,
    /// "<txid_hex>:<vout>" -> address
    pub outpoints: HashMap<String, String>,
    /// address -> events
    pub history: HashMap<String, Vec<HistoryEntry>>,
}

#[derive(Serialize, Deserialize)]
pub struct AssetInfoJson {
    pub kind: String, // "ft" | "nft"
    pub ticker: String,
    pub name: String,
    pub decimals: u8,
    pub supply: u64,
    pub issuer: String,
    pub royalty_bps: u16,
    pub royalty_addr: String,
    pub uri: String,
    pub content_hash: String, // hex
    pub mint_height: u64,
}

fn hex32(b: &[u8; 32]) -> String {
    Hash256(*b).to_hex()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let bytes = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok());
    let mut out = [0u8; 32];
    let mut n = 0;
    for b in bytes {
        let b = b?;
        if n >= 32 {
            return None;
        }
        out[n] = b;
        n += 1;
    }
    if n == 32 {
        Some(out)
    } else {
        None
    }
}

impl Snapshot {
    /// Build a snapshot from live indexer state.
    pub fn from_indexer(idx: &Indexer) -> Self {
        let mut assets = HashMap::new();
        for (id, info) in &idx.state.assets {
            assets.insert(
                hex32(id),
                AssetInfoJson {
                    kind: match info.kind {
                        AssetKind::Fungible => "ft".into(),
                        AssetKind::NonFungible => "nft".into(),
                    },
                    ticker: info.ticker.clone(),
                    name: info.name.clone(),
                    decimals: info.decimals,
                    supply: info.supply,
                    issuer: info.issuer.clone(),
                    royalty_bps: info.royalty_bps,
                    royalty_addr: info.royalty_addr.clone(),
                    uri: info.uri.clone(),
                    content_hash: hex32(&info.content_hash),
                    mint_height: info.mint_height,
                },
            );
        }
        let mut ft_balances = HashMap::new();
        for ((addr, id), bal) in &idx.state.ft_balances {
            ft_balances.insert(format!("{addr}|{}", hex32(id)), *bal);
        }
        let nft_owner =
            idx.state.nft_owner.iter().map(|(id, o)| (hex32(id), o.clone())).collect();

        Snapshot {
            last_height: idx.last_height,
            last_hash: idx.last_hash.clone(),
            scanned_any: idx.scanned_any,
            assets,
            ft_balances,
            nft_owner,
            outpoints: idx.outpoints.clone(),
            history: idx.history.clone(),
        }
    }

    /// Rehydrate indexer state from this snapshot.
    pub fn apply_to(self, idx: &mut Indexer) {
        idx.last_height = self.last_height;
        idx.last_hash = self.last_hash;
        idx.scanned_any = self.scanned_any;
        idx.outpoints = self.outpoints;
        idx.history = self.history;
        for (id_hex, info) in self.assets {
            let Some(id) = unhex32(&id_hex) else { continue };
            idx.state.assets.insert(
                id,
                AssetInfo {
                    kind: if info.kind == "nft" { AssetKind::NonFungible } else { AssetKind::Fungible },
                    ticker: info.ticker,
                    name: info.name,
                    decimals: info.decimals,
                    supply: info.supply,
                    issuer: info.issuer,
                    royalty_bps: info.royalty_bps,
                    royalty_addr: info.royalty_addr,
                    uri: info.uri,
                    content_hash: unhex32(&info.content_hash).unwrap_or([0u8; 32]),
                    mint_height: info.mint_height,
                },
            );
        }
        for (key, bal) in self.ft_balances {
            if let Some((addr, id_hex)) = key.rsplit_once('|') {
                if let Some(id) = unhex32(id_hex) {
                    idx.state.ft_balances.insert((addr.to_string(), id), bal);
                }
            }
        }
        for (id_hex, owner) in self.nft_owner {
            if let Some(id) = unhex32(&id_hex) {
                idx.state.nft_owner.insert(id, owner);
            }
        }
    }
}

/// Atomically write the snapshot to `path` (tmp + rename).
pub fn save(idx: &Indexer, path: &str) -> Result<(), String> {
    let snap = Snapshot::from_indexer(idx);
    let json = serde_json::to_string(&snap).map_err(|e| format!("serialize: {e}"))?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp}->{path}: {e}"))
}

/// Load a snapshot from `path` into a fresh indexer. Missing file → empty indexer.
pub fn load(path: &str) -> Result<Indexer, String> {
    let mut idx = Indexer::default();
    match std::fs::read_to_string(path) {
        Ok(json) => {
            let snap: Snapshot =
                serde_json::from_str(&json).map_err(|e| format!("parse snapshot: {e}"))?;
            snap.apply_to(&mut idx);
            Ok(idx)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(idx),
        Err(e) => Err(format!("read {path}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tensorium_core::assets::{AssetOp, IssueData};

    #[test]
    fn snapshot_roundtrip_preserves_balances() {
        let mut idx = Indexer::default();
        let txid = [3u8; 32];
        idx.state.apply(
            txid,
            5,
            "txm1alice",
            None,
            &AssetOp::Issue(IssueData {
                ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
            }),
        );
        idx.outpoints.insert("deadbeef:0".into(), "txm1alice".into());
        idx.last_height = 5;
        idx.last_hash = "abc".into();
        idx.scanned_any = true;

        let json = serde_json::to_string(&Snapshot::from_indexer(&idx)).unwrap();
        let snap: Snapshot = serde_json::from_str(&json).unwrap();
        let mut restored = Indexer::default();
        snap.apply_to(&mut restored);

        assert_eq!(restored.last_height, 5);
        assert_eq!(restored.last_hash, "abc");
        assert_eq!(restored.state.ft_balance("txm1alice", &txid), 1000);
        assert_eq!(restored.state.assets.get(&txid).unwrap().ticker, "GOLD");
        assert_eq!(restored.outpoints.get("deadbeef:0").unwrap(), "txm1alice");
    }

    #[test]
    fn load_missing_file_is_empty_indexer() {
        let idx = load("/tmp/does-not-exist-txm-indexer.json").unwrap();
        assert!(!idx.scanned_any);
        assert_eq!(idx.last_height, 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tensorium-indexer store::tests`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add crates/tensorium-indexer/src/store.rs
git commit -m "feat(indexer): atomic JSON snapshot persistence (hex-keyed mirror)"
```

---

### Task 6: Reorg decision + scan driver

**Files:**
- Modify: `crates/tensorium-indexer/src/sync.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/tensorium-indexer/src/sync.rs` with:
```rust
use crate::index::Indexer;
use tensorium_core::block::Block;
use tensorium_core::hash::Hash256;

/// Decide whether the indexer must rebuild from genesis. A rebuild is needed when
/// the indexer has scanned before AND the block hash now reported at its
/// `last_height` differs from the one it recorded (a reorg below the tip).
pub fn needs_rebuild(idx: &Indexer, hash_at_last_height: &str) -> bool {
    idx.scanned_any && idx.last_height > 0 && idx.last_hash != hash_at_last_height
}

/// Drive a scan up to `tip`, fetching blocks via `fetch(height) -> (hash, block)`.
/// On reorg (detected at `last_height`) the indexer is reset and rebuilt from 0.
/// Returns the number of blocks applied this call.
pub fn scan<F>(idx: &mut Indexer, tip: u64, mut fetch: F) -> Result<u64, String>
where
    F: FnMut(u64) -> Result<(Hash256, Block), String>,
{
    // Reorg check at the last scanned height.
    if idx.scanned_any && idx.last_height > 0 {
        let (hash, _) = fetch(idx.last_height)?;
        if needs_rebuild(idx, &hash.to_hex()) {
            *idx = Indexer::default();
        }
    }

    let start = if idx.scanned_any { idx.last_height + 1 } else { 0 };
    let mut applied = 0;
    for height in start..=tip {
        let (hash, block) = fetch(height)?;
        idx.apply_block(&block, height);
        idx.last_hash = hash.to_hex();
        applied += 1;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tensorium_core::block::{Block, BlockHeader, Transaction};

    fn block(height: u64, prev: u8) -> (Hash256, Block) {
        let header = BlockHeader {
            version: 1,
            chain_id: "test".into(),
            height,
            previous_hash: Hash256([prev; 32]),
            merkle_root: Hash256([0u8; 32]),
            timestamp_seconds: 0,
            leading_zero_bits: 0,
            nonce: 0,
        };
        // Deterministic synthetic hash per (height, prev).
        let hash = Hash256([height as u8 ^ prev; 32]);
        (hash, Block::new(header, vec![Transaction::coinbase(height, 1, "txm1m")]))
    }

    #[test]
    fn scans_forward_then_incrementally() {
        let chain: HashMap<u64, (Hash256, Block)> =
            (0..=3).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();

        let applied = scan(&mut idx, 2, |h| Ok(chain[&h].clone())).unwrap();
        assert_eq!(applied, 3); // heights 0,1,2
        assert_eq!(idx.last_height, 2);

        // Tip advances to 3: only the new block is applied.
        let applied = scan(&mut idx, 3, |h| Ok(chain[&h].clone())).unwrap();
        assert_eq!(applied, 1);
        assert_eq!(idx.last_height, 3);
    }

    #[test]
    fn reorg_below_tip_triggers_full_rebuild() {
        // First scan on chain A.
        let chain_a: HashMap<u64, (Hash256, Block)> =
            (0..=2).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();
        scan(&mut idx, 2, |h| Ok(chain_a[&h].clone())).unwrap();
        assert_eq!(idx.last_height, 2);

        // Chain B replaces history (different prev byte → different hashes).
        let chain_b: HashMap<u64, (Hash256, Block)> =
            (0..=2).map(|h| (h, block(h, 9))).collect();
        let applied = scan(&mut idx, 2, |h| Ok(chain_b[&h].clone())).unwrap();
        assert_eq!(applied, 3); // rebuilt from 0
        assert_eq!(idx.last_hash, chain_b[&2].0.to_hex());
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tensorium-indexer sync::tests`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add crates/tensorium-indexer/src/sync.rs
git commit -m "feat(indexer): reorg-aware scan driver (injected block fetch)"
```

---

### Task 7: REST API router

**Files:**
- Modify: `crates/tensorium-indexer/src/api.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/tensorium-indexer/src/api.rs` with:
```rust
use crate::index::Indexer;
use serde_json::{json, Value};
use tensorium_core::assets::AssetKind;
use tensorium_core::hash::Hash256;

/// Pure router: map (method, path) over the current indexer state to (status, body).
/// Read-only. Unknown routes → 404.
pub fn route(idx: &Indexer, method: &str, path: &str) -> (u16, Value) {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("GET", ["status"]) => (
            200,
            json!({
                "last_scanned_height": idx.last_height,
                "scanned_any": idx.scanned_any,
                "assets": idx.state.assets.len(),
            }),
        ),
        ("GET", ["asset", id]) => match parse_id(id).and_then(|k| idx.state.assets.get(&k)) {
            Some(info) => (200, asset_json(id, info)),
            None => (404, json!({ "error": "asset not found" })),
        },
        ("GET", ["nft", id, "owner"]) => {
            match parse_id(id).and_then(|k| idx.state.nft_owner.get(&k)) {
                Some(owner) => (200, json!({ "asset_id": id, "owner": owner })),
                None => (404, json!({ "error": "nft not found" })),
            }
        }
        ("GET", ["balance", addr]) => {
            let mut fts = vec![];
            for ((a, id), bal) in &idx.state.ft_balances {
                if a == addr && *bal > 0 {
                    fts.push(json!({ "asset_id": Hash256(*id).to_hex(), "amount": bal }));
                }
            }
            let nfts: Vec<String> = idx
                .state
                .nft_owner
                .iter()
                .filter(|(_, o)| o.as_str() == *addr)
                .map(|(id, _)| Hash256(*id).to_hex())
                .collect();
            (200, json!({ "address": addr, "fungible": fts, "nfts": nfts }))
        }
        ("GET", ["holders", id]) => match parse_id(id) {
            Some(key) => {
                let mut holders = vec![];
                for ((a, aid), bal) in &idx.state.ft_balances {
                    if *aid == key && *bal > 0 {
                        holders.push(json!({ "address": a, "amount": bal }));
                    }
                }
                if let Some(owner) = idx.state.nft_owner.get(&key) {
                    holders.push(json!({ "address": owner, "amount": 1 }));
                }
                (200, json!({ "asset_id": id, "holders": holders }))
            }
            None => (404, json!({ "error": "bad asset id" })),
        },
        ("GET", ["history", addr]) => {
            let events = idx.history.get(*addr).cloned().unwrap_or_default();
            (200, json!({ "address": addr, "events": events }))
        }
        ("GET", ["assets"]) => {
            let list: Vec<Value> = idx
                .state
                .assets
                .iter()
                .map(|(id, info)| asset_json(&Hash256(*id).to_hex(), info))
                .collect();
            (200, json!({ "assets": list }))
        }
        _ => (404, json!({ "error": "not found" })),
    }
}

fn parse_id(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn asset_json(id: &str, info: &tensorium_core::assets::AssetInfo) -> Value {
    json!({
        "asset_id": id,
        "kind": match info.kind { AssetKind::Fungible => "ft", AssetKind::NonFungible => "nft" },
        "ticker": info.ticker,
        "name": info.name,
        "decimals": info.decimals,
        "supply": info.supply,
        "issuer": info.issuer,
        "royalty_bps": info.royalty_bps,
        "royalty_addr": info.royalty_addr,
        "uri": info.uri,
        "content_hash": Hash256(info.content_hash).to_hex(),
        "mint_height": info.mint_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tensorium_core::assets::{AssetOp, IssueData, NftMintData, TransferData};

    fn seeded() -> (Indexer, String) {
        let mut idx = Indexer::default();
        let gold = [1u8; 32];
        idx.state.apply(
            gold, 1, "txm1alice", None,
            &AssetOp::Issue(IssueData { ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0 }),
        );
        idx.state.apply(
            [2u8; 32], 2, "txm1alice", Some("txm1bob"),
            &AssetOp::Transfer(TransferData { asset_id: gold, amount: 400, dest_output_index: 0 }),
        );
        let nft = [7u8; 32];
        idx.state.apply(
            nft, 3, "txm1alice", None,
            &AssetOp::NftMint(NftMintData { collection_id: [0u8; 32], royalty_bps: 500, royalty_addr: "txm1alice".into(), uri: "ipfs://x".into(), content_hash: [9u8; 32] }),
        );
        idx.last_height = 3;
        idx.scanned_any = true;
        (idx, Hash256(gold).to_hex())
    }

    #[test]
    fn status_reports_height_and_count() {
        let (idx, _) = seeded();
        let (code, body) = route(&idx, "GET", "/status");
        assert_eq!(code, 200);
        assert_eq!(body["last_scanned_height"], 3);
        assert_eq!(body["assets"], 2);
    }

    #[test]
    fn asset_and_balance_and_holders() {
        let (idx, gold) = seeded();

        let (code, body) = route(&idx, "GET", &format!("/asset/{gold}"));
        assert_eq!(code, 200);
        assert_eq!(body["ticker"], "GOLD");
        assert_eq!(body["supply"], 1000);

        let (code, body) = route(&idx, "GET", "/balance/txm1bob");
        assert_eq!(code, 200);
        assert_eq!(body["fungible"][0]["amount"], 400);

        let (code, body) = route(&idx, "GET", &format!("/holders/{gold}"));
        assert_eq!(code, 200);
        assert_eq!(body["holders"].as_array().unwrap().len(), 2); // alice 600 + bob 400

        let (code, _) = route(&idx, "GET", "/asset/deadbeef");
        assert_eq!(code, 404);
    }

    #[test]
    fn nft_owner_route() {
        let (idx, _) = seeded();
        let nft = Hash256([7u8; 32]).to_hex();
        let (code, body) = route(&idx, "GET", &format!("/nft/{nft}/owner"));
        assert_eq!(code, 200);
        assert_eq!(body["owner"], "txm1alice");
    }

    #[test]
    fn unknown_route_404() {
        let (idx, _) = seeded();
        assert_eq!(route(&idx, "GET", "/nope").0, 404);
        assert_eq!(route(&idx, "POST", "/status").0, 404);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tensorium-indexer api::tests`
Expected: PASS.

- [ ] **Step 3: Add the thin socket server**

Append to `crates/tensorium-indexer/src/api.rs` (above `#[cfg(test)]`):
```rust
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Serve the read-only REST API, reading shared indexer state under a mutex.
pub fn serve(bind: &str, idx: Arc<Mutex<Indexer>>) -> Result<(), String> {
    let listener = TcpListener::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    println!("txm-asset-indexer API on http://{bind}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let mut line = req.lines().next().unwrap_or("").split_whitespace();
        let method = line.next().unwrap_or("");
        let path = line.next().unwrap_or("/");
        let (code, body) = {
            let guard = idx.lock().map_err(|e| format!("lock: {e}"))?;
            route(&guard, method, path)
        };
        let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into());
        let status_text = if code == 200 { "OK" } else { "Not Found" };
        let resp = format!(
            "HTTP/1.1 {code} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
            text.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    }
    Ok(())
}
```

- [ ] **Step 4: Verify it still builds + tests pass**

Run: `cargo test -p tensorium-indexer api::tests`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-indexer/src/api.rs
git commit -m "feat(indexer): read-only REST API router + socket server"
```

---

### Task 8: main.rs wiring + full-suite verification

**Files:**
- Modify: `crates/tensorium-indexer/src/main.rs`

- [ ] **Step 1: Implement main (arg/env wiring + scanner thread + API)**

Replace `crates/tensorium-indexer/src/main.rs` with:
```rust
//! txm-asset-indexer — Layer 2 of the TXM asset marketplace.
//!
//! Env / args:
//!   TXM_INDEXER_RPC   node RPC host:port      (default 127.0.0.1:33332)
//!   TXM_INDEXER_BIND  REST API bind           (default 127.0.0.1:23340)
//!   TXM_INDEXER_DB    snapshot path           (default ./txm-asset-index.json)
//!   TXM_INDEXER_POLL  poll seconds            (default 10)
mod api;
mod index;
mod rpc;
mod store;
mod sync;

use rpc::NodeRpc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() -> Result<(), String> {
    let rpc_addr = env_or("TXM_INDEXER_RPC", "127.0.0.1:33332");
    let bind = env_or("TXM_INDEXER_BIND", "127.0.0.1:23340");
    let db = env_or("TXM_INDEXER_DB", "./txm-asset-index.json");
    let poll: u64 = env_or("TXM_INDEXER_POLL", "10").parse().unwrap_or(10);

    let idx = Arc::new(Mutex::new(store::load(&db)?));
    println!("loaded snapshot: last_height={}", idx.lock().unwrap().last_height);

    // Scanner thread.
    let scan_idx = Arc::clone(&idx);
    let rpc = NodeRpc::new(rpc_addr.clone());
    let db_path = db.clone();
    thread::spawn(move || loop {
        if let Err(e) = tick(&scan_idx, &rpc, &db_path) {
            eprintln!("scan tick error: {e}");
        }
        thread::sleep(Duration::from_secs(poll));
    });

    // REST API on the main thread.
    api::serve(&bind, idx)
}

/// One scan cycle: fetch tip, scan to it, persist if anything changed.
fn tick(idx: &Arc<Mutex<index::Indexer>>, rpc: &NodeRpc, db: &str) -> Result<(), String> {
    let Some(tip) = rpc.block_count()? else {
        return Ok(()); // empty chain
    };
    let mut guard = idx.lock().map_err(|e| format!("lock: {e}"))?;
    if guard.scanned_any && guard.last_height >= tip {
        return Ok(());
    }
    let applied = sync::scan(&mut guard, tip, |h| rpc.block_at(h))?;
    if applied > 0 {
        store::save(&guard, db)?;
        println!("scanned to height {} (+{applied})", guard.last_height);
    }
    Ok(())
}
```

- [ ] **Step 2: Build the crate**

Run: `cargo build -p tensorium-indexer`
Expected: builds (no errors).

- [ ] **Step 3: Run the indexer's full test suite**

Run: `cargo test -p tensorium-indexer`
Expected: all `rpc`/`index`/`store`/`sync`/`api` tests pass.

- [ ] **Step 4: Run the whole workspace suite (no regressions)**

Run: `cargo test --workspace`
Expected: all pass, including `tensorium-core::assets::*` and the new `tensorium-indexer` tests.

- [ ] **Step 5: Manual smoke test against the live node (optional, read-only)**

```bash
# Point at the live MC node RPC (read-only; indexer never holds keys).
TXM_INDEXER_RPC=127.0.0.1:33332 TXM_INDEXER_DB=/tmp/txm-idx.json \
  cargo run -p tensorium-indexer &
sleep 15
curl -s http://127.0.0.1:23340/status        # last_scanned_height climbs toward tip
curl -s http://127.0.0.1:23340/assets         # empty until the first ISSUE is mined
kill %1
```
Expected: `/status` shows `last_scanned_height` approaching the chain tip; `/assets` is an empty list (no asset ops on-chain yet).

- [ ] **Step 6: Commit**
```bash
git add crates/tensorium-indexer/src/main.rs Cargo.lock
git commit -m "feat(indexer): wire scanner thread + REST server in main"
```

---

## Done criteria

- `txm-asset-indexer` builds and runs: scans canonical blocks from the node RPC, applies `tensorium-core::assets` ops, persists a JSON snapshot atomically, rebuilds on reorg, and serves `/status`, `/asset/<id>`, `/balance/<addr>`, `/nft/<id>/owner`, `/holders/<id>`, `/history/<addr>`, `/assets`.
- TDD coverage: RPC response parsing, outpoint index + source resolution, `apply_block` (issue→transfer with history), snapshot round-trip + missing-file load, reorg-aware scan driver (forward + rebuild), and every API route incl. 404s.
- `cargo test --workspace` green; the indexer holds no keys/funds; the node is unmodified (no consensus change).

## Next plan (Layer 3 — wallet asset commands)

Wallet CLI commands in `txmwallet` to *build* asset txs (issue / mint / transfer) — fund `inputs[0]` from the asset owner's address, attach the `TXMA` `OP_RETURN`, and a destination P2PKH for transfers — reusing `tensorium-core::assets::encode_op`. Then Layer 4 (marketplace + escrow + 2.5% platform fee + royalty enforcement) and Layer 5 (frontend).
