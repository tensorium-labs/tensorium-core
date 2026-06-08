# TXM Asset Protocol — Layer 3 (wallet asset commands) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `txmwallet` commands to *build, sign, and save* asset transactions — `asset-issue` (create a TXM20), `asset-mint` (create an NFT), `asset-transfer` (move an asset) — by funding `inputs[0]` from the wallet's own address (so the indexer resolves the op's source to the owner) and attaching the `TXMA` `OP_RETURN` produced by the shared `tensorium-core::assets` codec.

**Architecture:** Asset txs are ordinary payment txs with one extra `OP_RETURN` output carrying `encode_op(op)`. The wallet selects mature UTXOs for its own address (guaranteeing `inputs[0]` belongs to the owner), builds outputs `[<dest P2PKH for transfer>, <TXMA OP_RETURN>, <change>]`, signs with the existing P2PKH signer, and writes the signed tx to disk (the operator broadcasts with the existing `broadcast` command — same sign/broadcast separation as `send`). The `OP_RETURN` push encoder lives in `tensorium-core::assets` so it stays symmetric with the decoder the indexer/codec already use; this plan also extends that decoder to accept `OP_PUSHDATA2` so payloads >255 B (long-URI NFTs, up to the 520 B spec cap) round-trip.

**Tech Stack:** Rust, `txmwallet` crate (`crates/txmwallet/src/main.rs`), `tensorium-core::assets` (codec) + `script::standard::{p2pkh_from_address, extract_address}`, node RPC (`/getutxos`, `/sendrawtransaction`), `cargo test`.

**Key facts (verified against the codebase):**
- The node already excludes `OP_RETURN` outputs from the UTXO set (`state.rs` `apply_utxo_delta_to_batch`), so zero-value `OP_RETURN` outputs are accepted with no consensus change.
- The mempool only enforces `fee = sum(inputs) − sum(outputs) ≥ MIN_RELAY_FEE_ATOMS`; there is no dust/zero-value-output rejection.
- `txmwallet` funds from `rpc_get(rpc, "/getutxos/<wallet.address>")`, selects mature UTXOs, and signs via `keypair.sign_transaction(&mut tx)` (existing `build_signed_payment_via_rpc`).
- The asset op's **source** = address of `inputs[0]`'s spent output. Funding only from the wallet's own UTXOs makes `inputs[0]` the owner by construction.

## File Structure

- Modify: `crates/tensorium-core/src/assets/codec.rs` — add `op_return_script` encoder; extend `op_return_data` to accept `OP_PUSHDATA2`.
- Modify: `crates/tensorium-core/src/assets/mod.rs` — re-export `op_return_script`.
- Modify: `crates/txmwallet/src/main.rs` — `build_asset_outputs` (pure) + `build_asset_tx_via_rpc` (RPC wrapper) + `asset-issue` / `asset-mint` / `asset-transfer` commands + help.

---

### Task 1: `OP_RETURN` push encoder + `OP_PUSHDATA2` decode (tensorium-core)

**Files:**
- Modify: `crates/tensorium-core/src/assets/codec.rs`
- Modify: `crates/tensorium-core/src/assets/mod.rs`

- [ ] **Step 1: Write the failing test**

Add inside the existing `mod tests` block in `crates/tensorium-core/src/assets/codec.rs` (after the extract tests):
```rust
    #[test]
    fn op_return_script_roundtrips_all_push_sizes() {
        // Direct push (<=0x4b), OP_PUSHDATA1 (<=0xff), OP_PUSHDATA2 (>0xff).
        for len in [10usize, 100, 300] {
            let data = vec![0xABu8; len];
            let spk = op_return_script(&data);
            assert_eq!(op_return_data(&spk), Some(data.as_slice()));
        }
    }

    #[test]
    fn large_nft_op_extracts_via_pushdata2() {
        // uri at the 200-byte cap → total payload ~285 B (> 255), forces PUSHDATA2.
        let op = AssetOp::NftMint(NftMintData {
            collection_id: [0u8; 32],
            royalty_bps: 100,
            royalty_addr: "txm1creator".into(),
            uri: "Q".repeat(200),
            content_hash: [1u8; 32],
        });
        let spk = op_return_script(&encode_op(&op));
        assert!(spk[1] == 0x4d, "expected OP_PUSHDATA2 for >255B payload");
        let tx = Transaction::payment(
            vec![],
            vec![TxOutput { value_atoms: 0, script_pubkey: spk }],
        );
        assert_eq!(extract_asset_op(&tx), Some(op));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-core assets::codec::tests::op_return_script_roundtrips_all_push_sizes`
