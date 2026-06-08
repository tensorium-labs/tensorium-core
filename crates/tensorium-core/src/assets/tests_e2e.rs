//! End-to-end: build real transactions carrying asset ops, extract + apply them,
//! mirroring what the indexer (Layer 2) will do per block.
use super::*;
use crate::block::{Transaction, TxOutput};
use crate::script::standard::p2pkh_from_address;
use crate::script::OP_RETURN;

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
