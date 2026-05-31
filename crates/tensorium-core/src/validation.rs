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

    let reward_limit = reward_at_height(params, block.header.height);
    let payload = String::from_utf8_lossy(&coinbase.payload);
    let reward = payload
        .split(':')
        .nth(2)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(u64::MAX);

    if reward > reward_limit {
        return Err(ValidationError::CoinbaseRewardTooHigh);
    }

    Ok(())
}