Expected: FAIL — `op_return_script` not found.

- [ ] **Step 3: Implement the encoder + extend the decoder**

In `crates/tensorium-core/src/assets/codec.rs`, replace the existing `op_return_data` function with this pair (the function currently handles only `0x01..=0x4b` and `0x4c`; this adds `0x4d` and the matching encoder):
```rust
/// Build an `OP_RETURN` data-carrier `script_pubkey` for `data`, choosing the
/// smallest push opcode: direct (≤0x4b), `OP_PUSHDATA1` (≤0xff), else
/// `OP_PUSHDATA2` (little-endian 2-byte length; covers the 520-byte cap).
pub fn op_return_script(data: &[u8]) -> Vec<u8> {
    let mut spk = vec![OP_RETURN];
    let n = data.len();
    if n <= 0x4b {
        spk.push(n as u8);
    } else if n <= 0xff {
        spk.push(0x4c);
        spk.push(n as u8);
    } else {
        spk.push(0x4d);
        spk.push((n & 0xff) as u8);
        spk.push(((n >> 8) & 0xff) as u8);
    }
    spk.extend_from_slice(data);
    spk
}

/// Read the data bytes pushed after an `OP_RETURN`. Supports a direct push
/// (0x01..=0x4b), `OP_PUSHDATA1` (0x4c), or `OP_PUSHDATA2` (0x4d, 2-byte LE len).
/// Returns None if the output is not an OP_RETURN data carrier.
fn op_return_data(spk: &[u8]) -> Option<&[u8]> {
    if spk.first() != Some(&OP_RETURN) {
        return None;
    }
    let mut i = 1;
    let len = match spk.get(i)? {
        n @ 0x01..=0x4b => {
            i += 1;
            *n as usize
        }
        0x4c => {
            i += 1;
            let l = *spk.get(i)? as usize;
            i += 1;
            l
        }
        0x4d => {
            i += 1;
            let lo = *spk.get(i)? as usize;
            i += 1;
            let hi = *spk.get(i)? as usize;
            i += 1;
            lo | (hi << 8)
        }
        _ => return None,
    };
    spk.get(i..i + len)
}
```

- [ ] **Step 4: Re-export `op_return_script`**

In `crates/tensorium-core/src/assets/mod.rs`, change the codec re-export line:
```rust
pub use codec::{decode_op, encode_op, extract_asset_op};
```
to:
```rust
pub use codec::{decode_op, encode_op, extract_asset_op, op_return_script};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tensorium-core assets::codec`
Expected: PASS (the two new tests + all existing codec tests, including the prior `OP_PUSHDATA1`-based `extract_*` tests).

- [ ] **Step 6: Commit**
```bash
git add crates/tensorium-core/src/assets/codec.rs crates/tensorium-core/src/assets/mod.rs
git commit -m "feat(assets): op_return_script encoder + OP_PUSHDATA2 decode for >255B payloads"
```

---

### Task 2: `build_asset_outputs` — pure output builder (txmwallet)

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the imports**

In `crates/txmwallet/src/main.rs`, extend the `tensorium_core` use block (it currently imports `block::{Transaction, TxInput, TxOutput}` and `script::standard::{...}`). Add an assets import line inside the `use tensorium_core::{ ... };` block, after the `script::standard::{...}` group:
```rust
    assets::{encode_op, op_return_script, AssetOp},
```
(Place it so the block reads `..., script::standard::{...}, assets::{encode_op, op_return_script, AssetOp}, ChainState, UtxoSet, WalletKeypair,`.)

- [ ] **Step 2: Write the failing test**

