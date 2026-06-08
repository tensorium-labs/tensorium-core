# TXM Marketplace — Atomic Settlement (Layer 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement trustless atomic asset⇄TXM settlement — pure `build_settlement_tx` / `verify_settlement` / `fee_split` in `tensorium-core`, a per-input signer `WalletKeypair::sign_input`, and the `txmwallet` co-sign flow (`asset-sell` → `asset-buy` → `asset-accept`).

**Architecture:** A trade is one transaction: `inputs[0]` = seller (asset source), `inputs[1..]` = buyer; outputs = `[buyer carrier, TXMA transfer OP_RETURN, seller proceeds, 2.5% platform fee, royalty, buyer change]`. Both parties sign the same whole-tx `signature_hash()` and stamp only their own input. The buyer/seller each run `verify_settlement` (input-value-independent: exact fee/royalty/dest/transfer, `≥` seller proceeds) before signing — that's the trust anchor. The node enforces TXM conservation + signatures; the indexer applies the asset transfer. No consensus change, no custody.

**Tech Stack:** Rust, `tensorium-core` (`block`, `assets`, `script::standard`, `hash`, `wallet`), `txmwallet` (`ureq` RPC, `serde_json`), `cargo test`.

**Key facts (verified):**
- `signature_hash()` hashes outpoints + outputs + payload with signature-scripts zeroed → both parties sign the same hash, stamp their own input, order-independent.
- `sign_transaction` stamps the same P2PKH scriptSig onto *every* input (single-owner only); co-signing needs a per-input variant.
- Node skips `OP_RETURN` outputs from the UTXO set; the carrier + fee + royalty are ordinary P2PKH outputs.
- `txmwallet` HTTP is `ureq` via `rpc_get(host, path)` / `rpc_post(host, path, body)`; `/getutxos/<addr>` returns `{utxos:[{txid_bytes,output_index,value_atoms,mature,...}]}`.
- Platform fee address = pool-treasury `txm13vgxzj5ulrfhe7x0mlzxg0q6veq42tkku4g3jr` (reused; no new wallet).

## File Structure

- Modify: `crates/tensorium-core/src/wallet.rs` — `WalletError::InputIndexOutOfRange` + `WalletKeypair::sign_input`.
- Create: `crates/tensorium-core/src/settlement.rs` — constants, `SettlementTerms`, `fee_split`, `build_settlement_tx`, `verify_settlement`.
- Modify: `crates/tensorium-core/src/lib.rs` — register `settlement` module + re-exports.
- Modify: `crates/txmwallet/src/main.rs` — `AssetOrder`/`SettlementFile` structs + `asset-sell` / `asset-buy` / `asset-accept` commands + help.

---

### Task 1: `WalletKeypair::sign_input` (per-input signer)

**Files:**
- Modify: `crates/tensorium-core/src/wallet.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/tensorium-core/src/wallet.rs` (if no test module exists, create `#[cfg(test)] mod tests { use super::*; ... }` at the end of the file):
```rust
    #[test]
    fn sign_input_stamps_only_target_index() {
        use crate::block::{OutPoint, Transaction, TxInput, TxOutput};
        let seller = WalletKeypair::generate();
        let buyer = WalletKeypair::generate();
        let mut tx = Transaction::payment(
            vec![
                TxInput { previous_output: OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, signature_script: vec![] },
                TxInput { previous_output: OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, signature_script: vec![] },
            ],
            vec![TxOutput { value_atoms: 10, script_pubkey: vec![0x6a] }],
        );
        let hash_before = tx.signature_hash();
        seller.sign_input(&mut tx, 0).unwrap();
        // Only input 0 is stamped.
        assert!(!tx.inputs[0].signature_script.is_empty());
        assert!(tx.inputs[1].signature_script.is_empty());
        // The signed hash is unchanged (scripts are excluded from signature_hash).
        assert_eq!(tx.signature_hash(), hash_before);
        buyer.sign_input(&mut tx, 1).unwrap();
        assert!(!tx.inputs[1].signature_script.is_empty());
        // Out-of-range index errors.
        assert!(seller.sign_input(&mut tx, 9).is_err());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core wallet::tests::sign_input_stamps_only_target_index`
Expected: FAIL — `sign_input` not found.

- [ ] **Step 3: Add the error variant + `sign_input`**

