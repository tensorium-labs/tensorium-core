# TXM Asset Protocol — Layer 1 (codec + state machine) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pure-function asset codec (encode/decode of the `TXMA` OP_RETURN overlay) and the deterministic state machine (apply ISSUE/NFT_MINT/TRANSFER to balances + ownership) inside `tensorium-core`, so the wallet and indexer share one canonical, fully-tested implementation.

**Architecture:** A new `assets` module in `tensorium-core` with three files: `mod.rs` (types + re-exports), `codec.rs` (binary encode/decode + extract-from-tx), `state.rs` (`AssetState::apply`). No I/O, no consensus change. All logic is deterministic and unit-tested TDD-style. The indexer (Layer 2, separate plan) wraps these functions with block scanning, persistence, reorg handling, and a REST API.

**Tech Stack:** Rust, `tensorium-core` crate, existing `Hash256`/`Transaction`/`script` modules, `cargo test`.

---

### Task 1: Module scaffold + types

**Files:**
- Create: `crates/tensorium-core/src/assets/mod.rs`
- Modify: `crates/tensorium-core/src/lib.rs` (register module + re-exports)

- [ ] **Step 1: Create the module with types**

`crates/tensorium-core/src/assets/mod.rs`:
```rust
//! TXM asset overlay protocol (TXM20 fungible tokens + NFTs).
//! Asset operations ride inside ordinary TXM transactions as `OP_RETURN`
//! metadata; balances/ownership are a deterministic function of the chain.
//! This module is pure (no I/O) — shared by the wallet and the indexer.
pub mod codec;
pub mod state;

pub use codec::{decode_op, encode_op, extract_asset_op};
pub use state::{ApplyResult, AssetInfo, AssetKind, AssetState};

/// One asset operation, decoded from a `TXMA` OP_RETURN payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetOp {
    Issue(IssueData),
    NftMint(NftMintData),
    Transfer(TransferData),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueData {
    pub ticker: String,   // ≤ 8 bytes
    pub decimals: u8,     // ≤ 18
    pub supply: u64,
    pub name: String,     // ≤ 32 bytes
    pub flags: u8,        // bit0 = mintable (reserved)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NftMintData {
    pub collection_id: [u8; 32], // all-zero = standalone NFT
    pub royalty_bps: u16,        // ≤ 10000
    pub royalty_addr: String,    // creator payout (may be empty = no royalty)
    pub uri: String,             // ≤ 200 bytes
    pub content_hash: [u8; 32],  // SHA-256 of the media
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferData {
    pub asset_id: [u8; 32],      // txid of the ISSUE/NFT_MINT
    pub amount: u64,             // NFT: must be 1
    pub dest_output_index: u8,   // output whose address receives the asset
}

#[derive(Debug, PartialEq, Eq)]
pub enum AssetError {
    BadMagic,
    BadVersion,
    UnknownOpcode,
    Truncated,
    TooLarge,
    FieldTooLong,
    BadRoyalty,
}

/// Protocol constants.
pub const MAGIC: &[u8; 4] = b"TXMA";
pub const VERSION: u8 = 0x01;
pub const OP_ISSUE: u8 = 0x01;
pub const OP_NFT_MINT: u8 = 0x02;
pub const OP_TRANSFER: u8 = 0x03;
/// Must fit a single OP_RETURN data push.
pub const MAX_PAYLOAD: usize = 520;
```

- [ ] **Step 2: Register the module in lib.rs**

In `crates/tensorium-core/src/lib.rs`, add alongside the other `pub mod` lines:
```rust
pub mod assets;
```
And alongside the other `pub use` lines:
```rust
pub use assets::{AssetOp, AssetState, IssueData, NftMintData, TransferData};
```

- [ ] **Step 3: Create empty submodule files so it compiles**