Append a test module to the **end** of `crates/txmwallet/src/main.rs`:
```rust
#[cfg(test)]
mod asset_tests {
    use super::*;
    use tensorium_core::assets::{extract_asset_op, IssueData, TransferData};
    use tensorium_core::script::standard::extract_address;
    use tensorium_core::WalletKeypair;

    fn addr() -> String {
        WalletKeypair::generate().address.as_str().to_string()
    }

    #[test]
    fn issue_outputs_carry_op_return_and_change() {
        let owner = addr();
        let op = AssetOp::Issue(IssueData {
            ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
        });
        // total_in 50_000, fee 10_000, no dest → [OP_RETURN, change 40_000].
        let outs = build_asset_outputs(&op, None, &owner, 50_000, 10_000).unwrap();
        assert_eq!(outs.len(), 2);
        // The carrier decodes back to the op.
        let tx = Transaction::payment(vec![], outs.clone());
        assert_eq!(extract_asset_op(&tx), Some(op));
        // Change goes back to the owner.
        assert_eq!(outs[1].value_atoms, 40_000);
        assert_eq!(extract_address(&outs[1].script_pubkey).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn transfer_outputs_put_dest_at_index_zero() {
        let owner = addr();
        let bob = addr();
        let op = AssetOp::Transfer(TransferData {
            asset_id: [4u8; 32], amount: 250, dest_output_index: 0,
        });
        // dest carrier 1_000 atoms, fee 10_000, total_in 30_000.
        let outs = build_asset_outputs(&op, Some((&bob, 1_000)), &owner, 30_000, 10_000).unwrap();
        assert_eq!(outs.len(), 3);
        // Output 0 = dest (matches dest_output_index 0).
        assert_eq!(extract_address(&outs[0].script_pubkey).as_deref(), Some(bob.as_str()));
        assert_eq!(outs[0].value_atoms, 1_000);
        // Output 1 = TXMA carrier.
        let tx = Transaction::payment(vec![], outs.clone());
        assert_eq!(extract_asset_op(&tx), Some(op));
        // Output 2 = change = 30_000 - 1_000 - 10_000.
        assert_eq!(outs[2].value_atoms, 19_000);
        assert_eq!(extract_address(&outs[2].script_pubkey).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn rejects_insufficient_input() {
        let owner = addr();
        let op = AssetOp::Issue(IssueData {
            ticker: "X".into(), decimals: 0, supply: 1, name: "X".into(), flags: 0,
        });
        assert!(build_asset_outputs(&op, None, &owner, 5_000, 10_000).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p txmwallet asset_tests::issue_outputs_carry_op_return_and_change`
Expected: FAIL — `build_asset_outputs` not found.

- [ ] **Step 4: Implement `build_asset_outputs`**

Add this function in `crates/txmwallet/src/main.rs` (near the other `build_*` helpers, e.g. just above `fn build_signed_payment_via_rpc`):
```rust
/// Build the outputs for an asset tx: `[<dest P2PKH (transfer only)>, <TXMA
/// OP_RETURN>, <change to owner>]`. For a transfer, `dest` is `(recipient,
/// carrier_atoms)` and the op's `dest_output_index` must be 0 (this places the
/// recipient at output 0). Pure — no I/O.
fn build_asset_outputs(
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
    let change = total_in - dest_atoms - fee_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(change_addr)
                .map_err(|_| "invalid wallet address".to_owned())?,
        });
    }
    Ok(outputs)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p txmwallet asset_tests`
Expected: PASS (all three).

- [ ] **Step 6: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): build_asset_outputs — TXMA OP_RETURN + dest + change"
```

---

### Task 3: `build_asset_tx_via_rpc` + `asset-issue` command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Implement the RPC wrapper**

Add this function in `crates/txmwallet/src/main.rs`, directly below `build_asset_outputs`:
```rust
/// Fund an asset tx from the wallet's own mature UTXOs (so `inputs[0]` is the
/// owner), attach the asset op, sign, and return the signed tx. `dest` is the
/// transfer recipient + carrier atoms (None for issue/mint).
fn build_asset_tx_via_rpc(
    wallet: &WalletFile,
    keypair: &WalletKeypair,
    rpc: &str,
    op: &AssetOp,
    dest: Option<(&str, u64)>,
    fee_atoms: u64,
) -> Result<Transaction, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

    let needed = dest.map(|(_, a)| a).unwrap_or(0).saturating_add(fee_atoms);

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp {
        utxos: Vec<RpcUtxo>,
    }

    let body = rpc_get(rpc, &format!("/getutxos/{}", wallet.address))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;

    let mut inputs: Vec<TxInput> = Vec::new();
    let mut total_in = 0u64;
    for u in resp.utxos {
        if !u.mature {
            continue;
        }
        let hash = Hash256(
            u.txid_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid txid length from RPC".to_owned())?,
        );
        inputs.push(TxInput {
            previous_output: OutPoint { txid: hash, output_index: u.output_index },
            signature_script: Vec::new(),
        });
        total_in = total_in.saturating_add(u.value_atoms);
        if total_in >= needed {
            break;
        }
    }
    if total_in < needed {
        return Err(format!(
            "insufficient mature balance via RPC: have {total_in}, need {needed}"
        ));
    }

    let outputs = build_asset_outputs(op, dest, &wallet.address, total_in, fee_atoms)?;
    let mut tx = Transaction::payment(inputs, outputs);
    keypair.sign_transaction(&mut tx).map_err(|e| e.to_string())?;
    Ok(tx)
}
```

- [ ] **Step 2: Add the `asset-issue` command**

