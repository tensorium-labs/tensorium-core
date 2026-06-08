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

/// A rolled-back-to snapshot of the indexer's deterministic state (everything
/// except the checkpoint itself), used to recover from a chain reorg without
/// rescanning from genesis.
#[derive(Clone, Default)]
pub struct CheckpointSnap {
    pub state: AssetState,
    pub outpoints: HashMap<String, String>,
    pub history: HashMap<String, Vec<HistoryEntry>>,
    pub last_height: u64,
    pub last_hash: String,
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
    /// Periodic buried snapshot `(height, block_hash_hex, state)` for cheap reorg
    /// rollback. Not persisted — rebuilt as scanning proceeds.
    pub checkpoint: Option<(u64, String, CheckpointSnap)>,
}

/// Key for the outpoint index.
pub fn outpoint_key(txid: &Hash256, vout: u32) -> String {
    format!("{}:{}", txid.to_hex(), vout)
}

impl Indexer {
    /// Clone the deterministic state (without the checkpoint) for a checkpoint.
    pub fn snapshot(&self) -> CheckpointSnap {
        CheckpointSnap {
            state: self.state.clone(),
            outpoints: self.outpoints.clone(),
            history: self.history.clone(),
            last_height: self.last_height,
            last_hash: self.last_hash.clone(),
        }
    }

    /// Restore the deterministic state from a checkpoint snapshot (keeps the
    /// existing `checkpoint` field so it can be reused for the next reorg).
    pub fn restore(&mut self, snap: CheckpointSnap) {
        self.state = snap.state;
        self.outpoints = snap.outpoints;
        self.history = snap.history;
        self.last_height = snap.last_height;
        self.last_hash = snap.last_hash;
        self.scanned_any = true;
    }
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

    /// Apply every tx in `block` in canonical order: record outputs, and for the
    /// first valid `TXMA` op, resolve source + dest and apply it to the asset state.
    pub fn apply_block(&mut self, block: &tensorium_core::block::Block, height: u64) {
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
}
