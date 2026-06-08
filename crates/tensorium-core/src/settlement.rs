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
}
