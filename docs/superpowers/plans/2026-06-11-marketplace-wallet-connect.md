# Marketplace Wallet-Connect + TXM20/NFT Creation (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users connect their Tensorium wallet extension to marketplace.tensoriumlabs.com and issue TXM20 tokens, mint NFTs, and transfer either, entirely from the browser.

**Architecture:** A new `POST /buildAssetTx` endpoint on the node's existing public RPC (`rpc.tensoriumlabs.com`) builds an unsigned transaction (UTXO selection + `OP_RETURN` asset payload via the shared `tensorium_core::assets` codec, with a balance/ownership check against the read-only `txm-asset-indexer` for transfers). The wallet extension gains `signAssetTx`/`getAssets` on `window.tensorium`, signing with its existing TS secp256k1 implementation and broadcasting via `/sendrawtransaction`. The marketplace UI adds Connect Wallet, Create Token, Mint NFT, Transfer, and My Assets.

**Tech Stack:** Rust (tensorium-core, tensorium-node, tensorium-indexer, txmwallet), TypeScript (tensorium-wallet-extension, Vite/React popup), static HTML/CSS/JS (tensorium-sites/marketplace).

**Spec:** `docs/superpowers/specs/2026-06-11-marketplace-wallet-connect-design.md`

---

## Task 1: Shared `build_outputs` helper in `tensorium-core`

Extract the output-building logic that `txmwallet`'s `build_asset_outputs` already implements into `tensorium-core::assets`, so both `txmwallet` and the new node RPC endpoint use one implementation.

**Files:**
- Modify: `crates/tensorium-core/src/assets/mod.rs`
- Test: `crates/tensorium-core/src/assets/mod.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/tensorium-core/src/assets/mod.rs` (create a `#[cfg(test)] mod tests` block if one doesn't already exist in this file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::TxOutput;
    use crate::script::standard::{op_return_script_for_test_helper, p2pkh_from_address};

    fn issue_op() -> AssetOp {
        AssetOp::Issue(IssueData {
            ticker: "GOLD".into(),
            decimals: 0,
            supply: 1_000_000,
            name: "Gold Token".into(),
            flags: 0,
        })
    }

    #[test]
    fn build_outputs_issue_no_dest_has_op_return_and_change() {
        let op = issue_op();
        let change_addr = "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38";
        let outputs = build_outputs(&op, None, change_addr, 1_000_000, 100_000).unwrap();
        // [OP_RETURN, change] — no dest output for Issue with dest=None
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].value_atoms, 0);
        assert_eq!(outputs[0].script_pubkey, op_return_script(&encode_op(&op)));
        assert_eq!(outputs[1].value_atoms, 900_000);
        assert_eq!(outputs[1].script_pubkey, p2pkh_from_address(change_addr).unwrap());
    }

    #[test]
    fn build_outputs_transfer_with_dest_and_change() {
        let op = AssetOp::Transfer(TransferData {
            asset_id: [7u8; 32],
            amount: 50,
            dest_output_index: 0,
        });
        let to_addr = "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38";
        let change_addr = "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38";
        let outputs = build_outputs(&op, Some((to_addr, 1_000)), change_addr, 200_000, 100_000).unwrap();
        // [dest, OP_RETURN, change]
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].value_atoms, 1_000);
        assert_eq!(outputs[0].script_pubkey, p2pkh_from_address(to_addr).unwrap());
        assert_eq!(outputs[1].value_atoms, 0);
        assert_eq!(outputs[2].value_atoms, 98_900); // 200_000 - 1_000 - 100_000
    }

    #[test]
    fn build_outputs_insufficient_funds_errors() {
        let op = issue_op();
        let change_addr = "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38";
        let err = build_outputs(&op, None, change_addr, 50_000, 100_000).unwrap_err();
        assert!(err.contains("insufficient"), "unexpected error: {err}");
    }

    #[test]
    fn build_outputs_invalid_dest_address_errors() {
        let op = issue_op();
        let change_addr = "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38";
        let err = build_outputs(&op, Some(("not-an-address", 1_000)), change_addr, 1_000_000, 100_000)
            .unwrap_err();
        assert!(err.contains("invalid recipient"), "unexpected error: {err}");
    }
}
```

Note: the `op_return_script_for_test_helper` import above is a placeholder name to make the test file self-checking against typos — replace it with the real re-exported `op_return_script` (already re-exported at the top of `mod.rs` via `pub use codec::{decode_op, encode_op, extract_asset_op, op_return_script};`). The corrected import line is:

```rust
    use crate::script::standard::p2pkh_from_address;
```

(drop the bogus `op_return_script_for_test_helper` import entirely — `op_return_script` and `encode_op` are already in scope via `use super::*;`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /root/tensorium-core && cargo test -p tensorium-core assets::tests:: 2>&1 | tail -20`
Expected: FAIL with `cannot find function 'build_outputs' in module 'crate::assets'`

- [ ] **Step 3: Implement `build_outputs`**

Add to `crates/tensorium-core/src/assets/mod.rs` (above the `#[cfg(test)]` block, after the existing type/error definitions):

```rust
use crate::block::TxOutput;
use crate::script::standard::p2pkh_from_address;

/// Build the output set for an asset-bearing transaction:
/// optional `dest` (recipient carrier output), an `OP_RETURN` carrying the
/// encoded asset op, and change back to `change_addr` if any remains.
///
/// `total_in` is the sum of selected input values; `fee_atoms` is the flat
/// network fee. Returns a human-readable error string on invalid input —
/// shared by `txmwallet` (CLI) and the node's `/buildAssetTx` RPC endpoint.
pub fn build_outputs(
    op: &AssetOp,
    dest: Option<(&str, u64)>,
    change_addr: &str,
    total_in: u64,
    fee_atoms: u64,
) -> Result<Vec<TxOutput>, String> {
    let dest_atoms = dest.map(|(_, a)| a).unwrap_or(0);
    let spent = dest_atoms.saturating_add(fee_atoms);
    if total_in < spent {
        return Err(format!(
            "insufficient mature balance: have {total_in}, need {spent} (carrier {dest_atoms} + fee {fee_atoms})"
        ));
    }

    let mut outputs = Vec::new();
    if let Some((addr, atoms)) = dest {
        outputs.push(TxOutput {
            value_atoms: atoms,
            script_pubkey: p2pkh_from_address(addr)
                .map_err(|_| format!("invalid recipient address: {addr}"))?,
        });
    }
    outputs.push(TxOutput {
        value_atoms: 0,
        script_pubkey: op_return_script(&encode_op(op)),
    });
    let change = total_in - spent;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(change_addr)
                .map_err(|_| format!("invalid change address: {change_addr}"))?,
        });
    }
    Ok(outputs)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /root/tensorium-core && cargo test -p tensorium-core assets::tests:: 2>&1 | tail -20`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Commit**