In the `match command { ... }` block in `run()`, add this arm after the `"send" => { ... }` arm:
```rust
        "asset-issue" => {
            let ticker = args.get(2).ok_or(
                "usage: txmwallet asset-issue <ticker> <decimals> <supply> <name...>",
            )?;
            let decimals: u8 = args
                .get(3)
                .ok_or("missing decimals")?
                .parse()
                .map_err(|_| "decimals must be 0-18")?;
            let supply: u64 = args
                .get(4)
                .ok_or("missing supply")?
                .parse()
                .map_err(|_| "supply must be a positive integer")?;
            let name = args.get(5..).map(|s| s.join(" ")).unwrap_or_default();

            let op = AssetOp::Issue(tensorium_core::assets::IssueData {
                ticker: ticker.to_string(),
                decimals,
                supply,
                name,
                flags: 0,
            });

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let fee_atoms = tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;
            let tx = build_asset_tx_via_rpc(&wallet, &keypair, &rpc, &op, None, fee_atoms)?;

            let tx_path = PathBuf::from(DEFAULT_SIGNED_TX_PATH);
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize signed tx: {e}"))?;
            fs::write(&tx_path, raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            // asset_id = this tx's id.
            println!("asset_id={}", tx.id);
            println!("txid={}", tx.id);
            println!("written={}", tx_path.display());
            println!("next: txmwallet broadcast");
        }
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p txmwallet`
Expected: builds.

- [ ] **Step 4: Run the wallet test suite (no regressions)**

Run: `cargo test -p txmwallet`
Expected: PASS (the `asset_tests` from Task 2 still pass).

- [ ] **Step 5: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): asset-issue command + build_asset_tx_via_rpc"
```

---

### Task 4: `asset-mint` command (NFT)

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the `asset-mint` command**

In the `match command { ... }` block, add this arm after the `"asset-issue" => { ... }` arm:
```rust
        "asset-mint" => {
            // usage: txmwallet asset-mint <royalty_bps> <royalty_addr> <content_hash_hex> <uri...>
            let royalty_bps: u16 = args
                .get(2)
                .ok_or("usage: txmwallet asset-mint <royalty_bps> <royalty_addr> <content_hash_hex> <uri...>")?
                .parse()
                .map_err(|_| "royalty_bps must be 0-10000")?;
            if royalty_bps > 10_000 {
                return Err("royalty_bps must be 0-10000".to_owned());
            }
            let royalty_addr = args.get(3).ok_or("missing royalty_addr")?.to_string();
            let content_hash_hex = args.get(4).ok_or("missing content_hash_hex")?;
            let hash_bytes = hex::decode(content_hash_hex)
                .map_err(|_| "content_hash_hex must be hex".to_owned())?;
            let content_hash: [u8; 32] = hash_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "content_hash must be 32 bytes (64 hex chars)".to_owned())?;
            let uri = args.get(5..).map(|s| s.join(" ")).unwrap_or_default();

            let op = AssetOp::NftMint(tensorium_core::assets::NftMintData {
                collection_id: [0u8; 32], // standalone NFT (MVP)
                royalty_bps,
                royalty_addr,
                uri,
                content_hash,
            });

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let fee_atoms = tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;
            let tx = build_asset_tx_via_rpc(&wallet, &keypair, &rpc, &op, None, fee_atoms)?;

            let tx_path = PathBuf::from(DEFAULT_SIGNED_TX_PATH);
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize signed tx: {e}"))?;
            fs::write(&tx_path, raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("nft_asset_id={}", tx.id);
            println!("txid={}", tx.id);
            println!("written={}", tx_path.display());
            println!("next: txmwallet broadcast");
        }
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build -p txmwallet`
Expected: builds. (If `hex` is unresolved, confirm it is in `crates/txmwallet/Cargo.toml`; the wallet already uses `hex::decode` elsewhere, so it is.)

- [ ] **Step 3: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): asset-mint command (standalone NFT)"
```

---

### Task 5: `asset-transfer` command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add a carrier-amount constant**

Near the other `const` declarations at the top of `crates/txmwallet/src/main.rs` (after `const DEFAULT_RPC`), add:
```rust
/// Atoms placed on the recipient's P2PKH output in an asset transfer. The asset
/// itself rides in the OP_RETURN; this carrier just makes the destination
/// address resolvable + spendable. 1000 atoms = 0.00001 TXM.
const ASSET_CARRIER_ATOMS: u64 = 1_000;
```

- [ ] **Step 2: Add the `asset-transfer` command**