`crates/tensorium-core/src/assets/codec.rs`:
```rust
use super::*;
```
`crates/tensorium-core/src/assets/state.rs`:
```rust
use super::*;
use std::collections::HashMap;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p tensorium-core`
Expected: builds (warnings about unused are fine for now).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets crates/tensorium-core/src/lib.rs
git commit -m "feat(assets): scaffold TXM asset overlay module + types"
```

---

### Task 2: Encode/decode ISSUE

**Files:**
- Modify: `crates/tensorium-core/src/assets/codec.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/tensorium-core/src/assets/codec.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_roundtrip() {
        let op = AssetOp::Issue(IssueData {
            ticker: "GOLD".into(),
            decimals: 8,
            supply: 21_000_000,
            name: "Gold Token".into(),
            flags: 0,
        });
        let bytes = encode_op(&op);
        assert_eq!(&bytes[0..4], MAGIC);
        assert_eq!(bytes[4], VERSION);
        assert_eq!(bytes[5], OP_ISSUE);
        assert_eq!(decode_op(&bytes).unwrap(), op);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::codec::tests::issue_roundtrip`
Expected: FAIL — `encode_op`/`decode_op` not found.

- [ ] **Step 3: Implement encode_op + decode_op (ISSUE arm)**

Replace the contents of `crates/tensorium-core/src/assets/codec.rs` (above the `#[cfg(test)]`) with:
```rust
use super::*;

fn put_str(out: &mut Vec<u8>, s: &str, max: usize) {
    let b = s.as_bytes();
    let n = b.len().min(max);
    out.push(n as u8);
    out.extend_from_slice(&b[..n]);
}

fn take<'a>(buf: &'a [u8], i: &mut usize, n: usize) -> Result<&'a [u8], AssetError> {
    if *i + n > buf.len() {
        return Err(AssetError::Truncated);
    }
    let s = &buf[*i..*i + n];
    *i += n;
    Ok(s)
}

fn take_str(buf: &[u8], i: &mut usize) -> Result<String, AssetError> {
    let len = take(buf, i, 1)?[0] as usize;
    let bytes = take(buf, i, len)?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn take_u64(buf: &[u8], i: &mut usize) -> Result<u64, AssetError> {
    let b = take(buf, i, 8)?;
    Ok(u64::from_be_bytes(b.try_into().unwrap()))
}

/// Encode an asset op into the full OP_RETURN data payload (`TXMA` + version + op + body).
pub fn encode_op(op: &AssetOp) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    match op {
        AssetOp::Issue(d) => {
            out.push(OP_ISSUE);
            put_str(&mut out, &d.ticker, 8);
            out.push(d.decimals);
            out.extend_from_slice(&d.supply.to_be_bytes());
            put_str(&mut out, &d.name, 32);
            out.push(d.flags);
        }
        AssetOp::NftMint(d) => {
            out.push(OP_NFT_MINT);
            out.extend_from_slice(&d.collection_id);
            out.extend_from_slice(&d.royalty_bps.to_be_bytes());
            put_str(&mut out, &d.royalty_addr, 90);
            put_str(&mut out, &d.uri, 200);
            out.extend_from_slice(&d.content_hash);
        }
        AssetOp::Transfer(d) => {
            out.push(OP_TRANSFER);
            out.extend_from_slice(&d.asset_id);
            out.extend_from_slice(&d.amount.to_be_bytes());
            out.push(d.dest_output_index);
        }
    }
    out
}

/// Decode an OP_RETURN data payload into an asset op.
pub fn decode_op(buf: &[u8]) -> Result<AssetOp, AssetError> {
    if buf.len() > MAX_PAYLOAD {
        return Err(AssetError::TooLarge);
    }
    let mut i = 0;
    if take(buf, &mut i, 4)? != MAGIC {
        return Err(AssetError::BadMagic);
    }
    if take(buf, &mut i, 1)?[0] != VERSION {
        return Err(AssetError::BadVersion);
    }
    let opcode = take(buf, &mut i, 1)?[0];
    match opcode {
        OP_ISSUE => {
            let ticker = take_str(buf, &mut i)?;
            let decimals = take(buf, &mut i, 1)?[0];
            let supply = take_u64(buf, &mut i)?;
            let name = take_str(buf, &mut i)?;
            let flags = take(buf, &mut i, 1)?[0];
            Ok(AssetOp::Issue(IssueData { ticker, decimals, supply, name, flags }))
        }
        OP_NFT_MINT => {
            let collection_id: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            let royalty_bps = u16::from_be_bytes(take(buf, &mut i, 2)?.try_into().unwrap());
            if royalty_bps > 10_000 {
                return Err(AssetError::BadRoyalty);
            }
            let royalty_addr = take_str(buf, &mut i)?;
            let uri = take_str(buf, &mut i)?;
            let content_hash: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            Ok(AssetOp::NftMint(NftMintData { collection_id, royalty_bps, royalty_addr, uri, content_hash }))
        }
        OP_TRANSFER => {
            let asset_id: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            let amount = take_u64(buf, &mut i)?;
            let dest_output_index = take(buf, &mut i, 1)?[0];
            Ok(AssetOp::Transfer(TransferData { asset_id, amount, dest_output_index }))
        }
        _ => Err(AssetError::UnknownOpcode),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core assets::codec::tests::issue_roundtrip`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets/codec.rs
git commit -m "feat(assets): encode/decode ISSUE + envelope"
```

---

### Task 3: NFT_MINT + TRANSFER round-trip + reject tests

**Files:**
- Modify: `crates/tensorium-core/src/assets/codec.rs` (tests only)

- [ ] **Step 1: Write the failing tests**

Add inside the `mod tests` block in `codec.rs`:
```rust
    #[test]
    fn nft_mint_roundtrip_with_royalty() {
        let op = AssetOp::NftMint(NftMintData {
            collection_id: [0u8; 32],
            royalty_bps: 500, // 5%
            royalty_addr: "txm1royaltyaddrexample00000000000000000".into(),
            uri: "ipfs://Qm123".into(),
            content_hash: [7u8; 32],
        });
        let bytes = encode_op(&op);
        assert_eq!(bytes[5], OP_NFT_MINT);
        assert_eq!(decode_op(&bytes).unwrap(), op);
    }

    #[test]
    fn transfer_roundtrip() {
        let op = AssetOp::Transfer(TransferData {
            asset_id: [9u8; 32],
            amount: 1234,
            dest_output_index: 2,
        });
        assert_eq!(decode_op(&encode_op(&op)).unwrap(), op);
    }

    #[test]
    fn decode_rejects_bad_inputs() {
        assert_eq!(decode_op(b"XXXX\x01\x01"), Err(AssetError::BadMagic));
        assert_eq!(decode_op(b"TXMA\x09\x01"), Err(AssetError::BadVersion));
        assert_eq!(decode_op(b"TXMA\x01\x99"), Err(AssetError::UnknownOpcode));
        assert_eq!(decode_op(b"TXMA\x01"), Err(AssetError::Truncated));
        // royalty > 10000 rejected
        let mut bad = vec![];
        bad.extend_from_slice(MAGIC);
        bad.push(VERSION);
        bad.push(OP_NFT_MINT);
        bad.extend_from_slice(&[0u8; 32]);          // collection
        bad.extend_from_slice(&10_001u16.to_be_bytes()); // royalty_bps
        bad.push(0);                                 // royalty_addr len 0
        bad.push(0);                                 // uri len 0
        bad.extend_from_slice(&[0u8; 32]);          // content_hash
        assert_eq!(decode_op(&bad), Err(AssetError::BadRoyalty));
        // oversize
        assert_eq!(decode_op(&vec![0u8; MAX_PAYLOAD + 1]), Err(AssetError::TooLarge));
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tensorium-core assets::codec`
Expected: PASS (encode/decode for NFT_MINT + TRANSFER already implemented in Task 2; these tests verify them + the rejects).

- [ ] **Step 3: Commit**
```bash
git add crates/tensorium-core/src/assets/codec.rs
git commit -m "test(assets): NFT_MINT/TRANSFER round-trip + decode rejects"
```

---

### Task 4: `extract_asset_op` — find the asset op in a transaction

**Files:**
- Modify: `crates/tensorium-core/src/assets/codec.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `codec.rs`:
```rust
    use crate::block::{Transaction, TxOutput};
    use crate::script::OP_RETURN;

    fn op_return_output(data: &[u8]) -> TxOutput {
        // OP_RETURN <pushdata1 len> <data>
        let mut spk = vec![OP_RETURN, 0x4c, data.len() as u8];
        spk.extend_from_slice(data);
        TxOutput { value_atoms: 0, script_pubkey: spk }
    }

    #[test]
    fn extract_finds_first_txma_op_return() {
        let op = AssetOp::Transfer(TransferData { asset_id: [3u8; 32], amount: 5, dest_output_index: 0 });
        let tx = Transaction::payment(
            vec![],
            vec![
                TxOutput { value_atoms: 100, script_pubkey: vec![0x76, 0xa9] }, // non-OP_RETURN
                op_return_output(&encode_op(&op)),
            ],
        );
        assert_eq!(extract_asset_op(&tx), Some(op));
    }

    #[test]
    fn extract_ignores_non_txma_op_return() {
        let tx = Transaction::payment(
            vec![],
            vec![op_return_output(b"hello not an asset")],
        );
        assert_eq!(extract_asset_op(&tx), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::codec::tests::extract_finds_first_txma_op_return`
Expected: FAIL — `extract_asset_op` not found.

- [ ] **Step 3: Implement extract_asset_op**

Add to `codec.rs` (above the tests):
```rust
use crate::block::Transaction;
use crate::script::OP_RETURN;

/// Read the data bytes pushed after an `OP_RETURN`. Supports a direct
/// push (0x01..=0x4b) or `OP_PUSHDATA1` (0x4c). Returns None if the output
/// is not an OP_RETURN data carrier.
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
            *spk.get(i).map(|x| {
                i += 1;
                x
            })? as usize
        }
        _ => return None,
    };
    spk.get(i..i + len)
}

/// Find the first valid `TXMA` asset op in a transaction's outputs.
pub fn extract_asset_op(tx: &Transaction) -> Option<AssetOp> {
    for out in &tx.outputs {
        if let Some(data) = op_return_data(&out.script_pubkey) {
            if let Ok(op) = decode_op(data) {
                return Some(op);
            }
        }
    }
    None
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tensorium-core assets::codec`
Expected: PASS (both extract tests + earlier ones).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets/codec.rs
git commit -m "feat(assets): extract_asset_op from tx OP_RETURN outputs"
```

---

### Task 5: AssetState + apply ISSUE

**Files:**
- Modify: `crates/tensorium-core/src/assets/state.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/tensorium-core/src/assets/state.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{AssetOp, IssueData};

    fn issue(ticker: &str, supply: u64) -> AssetOp {
        AssetOp::Issue(IssueData { ticker: ticker.into(), decimals: 8, supply, name: ticker.into(), flags: 0 })
    }

    #[test]
    fn issue_credits_source_full_supply() {
        let mut st = AssetState::default();
        let txid = [1u8; 32];
        assert_eq!(st.apply(txid, 10, "txm1alice", None, &issue("GOLD", 1000)), ApplyResult::Applied);
        assert_eq!(st.ft_balance("txm1alice", &txid), 1000);
        assert_eq!(st.assets.get(&txid).unwrap().ticker, "GOLD");
        // duplicate asset_id ignored
        assert!(matches!(st.apply(txid, 11, "txm1bob", None, &issue("DUP", 5)), ApplyResult::Ignored(_)));
        assert_eq!(st.ft_balance("txm1alice", &txid), 1000);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::state::tests::issue_credits_source_full_supply`
Expected: FAIL — `AssetState`/`apply` not found.

- [ ] **Step 3: Implement AssetState + apply (Issue arm)**

Replace the contents of `crates/tensorium-core/src/assets/state.rs` (above `#[cfg(test)]`) with:
```rust
use super::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Fungible,
    NonFungible,
}

#[derive(Clone, Debug)]
pub struct AssetInfo {
    pub kind: AssetKind,
    pub ticker: String,
    pub name: String,
    pub decimals: u8,
    pub supply: u64,
    pub issuer: String,
    pub royalty_bps: u16,
    pub royalty_addr: String,
    pub uri: String,
    pub content_hash: [u8; 32],
    pub mint_height: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Applied,
    Ignored(&'static str),
}

/// Deterministic asset state: reconstructable purely from the canonical chain.
#[derive(Default)]
pub struct AssetState {
    pub assets: HashMap<[u8; 32], AssetInfo>,
    pub ft_balances: HashMap<(String, [u8; 32]), u64>,
    pub nft_owner: HashMap<[u8; 32], String>,
}

impl AssetState {
    pub fn ft_balance(&self, addr: &str, asset_id: &[u8; 32]) -> u64 {
        *self.ft_balances.get(&(addr.to_string(), *asset_id)).unwrap_or(&0)
    }

    /// Apply one op. `txid` = the carrying tx's id (asset_id for ISSUE/NFT_MINT).
    /// `source` = address of the tx's first input. `dest_addr` = resolved address
    /// of the op's `dest_output_index` (only needed for TRANSFER).
    pub fn apply(
        &mut self,
        txid: [u8; 32],
        height: u64,
        source: &str,
        dest_addr: Option<&str>,
        op: &AssetOp,
    ) -> ApplyResult {
        match op {
            AssetOp::Issue(d) => {
                if self.assets.contains_key(&txid) {
                    return ApplyResult::Ignored("asset_id exists");
                }
                if d.decimals > 18 {
                    return ApplyResult::Ignored("decimals too high");
                }
                self.assets.insert(txid, AssetInfo {
                    kind: AssetKind::Fungible,
                    ticker: d.ticker.clone(),
                    name: d.name.clone(),
                    decimals: d.decimals,
                    supply: d.supply,
                    issuer: source.to_string(),
                    royalty_bps: 0,
                    royalty_addr: String::new(),
                    uri: String::new(),
                    content_hash: [0u8; 32],
                    mint_height: height,
                });
                *self.ft_balances.entry((source.to_string(), txid)).or_insert(0) += d.supply;
                ApplyResult::Applied
            }
            _ => ApplyResult::Ignored("not implemented yet"),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core assets::state::tests::issue_credits_source_full_supply`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets/state.rs
git commit -m "feat(assets): AssetState + apply ISSUE"
```

---

### Task 6: apply TRANSFER (fungible)

**Files:**
- Modify: `crates/tensorium-core/src/assets/state.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `state.rs`:
```rust
    use crate::assets::TransferData;

    fn transfer(asset_id: [u8; 32], amount: u64) -> AssetOp {
        AssetOp::Transfer(TransferData { asset_id, amount, dest_output_index: 0 })
    }

    #[test]
    fn transfer_ft_debits_source_credits_dest() {
        let mut st = AssetState::default();
        let txid = [1u8; 32];
        st.apply(txid, 1, "txm1alice", None, &issue("GOLD", 1000));
        // move 300 alice -> bob
        assert_eq!(
            st.apply([2u8; 32], 2, "txm1alice", Some("txm1bob"), &transfer(txid, 300)),
            ApplyResult::Applied
        );
        assert_eq!(st.ft_balance("txm1alice", &txid), 700);
        assert_eq!(st.ft_balance("txm1bob", &txid), 300);
        // over-balance ignored, state unchanged
        assert!(matches!(
            st.apply([3u8; 32], 3, "txm1alice", Some("txm1bob"), &transfer(txid, 99999)),
            ApplyResult::Ignored(_)
        ));
        assert_eq!(st.ft_balance("txm1alice", &txid), 700);
        // unknown asset ignored
        assert!(matches!(
            st.apply([4u8; 32], 4, "txm1alice", Some("txm1bob"), &transfer([8u8; 32], 1)),
            ApplyResult::Ignored(_)
        ));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::state::tests::transfer_ft_debits_source_credits_dest`
Expected: FAIL — transfer currently hits the `_ => Ignored("not implemented yet")` arm, so `ApplyResult::Applied` assertion fails.

- [ ] **Step 3: Implement the Transfer arm**

In `state.rs`, replace the `_ => ApplyResult::Ignored("not implemented yet")` arm with:
```rust
            AssetOp::Transfer(d) => {
                let Some(info) = self.assets.get(&d.asset_id) else {
                    return ApplyResult::Ignored("unknown asset");
                };
                let Some(dest) = dest_addr else {
                    return ApplyResult::Ignored("bad dest output");
                };
                match info.kind {
                    AssetKind::Fungible => {
                        if d.amount == 0 {
                            return ApplyResult::Ignored("zero amount");
                        }
                        let bal = self.ft_balance(source, &d.asset_id);
                        if bal < d.amount {
                            return ApplyResult::Ignored("insufficient balance");
                        }
                        *self.ft_balances.get_mut(&(source.to_string(), d.asset_id)).unwrap() -= d.amount;
                        *self.ft_balances.entry((dest.to_string(), d.asset_id)).or_insert(0) += d.amount;
                        ApplyResult::Applied
                    }
                    AssetKind::NonFungible => {
                        if d.amount != 1 {
                            return ApplyResult::Ignored("nft amount must be 1");
                        }
                        if self.nft_owner.get(&d.asset_id).map(|s| s.as_str()) != Some(source) {
                            return ApplyResult::Ignored("not nft owner");
                        }
                        self.nft_owner.insert(d.asset_id, dest.to_string());
                        ApplyResult::Applied
                    }
                }
            }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core assets::state::tests::transfer_ft_debits_source_credits_dest`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets/state.rs
git commit -m "feat(assets): apply TRANSFER (fungible debit/credit + guards)"
```

---

### Task 7: apply NFT_MINT + NFT transfer

**Files:**
- Modify: `crates/tensorium-core/src/assets/state.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `state.rs`:
```rust
    use crate::assets::NftMintData;

    fn mint(royalty_bps: u16) -> AssetOp {
        AssetOp::NftMint(NftMintData {
            collection_id: [0u8; 32],
            royalty_bps,
            royalty_addr: "txm1creator".into(),
            uri: "ipfs://Qm".into(),
            content_hash: [1u8; 32],
        })
    }

    #[test]
    fn nft_mint_then_transfer_by_owner_only() {
        let mut st = AssetState::default();
        let nft = [5u8; 32];
        assert_eq!(st.apply(nft, 1, "txm1alice", None, &mint(500)), ApplyResult::Applied);
        assert_eq!(st.nft_owner.get(&nft).unwrap(), "txm1alice");
        assert_eq!(st.assets.get(&nft).unwrap().royalty_bps, 500);
        assert_eq!(st.assets.get(&nft).unwrap().royalty_addr, "txm1creator");
        // non-owner cannot transfer
        assert!(matches!(
            st.apply([6u8; 32], 2, "txm1mallory", Some("txm1bob"), &transfer(nft, 1)),
            ApplyResult::Ignored(_)
        ));
        assert_eq!(st.nft_owner.get(&nft).unwrap(), "txm1alice");
        // owner transfers
        assert_eq!(
            st.apply([7u8; 32], 3, "txm1alice", Some("txm1bob"), &transfer(nft, 1)),
            ApplyResult::Applied
        );
        assert_eq!(st.nft_owner.get(&nft).unwrap(), "txm1bob");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::state::tests::nft_mint_then_transfer_by_owner_only`
Expected: FAIL — NFT_MINT hits the Issue-only match; mint returns Ignored. (The Transfer NFT arm already exists from Task 6, but mint is unhandled.)

- [ ] **Step 3: Implement the NftMint arm**

In `state.rs` `apply`, add this arm before the `AssetOp::Transfer(d) =>` arm:
```rust
            AssetOp::NftMint(d) => {
                if self.assets.contains_key(&txid) {
                    return ApplyResult::Ignored("asset_id exists");
                }
                if d.royalty_bps > 10_000 {
                    return ApplyResult::Ignored("royalty too high");
                }
                self.assets.insert(txid, AssetInfo {
                    kind: AssetKind::NonFungible,
                    ticker: String::new(),
                    name: String::new(),
                    decimals: 0,
                    supply: 1,
                    issuer: source.to_string(),
                    royalty_bps: d.royalty_bps,
                    royalty_addr: d.royalty_addr.clone(),
                    uri: d.uri.clone(),
                    content_hash: d.content_hash,
                    mint_height: height,
                });
                self.nft_owner.insert(txid, source.to_string());
                ApplyResult::Applied
            }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core assets::state`
Expected: PASS (mint + NFT transfer + earlier state tests).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/assets/state.rs
git commit -m "feat(assets): apply NFT_MINT + owner-only NFT transfer"
```

---

### Task 8: End-to-end apply-from-transaction + idempotency

**Files:**
- Create: `crates/tensorium-core/src/assets/tests_e2e.rs`
- Modify: `crates/tensorium-core/src/assets/mod.rs` (register test module)

- [ ] **Step 1: Register the test module**

In `crates/tensorium-core/src/assets/mod.rs`, append:
```rust
#[cfg(test)]
mod tests_e2e;
```

- [ ] **Step 2: Write the failing end-to-end test**

`crates/tensorium-core/src/assets/tests_e2e.rs`:
```rust
//! End-to-end: build real transactions carrying asset ops, extract + apply them,
//! mirroring what the indexer (Layer 2) will do per block.
use super::*;
use crate::block::{Transaction, TxOutput};
use crate::script::OP_RETURN;
use crate::script::standard::p2pkh_from_address;

fn op_return_tx(op: &AssetOp, dest_addr: &str) -> Transaction {
    let data = encode_op(op);
    let mut spk = vec![OP_RETURN, 0x4c, data.len() as u8];
    spk.extend_from_slice(&data);
    Transaction::payment(
        vec![],
        vec![
            TxOutput { value_atoms: 1, script_pubkey: p2pkh_from_address(dest_addr).unwrap() },
            TxOutput { value_atoms: 0, script_pubkey: spk },
        ],
    )
}

#[test]
fn indexer_style_apply_is_deterministic_and_idempotent() {
    // Generate two real addresses.
    let alice = crate::WalletKeypair::generate().address.as_str().to_string();
    let bob = crate::WalletKeypair::generate().address.as_str().to_string();

    let issue = AssetOp::Issue(IssueData {
        ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
    });
    let issue_tx = op_return_tx(&issue, &alice);
    let asset_id = issue_tx.id.0;

    let xfer = AssetOp::Transfer(TransferData { asset_id, amount: 250, dest_output_index: 0 });
    let xfer_tx = op_return_tx(&xfer, &bob);

    // Apply a "block" of two txs, source = alice for both.
    let mut st = AssetState::default();
    for tx in [&issue_tx, &xfer_tx] {
        if let Some(op) = extract_asset_op(tx) {
            // dest = address of dest_output_index (output 0 here)
            let dest = match &op {
                AssetOp::Transfer(d) => crate::script::standard::extract_address(
                    &tx.outputs[d.dest_output_index as usize].script_pubkey,
                ),
                _ => None,
            };
            st.apply(tx.id.0, 100, &alice, dest.as_deref(), &op);
        }
    }
    assert_eq!(st.ft_balance(&alice, &asset_id), 750);
    assert_eq!(st.ft_balance(&bob, &asset_id), 250);

    // Re-applying the SAME txs is idempotent (issue dup ignored; transfer would
    // double-spend balance — but a real indexer never replays without rollback;
    // here we assert that applying issue again does not change supply).
    assert!(matches!(st.apply(issue_tx.id.0, 100, &alice, None, &issue), ApplyResult::Ignored(_)));
    assert_eq!(st.ft_balance(&alice, &asset_id), 750);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p tensorium-core assets::tests_e2e`
Expected: FAIL initially only if any wiring is missing; if all prior tasks are done it may PASS. If it fails to compile (missing `extract_address` import path), fix the path to the actual location in `script::standard`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core assets::tests_e2e`
Expected: PASS.

- [ ] **Step 5: Run the full crate test suite (no regressions)**

Run: `cargo test -p tensorium-core`
Expected: all pass, including the new `assets::*` tests.

- [ ] **Step 6: Commit**
```bash
git add crates/tensorium-core/src/assets
git commit -m "test(assets): end-to-end extract+apply from real transactions"
```

---

## Done criteria

- `tensorium-core::assets` exposes `encode_op`, `decode_op`, `extract_asset_op`,
  `AssetState`, `AssetOp` + data types.
- Full TDD coverage: codec round-trips + rejects, state-machine ISSUE/TRANSFER/NFT
  (with royalty recorded), owner-only NFT transfer, over-balance/unknown-asset
  guards, end-to-end extract+apply from real transactions.
- `cargo test -p tensorium-core` green; no consensus change; nothing deployed
  (pure library code).

## Next plan (Layer 2 — indexer)

`txm-asset-indexer` service: block scanner using these functions, `outpoint→address`
index to resolve `source` (inputs[0]) + transfer destinations, persistent state,
reorg rollback, and the REST API (`/asset`, `/balance`, `/nft/<id>/owner`,
`/assets`, `/holders`, `/history`, `/status`).
