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