In `crates/tensorium-core/src/wallet.rs`, add a variant to the `WalletError` enum (after `InvalidSignature`):
```rust
    #[error("input index out of range")]
    InputIndexOutOfRange,
```
Add this method to the `impl WalletKeypair` block, right after `sign_transaction`:
```rust
    /// Sign the whole-tx `signature_hash()` and stamp the P2PKH scriptSig onto
    /// ONLY input `index` (the others are left untouched). Used for co-signed
    /// multi-party transactions where each party signs only its own input.
    pub fn sign_input(&self, tx: &mut Transaction, index: usize) -> Result<(), WalletError> {
        if index >= tx.inputs.len() {
            return Err(WalletError::InputIndexOutOfRange);
        }
        let private_key_bytes =
            hex::decode(&self.private_key_hex).map_err(|_| WalletError::InvalidPrivateKey)?;
        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(secret_key);
        let signature_hash = tx.signature_hash();
        let signature: Signature = signing_key.sign(&signature_hash.0);
        let der_bytes = signature.to_der().as_bytes().to_vec();
        let pubkey_bytes = hex::decode(&self.public_key_hex)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        let script_sig = crate::script::standard::p2pkh_script_sig(&der_bytes, &pubkey_bytes);
        tx.inputs[index].signature_script = script_sig;
        tx.refresh_id();
        Ok(())
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core wallet::tests::sign_input_stamps_only_target_index`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/wallet.rs
git commit -m "feat(wallet): WalletKeypair::sign_input for co-signed multi-party txs"
```

---

### Task 2: Settlement module scaffold + `fee_split`

**Files:**
- Create: `crates/tensorium-core/src/settlement.rs`
- Modify: `crates/tensorium-core/src/lib.rs`

- [ ] **Step 1: Create the module with constants, terms, and `fee_split`**

`crates/tensorium-core/src/settlement.rs`:
```rust
//! Trustless atomic asset⇄TXM settlement (marketplace Layer 4).
//! One co-signed tx moves the asset, pays the seller, collects the platform
//! fee + creator royalty, and returns change. Pure — no I/O.
use crate::assets::{encode_op, op_return_script, AssetOp, TransferData};
use crate::block::{OutPoint, Transaction, TxInput, TxOutput};
use crate::script::standard::{extract_address, p2pkh_from_address};

/// Platform fee in basis points (2.5%).
pub const PLATFORM_FEE_BPS: u16 = 250;
/// Platform fee recipient — the existing pool-treasury / operations wallet.
pub const PLATFORM_FEE_ADDRESS: &str = "txm13vgxzj5ulrfhe7x0mlzxg0q6veq42tkku4g3jr";
/// Dust placed on the buyer's asset-destination output.
pub const CARRIER_ATOMS: u64 = 1_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SettlementTerms {
    pub asset_id: [u8; 32],
    pub amount: u64,
    pub price_atoms: u64,
    pub royalty_bps: u16,
    pub royalty_addr: String,
    pub seller_addr: String,
    pub buyer_addr: String,
    pub miner_fee_atoms: u64,
}

