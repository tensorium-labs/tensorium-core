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
}
