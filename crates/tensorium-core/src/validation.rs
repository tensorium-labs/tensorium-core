use thiserror::Error;

use crate::{
    block::{merkle_root, Block},
    chain::ConsensusParams,
    emission::reward_at_height,
    pow::header_meets_work,
};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ValidationError {
    #[error("block chain id does not match consensus params")]
    WrongChainId,
    #[error("block height is not the expected next height")]
    WrongHeight,
    #[error("previous hash does not match parent")]
    WrongPreviousHash,
    #[error("block timestamp is too far in the future")]
    FutureTimestamp,
    #[error("block merkle root is invalid")]
    InvalidMerkleRoot,
    #[error("block proof-of-work is invalid")]
    InvalidProofOfWork,
    #[error("coinbase transaction is missing")]
    MissingCoinbase,
    #[error("coinbase reward exceeds consensus emission schedule")]
    CoinbaseRewardTooHigh,
    #[error("first transaction must be coinbase")]
    FirstTransactionNotCoinbase,
}

pub fn validate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    block: &Block,
    now_seconds: u64,
) -> Result<(), ValidationError> {
    if block.header.chain_id != params.chain_id {
        return Err(ValidationError::WrongChainId);
    }

    let expected_height = parent.map_or(0, |parent| parent.header.height + 1);
    if block.header.height != expected_height {
        return Err(ValidationError::WrongHeight);
    }

    if let Some(parent) = parent {
        if block.header.previous_hash != parent.hash() {
            return Err(ValidationError::WrongPreviousHash);
        }
    }

    if block.header.timestamp_seconds > now_seconds + params.max_future_block_time_seconds {
        return Err(ValidationError::FutureTimestamp);
    }

    if block.header.merkle_root != merkle_root(&block.transactions) {
        return Err(ValidationError::InvalidMerkleRoot);
    }

    if !header_meets_work(&block.header) {
        return Err(ValidationError::InvalidProofOfWork);
    }

    validate_coinbase(params, block)?;

    Ok(())
}

fn validate_coinbase(params: &ConsensusParams, block: &Block) -> Result<(), ValidationError> {
    let coinbase = block
        .transactions
        .first()
        .ok_or(ValidationError::MissingCoinbase)?;
    if !coinbase.is_coinbase() {
        return Err(ValidationError::FirstTransactionNotCoinbase);
    }

    // Genesis block (height 0) may carry the founder allocation on top of the
    // normal mining reward. All other heights are bounded by reward only.
    let reward_limit = if block.header.height == 0 && !params.founder_address.is_empty() {
        reward_at_height(params, 0).saturating_add(params.founder_allocation_atoms)
    } else {
        reward_at_height(params, block.header.height)
    };
    let reward = coinbase.total_output_atoms();

    if reward > reward_limit {
        return Err(ValidationError::CoinbaseRewardTooHigh);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{block::Transaction, chain::TEST_PARAMS, pow::mine_header, state::ChainState};

    use super::*;

    #[test]
    fn accepts_mined_genesis_block() {
        let mut state = ChainState::new();
        let block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, block, 1_700_000_000),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_merkle_root() {
        let mut state = ChainState::new();
        let mut block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap()
            .clone();
        block.transactions.clear();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000),
            Err(ValidationError::InvalidMerkleRoot)
        );
    }

    #[test]
    fn rejects_wrong_chain_id() {
        let mut state = ChainState::new();
        let mut block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap()
            .clone();
        block.header.chain_id = "tensorium-wrong-chain".to_owned();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000),
            Err(ValidationError::WrongChainId)
        );
    }

    #[test]
    fn rejects_future_timestamp() {
        let mut state = ChainState::new();
        let block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        let now_before_allowed_window =
            1_700_000_000 - TEST_PARAMS.max_future_block_time_seconds - 1;

        assert_eq!(
            validate_block(&TEST_PARAMS, None, block, now_before_allowed_window),
            Err(ValidationError::FutureTimestamp)
        );
    }

    #[test]
    fn rejects_coinbase_reward_above_schedule() {
        let mut state = ChainState::new();
        let mut block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap()
            .clone();
        block.transactions[0] = Transaction::coinbase(
            block.header.height,
            reward_at_height(&TEST_PARAMS, block.header.height) + 1,
            "miner",
        );
        block.header.merkle_root = merkle_root(&block.transactions);
        block.header.nonce = 0;
        block.header = mine_header(block.header.clone(), 1_000_000).unwrap();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000),
            Err(ValidationError::CoinbaseRewardTooHigh)
        );
    }
}