/// `(platform_fee, royalty)` = `floor(price·bps/10000)`. Royalty is zero when
/// `royalty_bps == 0` or the seller IS the royalty address (no self-payment).
pub fn fee_split(price: u64, royalty_bps: u16, seller_addr: &str, royalty_addr: &str) -> (u64, u64) {
    let platform_fee = (price as u128 * PLATFORM_FEE_BPS as u128 / 10_000) as u64;
    let royalty = if royalty_bps == 0 || seller_addr == royalty_addr {
        0
    } else {
        (price as u128 * royalty_bps as u128 / 10_000) as u64
    };
    (platform_fee, royalty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_split_computes_platform_and_royalty() {
        // 2.5% of 1_000_000 = 25_000; 5% royalty = 50_000.
        assert_eq!(fee_split(1_000_000, 500, "txm1seller", "txm1creator"), (25_000, 50_000));
        // Seller == royalty address → no royalty (primary sale).
        assert_eq!(fee_split(1_000_000, 500, "txm1creator", "txm1creator"), (25_000, 0));
        // Zero royalty bps → no royalty.
        assert_eq!(fee_split(1_000_000, 0, "txm1seller", "txm1creator"), (25_000, 0));
        // Floor rounding.
        assert_eq!(fee_split(999, 0, "a", "b"), (24, 0)); // 999*250/10000 = 24.975 → 24
    }
}
```

- [ ] **Step 2: Register the module in lib.rs**

In `crates/tensorium-core/src/lib.rs`, add alongside the other `pub mod` lines (e.g. after `pub mod script;`):
```rust
pub mod settlement;
```
And alongside the other `pub use` lines:
```rust
pub use settlement::{build_settlement_tx, fee_split, verify_settlement, SettlementTerms};
```

- [ ] **Step 3: Add temporary stubs so the re-export resolves**

`build_settlement_tx` and `verify_settlement` are added in Tasks 3–4. To keep `lib.rs` compiling now, append these stubs to `settlement.rs` (above `#[cfg(test)]`); they are replaced with real bodies in the next tasks:
```rust
/// (stub — implemented in Task 3)
pub fn build_settlement_tx(
    _terms: &SettlementTerms,
    _seller_input: (OutPoint, u64),
    _buyer_inputs: &[(OutPoint, u64)],
) -> Result<Transaction, String> {
    unimplemented!()
}

/// (stub — implemented in Task 4)
pub fn verify_settlement(_tx: &Transaction, _terms: &SettlementTerms) -> Vec<String> {
    unimplemented!()
}
```

- [ ] **Step 4: Run the test + build**

Run: `cargo test -p tensorium-core settlement::tests::fee_split_computes_platform_and_royalty`
Expected: PASS (and the crate builds with the stubs).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/settlement.rs crates/tensorium-core/src/lib.rs
git commit -m "feat(settlement): module scaffold + fee_split (2.5% fee + royalty)"
```

---

### Task 3: `build_settlement_tx`

**Files:**
- Modify: `crates/tensorium-core/src/settlement.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `settlement.rs`:
```rust
    use crate::script::standard::extract_address;
    use crate::WalletKeypair;

    fn terms(price: u64, royalty_bps: u16, seller: &str, buyer: &str, royalty: &str) -> SettlementTerms {
        SettlementTerms {
            asset_id: [7u8; 32],
            amount: 5,
            price_atoms: price,
            royalty_bps,
            royalty_addr: royalty.into(),
            seller_addr: seller.into(),
            buyer_addr: buyer.into(),
            miner_fee_atoms: 10_000,
        }
    }

    #[test]
    fn build_lays_out_outputs_and_conserves_value() {
        let seller = WalletKeypair::generate().address.as_str().to_string();
        let buyer = WalletKeypair::generate().address.as_str().to_string();
        let creator = WalletKeypair::generate().address.as_str().to_string();
        let t = terms(1_000_000, 500, &seller, &buyer, &creator); // 2.5% fee=25k, 5% royalty=50k

        let v_seller = 3_000u64;
        let v_buyer = 1_100_000u64; // covers price + carrier + miner_fee
        let tx = build_settlement_tx(
            &t,
            (OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, v_seller),
            &[(OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, v_buyer)],
        )
        .unwrap();

        // inputs: seller first, then buyer.
        assert_eq!(tx.inputs.len(), 2);
        // outputs: carrier, OP_RETURN, seller, fee, royalty, change = 6.
        assert_eq!(tx.outputs.len(), 6);
        assert_eq!(extract_address(&tx.outputs[0].script_pubkey).as_deref(), Some(buyer.as_str()));
        assert_eq!(tx.outputs[0].value_atoms, CARRIER_ATOMS);
        // seller proceeds = v_seller + price - fee - royalty.
        assert_eq!(extract_address(&tx.outputs[2].script_pubkey).as_deref(), Some(seller.as_str()));
        assert_eq!(tx.outputs[2].value_atoms, 3_000 + 1_000_000 - 25_000 - 50_000);
        // fee + royalty present.
        assert_eq!(tx.outputs[3].value_atoms, 25_000);
        assert_eq!(extract_address(&tx.outputs[3].script_pubkey).as_deref(), Some(PLATFORM_FEE_ADDRESS));
        assert_eq!(tx.outputs[4].value_atoms, 50_000);
        assert_eq!(extract_address(&tx.outputs[4].script_pubkey).as_deref(), Some(creator.as_str()));
        // conservation: inputs - outputs = miner_fee.
        let in_sum = v_seller + v_buyer;
        let out_sum: u64 = tx.outputs.iter().map(|o| o.value_atoms).sum();
        assert_eq!(in_sum - out_sum, t.miner_fee_atoms);
    }

    #[test]
    fn build_omits_royalty_and_change_when_zero() {
        let seller = WalletKeypair::generate().address.as_str().to_string();
        let buyer = WalletKeypair::generate().address.as_str().to_string();
        // No royalty; buyer funds EXACTLY price + carrier + miner_fee → no change.
        let t = terms(1_000_000, 0, &seller, &buyer, &seller);
        let v_buyer = 1_000_000 + CARRIER_ATOMS + 10_000;
        let tx = build_settlement_tx(
            &t,
            (OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, 2_000),
            &[(OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, v_buyer)],
        )
        .unwrap();
        // carrier, OP_RETURN, seller, fee = 4 (no royalty, no change).
        assert_eq!(tx.outputs.len(), 4);
    }

    #[test]
    fn build_rejects_insufficient_buyer_funds() {
        let seller = WalletKeypair::generate().address.as_str().to_string();
        let buyer = WalletKeypair::generate().address.as_str().to_string();
        let t = terms(1_000_000, 0, &seller, &buyer, &seller);
        assert!(build_settlement_tx(
            &t,
            (OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, 2_000),
            &[(OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, 500_000)],
        )
        .is_err());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core settlement::tests::build_lays_out_outputs_and_conserves_value`
Expected: FAIL — hits the `unimplemented!()` stub.

- [ ] **Step 3: Replace the `build_settlement_tx` stub with the real body**

In `settlement.rs`, replace the `build_settlement_tx` stub with:
```rust
/// Build the unsigned settlement tx in canonical layout. `seller_input` and
/// `buyer_inputs` are `(OutPoint, value)`. Errors on insufficient buyer funds
/// or `fee + royalty > price`.
pub fn build_settlement_tx(
    terms: &SettlementTerms,
    seller_input: (OutPoint, u64),
    buyer_inputs: &[(OutPoint, u64)],
) -> Result<Transaction, String> {
    let (platform_fee, royalty) =
        fee_split(terms.price_atoms, terms.royalty_bps, &terms.seller_addr, &terms.royalty_addr);
    if platform_fee + royalty > terms.price_atoms {
        return Err("fee + royalty exceeds price".to_owned());
    }
    let v_seller = seller_input.1;
    let v_buyer: u64 = buyer_inputs.iter().map(|(_, v)| *v).sum();
    let buyer_need = terms.price_atoms + CARRIER_ATOMS + terms.miner_fee_atoms;
    if v_buyer < buyer_need {
        return Err(format!("insufficient buyer funds: have {v_buyer}, need {buyer_need}"));
    }
    let seller_proceeds = v_seller + terms.price_atoms - platform_fee - royalty;
    let change = v_buyer - terms.price_atoms - CARRIER_ATOMS - terms.miner_fee_atoms;

    let mut inputs = vec![TxInput { previous_output: seller_input.0, signature_script: Vec::new() }];
    for (op, _) in buyer_inputs {
        inputs.push(TxInput { previous_output: *op, signature_script: Vec::new() });
    }

    let transfer = AssetOp::Transfer(TransferData {
        asset_id: terms.asset_id,
        amount: terms.amount,
        dest_output_index: 0,
    });
    let p2pkh = |a: &str| p2pkh_from_address(a).map_err(|_| format!("invalid address: {a}"));
    let mut outputs = vec![
        TxOutput { value_atoms: CARRIER_ATOMS, script_pubkey: p2pkh(&terms.buyer_addr)? },
        TxOutput { value_atoms: 0, script_pubkey: op_return_script(&encode_op(&transfer)) },
        TxOutput { value_atoms: seller_proceeds, script_pubkey: p2pkh(&terms.seller_addr)? },
    ];
    if platform_fee > 0 {
        outputs.push(TxOutput { value_atoms: platform_fee, script_pubkey: p2pkh(PLATFORM_FEE_ADDRESS)? });
    }
    if royalty > 0 {
        outputs.push(TxOutput { value_atoms: royalty, script_pubkey: p2pkh(&terms.royalty_addr)? });
    }
    if change > 0 {
        outputs.push(TxOutput { value_atoms: change, script_pubkey: p2pkh(&terms.buyer_addr)? });
    }
    Ok(Transaction::payment(inputs, outputs))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tensorium-core settlement::tests`
Expected: the three `build_*` tests pass (plus `fee_split`).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/settlement.rs
git commit -m "feat(settlement): build_settlement_tx canonical co-signed layout"
```

---

### Task 4: `verify_settlement` (trust anchor)

**Files:**
- Modify: `crates/tensorium-core/src/settlement.rs`

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `settlement.rs`:
```rust
    fn built() -> (Transaction, SettlementTerms) {
        let seller = WalletKeypair::generate().address.as_str().to_string();
        let buyer = WalletKeypair::generate().address.as_str().to_string();
        let creator = WalletKeypair::generate().address.as_str().to_string();
        let t = terms(1_000_000, 500, &seller, &buyer, &creator);
        let tx = build_settlement_tx(
            &t,
            (OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, 3_000),
            &[(OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, 1_100_000)],
        )
        .unwrap();
        (tx, t)
    }

    #[test]
    fn verify_accepts_a_well_formed_settlement() {
        let (tx, t) = built();
        assert!(verify_settlement(&tx, &t).is_empty());
    }

    #[test]
    fn verify_rejects_reduced_platform_fee() {
        let (mut tx, t) = built();
        tx.outputs[3].value_atoms -= 1; // skim the platform fee
        assert!(!verify_settlement(&tx, &t).is_empty());
    }

    #[test]
    fn verify_rejects_wrong_buyer_destination() {
        let (mut tx, t) = built();
        let attacker = WalletKeypair::generate().address.as_str().to_string();
        tx.outputs[0].script_pubkey = p2pkh_from_address(&attacker).unwrap();
        assert!(!verify_settlement(&tx, &t).is_empty());
    }

    #[test]
    fn verify_rejects_removed_royalty() {
        let (mut tx, t) = built();
        tx.outputs.remove(4); // drop the royalty output
        assert!(!verify_settlement(&tx, &t).is_empty());
    }

    #[test]
    fn verify_rejects_underpaid_seller() {
        let (mut tx, t) = built();
        tx.outputs[2].value_atoms = 1; // seller proceeds far below net
        assert!(!verify_settlement(&tx, &t).is_empty());
    }

    #[test]
    fn verify_rejects_wrong_asset_or_amount() {
        let (tx, t) = built();
        let mut wrong_amount = t.clone();
        wrong_amount.amount = 999;
        assert!(!verify_settlement(&tx, &wrong_amount).is_empty());
        let mut wrong_asset = t.clone();
        wrong_asset.asset_id = [0u8; 32];
        assert!(!verify_settlement(&tx, &wrong_asset).is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-core settlement::tests::verify_accepts_a_well_formed_settlement`
Expected: FAIL — hits the `unimplemented!()` stub.

- [ ] **Step 3: Replace the `verify_settlement` stub with the real body**

In `settlement.rs`, replace the `verify_settlement` stub with:
```rust
/// Trust anchor: assert the trust-critical invariants derivable from `terms`
/// alone. Returns the list of mismatches (empty = valid). Input-value-independent.
pub fn verify_settlement(tx: &Transaction, terms: &SettlementTerms) -> Vec<String> {
    use crate::assets::extract_asset_op;
    let mut bad = Vec::new();
    let (platform_fee, royalty) =
        fee_split(terms.price_atoms, terms.royalty_bps, &terms.seller_addr, &terms.royalty_addr);

    // out[0]: buyer carrier (asset destination).
    match tx.outputs.first() {
        Some(o)
            if o.value_atoms == CARRIER_ATOMS
                && extract_address(&o.script_pubkey).as_deref() == Some(terms.buyer_addr.as_str()) => {}
        _ => bad.push("out[0] is not the buyer carrier".to_owned()),
    }

    // The first TXMA op must be the expected transfer.
    match extract_asset_op(tx) {
        Some(AssetOp::Transfer(d))
            if d.asset_id == terms.asset_id && d.amount == terms.amount && d.dest_output_index == 0 => {}
        _ => bad.push("transfer op mismatch".to_owned()),
    }

    // Platform fee output, exact.
    if platform_fee > 0 && !has_output_exact(tx, PLATFORM_FEE_ADDRESS, platform_fee) {
        bad.push("platform fee output missing/incorrect".to_owned());
    }
    // Royalty output, exact (when applicable).
    if royalty > 0 && !has_output_exact(tx, &terms.royalty_addr, royalty) {
        bad.push("royalty output missing/incorrect".to_owned());
    }
    // Seller receives at least net proceeds (surplus = their refunded input).
    let min_proceeds = terms.price_atoms.saturating_sub(platform_fee + royalty);
    if !has_output_at_least(tx, &terms.seller_addr, min_proceeds) {
        bad.push("seller proceeds below net".to_owned());
    }
    bad
}

fn has_output_exact(tx: &Transaction, addr: &str, value: u64) -> bool {
    tx.outputs
        .iter()
        .any(|o| o.value_atoms == value && extract_address(&o.script_pubkey).as_deref() == Some(addr))
}

fn has_output_at_least(tx: &Transaction, addr: &str, min: u64) -> bool {
    tx.outputs
        .iter()
        .any(|o| o.value_atoms >= min && extract_address(&o.script_pubkey).as_deref() == Some(addr))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tensorium-core settlement::tests`
Expected: all settlement tests pass (fee_split + build + verify).

- [ ] **Step 5: Commit**
```bash
git add crates/tensorium-core/src/settlement.rs
git commit -m "feat(settlement): verify_settlement trust anchor + tamper detection"
```

---

### Task 5: End-to-end co-sign test (build + sign_input + verify)

**Files:**
- Modify: `crates/tensorium-core/src/settlement.rs`

- [ ] **Step 1: Write the test**

Add inside `mod tests` in `settlement.rs`:
```rust
    #[test]
    fn two_party_cosign_produces_a_valid_settlement() {
        use crate::assets::{extract_asset_op, AssetOp};
        let seller_kp = WalletKeypair::generate();
        let buyer_kp = WalletKeypair::generate();
        let seller = seller_kp.address.as_str().to_string();
        let buyer = buyer_kp.address.as_str().to_string();
        let t = terms(1_000_000, 0, &seller, &buyer, &seller); // no royalty

        let mut tx = build_settlement_tx(
            &t,
            (OutPoint { txid: crate::hash::Hash256([1u8; 32]), output_index: 0 }, 2_000),
            &[(OutPoint { txid: crate::hash::Hash256([2u8; 32]), output_index: 0 }, 1_100_000)],
        )
        .unwrap();

        // Verify clean, then both parties sign their own input.
        assert!(verify_settlement(&tx, &t).is_empty());
        buyer_kp.sign_input(&mut tx, 1).unwrap();
        seller_kp.sign_input(&mut tx, 0).unwrap();

        assert!(!tx.inputs[0].signature_script.is_empty());
        assert!(!tx.inputs[1].signature_script.is_empty());
        // The asset still transfers to the buyer.
        match extract_asset_op(&tx) {
            Some(AssetOp::Transfer(d)) => {
                assert_eq!(d.asset_id, t.asset_id);
                assert_eq!(
                    extract_address(&tx.outputs[d.dest_output_index as usize].script_pubkey).as_deref(),
                    Some(buyer.as_str())
                );
            }
            _ => panic!("expected transfer"),
        }
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p tensorium-core settlement::tests::two_party_cosign_produces_a_valid_settlement`
Expected: PASS.

- [ ] **Step 3: Run the full core suite (no regressions)**

Run: `cargo test -p tensorium-core`
Expected: all pass (assets, settlement, wallet, etc.).

- [ ] **Step 4: Commit**
```bash
git add crates/tensorium-core/src/settlement.rs
git commit -m "test(settlement): end-to-end two-party co-sign produces valid tx"
```

---

### Task 6: Wallet `asset-sell` command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the settlement imports + handoff structs**

Extend the `use tensorium_core::{ ... }` block — add to the `assets::{...}` line so it reads `assets::{encode_op, op_return_script, AssetOp}` (unchanged) and add a new line for settlement:
```rust
    settlement::{build_settlement_tx, verify_settlement, SettlementTerms, CARRIER_ATOMS},
```
Then add these `serde` structs near the top of `main.rs` (after the `MultisigSigFile` struct):
```rust
/// Seller's listing handoff (seller → buyer).
#[derive(Debug, Serialize, Deserialize)]
struct AssetOrder {
    asset_id_hex: String,
    amount: u64,
    price_atoms: u64,
    seller_addr: String,
    seller_txid_hex: String,
    seller_vout: u32,
    seller_value: u64,
}

/// Built + buyer-signed settlement handoff (buyer → seller).
#[derive(Debug, Serialize, Deserialize)]
struct SettlementFile {
    tx: Transaction,
    terms: SettlementTerms,
}
```

- [ ] **Step 2: Add a small `/getutxos` fetch helper**

Add this helper near the other `build_*`/`rpc_*` functions in `main.rs`:
```rust
/// Fetch mature UTXOs for an address via the node RPC as `(OutPoint, value)`.
fn fetch_mature_utxos(
    rpc: &str,
    address: &str,
) -> Result<Vec<(tensorium_core::block::OutPoint, u64)>, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

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

    let body = rpc_get(rpc, &format!("/getutxos/{address}"))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;
    let mut out = Vec::new();
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
        out.push((OutPoint { txid: hash, output_index: u.output_index }, u.value_atoms));
    }
    Ok(out)
}
```

- [ ] **Step 3: Add the `asset-sell` command**

In the `match command { ... }` block, after the `"asset-transfer" => { ... }` arm, add:
```rust
        "asset-sell" => {
            // usage: txmwallet asset-sell <asset_id_hex> <amount> <price_atoms>
            let asset_id_hex = args
                .get(2)
                .ok_or("usage: txmwallet asset-sell <asset_id_hex> <amount> <price_atoms>")?
                .to_string();
            // validate hex length (32 bytes).
            if hex::decode(&asset_id_hex).map(|b| b.len()).unwrap_or(0) != 32 {
                return Err("asset_id must be 32 bytes (64 hex chars)".to_owned());
            }
            let amount: u64 = args.get(3).ok_or("missing amount")?.parse().map_err(|_| "amount must be a positive integer")?;
            let price_atoms: u64 = args.get(4).ok_or("missing price_atoms")?.parse().map_err(|_| "price_atoms must be a positive integer")?;

            let wallet = load_wallet(&wallet_path)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let utxos = fetch_mature_utxos(&rpc, &wallet.address)?;
            // Pick the smallest mature UTXO as inputs[0] (just needs to prove source).
            let (op, value) = utxos
                .into_iter()
                .min_by_key(|(_, v)| *v)
                .ok_or("no mature UTXO to anchor the sale (fund the wallet first)")?;

            let order = AssetOrder {
                asset_id_hex,
                amount,
                price_atoms,
                seller_addr: wallet.address.clone(),
                seller_txid_hex: op.txid.to_hex(),
                seller_vout: op.output_index,
                seller_value: value,
            };
            let path = PathBuf::from("asset-order.json");
            fs::write(&path, serde_json::to_string_pretty(&order).map_err(|e| format!("serialize: {e}"))?)
                .map_err(|e| format!("write {}: {e}", path.display()))?;
            println!("order_written={}", path.display());
            println!("send asset-order.json to the buyer; they run: txmwallet asset-buy asset-order.json");
        }
```
(Insert this arm before the existing `"unlock-check" => { ... }` arm.)

- [ ] **Step 4: Build to verify it compiles**

Run: `cargo build -p txmwallet`
Expected: builds. (If `Transaction` isn't already imported in `main.rs`, it is — via the existing `block::{Transaction, TxInput, TxOutput}` use.)

- [ ] **Step 5: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): asset-sell command + settlement handoff structs"
```

---

### Task 7: Wallet `asset-buy` + `asset-accept` commands + help

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the `asset-buy` command**

In the `match command { ... }` block, after the `"asset-sell" => { ... }` arm, add:
```rust
        "asset-buy" => {
            // usage: txmwallet asset-buy <asset-order.json>
            let order_path = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("asset-order.json"));
            let order: AssetOrder = serde_json::from_str(
                &fs::read_to_string(&order_path).map_err(|e| format!("read {}: {e}", order_path.display()))?,
            )
            .map_err(|e| format!("parse order: {e}"))?;

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let indexer = env::var("TENSORIUM_INDEXER").unwrap_or_else(|_| "127.0.0.1:23340".to_owned());

            // Fetch royalty terms from the indexer (deterministic, tamper-proof).
            #[derive(serde::Deserialize)]
            struct AssetInfoResp {
                royalty_bps: u16,
                royalty_addr: String,
            }
            let info_body = rpc_get(&indexer, &format!("/asset/{}", order.asset_id_hex))
                .map_err(|e| format!("indexer /asset lookup failed: {e}"))?;
            let info: AssetInfoResp =
                serde_json::from_str(&info_body).map_err(|e| format!("parse asset info: {e}"))?;

            let asset_id: [u8; 32] = hex::decode(&order.asset_id_hex)
                .map_err(|_| "bad asset_id hex".to_owned())?
                .as_slice()
                .try_into()
                .map_err(|_| "asset_id must be 32 bytes".to_owned())?;

            let terms = SettlementTerms {
                asset_id,
                amount: order.amount,
                price_atoms: order.price_atoms,
                royalty_bps: info.royalty_bps,
                royalty_addr: info.royalty_addr,
                seller_addr: order.seller_addr.clone(),
                buyer_addr: wallet.address.clone(),
                miner_fee_atoms: tensorium_core::mempool::MIN_RELAY_FEE_ATOMS,
            };

            // Fund the buyer side.
            let need = order.price_atoms + CARRIER_ATOMS + terms.miner_fee_atoms;
            let mut buyer_inputs = Vec::new();
            let mut total = 0u64;
            for (op, v) in fetch_mature_utxos(&rpc, &wallet.address)? {
                buyer_inputs.push((op, v));
                total += v;
                if total >= need {
                    break;
                }
            }
            if total < need {
                return Err(format!("insufficient buyer funds: have {total}, need {need}"));
            }

            let seller_txid = tensorium_core::hash::Hash256(
                hex::decode(&order.seller_txid_hex)
                    .map_err(|_| "bad seller txid hex".to_owned())?
                    .as_slice()
                    .try_into()
                    .map_err(|_| "seller txid must be 32 bytes".to_owned())?,
            );
            let seller_input = (
                tensorium_core::block::OutPoint { txid: seller_txid, output_index: order.seller_vout },
                order.seller_value,
            );

            let mut tx = build_settlement_tx(&terms, seller_input, &buyer_inputs)?;
            let mismatches = verify_settlement(&tx, &terms);
            if !mismatches.is_empty() {
                return Err(format!("self-built settlement failed verify: {mismatches:?}"));
            }
            // Sign only the buyer inputs (indices 1..).
            for i in 1..tx.inputs.len() {
                keypair.sign_input(&mut tx, i).map_err(|e| e.to_string())?;
            }

            let out = SettlementFile { tx, terms };
            let path = PathBuf::from("asset-settlement.json");
            fs::write(&path, serde_json::to_string_pretty(&out).map_err(|e| format!("serialize: {e}"))?)
                .map_err(|e| format!("write {}: {e}", path.display()))?;
            println!("settlement_written={}", path.display());
            println!("send asset-settlement.json back to the seller; they run: txmwallet asset-accept asset-settlement.json");
        }
        "asset-accept" => {
            // usage: txmwallet asset-accept <asset-settlement.json>
            let path = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("asset-settlement.json"));
            let mut file: SettlementFile = serde_json::from_str(
                &fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?,
            )
            .map_err(|e| format!("parse settlement: {e}"))?;

            // Seller's trust anchor: verify before signing.
            let mismatches = verify_settlement(&file.tx, &file.terms);
            if !mismatches.is_empty() {
                return Err(format!("settlement failed verify, refusing to sign: {mismatches:?}"));
            }

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            if file.terms.seller_addr != wallet.address {
                return Err("this wallet is not the seller for this settlement".to_owned());
            }
            // Seller signs input[0] only.
            keypair.sign_input(&mut file.tx, 0).map_err(|e| e.to_string())?;

            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let raw = serde_json::to_string(&file.tx).map_err(|e| format!("serialize tx: {e}"))?;
            let resp = rpc_post(&rpc, "/sendrawtransaction", &raw)?;
            println!("settlement_txid={}", file.tx.id);
            println!("node_response={resp}");
        }
        "unlock-check" => {
```
(Both arms go before the existing `"unlock-check" => { ... }` arm.)

- [ ] **Step 2: Add the commands to the help text**

In `fn print_help()`, after the `asset-transfer` line added in Layer 3, insert:
```rust
    println!("  asset-sell <asset_id_hex> <amount> <price_atoms>      list an asset for sale → asset-order.json");
    println!("  asset-buy <asset-order.json>                          build+sign the buyer side → asset-settlement.json");
    println!("  asset-accept <asset-settlement.json>                  verify+sign the seller side and broadcast");
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p txmwallet`
Expected: builds.

- [ ] **Step 4: Commit**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(wallet): asset-buy + asset-accept co-sign settlement commands"
```

---

### Task 8: Full-suite verification

**Files:** (none — verification only)

- [ ] **Step 1: Run the whole workspace suite**

Run: `cargo test --workspace`
Expected: all pass, including `tensorium-core::settlement::*`, `tensorium-core::wallet::tests::sign_input_*`, `tensorium-indexer`, and `txmwallet::asset_tests`.

- [ ] **Step 2: Build all binaries (release sanity)**

Run: `cargo build --workspace`
Expected: builds with no errors.

- [ ] **Step 3: Manual smoke (optional — requires two funded wallets + live node + running indexer)**

```bash
# Seller lists; buyer builds+signs; seller verifies+signs+broadcasts.
TENSORIUM_WALLET=seller.json txmwallet asset-sell <asset_id_hex> 100 5000000
# (send asset-order.json to buyer)
TENSORIUM_WALLET=buyer.json TENSORIUM_WALLET_PASSPHRASE=... \
  TENSORIUM_RPC=127.0.0.1:33332 TENSORIUM_INDEXER=127.0.0.1:23340 \
  txmwallet asset-buy asset-order.json
# (send asset-settlement.json back to seller)
TENSORIUM_WALLET=seller.json TENSORIUM_WALLET_PASSPHRASE=... \
  txmwallet asset-accept asset-settlement.json
# After it confirms: indexer shows the asset moved + fee/royalty paid.
curl -s http://127.0.0.1:23340/balance/<buyer_addr>
curl -s http://127.0.0.1:23340/balance/<seller_addr>
```
Expected: the asset moves seller→buyer, the seller is paid `price − fee − royalty`, the platform-fee address receives 2.5%, and (for a secondary sale) the creator receives the royalty.

- [ ] **Step 4: Commit (if help/text touched) or note clean**

```bash
git status   # if anything staged from verification fixes, commit; otherwise nothing to do
```

---

## Done criteria

- `tensorium-core::settlement` exposes `build_settlement_tx`, `verify_settlement`, `fee_split`, `SettlementTerms` + constants; `WalletKeypair::sign_input` stamps a single input.
- `txmwallet` has `asset-sell` / `asset-buy` / `asset-accept`: a trustless, atomic, co-signed asset⇄TXM trade with a 2.5% platform fee + creator royalty enforced by construction and verified before signing.
- TDD coverage: `sign_input` per-index stamping; `fee_split`; `build_settlement_tx` layout/conservation/insufficient-funds; `verify_settlement` clean + tamper detection (fee/dest/royalty/seller/asset/amount); end-to-end two-party co-sign.
- `cargo test --workspace` green; no consensus change; no custody (the only artifacts are ordinary outputs + an `OP_RETURN` the node treats as unspendable).

## Next cycles

- **Layer 4b (relay/backend):** listings board + order relay (price/status metadata only) so the order→partial→signed handoff isn't manual files.
- **Layer 5 (frontend):** `marketplace.tensoriumlabs.com` UI over the indexer + relay.