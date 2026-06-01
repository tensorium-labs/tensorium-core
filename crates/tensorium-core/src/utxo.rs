use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{Block, OutPoint, Transaction, TxOutput},
    chain::ConsensusParams,
    emission::reward_at_height,
    wallet::verify_transaction_input,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub output: TxOutput,
    pub created_height: u64,
    pub coinbase: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UtxoSet {
    pub entries: HashMap<OutPoint, UtxoEntry>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum UtxoError {
    #[error("transaction spends an output that does not exist")]
    MissingInput,
    #[error("transaction spends the same output more than once")]
    DuplicateInput,
    #[error("coinbase transaction is missing")]
    MissingCoinbase,
    #[error("coinbase output exceeds block reward")]
    CoinbaseOutputTooHigh,
    #[error("transaction signature is invalid")]
    InvalidSignature,
    #[error("coinbase output is not mature enough to spend")]
    ImmatureCoinbaseSpend,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn total_atoms(&self) -> u64 {
        self.entries.values().fold(0u64, |sum, entry| {
            sum.saturating_add(entry.output.value_atoms)
        })
    }

    /// Validate that `tx` can be spent given the current UTXO set.
    /// Does not mutate the set — safe to call for mempool acceptance checks.
    pub fn validate_transaction(
        &self,
        tx: &Transaction,
        tip_height: u64,
        params: &ConsensusParams,
    ) -> Result<(), UtxoError> {
        let mut seen: HashSet<OutPoint> = HashSet::new();
        for input in &tx.inputs {
            if !seen.insert(input.previous_output) {
                return Err(UtxoError::DuplicateInput);
            }
            let entry = self
                .entries
                .get(&input.previous_output)
                .ok_or(UtxoError::MissingInput)?;
            if entry.coinbase
                && tip_height
                    < entry
                        .created_height
                        .saturating_add(params.coinbase_maturity_blocks)
            {
                return Err(UtxoError::ImmatureCoinbaseSpend);
            }
            verify_transaction_input(tx, input, &entry.output.address)
                .map_err(|_| UtxoError::InvalidSignature)?;
        }
        Ok(())
    }

    pub fn apply_block(
        &mut self,
        params: &ConsensusParams,
        block: &Block,
    ) -> Result<(), UtxoError> {
        let coinbase = block
            .transactions
            .first()
            .ok_or(UtxoError::MissingCoinbase)?;
        // Genesis block (height 0) may include the founder allocation on top of
        // the normal mining reward. All other blocks are bounded by reward only.
        let max_coinbase_atoms = if block.header.height == 0 && !params.founder_address.is_empty() {
            reward_at_height(params, 0).saturating_add(params.founder_allocation_atoms)
        } else {
            reward_at_height(params, block.header.height)
        };
        if coinbase.total_output_atoms() > max_coinbase_atoms {
            return Err(UtxoError::CoinbaseOutputTooHigh);
        }

        for tx in block.transactions.iter().skip(1) {
            for input in &tx.inputs {
                let spent = self
                    .entries
                    .remove(&input.previous_output)
                    .ok_or(UtxoError::MissingInput)?;
                if spent.coinbase
                    && block.header.height
                        < spent
                            .created_height
                            .saturating_add(params.coinbase_maturity_blocks)
                {
                    return Err(UtxoError::ImmatureCoinbaseSpend);
                }
                verify_transaction_input(tx, input, &spent.output.address)
                    .map_err(|_| UtxoError::InvalidSignature)?;
            }
        }

        for tx in &block.transactions {
            for (index, output) in tx.outputs.iter().enumerate() {
                self.entries.insert(
                    OutPoint {
                        txid: tx.id,
                        output_index: index as u32,
                    },
                    UtxoEntry {
                        output: output.clone(),
                        created_height: block.header.height,
                        coinbase: tx.is_coinbase(),
                    },
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        block::{merkle_root, BlockHeader, Transaction, TxInput, TxOutput},
        chain::TEST_PARAMS,
        hash::Hash256,
        state::ChainState,
        wallet::WalletKeypair,
    };

    use super::*;

    #[test]
    fn tracks_coinbase_outputs() {
        let mut state = ChainState::new();
        let mut utxos = UtxoSet::new();

        let genesis = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        utxos.apply_block(&TEST_PARAMS, genesis).unwrap();
        assert_eq!(utxos.total_atoms(), reward_at_height(&TEST_PARAMS, 0));

        let next = state
            .mine_next_block(&TEST_PARAMS, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        utxos.apply_block(&TEST_PARAMS, next).unwrap();
        assert_eq!(
            utxos.total_atoms(),
            reward_at_height(&TEST_PARAMS, 0) + reward_at_height(&TEST_PARAMS, 1)
        );
    }

    #[test]
    fn rejects_immature_coinbase_spend() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();
        let coinbase = Transaction::coinbase(1, 100, keypair.address.as_str());
        let coinbase_block = test_block(1, vec![coinbase.clone()]);
        utxos.apply_block(&TEST_PARAMS, &coinbase_block).unwrap();

        let mut spend = Transaction::payment(
            vec![TxInput {
                previous_output: crate::block::OutPoint {
                    txid: coinbase.id,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 100,
                address: keypair.address.as_str().to_owned(),
            }],
        );
        keypair.sign_transaction(&mut spend).unwrap();
        let block = test_block(2, vec![Transaction::coinbase(2, 100, "miner"), spend]);

        assert_eq!(
            utxos.apply_block(&TEST_PARAMS, &block),
            Err(UtxoError::ImmatureCoinbaseSpend)
        );
    }

    fn test_block(height: u64, transactions: Vec<Transaction>) -> Block {
        let merkle_root = merkle_root(&transactions);
        Block::new(
            BlockHeader {
                version: 1,
                chain_id: TEST_PARAMS.chain_id.to_owned(),
                height,
                previous_hash: Hash256::ZERO,
                merkle_root,
                timestamp_seconds: 1_700_000_000 + height,
                leading_zero_bits: TEST_PARAMS.initial_leading_zero_bits,
                nonce: 0,
            },
            transactions,
        )
    }
}
