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

/// (stub — implemented in Task 4)
pub fn verify_settlement(_tx: &Transaction, _terms: &SettlementTerms) -> Vec<String> {
    unimplemented!()
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
}