```bash
cd /root/tensorium-core
git add crates/tensorium-core/src/assets/mod.rs
git commit -m "feat(assets): extract shared build_outputs helper for asset tx construction"
```

---

## Task 2: Refactor `txmwallet` to use the shared `build_outputs`

Remove the duplicate `build_asset_outputs` from `txmwallet` and call `tensorium_core::assets::build_outputs` instead. This is a pure refactor — existing `txmwallet` tests (asset-issue/mint/transfer CLI behavior) must keep passing unchanged.

**Files:**
- Modify: `crates/txmwallet/src/main.rs:1368-1409` (the `build_asset_outputs` function and its call site in `build_asset_tx_via_rpc`)

- [ ] **Step 1: Confirm current tests pass before refactor (baseline)**

Run: `cd /root/tensorium-core && cargo test -p txmwallet 2>&1 | tail -15`
Expected: `test result: ok.` (note the pass count to compare after)

- [ ] **Step 2: Replace `build_asset_outputs` with a thin call to the shared helper**

In `crates/txmwallet/src/main.rs`, delete the existing `build_asset_outputs` function body (lines 1368-1409) and replace the whole function with:

```rust
fn build_asset_outputs(
    op: &AssetOp,
    dest: Option<(&str, u64)>,
    change_addr: &str,
    total_in: u64,
    fee_atoms: u64,
) -> Result<Vec<TxOutput>, String> {
    tensorium_core::assets::build_outputs(op, dest, change_addr, total_in, fee_atoms)
}
```

(Keep the function name and signature identical so `build_asset_tx_via_rpc` at line 1465 needs no change.)

- [ ] **Step 3: Run tests to verify nothing broke**

Run: `cd /root/tensorium-core && cargo test -p txmwallet 2>&1 | tail -15`
Expected: same `test result: ok.` pass count as Step 1.

- [ ] **Step 4: Commit**

```bash
cd /root/tensorium-core
git add crates/txmwallet/src/main.rs
git commit -m "refactor(txmwallet): use shared tensorium_core::assets::build_outputs"
```

---

## Task 3: `POST /buildAssetTx` RPC endpoint on `tensorium-node`

Add the endpoint that the marketplace/extension call to get an unsigned asset transaction.

