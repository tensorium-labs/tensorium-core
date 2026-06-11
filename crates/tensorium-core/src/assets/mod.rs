//! TXM asset overlay protocol (TXM20 fungible tokens + NFTs).
//! Asset operations ride inside ordinary TXM transactions as `OP_RETURN`
//! metadata; balances/ownership are a deterministic function of the chain.
//! This module is pure (no I/O) — shared by the wallet and the indexer.
pub mod codec;
pub mod state;

pub use codec::{decode_op, encode_op, extract_asset_op, op_return_script};
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

#[cfg(test)]
mod tests_e2e;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::standard::p2pkh_from_address;

    const VALID_ADDR: &str = "txm1px2nmtp087mz8dv3lplqadwzxawk0c5kg0mt24";

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
        let change_addr = VALID_ADDR;
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
        let to_addr = VALID_ADDR;
        let change_addr = VALID_ADDR;
        let outputs = build_outputs(&op, Some((to_addr, 1_000)), change_addr, 200_000, 100_000).unwrap();
        // [dest, OP_RETURN, change]
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].value_atoms, 1_000);
        assert_eq!(outputs[0].script_pubkey, p2pkh_from_address(to_addr).unwrap());
        assert_eq!(outputs[1].value_atoms, 0);
        assert_eq!(outputs[2].value_atoms, 99_000); // 200_000 - 1_000 - 100_000
    }

    #[test]
    fn build_outputs_insufficient_funds_errors() {
        let op = issue_op();
        let change_addr = VALID_ADDR;
        let err = build_outputs(&op, None, change_addr, 50_000, 100_000).unwrap_err();
        assert!(err.contains("insufficient"), "unexpected error: {err}");
    }

    #[test]
    fn build_outputs_invalid_dest_address_errors() {
        let op = issue_op();
        let change_addr = VALID_ADDR;
        let err = build_outputs(&op, Some(("not-an-address", 1_000)), change_addr, 1_000_000, 100_000)
            .unwrap_err();
        assert!(err.contains("invalid recipient"), "unexpected error: {err}");
    }
}
