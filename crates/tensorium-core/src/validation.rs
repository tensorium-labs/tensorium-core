use thiserror::Error;

use crate::{
    block::{merkle_root, Block},
    chain::ConsensusParams,
    hash::Hash256,
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
    #[error("block declares an unexpected difficulty (consensus retarget mismatch)")]
    UnexpectedDifficulty,
    #[error("coinbase transaction is missing")]
    MissingCoinbase,
    #[error("first transaction must be coinbase")]
    FirstTransactionNotCoinbase,
}

pub fn validate_block(
    params: &ConsensusParams,
    parent: Option<&Block>,
    block: &Block,
    now_seconds: u64,
    expected_leading_zero_bits: u8,
    epoch_seed: Hash256,
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

    // Consensus-enforced difficulty: the block must declare exactly the
    // `leading_zero_bits` the retargeting rule requires for this height — a
    // miner cannot just pick an easier target and still pass `header_meets_work`.
    if block.header.leading_zero_bits != expected_leading_zero_bits {
        return Err(ValidationError::UnexpectedDifficulty);
    }

    if !header_meets_work(&block.header, epoch_seed) {
        return Err(ValidationError::InvalidProofOfWork);
    }

    validate_coinbase(params, block)?;

    Ok(())
}

fn validate_coinbase(_params: &ConsensusParams, block: &Block) -> Result<(), ValidationError> {
    let coinbase = block
        .transactions
        .first()
        .ok_or(ValidationError::MissingCoinbase)?;
    if !coinbase.is_coinbase() {
        return Err(ValidationError::FirstTransactionNotCoinbase);
    }
    // Amount validation (subsidy + fees) is done in utxo::apply_block which has
    // access to the full UTXO set needed to compute transaction fees.
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        block::Transaction,
        chain::TEST_PARAMS,
        emission::reward_at_height,
        hash::Hash256,
        pow::mine_header,
        state::ChainState,
    };

    use super::*;

    #[test]
    fn accepts_mined_genesis_block() {
        let mut state = ChainState::new();
        let block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
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
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
            Err(ValidationError::InvalidMerkleRoot)
        );
    }

    #[test]
    fn rejects_block_declaring_a_different_difficulty_than_expected() {
        let mut state = ChainState::new();
        let block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

        // The block is internally consistent (PoW matches its own claimed
        // bits) but does not match what consensus requires for this height —
        // a miner picking an easier-than-required target must be rejected.
        assert_eq!(
            validate_block(
                &TEST_PARAMS,
                None,
                block,
                1_700_000_000,
                TEST_PARAMS.initial_leading_zero_bits + 1,
                Hash256::ZERO,
            ),
            Err(ValidationError::UnexpectedDifficulty)
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
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
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
            validate_block(&TEST_PARAMS, None, block, now_before_allowed_window, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
            Err(ValidationError::FutureTimestamp)
        );
    }

    #[test]
    fn accepts_coinbase_with_fees_above_base_reward() {
        // validate_block does NOT reject a coinbase that includes tx fees on top
        // of the base subsidy — fee accounting is enforced by utxo::apply_block.
        let mut state = ChainState::new();
        let mut block = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap()
            .clone();
        block.transactions[0] = Transaction::coinbase(
            block.header.height,
            reward_at_height(&TEST_PARAMS, block.header.height) + 1_000,
            "miner",
        );
        block.header.merkle_root = merkle_root(&block.transactions);
        block.header.nonce = 0;
        block.header = mine_header(block.header.clone(), Hash256::ZERO, 1_000_000).unwrap();

        assert_eq!(
            validate_block(&TEST_PARAMS, None, &block, 1_700_000_000, TEST_PARAMS.initial_leading_zero_bits, Hash256::ZERO),
            Ok(())
        );
    }
}
