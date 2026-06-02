use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{Block, OutPoint, Transaction},
    chain::ConsensusParams,
    utxo::{UtxoError, UtxoSet},
};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MempoolError {
    #[error("coinbase transactions are not allowed in the mempool")]
    CoinbaseNotAllowed,
    #[error("transaction is already in the mempool")]
    AlreadyKnown,
    #[error("transaction conflicts with a transaction already in the mempool")]
    PendingConflict,
    #[error(transparent)]
    InvalidTransaction(#[from] UtxoError),
}

/// In-memory pool of unconfirmed transactions, persisted as JSON.
///
/// The map key is the hex-encoded txid so it serialises cleanly.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mempool {
    pub pending: HashMap<String, Transaction>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate `tx` against `utxos` and add it to the pool.
    pub fn add(
        &mut self,
        utxos: &UtxoSet,
        params: &ConsensusParams,
        tx: Transaction,
        tip_height: u64,
    ) -> Result<(), MempoolError> {
        if tx.is_coinbase() {
            return Err(MempoolError::CoinbaseNotAllowed);
        }
        let key = tx.id.to_hex();
        if self.pending.contains_key(&key) {
            return Err(MempoolError::AlreadyKnown);
        }
        if self.conflicts_with_pending(&tx) {
            return Err(MempoolError::PendingConflict);
        }
        utxos.validate_transaction(&tx, tip_height, params)?;
        self.pending.insert(key, tx);
        Ok(())
    }

    fn conflicts_with_pending(&self, tx: &Transaction) -> bool {
        let new_inputs: HashSet<OutPoint> = tx
            .inputs
            .iter()
            .map(|input| input.previous_output)
            .collect();
        self.pending.values().any(|pending| {
            pending
                .inputs
                .iter()
                .any(|input| new_inputs.contains(&input.previous_output))
        })
    }

    /// Return transactions suitable for inclusion in the next block.
    ///
    /// If two transactions spend the same output, only the first encountered
    /// (arbitrary order) is included.
    pub fn select_for_block(&self) -> Vec<Transaction> {
        let mut spent: HashSet<OutPoint> = HashSet::new();
        let mut selected = Vec::new();
        for tx in self.pending.values() {
            let conflict = tx.inputs.iter().any(|i| spent.contains(&i.previous_output));
            if conflict {
                continue;
            }
            for input in &tx.inputs {
                spent.insert(input.previous_output);
            }
            selected.push(tx.clone());
        }
        selected
    }

    /// Remove every transaction whose txid appears in `block`.
    pub fn remove_confirmed(&mut self, block: &Block) {
        for tx in &block.transactions {
            self.pending.remove(&tx.id.to_hex());
        }
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn is_known(&self, txid_hex: &str) -> bool {
        self.pending.contains_key(txid_hex)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        block::{Block, BlockHeader, OutPoint, Transaction, TxInput, TxOutput},
        chain::{TESTNET, TEST_PARAMS},
        hash::Hash256,
        script::standard::p2pkh_from_address,
        utxo::{UtxoEntry, UtxoSet},
        wallet::WalletKeypair,
    };

    use super::*;

    #[test]
    fn rejects_coinbase() {
        let mut mp = Mempool::new();
        let utxos = UtxoSet::new();
        let tx = Transaction::coinbase(1, 100, "miner");
        assert_eq!(
            mp.add(&utxos, &TESTNET, tx, 0),
            Err(MempoolError::CoinbaseNotAllowed)
        );
    }

    #[test]
    fn rejects_duplicate() {
        let mut mp = Mempool::new();
        let utxos = UtxoSet::new();

        // Create a non-coinbase tx with a fake input.  The first add will fail
        // MissingInput, so pre-insert directly to simulate "already in pool".
        let tx = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 1,
                script_pubkey: vec![0x00],
            }],
        );
        mp.pending.insert(tx.id.to_hex(), tx.clone());

        // Second add must fail with AlreadyKnown (checked before validation).
        assert_eq!(
            mp.add(&utxos, &TESTNET, tx, 0),
            Err(MempoolError::AlreadyKnown)
        );
    }

    #[test]
    fn remove_confirmed_clears_txs() {
        let mut mp = Mempool::new();
        let tx = Transaction::coinbase(99, 0, "placeholder");
        mp.pending.insert(tx.id.to_hex(), tx.clone());
        assert_eq!(mp.len(), 1);

        // A block that does not contain our tx should leave it alone.
        let other_tx = Transaction::coinbase(100, 0, "other");
        let fake_block = Block::new(
            BlockHeader {
                version: 1,
                chain_id: TESTNET.chain_id.to_owned(),
                height: 1,
                previous_hash: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                timestamp_seconds: 0,
                leading_zero_bits: 0,
                nonce: 0,
            },
            vec![other_tx],
        );
        mp.remove_confirmed(&fake_block);
        assert_eq!(mp.len(), 1);

        // A block that does contain our tx should remove it.
        let confirming_block = Block::new(
            fake_block.header.clone(), // reuse header; content differs via txs
            vec![tx],
        );
        mp.remove_confirmed(&confirming_block);
        assert_eq!(mp.len(), 0);
    }

    #[test]
    fn rejects_pending_double_spend() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();
        let outpoint = OutPoint {
            txid: Hash256([7; 32]),
            output_index: 0,
        };
        utxos.entries.insert(
            outpoint,
            UtxoEntry {
                output: TxOutput {
                    value_atoms: 100,
                    script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
                },
                created_height: 0,
                coinbase: false,
            },
        );

        let mut first = Transaction::payment(
            vec![TxInput {
                previous_output: outpoint,
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 60,
                script_pubkey: vec![0x00],
            }],
        );
        keypair.sign_transaction(&mut first).unwrap();

        let mut second = Transaction::payment(
            vec![TxInput {
                previous_output: outpoint,
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 40,
                script_pubkey: vec![0x00],
            }],
        );
        keypair.sign_transaction(&mut second).unwrap();

        let mut mp = Mempool::new();
        mp.add(&utxos, &TEST_PARAMS, first, 200).unwrap();

        assert_eq!(
            mp.add(&utxos, &TEST_PARAMS, second, 200),
            Err(MempoolError::PendingConflict)
        );
        assert_eq!(mp.len(), 1);
    }
}
