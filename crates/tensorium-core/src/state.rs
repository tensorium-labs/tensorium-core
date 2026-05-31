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

    pub fn candidate_block(
        &self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        miner: &str,
    ) -> Result<Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?;
        Ok(candidate_block(params, Some(parent), timestamp_seconds, miner, vec![]))
    }

    /// Like `candidate_block` but includes `extra_txs` (e.g. from the mempool)
    /// after the coinbase.
    pub fn candidate_block_with_mempool(
        &self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        miner: &str,
        extra_txs: Vec<Transaction>,
    ) -> Result<Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?;
        Ok(candidate_block(params, Some(parent), timestamp_seconds, miner, extra_txs))
    }

    pub fn submit_block(
        &mut self,
        params: &ConsensusParams,
        block: Block,
        now_seconds: u64,
    ) -> Result<&Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?.clone();
        validate_block(params, Some(&parent), &block, now_seconds)?;
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
    let block = candidate_block(params, parent, timestamp_seconds, miner, vec![]);
    let header = block.header;
    let mined_header = mine_header(header, max_nonce).ok_or(StateError::MiningFailed)?;
    Ok(Block::new(mined_header, block.transactions))
}

fn candidate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    timestamp_seconds: u64,
    miner: &str,
    extra_txs: Vec<Transaction>,
) -> Block {
    let height = parent.map_or(0, |block| block.header.height + 1);
    let previous_hash = parent.map_or(Hash256::ZERO, Block::hash);
    let reward = reward_at_height(params, height);
    let coinbase_tx = Transaction::coinbase(height, reward, miner);
    let mut transactions = Vec::with_capacity(1 + extra_txs.len());
    transactions.push(coinbase_tx);
    transactions.extend(extra_txs);
    let header = BlockHeader {
        version: 1,
        chain_id: params.chain_id.to_owned(),
        height,
        previous_hash,
        merkle_root: merkle_root(&transactions),
        timestamp_seconds,
        leading_zero_bits: params.initial_leading_zero_bits,
        nonce: 0,
    };

    Block::new(header, transactions)
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

    #[test]
    fn builds_candidate_then_accepts_mined_submit() {
        let mut state = ChainState::new();
        state.init_genesis(&TESTNET, 1_700_000_000, 1_000_000).unwrap();

        let candidate = state
            .candidate_block(&TESTNET, 1_700_000_060, "template-miner")
            .unwrap();
        let mined_header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
        let mined_block = Block::new(mined_header, candidate.transactions);

        state
            .submit_block(&TESTNET, mined_block, 1_700_000_060)
            .unwrap();
        assert_eq!(state.height(), Some(1));
    }
}
