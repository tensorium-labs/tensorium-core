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
}

/// Key for the outpoint index.
pub fn outpoint_key(txid: &Hash256, vout: u32) -> String {
    format!("{}:{}", txid.to_hex(), vout)
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
}