In the `match command { ... }` block, add this arm after the `"asset-mint" => { ... }` arm:
```rust
        "asset-transfer" => {
            // usage: txmwallet asset-transfer <asset_id_hex> <amount> <to_address>
            let asset_id_hex = args
                .get(2)
                .ok_or("usage: txmwallet asset-transfer <asset_id_hex> <amount> <to_address>")?;
            let id_bytes = hex::decode(asset_id_hex)
                .map_err(|_| "asset_id_hex must be hex".to_owned())?;
            let asset_id: [u8; 32] = id_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "asset_id must be 32 bytes (64 hex chars)".to_owned())?;
            let amount: u64 = args
                .get(3)
                .ok_or("missing amount")?
                .parse()
                .map_err(|_| "amount must be a positive integer")?;
            let to_address = args.get(4).ok_or("missing to_address")?;

            let op = AssetOp::Transfer(tensorium_core::assets::TransferData {
                asset_id,
                amount,
                dest_output_index: 0, // recipient is placed at output 0
            });

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let fee_atoms = tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;
            let tx = build_asset_tx_via_rpc(
                &wallet,
                &keypair,
                &rpc,
                &op,
                Some((to_address, ASSET_CARRIER_ATOMS)),
                fee_atoms,
            )?;

            let tx_path = PathBuf::from(DEFAULT_SIGNED_TX_PATH);
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize signed tx: {e}"))?;
            fs::write(&tx_path, raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("asset_id={asset_id_hex}");
            println!("amount={amount}");
            println!("to={to_address}");
            println!("txid={}", tx.id);
            println!("written={}", tx_path.display());
            println!("next: txmwallet broadcast");
        }
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p txmwallet`
Expected: builds.

- [ ] **Step 4: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): asset-transfer command"
```

---

### Task 6: Help text + full-suite verification

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the asset commands to the help text**

In `fn print_help()`, insert these three lines immediately after the `vesting-claim` line (`println!("  vesting-claim <spk_hex> <dest_addr> [rpc] ...");`) and before the blank `println!();` that precedes the `env:` section:
```rust
    println!("  asset-issue <ticker> <decimals> <supply> <name...>    create a TXM20 fungible token");
    println!("  asset-mint <royalty_bps> <royalty_addr> <content_hash_hex> <uri...>  mint a standalone NFT");
    println!("  asset-transfer <asset_id_hex> <amount> <to_address>   transfer a TXM20/NFT to an address");
```

- [ ] **Step 2: Build + run the wallet test suite**

Run: `cargo test -p txmwallet`
Expected: PASS (`asset_tests`).

- [ ] **Step 3: Run the whole workspace suite (no regressions)**

Run: `cargo test --workspace`
Expected: all pass, including `tensorium-core::assets::*`, `tensorium-indexer`, and `txmwallet::asset_tests`.

- [ ] **Step 4: Manual smoke (optional — requires a funded wallet + live node)**

```bash
# Issue a token (writes + signs; broadcast separately).
TENSORIUM_WALLET_PASSPHRASE=... TENSORIUM_RPC=127.0.0.1:33332 \
  txmwallet asset-issue GOLD 8 21000000 "Gold Token"
txmwallet broadcast        # submits tensorium-signed-tx.json
# After it confirms, the indexer (Layer 2) shows it:
curl -s http://127.0.0.1:23340/assets
curl -s http://127.0.0.1:23340/balance/<your-address>
```
Expected: after the issue tx confirms and the indexer scans it, `/assets` lists the new `asset_id` and `/balance` credits the full supply to the issuer.

- [ ] **Step 5: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "docs(wallet): list asset commands in help"
```

---

## Done criteria

- `txmwallet` exposes `asset-issue`, `asset-mint`, `asset-transfer`: each funds `inputs[0]` from the wallet's own address, attaches the `TXMA` `OP_RETURN` via the shared codec, signs, and writes a broadcastable signed tx.
- `tensorium-core::assets` exposes `op_return_script`; its `OP_RETURN` decoder round-trips direct / `OP_PUSHDATA1` / `OP_PUSHDATA2` pushes (≤520 B).
- TDD coverage: push-size round-trips + large-NFT extract (core), and `build_asset_outputs` issue/transfer/insufficient (wallet).
- `cargo test --workspace` green; no consensus change; the only on-chain artifact is an `OP_RETURN` output the node already treats as unspendable.

## Next plan (Layer 4 — marketplace + escrow)

`marketplace.tensoriumlabs.com` backend: listings DB (price/status UX metadata only), HTLC/CLTV-style escrow for atomic asset⇄TXM trades reusing the S3 script layer, the 2.5% platform fee, and royalty enforcement (secondary sales pay `royalty_bps` to `royalty_addr` recorded at mint). Then Layer 5 (frontend).
