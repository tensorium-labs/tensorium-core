use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{Block, OutPoint, TxOutput},
    chain::ConsensusParams,
    emission::reward_at_height,
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
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn total_atoms(&self) -> u64 {
        self.entries
            .values()
            .fold(0u64, |sum, entry| sum.saturating_add(entry.output.value_atoms))
    }

    pub fn apply_block(
        &mut self,
        params: &ConsensusParams,
        block: &Block,
    ) -> Result<(), UtxoError> {
        let coinbase = block.transactions.first().ok_or(UtxoError::MissingCoinbase)?;
        if coinbase.total_output_atoms() > reward_at_height(params, block.header.height) {
            return Err(UtxoError::CoinbaseOutputTooHigh);
        }

        for tx in block.transactions.iter().skip(1) {
            for input in &tx.inputs {
                if self.entries.remove(&input.previous_output).is_none() {
                    return Err(UtxoError::MissingInput);
                }
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
    use crate::{chain::TESTNET, state::ChainState};

    use super::*;

    #[test]
    fn tracks_coinbase_outputs() {
        let mut state = ChainState::new();
        let mut utxos = UtxoSet::new();

        let genesis = state.init_genesis(&TESTNET, 1_700_000_000, 1_000_000).unwrap();
        utxos.apply_block(&TESTNET, genesis).unwrap();
        assert_eq!(utxos.total_atoms(), reward_at_height(&TESTNET, 0));

        let next = state
            .mine_next_block(&TESTNET, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        utxos.apply_block(&TESTNET, next).unwrap();
        assert_eq!(
            utxos.total_atoms(),
            reward_at_height(&TESTNET, 0) + reward_at_height(&TESTNET, 1)
        );
    }
}
