use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{Block, OutPoint, Transaction},
    chain::ConsensusParams,
    utxo::{UtxoError, UtxoSet},
};

// ---------------------------------------------------------------------------
// Fee policy
// ---------------------------------------------------------------------------

/// Minimum fee a transaction must carry to be accepted into the mempool.
/// Blocks may include zero-fee transactions (miner's discretion), but the
/// reference node rejects sub-minimum-fee transactions at the RPC layer.
pub const MIN_RELAY_FEE_ATOMS: u64 = 10_000; // 0.0001 TXM

/// Suggested priority fee for faster inclusion when the mempool is congested.
pub const PRIORITY_FEE_ATOMS: u64 = 100_000; // 0.001 TXM

// ---------------------------------------------------------------------------
// MempoolEntry — stores a tx alongside its pre-calculated fee
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MempoolEntry {
    pub tx: Transaction,
    /// Implicit fee: sum(inputs) − sum(outputs), computed at insertion time.
    pub fee_atoms: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MempoolError {
    #[error("coinbase transactions are not allowed in the mempool")]
    CoinbaseNotAllowed,
    #[error("transaction is already in the mempool")]
    AlreadyKnown,
    #[error("transaction conflicts with a transaction already in the mempool")]
    PendingConflict,
    #[error("transaction fee {fee} atoms is below minimum relay fee {min} atoms")]
    FeeTooLow { fee: u64, min: u64 },
    #[error(transparent)]
    InvalidTransaction(#[from] UtxoError),
}

// ---------------------------------------------------------------------------
// Mempool
// ---------------------------------------------------------------------------

/// In-memory pool of unconfirmed transactions, persisted as JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mempool {
    pub pending: HashMap<String, MempoolEntry>,
}

impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate `tx` against `utxos` and add it to the pool.
    /// Rejects transactions below `MIN_RELAY_FEE_ATOMS`.
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
        let fee = utxos.validate_transaction(&tx, tip_height, params)?;
        if fee < MIN_RELAY_FEE_ATOMS {
            return Err(MempoolError::FeeTooLow {
                fee,
                min: MIN_RELAY_FEE_ATOMS,
            });
        }
        self.pending.insert(key, MempoolEntry { tx, fee_atoms: fee });
        Ok(())
    }

    fn conflicts_with_pending(&self, tx: &Transaction) -> bool {
        let new_inputs: HashSet<OutPoint> = tx
            .inputs
            .iter()
            .map(|input| input.previous_output)
            .collect();
        self.pending.values().any(|entry| {
            entry
                .tx
                .inputs
                .iter()
                .any(|input| new_inputs.contains(&input.previous_output))
        })
    }

    /// Return transactions for the next block, sorted by fee descending
    /// (highest-fee transactions are included first).
    ///
    /// Returns `(transactions, total_fee_atoms)` so callers can add fees
    /// to the coinbase reward without re-scanning.
    pub fn select_for_block(&self) -> (Vec<Transaction>, u64) {
        // Sort entries by fee descending.
        let mut entries: Vec<&MempoolEntry> = self.pending.values().collect();
        entries.sort_by(|a, b| b.fee_atoms.cmp(&a.fee_atoms));

        let mut spent: HashSet<OutPoint> = HashSet::new();
        let mut selected: Vec<Transaction> = Vec::new();
        let mut total_fees: u64 = 0;

        for entry in entries {
            let conflict = entry
                .tx
                .inputs
                .iter()
                .any(|i| spent.contains(&i.previous_output));
            if conflict {
                continue;
            }
            for input in &entry.tx.inputs {
                spent.insert(input.previous_output);
            }
            total_fees = total_fees.saturating_add(entry.fee_atoms);
            selected.push(entry.tx.clone());
        }
        (selected, total_fees)
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

    /// Fee statistics for the `/getmempoolinfo` RPC endpoint.
    pub fn fee_stats(&self) -> FeeStats {
        let count = self.pending.len() as u64;
        if count == 0 {
            return FeeStats {
                count,
                total_fee_atoms: 0,
                min_fee_atoms: 0,
                max_fee_atoms: 0,
                median_fee_atoms: 0,
                min_relay_fee_atoms: MIN_RELAY_FEE_ATOMS,
                priority_fee_atoms: PRIORITY_FEE_ATOMS,
            };
        }
        let mut fees: Vec<u64> = self.pending.values().map(|e| e.fee_atoms).collect();
        fees.sort_unstable();
        let total = fees.iter().sum();
        let median = fees[fees.len() / 2];
        FeeStats {
            count,
            total_fee_atoms: total,
            min_fee_atoms: *fees.first().unwrap_or(&0),
            max_fee_atoms: *fees.last().unwrap_or(&0),
            median_fee_atoms: median,
            min_relay_fee_atoms: MIN_RELAY_FEE_ATOMS,
            priority_fee_atoms: PRIORITY_FEE_ATOMS,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct FeeStats {
    pub count: u64,
    pub total_fee_atoms: u64,
    pub min_fee_atoms: u64,
    pub max_fee_atoms: u64,
    pub median_fee_atoms: u64,
    pub min_relay_fee_atoms: u64,
    pub priority_fee_atoms: u64,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

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

    fn funded_utxos(keypair: &WalletKeypair, atoms: u64) -> (UtxoSet, OutPoint) {
        use crate::script::standard::p2pkh_from_address;
        let outpoint = OutPoint {
            txid: Hash256([1u8; 32]),
            output_index: 0,
        };
        let mut utxos = UtxoSet::new();
        utxos.entries.insert(
            outpoint,
            UtxoEntry {
                output: TxOutput {
                    value_atoms: atoms,
                    script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
                },
                created_height: 0,
                coinbase: false,
            },
        );
        (utxos, outpoint)
    }

    fn payment_tx(keypair: &WalletKeypair, outpoint: OutPoint, out_atoms: u64) -> Transaction {
        let mut tx = Transaction::payment(
            vec![TxInput {
                previous_output: outpoint,
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: out_atoms,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut tx).unwrap();
        tx
    }

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
    fn rejects_below_min_fee() {
        let keypair = WalletKeypair::generate();
        // fund with 1_000_000 atoms, output 999_999 → fee = 1 atom (below min)
        let (utxos, op) = funded_utxos(&keypair, 1_000_000);
        let tx = payment_tx(&keypair, op, 999_999);
        let mut mp = Mempool::new();
        assert_eq!(
            mp.add(&utxos, &TEST_PARAMS, tx, 0),
            Err(MempoolError::FeeTooLow { fee: 1, min: MIN_RELAY_FEE_ATOMS })
        );
    }

    #[test]
    fn accepts_min_fee() {
        let keypair = WalletKeypair::generate();
        let total = 1_000_000u64;
        let (utxos, op) = funded_utxos(&keypair, total);
        // fee = MIN_RELAY_FEE_ATOMS exactly
        let tx = payment_tx(&keypair, op, total - MIN_RELAY_FEE_ATOMS);
        let mut mp = Mempool::new();
        assert!(mp.add(&utxos, &TEST_PARAMS, tx, 0).is_ok());
        assert_eq!(mp.len(), 1);
    }

    #[test]
    fn rejects_duplicate() {
        let keypair = WalletKeypair::generate();
        let (utxos, op) = funded_utxos(&keypair, 1_000_000);
        let tx = payment_tx(&keypair, op, 1_000_000 - MIN_RELAY_FEE_ATOMS);
        let mut mp = Mempool::new();
        mp.add(&utxos, &TEST_PARAMS, tx.clone(), 0).unwrap();
        assert_eq!(
            mp.add(&utxos, &TEST_PARAMS, tx, 0),
            Err(MempoolError::AlreadyKnown)
        );
    }

    #[test]
    fn select_for_block_ordered_by_fee() {
        let kp1 = WalletKeypair::generate();
        let kp2 = WalletKeypair::generate();
        let op1 = OutPoint { txid: Hash256([1u8; 32]), output_index: 0 };
        let op2 = OutPoint { txid: Hash256([2u8; 32]), output_index: 0 };
        let mut utxos = UtxoSet::new();
        for (op, kp, atoms) in [
            (op1, &kp1, 1_000_000u64),
            (op2, &kp2, 1_000_000u64),
        ] {
            utxos.entries.insert(op, UtxoEntry {
                output: TxOutput {
                    value_atoms: atoms,
                    script_pubkey: p2pkh_from_address(kp.address.as_str()).unwrap(),
                },
                created_height: 0,
                coinbase: false,
            });
        }

        // tx1: fee = PRIORITY_FEE_ATOMS (high)
        let tx1 = payment_tx(&kp1, op1, 1_000_000 - PRIORITY_FEE_ATOMS);
        // tx2: fee = MIN_RELAY_FEE_ATOMS (low)
        let tx2 = payment_tx(&kp2, op2, 1_000_000 - MIN_RELAY_FEE_ATOMS);

        let mut mp = Mempool::new();
        // Add low-fee first, then high-fee
        mp.add(&utxos, &TEST_PARAMS, tx2, 0).unwrap();
        mp.add(&utxos, &TEST_PARAMS, tx1.clone(), 0).unwrap();

        let (selected, total_fees) = mp.select_for_block();
        // High-fee tx1 must come first
        assert_eq!(selected[0].id, tx1.id);
        assert_eq!(total_fees, PRIORITY_FEE_ATOMS + MIN_RELAY_FEE_ATOMS);
    }

    #[test]
    fn remove_confirmed_clears_txs() {
        let mut mp = Mempool::new();
        let tx = Transaction::coinbase(99, 0, "placeholder");
        mp.pending.insert(tx.id.to_hex(), MempoolEntry { tx: tx.clone(), fee_atoms: 0 });
        assert_eq!(mp.len(), 1);

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

        let confirming_block = Block::new(fake_block.header.clone(), vec![tx]);
        mp.remove_confirmed(&confirming_block);
        assert_eq!(mp.len(), 0);
    }

    #[test]
    fn rejects_pending_double_spend() {
        let keypair = WalletKeypair::generate();
        let (utxos, op) = funded_utxos(&keypair, 2_000_000);

        let first  = payment_tx(&keypair, op, 2_000_000 - MIN_RELAY_FEE_ATOMS);
        let second = payment_tx(&keypair, op, 2_000_000 - MIN_RELAY_FEE_ATOMS - 1);

        let mut mp = Mempool::new();
        mp.add(&utxos, &TEST_PARAMS, first, 200).unwrap();
        assert_eq!(
            mp.add(&utxos, &TEST_PARAMS, second, 200),
            Err(MempoolError::PendingConflict)
        );
        assert_eq!(mp.len(), 1);
    }
}