**Files:**
- Modify: `crates/tensorium-node/src/main.rs` (add handler in `handle_rpc_stream`'s match arm, plus a small indexer-client helper)
- Test: `crates/tensorium-node/src/main.rs` (inline `#[cfg(test)] mod tests`, around line 2017)

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` block in `crates/tensorium-node/src/main.rs` (near line 2017, alongside the existing `/getutxos` path tests):

```rust
    #[test]
    fn build_asset_tx_request_parses_issue() {
        let body = r#"{"op":"issue","from":"txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38","ticker":"GOLD","decimals":0,"supply":1000000,"name":"Gold Token"}"#;
        let req: BuildAssetTxRequest = serde_json::from_str(body).unwrap();
        match req {
            BuildAssetTxRequest::Issue { from, ticker, decimals, supply, name } => {
                assert_eq!(from, "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38");
                assert_eq!(ticker, "GOLD");
                assert_eq!(decimals, 0);
                assert_eq!(supply, 1_000_000);
                assert_eq!(name, "Gold Token");
            }
            _ => panic!("expected Issue variant"),
        }
    }

    #[test]
    fn build_asset_tx_request_parses_transfer() {
        let body = r#"{"op":"transfer","from":"txm1a","to":"txm1b","asset_id":"0707070707070707070707070707070707070707070707070707070707070707","amount":50}"#;
        // asset_id above is 33 bytes hex (66 chars) on purpose to exercise the length check below
        let req: BuildAssetTxRequest = serde_json::from_str(body).unwrap();
        match req {
            BuildAssetTxRequest::Transfer { from, to, asset_id, amount } => {
                assert_eq!(from, "txm1a");
                assert_eq!(to, "txm1b");
                assert_eq!(amount, 50);
                // 33 bytes of hex decodes fine; the *length* check (must be 32 bytes)
                // happens in the handler, not at parse time.
                assert_eq!(hex::decode(&asset_id).unwrap().len(), 33);
            }
            _ => panic!("expected Transfer variant"),
        }
    }

    #[test]
    fn build_asset_tx_request_rejects_unknown_op() {
        let body = r#"{"op":"frobnicate","from":"txm1a"}"#;
        let result: Result<BuildAssetTxRequest, _> = serde_json::from_str(body);
        assert!(result.is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /root/tensorium-core && cargo test -p tensorium-node build_asset_tx_request 2>&1 | tail -20`
Expected: FAIL with `cannot find type 'BuildAssetTxRequest' in this scope`

- [ ] **Step 3: Define the request type and the `/buildAssetTx` handler**

Add near the top of `crates/tensorium-node/src/main.rs`, after the existing `use tensorium_core::{...}` block (around line 23), import the asset types:

```rust
use tensorium_core::assets::{build_outputs, encode_op, AssetOp, IssueData, NftMintData, TransferData};
```

Add a new tagged-enum request type just above `fn handle_rpc_stream` (search for `fn handle_rpc_stream(` around line 1558):

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum BuildAssetTxRequest {
    Issue {
        from: String,
        ticker: String,
        decimals: u8,
        supply: u64,
        name: String,
    },
    NftMint {
        from: String,
        #[serde(default)]
        collection_id: Option<String>, // 64-hex, defaults to all-zero (standalone)
        royalty_bps: u16,
        royalty_addr: String,
        uri: String,
        content_hash: String, // 64-hex sha256
    },
    Transfer {
        from: String,
        to: String,
        asset_id: String, // 64-hex
        amount: u64,
    },
}
```

Add the handler arm in `handle_rpc_stream`'s match, just before the final `_ => write_json_response(stream, 404, ...)` arm (around line 1906):

```rust
        ("POST", "/buildAssetTx") => {
            let req: BuildAssetTxRequest = match serde_json::from_str(parsed.body) {
                Ok(r) => r,
                Err(err) => {
                    return write_json_response(
                        stream,
                        400,
                        &RpcError::new(&format!("invalid request: {err}")),
                    )
                }
            };

            const ASSET_CARRIER_ATOMS: u64 = 1_000;
            let fee_atoms = tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;

            let (from, op, dest): (String, AssetOp, Option<(String, u64)>) = match req {
                BuildAssetTxRequest::Issue { from, ticker, decimals, supply, name } => {
                    if ticker.as_bytes().len() > 8 {
                        return write_json_response(stream, 400, &RpcError::new("ticker must be <= 8 bytes"));
                    }
                    if name.as_bytes().len() > 32 {
                        return write_json_response(stream, 400, &RpcError::new("name must be <= 32 bytes"));
                    }
                    if decimals > 18 {
                        return write_json_response(stream, 400, &RpcError::new("decimals must be <= 18"));
                    }
                    if supply == 0 {
                        return write_json_response(stream, 400, &RpcError::new("supply must be > 0"));
                    }
                    (from, AssetOp::Issue(IssueData { ticker, decimals, supply, name, flags: 0 }), None)
                }
                BuildAssetTxRequest::NftMint { from, collection_id, royalty_bps, royalty_addr, uri, content_hash } => {
                    if royalty_bps > 10_000 {
                        return write_json_response(stream, 400, &RpcError::new("royalty_bps must be <= 10000"));
                    }
                    if uri.as_bytes().len() > 200 {
                        return write_json_response(stream, 400, &RpcError::new("uri must be <= 200 bytes"));
                    }
                    let content_hash_bytes = match hex::decode(&content_hash) {
                        Ok(b) if b.len() == 32 => b,
                        _ => return write_json_response(stream, 400, &RpcError::new("content_hash must be 32 bytes (64 hex chars)")),
                    };
                    let collection_id_bytes: [u8; 32] = match collection_id {
                        Some(hexstr) => match hex::decode(&hexstr) {
                            Ok(b) if b.len() == 32 => b.try_into().unwrap(),
                            _ => return write_json_response(stream, 400, &RpcError::new("collection_id must be 32 bytes (64 hex chars)")),
                        },
                        None => [0u8; 32],
                    };
                    (from, AssetOp::NftMint(NftMintData {
                        collection_id: collection_id_bytes,
                        royalty_bps,
                        royalty_addr,
                        uri,
                        content_hash: content_hash_bytes.try_into().unwrap(),
                    }), None)
                }
                BuildAssetTxRequest::Transfer { from, to, asset_id, amount } => {
                    if amount == 0 {
                        return write_json_response(stream, 400, &RpcError::new("amount must be > 0"));
                    }
                    let asset_id_bytes: [u8; 32] = match hex::decode(&asset_id) {
                        Ok(b) if b.len() == 32 => b.try_into().unwrap(),
                        _ => return write_json_response(stream, 400, &RpcError::new("asset_id must be 32 bytes (64 hex chars)")),
                    };
                    // Balance/ownership check against the read-only asset indexer.
                    match indexer_check_transfer(&from, &asset_id, amount) {
                        Ok(true) => {}
                        Ok(false) => return write_json_response(stream, 400, &RpcError::new("insufficient asset balance or not the NFT owner")),
                        Err(e) => return write_json_response(stream, 503, &RpcError::new(&format!("asset index unavailable: {e}"))),
                    }
                    (from.clone(), AssetOp::Transfer(TransferData { asset_id: asset_id_bytes, amount, dest_output_index: 0 }), Some((to, ASSET_CARRIER_ATOMS)))
                }
            };

            let script = match p2pkh_from_address(&from) {
                Ok(s) => s,
                Err(_) => return write_json_response(stream, 400, &RpcError::new("invalid 'from' address")),
            };

            let mut state = rpc_state(stream, state_path)?;
            state.ensure_utxo_synced(params).map_err(|e| e.to_string())?;
            let tip_height = state.height().unwrap_or(0);
            let needed = dest.as_ref().map(|(_, a)| *a).unwrap_or(0).saturating_add(fee_atoms);

            let mut inputs = Vec::new();
            let mut total_in = 0u64;
            for (outpoint, entry) in state.utxos_for_script(&script) {
                let mature = !entry.coinbase
                    || tip_height >= entry.created_height.saturating_add(params.coinbase_maturity_blocks);
                if !mature {
                    continue;
                }
                inputs.push(tensorium_core::block::TxInput {
                    previous_output: *outpoint,
                    signature_script: Vec::new(),
                });
                total_in = total_in.saturating_add(entry.output.value_atoms);
                if total_in >= needed {
                    break;
                }
            }
            if total_in < needed {
                return write_json_response(
                    stream,
                    400,
                    &RpcError::new(&format!("insufficient mature balance: have {total_in}, need {needed}")),
                );
            }

            let dest_ref = dest.as_ref().map(|(addr, atoms)| (addr.as_str(), *atoms));
            let outputs = match build_outputs(&op, dest_ref, &from, total_in, fee_atoms) {
                Ok(o) => o,
                Err(e) => return write_json_response(stream, 400, &RpcError::new(&e)),
            };

            let tx = Transaction::payment(inputs, outputs);
            let description = match &op {
                AssetOp::Issue(d) => format!("Issue token {} — supply {}, decimals {}", d.ticker, d.supply, d.decimals),
                AssetOp::NftMint(d) => format!("Mint NFT — {}", d.uri),
                AssetOp::Transfer(d) => format!("Transfer {} of asset {} to {}", d.amount, hex::encode(d.asset_id), dest.as_ref().unwrap().0),
            };

            write_json_response(
                stream,
                200,
                &json!({
                    "unsigned_tx": tx,
                    "summary": {
                        "op": match &op { AssetOp::Issue(_) => "issue", AssetOp::NftMint(_) => "nft_mint", AssetOp::Transfer(_) => "transfer" },
                        "description": description,
                        "fee_atoms": fee_atoms,
                    },
                }),
            )
        }
```

Add the indexer HTTP helper near the other free functions at the bottom of the file (e.g. just above `fn write_json_response`):

```rust
/// Query the local read-only asset indexer to check whether `from` can
/// perform a transfer of `amount` of `asset_id` (fungible balance, or NFT
/// ownership when `amount == 1` and the asset is non-fungible).
/// Returns `Ok(true)`/`Ok(false)` for a definitive answer, `Err` if the
/// indexer is unreachable or returns malformed data.
fn indexer_check_transfer(from: &str, asset_id: &str, amount: u64) -> Result<bool, String> {
    let indexer_base = env::var("TENSORIUM_INDEXER_URL").unwrap_or_else(|_| "127.0.0.1:23340".to_string());
    let body = http_get(&indexer_base, &format!("/balance/{from}"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad indexer response: {e}"))?;

    if let Some(nfts) = v.get("nfts").and_then(|n| n.as_array()) {
        if nfts.iter().any(|id| id.as_str() == Some(asset_id)) {
            return Ok(amount == 1);
        }
    }
    if let Some(fts) = v.get("fungible").and_then(|n| n.as_array()) {
        for ft in fts {
            if ft.get("asset_id").and_then(|i| i.as_str()) == Some(asset_id) {
                let bal = ft.get("amount").and_then(|a| a.as_u64()).unwrap_or(0);
                return Ok(bal >= amount);
            }
        }
    }
    Ok(false)
}

/// Minimal blocking HTTP/1.1 GET over a raw TCP socket — avoids adding an
/// HTTP client dependency for this one internal call to the indexer.
fn http_get(host_port: &str, path: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    let mut stream = TcpStream::connect(host_port).map_err(|e| format!("connect {host_port}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).map_err(|e| e.to_string())?;
    if !status_line.contains("200") {
        return Err(format!("indexer returned: {}", status_line.trim()));
    }
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 || line == "\r\n" {
            break;
        }
    }
    let mut body = String::new();
    use std::io::Read as _;
    reader.read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /root/tensorium-core && cargo test -p tensorium-node build_asset_tx_request 2>&1 | tail -20`
Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Build the full workspace to catch type errors in the new handler**

Run: `cd /root/tensorium-core && cargo build --release -p tensorium-node 2>&1 | tail -30`
Expected: builds cleanly (warnings OK, no errors)

- [ ] **Step 6: Commit**

```bash
cd /root/tensorium-core
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node-rpc): add POST /buildAssetTx (issue/nft_mint/transfer)"
```

---

## Task 4: Integration test — build, sign, broadcast, verify via indexer

End-to-end test on a local single-node chain: confirms `/buildAssetTx` output is sign-able by `WalletKeypair::sign_input` and accepted by `/sendrawtransaction`.

**Files:**
- Test: `crates/tensorium-node/tests/build_asset_tx.rs` (new integration test file)

- [ ] **Step 1: Write the integration test**

Create `crates/tensorium-node/tests/build_asset_tx.rs`:

```rust
//! Integration test: /buildAssetTx → sign → /sendrawtransaction round trip
//! on a local node bound to an ephemeral port with a fresh state dir.
use std::process::{Child, Command};
use std::time::Duration;
use tensorium_core::block::Transaction;
use tensorium_core::wallet::WalletKeypair;

struct NodeProc(Child);
impl Drop for NodeProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn wait_for_health(base: &str) {
    for _ in 0..50 {
        if ureq_get(base, "/health").is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("node did not become healthy in time");
}

// Tiny GET helper using std TcpStream (mirrors the node's own http_get).
fn ureq_get(base: &str, path: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(base).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {base}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|e| e.to_string())?;
    if !status.contains("200") {
        return Err(status);
    }
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line == "\r\n" {
            break;
        }
    }
    let mut body = String::new();
    reader.read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn ureq_post(base: &str, path: &str, body: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(base).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {base}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|e| e.to_string())?;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line == "\r\n" {
            break;
        }
    }
    let mut resp_body = String::new();
    reader.read_to_string(&mut resp_body).map_err(|e| e.to_string())?;
    if !status.contains("200") {
        return Err(format!("{}: {}", status.trim(), resp_body));
    }
    Ok(resp_body)
}

#[test]
fn build_sign_broadcast_issue_tx() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let mempool = dir.path().join("mempool.json");
    let bans = dir.path().join("bans.json");
    let bin = env!("CARGO_BIN_EXE_tensorium-node");

    // init genesis on TESTNET (low difficulty, fast)
    let status = Command::new(bin)
        .arg("init")
        .arg("--chain=testnet")
        .env("TENSORIUM_STATE", &state)
        .env("TENSORIUM_MEMPOOL", &mempool)
        .env("TENSORIUM_BANS", &bans)
        .status()
        .unwrap();
    assert!(status.success());

    let bind = "127.0.0.1:39001";
    let mut child = Command::new(bin)
        .arg("rpc")
        .arg(bind)
        .arg("--chain=testnet")
        .env("TENSORIUM_STATE", &state)
        .env("TENSORIUM_MEMPOOL", &mempool)
        .env("TENSORIUM_BANS", &bans)
        .spawn()
        .unwrap();
    let _guard = NodeProc(child.clone_or_panic());
    wait_for_health(bind);

    // Use a freshly generated keypair; fund it isn't possible without mining,
    // so this test asserts the *insufficient balance* error path end-to-end —
    // confirming request parsing, validation, and the indexer-unavailable
    // short-circuit (testnet has no indexer running) all work together.
    let keypair = WalletKeypair::generate().unwrap();
    let body = format!(
        r#"{{"op":"issue","from":"{}","ticker":"GOLD","decimals":0,"supply":1000000,"name":"Gold Token"}}"#,
        keypair.address
    );
    let err = ureq_post(bind, "/buildAssetTx", &body).unwrap_err();
    assert!(err.contains("insufficient mature balance"), "unexpected error: {err}");

    let _ = child.kill();
}
```

Note: `Child` doesn't implement `Clone`; replace `child.clone_or_panic()` — instead, construct `NodeProc` directly from `child` and remove the separate `child.kill()` at the end (the `Drop` impl handles it). Corrected last lines:

```rust
    let mut child = Command::new(bin)
        .arg("rpc")
        .arg(bind)
        .arg("--chain=testnet")
        .env("TENSORIUM_STATE", &state)
        .env("TENSORIUM_MEMPOOL", &mempool)
        .env("TENSORIUM_BANS", &bans)
        .spawn()
        .unwrap();
    wait_for_health(bind);

    let keypair = WalletKeypair::generate().unwrap();
    let body = format!(
        r#"{{"op":"issue","from":"{}","ticker":"GOLD","decimals":0,"supply":1000000,"name":"Gold Token"}}"#,
        keypair.address
    );
    let err = ureq_post(bind, "/buildAssetTx", &body).unwrap_err();
    assert!(err.contains("insufficient mature balance"), "unexpected error: {err}");

    let _ = child.kill();
}
```

(remove the unused `NodeProc`/`Drop` block entirely since we kill `child` directly at the end)

Before relying on `WalletKeypair::generate()` and the `--chain=testnet` CLI flag, verify both exist:

Run: `cd /root/tensorium-core && grep -n "fn generate\|\"--chain\"\|chain=testnet\|fn cli_chain" crates/tensorium-core/src/wallet.rs crates/tensorium-node/src/main.rs | head -10`

If `WalletKeypair::generate()` doesn't exist, use the existing `WalletFile`/keypair-creation path used by `txmwallet create` instead (check `crates/tensorium-core/src/wallet.rs` for the actual constructor name and adjust the test accordingly — keep the assertion on the `/buildAssetTx` error message, which is the part under test).
If `--chain=testnet` isn't a real flag, check `crates/tensorium-node/src/main.rs`'s `init`/`rpc` subcommand argument parsing (search for `"init"` and `"rpc"` in the `match` on `args[1]`) and use whatever default/flag selects the low-difficulty test chain — or omit the flag if `init` always defaults appropriately for a fresh empty dir.

- [ ] **Step 2: Run the test to verify it fails first for the right reason (compile, then assertion)**

Run: `cd /root/tensorium-core && cargo test -p tensorium-node --test build_asset_tx 2>&1 | tail -30`
Expected: either a compile error to fix per the notes above, or the test passes immediately once `/buildAssetTx` (Task 3) exists — if it fails on `/health` or CLI args, fix the flags/constructors per the verification commands above, not the production code.

- [ ] **Step 3: Iterate until the test passes**

Run: `cd /root/tensorium-core && cargo test -p tensorium-node --test build_asset_tx 2>&1 | tail -30`
Expected: `test result: ok. 1 passed`

- [ ] **Step 4: Commit**

```bash
cd /root/tensorium-core
git add crates/tensorium-node/tests/build_asset_tx.rs
git commit -m "test(node-rpc): integration test for /buildAssetTx error path"
```

---

## Task 5: Wallet extension — `signAssetTx` and `getAssets`

**Files:**
- Modify: `src/inpage/index.ts`
- Modify: `src/content/inject.ts` (verify no change needed — it's already a generic relay)
- Modify: `src/background/service_worker.ts`
- Create: `src/popup/pages/SignAssetTx.tsx`
- Test: `src/__tests__/asset-tx.test.ts` (new)

Working directory for this task: `/root/.openclaw/workspace/tensorium-wallet-extension`

- [ ] **Step 1: Write the failing test for the inpage provider shape**

Create `src/__tests__/asset-tx.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

describe('inpage provider — asset tx methods', () => {
  it('exposes signAssetTx and getAssets on window.tensorium', () => {
    const src = fs.readFileSync(path.join(__dirname, '../inpage/index.ts'), 'utf-8');
    expect(src).toContain('signAssetTx:');
    expect(src).toContain('getAssets:');
  });
});
```

Run: `npx vitest run src/__tests__/asset-tx.test.ts 2>&1 | tail -20`
Expected: FAIL — `signAssetTx:` not found in source

- [ ] **Step 2: Add `signAssetTx`/`getAssets` to the inpage provider**

In `src/inpage/index.ts`, extend the `window.tensorium` object (the `(window as any).tensorium = {...}` block at the end of the file):

```typescript
  (window as any).tensorium = {
    isInstalled: true,
    getAddress: () => request('getAddress'),
    requestAccounts: () => request('requestAccounts'),
    sendTransaction: (to: string, amount_atoms: number) =>
      request('sendTransaction', { to, amount_atoms }),
    signAssetTx: (unsignedTx: unknown, summary: unknown) =>
      request('signAssetTx', { unsignedTx, summary }),
    getAssets: (address: string) => request('getAssets', { address }),
  };
```

- [ ] **Step 3: Run test to verify it passes**

Run: `npx vitest run src/__tests__/asset-tx.test.ts 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 4: Write the failing test for the background dispatcher**

Add to `src/__tests__/asset-tx.test.ts`:

```typescript
describe('background dispatcher — asset tx methods', () => {
  it('handles signAssetTx and getAssets methods', () => {
    const src = fs.readFileSync(path.join(__dirname, '../background/service_worker.ts'), 'utf-8');
    expect(src).toContain("method === 'signAssetTx'");
    expect(src).toContain("method === 'getAssets'");
  });
});
```

Run: `npx vitest run src/__tests__/asset-tx.test.ts 2>&1 | tail -20`
Expected: FAIL — second test fails

- [ ] **Step 5: Implement the background dispatcher handlers**

In `src/background/service_worker.ts`, add two new branches after the existing `if (method === 'sendTransaction') {...}` block:

```typescript
  if (method === 'signAssetTx') {
    const unsignedTx = params['unsignedTx'];
    const summary = params['summary'];
    return await pendSignAssetTx(unsignedTx, summary);
  }

  if (method === 'getAssets') {
    const address = params['address'] as string;
    return await fetchAssets(address);
  }
```

Add the supporting functions near `pendSendTransaction`:

```typescript
async function pendSignAssetTx(unsignedTx: unknown, summary: unknown): Promise<string> {
  const reqId = Date.now().toString();
  await (chrome.storage.session as any).set({
    txm_asset_req: { reqId, unsignedTx, summary, status: 'pending' }
  });
  await chrome.action.setBadgeText({ text: '1' });
  await chrome.action.setBadgeBackgroundColor({ color: '#ef4444' });

  const deadline = Date.now() + 10 * 60 * 1000;
  while (Date.now() < deadline) {
    await sleep(600);
    const data = await (chrome.storage.session as any).get('txm_asset_req');
    const req = data['txm_asset_req'] as AssetReq | undefined;
    if (!req || req.reqId !== reqId || req.status === 'pending') continue;
    if (req.status === 'confirmed') return req.txid as string;
    throw new Error(req.error ?? 'Transaction rejected');
  }

  await (chrome.storage.session as any).remove('txm_asset_req');
  await chrome.action.setBadgeText({ text: '' });
  throw new Error('Confirmation timed out — please try again');
}

interface AssetReq {
  reqId: string;
  unsignedTx: unknown;
  summary: unknown;
  status: 'pending' | 'confirmed' | 'rejected';
  txid?: string;
  error?: string;
}

async function fetchAssets(address: string): Promise<{ fungible: unknown[]; nfts: unknown[] }> {
  const indexerBase = 'https://marketplace.tensoriumlabs.com/api';
  const resp = await fetch(`${indexerBase}/balance/${address}`);
  if (!resp.ok) throw new Error(`indexer error: ${resp.status}`);
  const data = await resp.json();
  return { fungible: data.fungible ?? [], nfts: data.nfts ?? [] };
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `npx vitest run src/__tests__/asset-tx.test.ts 2>&1 | tail -20`
Expected: PASS — both tests pass

- [ ] **Step 7: Add the approval popup page for asset transactions**

First, look at the existing `src/popup/pages/Send.tsx` and `src/popup/pages/BridgeConfirm.tsx` to copy the pending-request-detection and approve/reject pattern:

Run: `head -60 src/popup/pages/BridgeConfirm.tsx`

Create `src/popup/pages/SignAssetTx.tsx` following the same structure as `BridgeConfirm.tsx` but:
- reads `txm_asset_req` from `chrome.storage.session` instead of `txm_bridge_req`
- renders `req.summary.description` and `req.summary.fee_atoms` (formatted via the existing TXM-amount formatting helper used elsewhere in the popup — check `src/popup/pages/Send.tsx` for the formatter name and reuse it)
- on Approve: calls `signTransaction(req.unsignedTx, hexToBytes(privKeyHex))` (same as `Send.tsx`/`BridgeConfirm.tsx`), converts to the RPC tx shape (same `rpcTx` mapping as in `Send.tsx`), calls `rpc.sendRawTransaction(rpcTx)`, then writes `{ ...req, status: 'confirmed', txid }` back to `chrome.storage.session`
- on Reject: writes `{ ...req, status: 'rejected', error: 'User rejected' }`

Write the file by adapting `BridgeConfirm.tsx`'s structure — reuse its imports (`hexToBytes`, `signTransaction`, `createRpcClient`, `loadSelectedRpcUrl`, wallet-loading helper) and its approve/reject button JSX, swapping the storage key and confirmation-message content as described above.

Register the new page in the popup router (find where `BridgeConfirm` is registered — likely `src/popup/App.tsx` or a routes file):

Run: `grep -rn "BridgeConfirm" src/popup/*.tsx src/popup/**/*.tsx 2>/dev/null`

Add an equivalent route/conditional render for `SignAssetTx`, triggered when `chrome.storage.session` contains a `txm_asset_req` with `status === 'pending'` (mirror however `BridgeConfirm` is triggered for `txm_bridge_req`).

- [ ] **Step 8: Build the extension to verify no type errors**

Run: `npm run build 2>&1 | tail -30`
Expected: build succeeds

- [ ] **Step 9: Bump version and commit**

Edit `manifest.json`: change `"version": "0.1.5"` to `"version": "0.1.6"`.

```bash
git add manifest.json src/inpage/index.ts src/background/service_worker.ts src/popup/pages/SignAssetTx.tsx src/popup/App.tsx src/__tests__/asset-tx.test.ts
git commit -m "feat: add signAssetTx and getAssets to window.tensorium provider (v0.1.6)"
```

(adjust the `git add` file list to match whichever router file was actually edited in Step 7)

---

## Task 6: Marketplace UI — Connect Wallet, Create Token, Mint NFT, Transfer, My Assets

**Files:**
- Modify: `/root/tensorium-sites/marketplace/index.html`
- Create: `/root/tensorium-sites/marketplace/assets/wallet.js`

- [ ] **Step 1: Inspect the existing page structure**

Run: `grep -n '<section\|id="\|class="lookup' /root/tensorium-sites/marketplace/index.html | head -40`

This shows where the existing "lookup" section is (asset/balance lookup by ID/address) — the new sections follow the same `<section class="...">` + `.card` pattern.

- [ ] **Step 2: Add the Connect Wallet button to the nav**

In `/root/tensorium-sites/marketplace/index.html`, find the `<nav>` element (top of `<body>`). Add, as the last item:

```html
<button id="connect-wallet" class="btn btn-primary">Connect Wallet</button>
```

- [ ] **Step 3: Add Create Token, Mint NFT, Transfer, and My Assets sections**

Add new `<section>` blocks after the existing "lookup" section (same indentation/class conventions as the surrounding markup — copy a `<div class="card">` from the existing lookup section as the template for each form below):

```html
<section class="mk-section" id="create-token-section">
  <h2>Create Token (TXM20)</h2>
  <div class="card">
    <label>Ticker (max 8 chars)</label>
    <input id="ct-ticker" maxlength="8" placeholder="GOLD">
    <label>Decimals (0-18)</label>
    <input id="ct-decimals" type="number" min="0" max="18" value="0">
    <label>Supply</label>
    <input id="ct-supply" type="number" min="1" placeholder="1000000">
    <label>Name (max 32 chars)</label>
    <input id="ct-name" maxlength="32" placeholder="Gold Token">
    <button id="ct-submit" class="btn btn-primary">Issue Token</button>
    <div class="lookup-out" id="ct-result"></div>
  </div>
</section>

<section class="mk-section" id="mint-nft-section">
  <h2>Mint NFT</h2>
  <div class="card">
    <label>Media URI (max 200 chars)</label>
    <input id="mn-uri" maxlength="200" placeholder="ipfs://...">
    <label>Content SHA-256 (64 hex chars)</label>
    <input id="mn-hash" maxlength="64" placeholder="auto-filled if you upload a file below">
    <label>Upload media (computes hash locally, not uploaded)</label>
    <input id="mn-file" type="file">
    <label>Royalty (basis points, 0-10000)</label>
    <input id="mn-royalty" type="number" min="0" max="10000" value="0">
    <label>Royalty address</label>
    <input id="mn-royalty-addr" placeholder="txm1...">
    <button id="mn-submit" class="btn btn-primary">Mint NFT</button>
    <div class="lookup-out" id="mn-result"></div>
  </div>
</section>

<section class="mk-section" id="my-assets-section">
  <h2>My Assets</h2>
  <div class="asset-grid" id="my-assets-grid">
    <div class="empty"><b>Connect your wallet</b>to see your TXM20 balances and NFTs.</div>
  </div>
</section>
```

- [ ] **Step 4: Create `assets/wallet.js`**

Create `/root/tensorium-sites/marketplace/assets/wallet.js`:

```javascript
// Marketplace wallet-connect + asset creation/transfer.
// Talks to window.tensorium (wallet extension) and rpc.tensoriumlabs.com.

const RPC_BASE = 'https://rpc.tensoriumlabs.com';
const INDEXER_BASE = '/api'; // nginx-proxied to txm-asset-indexer

let connectedAddress = null;

async function connectWallet() {
  if (!window.tensorium || !window.tensorium.isInstalled) {
    alert('Tensorium wallet extension not found. Install it first.');
    return;
  }
  const accounts = await window.tensorium.requestAccounts();
  if (!accounts || accounts.length === 0) {
    alert('No accounts returned by wallet.');
    return;
  }
  connectedAddress = accounts[0];
  const btn = document.getElementById('connect-wallet');
  btn.textContent = connectedAddress.slice(0, 8) + '…' + connectedAddress.slice(-4);
  await refreshMyAssets();
}

async function buildAssetTx(payload) {
  const resp = await fetch(`${RPC_BASE}/buildAssetTx`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  const data = await resp.json();
  if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);
  return data;
}

async function submitAssetTx(payload, resultEl) {
  resultEl.textContent = 'Building transaction…';
  try {
    const { unsigned_tx, summary } = await buildAssetTx(payload);
    resultEl.textContent = `Confirm in your wallet: ${summary.description}`;
    const txid = await window.tensorium.signAssetTx(unsigned_tx, summary);
    resultEl.textContent = `Submitted! txid: ${txid}`;
    await refreshMyAssets();
    return txid;
  } catch (e) {
    resultEl.textContent = `Error: ${e.message || e}`;
    throw e;
  }
}

async function refreshMyAssets() {
  const grid = document.getElementById('my-assets-grid');
  if (!connectedAddress) {
    grid.innerHTML = '<div class="empty"><b>Connect your wallet</b>to see your TXM20 balances and NFTs.</div>';
    return;
  }
  grid.innerHTML = '<div class="empty">Loading…</div>';
  const { fungible = [], nfts = [] } = await window.tensorium.getAssets(connectedAddress);

  if (fungible.length === 0 && nfts.length === 0) {
    grid.innerHTML = '<div class="empty"><b>No assets yet</b>Issue a token or mint an NFT above.</div>';
    return;
  }

  const cards = [];
  for (const ft of fungible) {
    let info = {};
    try {
      const r = await fetch(`${INDEXER_BASE}/asset/${ft.asset_id}`);
      if (r.ok) info = await r.json();
    } catch (_) { /* indexer optional for display */ }
    cards.push(`
      <div class="asset-card">
        <div class="tick">${info.ticker || '???'}</div>
        <div class="id">${ft.asset_id}</div>
        <div class="row"><span>Balance</span><span>${ft.amount}</span></div>
        <button class="btn btn-secondary" onclick="openTransferModal('${ft.asset_id}', ${ft.amount})">Send</button>
      </div>
    `);
  }
  for (const assetId of nfts) {
    let info = {};
    try {
      const r = await fetch(`${INDEXER_BASE}/asset/${assetId}`);
      if (r.ok) info = await r.json();
    } catch (_) { /* indexer optional for display */ }
    cards.push(`
      <div class="asset-card">
        <div class="tick">NFT</div>
        <div class="id">${assetId}</div>
        <div class="row"><span>URI</span><span>${info.uri || ''}</span></div>
        <button class="btn btn-secondary" onclick="openTransferModal('${assetId}', 1)">Send</button>
      </div>
    `);
  }
  grid.innerHTML = cards.join('');
}

function openTransferModal(assetId, maxAmount) {
  const to = prompt('Recipient address (txm1...)');
  if (!to) return;
  let amount = 1;
  if (maxAmount > 1) {
    const input = prompt(`Amount to send (max ${maxAmount})`, String(maxAmount));
    if (!input) return;
    amount = parseInt(input, 10);
  }
  const resultEl = document.getElementById('ct-result'); // reuse the create-token result line for feedback
  submitAssetTx({ op: 'transfer', from: connectedAddress, to, asset_id: assetId, amount }, resultEl);
}

document.addEventListener('DOMContentLoaded', () => {
  document.getElementById('connect-wallet')?.addEventListener('click', connectWallet);

  document.getElementById('ct-submit')?.addEventListener('click', () => {
    const payload = {
      op: 'issue',
      from: connectedAddress,
      ticker: document.getElementById('ct-ticker').value.trim(),
      decimals: parseInt(document.getElementById('ct-decimals').value, 10) || 0,
      supply: parseInt(document.getElementById('ct-supply').value, 10),
      name: document.getElementById('ct-name').value.trim(),
    };
    const resultEl = document.getElementById('ct-result');
    if (!connectedAddress) { resultEl.textContent = 'Connect your wallet first.'; return; }
    submitAssetTx(payload, resultEl);
  });

  document.getElementById('mn-file')?.addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const buf = await file.arrayBuffer();
    const digest = await crypto.subtle.digest('SHA-256', buf);
    const hex = Array.from(new Uint8Array(digest)).map(b => b.toString(16).padStart(2, '0')).join('');
    document.getElementById('mn-hash').value = hex;
  });

  document.getElementById('mn-submit')?.addEventListener('click', () => {
    const payload = {
      op: 'nft_mint',
      from: connectedAddress,
      royalty_bps: parseInt(document.getElementById('mn-royalty').value, 10) || 0,
      royalty_addr: document.getElementById('mn-royalty-addr').value.trim(),
      uri: document.getElementById('mn-uri').value.trim(),
      content_hash: document.getElementById('mn-hash').value.trim(),
    };
    const resultEl = document.getElementById('mn-result');
    if (!connectedAddress) { resultEl.textContent = 'Connect your wallet first.'; return; }
    if (payload.content_hash.length !== 64) { resultEl.textContent = 'content_hash must be 64 hex chars (upload a file or paste a sha256).'; return; }
    submitAssetTx(payload, resultEl);
  });
});
```

- [ ] **Step 5: Include the script in `index.html`**

Add before the closing `</body>` tag in `/root/tensorium-sites/marketplace/index.html`:

```html
<script src="assets/wallet.js"></script>
```

- [ ] **Step 6: Manual browser test**

This UI has no automated test harness (per the spec). Manual verification steps:

1. Serve the marketplace locally: `cd /root/tensorium-sites/marketplace && python3 -m http.server 8088`
2. Open `http://localhost:8088` in a browser with the wallet extension (built in Task 5) loaded as an unpacked extension
3. Note: `window.tensorium` only injects on `*.tensoriumlabs.com` per the extension's `content_scripts.matches` — for local testing, either temporarily add `http://localhost:8088/*` to `manifest.json`'s `content_scripts.matches` and `host_permissions` (revert before shipping), or test directly against a staging subdomain.
4. Click "Connect Wallet" → approve in extension popup → button shows truncated address
5. Fill in "Create Token" (e.g. ticker `TEST`, decimals `0`, supply `1000`, name `Test Token`) → "Issue Token" → approve in popup → confirm `result` div shows a txid
6. After the tx is mined (or via mempool, depending on indexer lag), confirm "My Assets" shows the new token with balance `1000`
7. Repeat for "Mint NFT" with a small test image, confirm it appears under "My Assets"
8. Click "Send" on the token, transfer a partial amount to a second test address, confirm balance decreases accordingly

- [ ] **Step 7: Commit**

```bash
cd /root/tensorium-sites/marketplace
git add index.html assets/wallet.js
git commit -m "feat: wallet-connect + TXM20/NFT create/mint/transfer UI"
```

---

## Task 7: Deploy

- [ ] **Step 1: Push all repos**

```bash
cd /root/tensorium-core && git push origin main
cd /root/.openclaw/workspace/tensorium-wallet-extension && git push origin main
cd /root/tensorium-sites/marketplace && git push origin main
```

- [ ] **Step 2: Build and deploy `tensorium-node` + `tensorium-indexer` to the live VPS**

The live RPC (`rpc.tensoriumlabs.com`) is served by `tensorium-node`. Follow the existing deploy pattern (build release binary, canary against frozen DB, swap binary, restart service) — reference the prior asset-protocol deploy described in project memory for the exact systemd service names (`tensorium-rpc`, `txm-asset-indexer`).

```bash
# On the build host (this machine):
cd /root/tensorium-core && cargo build --release -p tensorium-node -p txm-asset-indexer

# Copy to VPS, e.g.:
scp target/release/tensorium-node target/release/txm-asset-indexer root@<vps>:/tmp/

# On the VPS: stop service, swap binary, restart, tail logs to confirm /buildAssetTx responds:
ssh root@<vps> 'systemctl stop tensorium-rpc && cp /tmp/tensorium-node /usr/local/bin/tensorium-node && systemctl start tensorium-rpc && sleep 2 && curl -s -X POST localhost:33332/buildAssetTx -d "{\"op\":\"issue\",\"from\":\"txm1invalid\",\"ticker\":\"X\",\"decimals\":0,\"supply\":1,\"name\":\"x\"}"'
```

Expected final curl output: `{"error":"invalid 'from' address"}` — confirms the new endpoint is live and validating input.

- [ ] **Step 3: Deploy marketplace static files**

```bash
ssh root@<vps> 'cd /var/www/marketplace && git pull origin main'
```

(confirm the actual deploy path matches what was used for prior marketplace updates — check `nginx` config or prior deploy commands in shell history if `/var/www/marketplace` isn't correct)

- [ ] **Step 4: Smoke test on live marketplace**

Open `https://marketplace.tensoriumlabs.com` in a browser with wallet extension v0.1.6 installed, repeat the manual test steps from Task 6 Step 6 against production.

---

## Self-review notes

- **Spec coverage**: `/buildAssetTx` (Task 3), `signAssetTx`/`getAssets` (Task 5), Connect Wallet/Create Token/Mint NFT/Transfer/My Assets UI (Task 6), error handling for validation/indexer-unavailable/insufficient-balance (Task 3 handler + Task 6 UI error display), deployment (Task 7). Settlement/order-relay correctly excluded (Phase 2).
- **`/address/:addr/assets` from the spec**: superseded — the existing `/balance/:addr` indexer route already returns both `fungible` and `nfts` for an address, so no new indexer endpoint is needed (Task 5/6 use `/balance/:addr` directly). This is a simplification, not a scope reduction.
- **Type consistency**: `BuildAssetTxRequest` (Task 3) field names (`from`, `ticker`, `decimals`, `supply`, `name`, `to`, `asset_id`, `amount`, `collection_id`, `royalty_bps`, `royalty_addr`, `uri`, `content_hash`) match exactly between the Rust handler (Task 3) and the JS payloads built in `wallet.js` (Task 6). The `unsigned_tx`/`summary` response shape matches what `signAssetTx` (Task 5) and `wallet.js` (Task 6) consume.
