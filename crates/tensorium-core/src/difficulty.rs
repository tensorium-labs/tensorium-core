use crate::chain::ConsensusParams;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DifficultySample {
    pub first_timestamp_seconds: u64,
    pub last_timestamp_seconds: u64,
    pub current_leading_zero_bits: u8,
}

pub fn next_leading_zero_bits(params: &ConsensusParams, sample: DifficultySample) -> u8 {
    let expected = params
        .target_block_seconds
        .saturating_mul(params.difficulty_adjustment_window);
    let actual = sample
        .last_timestamp_seconds
        .saturating_sub(sample.first_timestamp_seconds)
        .max(1);

    let next = if actual < expected / 2 {
        sample.current_leading_zero_bits.saturating_add(1)
    } else if actual > expected * 2 {
        sample.current_leading_zero_bits.saturating_sub(1)
    } else {
        sample.current_leading_zero_bits
    };

    next.clamp(params.min_leading_zero_bits, params.max_leading_zero_bits)
}

/// Returns the consensus-required `leading_zero_bits` for the block at `height`.
///
/// Below `params.difficulty_retarget_activation_height`, every block must use
/// the network's fixed `initial_leading_zero_bits` — this is the legacy rule
/// and keeps every block mined before the fork valid. At or above the
/// activation height, the most recently completed adjustment window's sample
/// (when one exists) is run through `next_leading_zero_bits` to compute the
/// retargeted difficulty; `sample = None` (e.g. the first window right after
/// activation, with no completed prior window to measure) falls back to the
/// fixed difficulty.
///
/// This function is pure — callers with chain-history access (`ChainState`)
/// are responsible for building `sample` from the relevant historical blocks.
pub fn expected_leading_zero_bits(
    params: &ConsensusParams,
    height: u64,
    sample: Option<DifficultySample>,
) -> u8 {
    if height < params.difficulty_retarget_activation_height {
        return params.initial_leading_zero_bits;
    }
    match sample {
        Some(sample) => next_leading_zero_bits(params, sample),
        None => params.initial_leading_zero_bits,
    }
}

#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET_CANDIDATE, TESTNET};

    use super::*;

    #[test]
    fn difficulty_adjustment_is_clamped() {
        let sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: TESTNET.max_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&TESTNET, sample),
            TESTNET.max_leading_zero_bits
        );
    }

    #[test]
    fn difficulty_moves_up_when_blocks_are_too_fast() {
        let sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 10,
            current_leading_zero_bits: TESTNET.initial_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&TESTNET, sample),
            TESTNET.initial_leading_zero_bits + 1
        );
    }

    #[test]
    fn difficulty_moves_down_when_blocks_are_too_slow() {
        let expected = TESTNET
            .target_block_seconds
            .saturating_mul(TESTNET.difficulty_adjustment_window);
        let sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: expected * 3,
            current_leading_zero_bits: TESTNET.initial_leading_zero_bits,
        };

        assert_eq!(
            next_leading_zero_bits(&TESTNET, sample),
            TESTNET.initial_leading_zero_bits - 1
        );
    }

    #[test]
    fn difficulty_stays_flat_inside_target_band() {
        let expected = TESTNET
            .target_block_seconds
            .saturating_mul(TESTNET.difficulty_adjustment_window);
        let sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: expected,
            current_leading_zero_bits: TESTNET.initial_leading_zero_bits,
        };

        assert_eq!(
            next_leading_zero_bits(&TESTNET, sample),
            TESTNET.initial_leading_zero_bits
        );
    }

    #[test]
    fn mainnet_candidate_difficulty_bounds_are_clamped() {
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: MAINNET_CANDIDATE.max_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET_CANDIDATE, fast_sample),
            MAINNET_CANDIDATE.max_leading_zero_bits
        );

        let slow_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: u64::MAX,
            current_leading_zero_bits: MAINNET_CANDIDATE.min_leading_zero_bits,
        };
        assert_eq!(
            next_leading_zero_bits(&MAINNET_CANDIDATE, slow_sample),
            MAINNET_CANDIDATE.min_leading_zero_bits
        );
    }

    fn params_with_activation(activation_height: u64) -> ConsensusParams {
        ConsensusParams {
            difficulty_retarget_activation_height: activation_height,
            ..TESTNET
        }
    }

    #[test]
    fn expected_difficulty_is_fixed_below_activation_height() {
        let params = params_with_activation(1_000);
        // Heights below activation always use the legacy fixed difficulty,
        // regardless of any sample a (buggy) caller might pass in.
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: TESTNET.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&params, 0, None),
            TESTNET.initial_leading_zero_bits
        );
        assert_eq!(
            expected_leading_zero_bits(&params, 999, Some(fast_sample)),
            TESTNET.initial_leading_zero_bits
        );
    }

    #[test]
    fn expected_difficulty_is_fixed_when_disabled() {
        // u64::MAX activation height == retargeting disabled network-wide.
        let params = TESTNET; // TESTNET ships with activation height = u64::MAX
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: TESTNET.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&params, 1_000_000, Some(fast_sample)),
            TESTNET.initial_leading_zero_bits
        );
    }

    #[test]
    fn expected_difficulty_falls_back_to_fixed_without_a_sample() {
        let params = params_with_activation(1_000);
        // At/after activation but no completed window to measure yet.
        assert_eq!(
            expected_leading_zero_bits(&params, 1_000, None),
            params.initial_leading_zero_bits
        );
    }

    #[test]
    fn expected_difficulty_retargets_at_and_after_activation_height() {
        let params = params_with_activation(1_000);
        let fast_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: 1,
            current_leading_zero_bits: params.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&params, 1_000, Some(fast_sample)),
            params.initial_leading_zero_bits + 1
        );

        let slow_sample = DifficultySample {
            first_timestamp_seconds: 0,
            last_timestamp_seconds: params.target_block_seconds * params.difficulty_adjustment_window * 3,
            current_leading_zero_bits: params.initial_leading_zero_bits,
        };
        assert_eq!(
            expected_leading_zero_bits(&params, 1_500, Some(slow_sample)),
            params.initial_leading_zero_bits - 1
        );
    }
}
