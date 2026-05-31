use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{merkle_root, Block, BlockHeader, Transaction},
    chain::ConsensusParams,
    emission::reward_at_height,
    hash::Hash256,
    pow::mine_header,
    validation::{validate_block, ValidationError},
};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChainState {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum StateError {
    #[error("chain state already has a genesis block")]
    GenesisAlreadyExists,
    #[error("chain state has no genesis block")]
    MissingGenesis,
    #[error("mining failed before nonce limit")]
    MiningFailed,
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

impl ChainState {
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    pub fn height(&self) -> Option<u64> {
        self.tip().map(|block| block.header.height)
    }

    pub fn tip(&self) -> Option<&Block> {
        self.blocks.last()
    }

    pub fn init_genesis(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        max_nonce: u64,
    ) -> Result<&Block, StateError> {
        if !self.blocks.is_empty() {
            return Err(StateError::GenesisAlreadyExists);
        }

        let block = mine_candidate_block(params, None, timestamp_seconds, "genesis", max_nonce)?;
        validate_block(params, None, &block, timestamp_seconds)?;
        self.blocks.push(block);

        Ok(self.tip().expect("genesis was just pushed"))
    }

    pub fn mine_next_block(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        miner: &str,
        max_nonce: u64,
    ) -> Result<&Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?.clone();
        let block = mine_candidate_block(
            params,
            Some(&parent),
            timestamp_seconds,
            miner,
            max_nonce,
        )?;
        validate_block(params, Some(&parent), &block, timestamp_seconds)?;
        self.blocks.push(block);

        Ok(self.tip().expect("block was just pushed"))
    }
}

fn mine_candidate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    timestamp_seconds: u64,
    miner: &str,
    max_nonce: u64,
) -> Result<Block, StateError> {
    let height = parent.map_or(0, |block| block.header.height + 1);
    let previous_hash = parent.map_or(Hash256::ZERO, Block::hash);
    let reward = reward_at_height(params, height);
    let tx = Transaction::coinbase(height, reward, miner);
    let header = BlockHeader {
        version: 1,
        chain_id: params.chain_id.to_owned(),
        height,
        previous_hash,
        merkle_root: merkle_root(core::slice::from_ref(&tx)),
        timestamp_seconds,
        leading_zero_bits: params.initial_leading_zero_bits,
        nonce: 0,
    };

    let mined_header = mine_header(header, max_nonce).ok_or(StateError::MiningFailed)?;
    Ok(Block::new(mined_header, vec![tx]))
}

#[cfg(test)]
mod tests {
    use crate::chain::TESTNET;

    use super::*;

    #[test]
    fn initializes_genesis_then_mines_next_block() {
        let mut state = ChainState::new();
        state.init_genesis(&TESTNET, 1_700_000_000, 1_000_000).unwrap();
        assert_eq!(state.height(), Some(0));

        state
            .mine_next_block(&TESTNET, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        assert_eq!(state.height(), Some(1));
        assert_eq!(state.blocks[1].header.previous_hash, state.blocks[0].hash());
    }
}
