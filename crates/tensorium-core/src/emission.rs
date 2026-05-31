use crate::chain::ConsensusParams;

pub fn reward_at_height(params: &ConsensusParams, height: u64) -> u64 {
    let era = emission_era(params, height);
    if era >= params.max_halving_eras {
        return 0;
    }

    params.initial_reward_atoms >> era
}

pub fn emission_era(params: &ConsensusParams, height: u64) -> u32 {
    (height / params.halving_interval_blocks) as u32
}

pub fn emitted_supply_until_height(params: &ConsensusParams, exclusive_height: u64) -> u64 {
    let mut emitted = 0u64;
    let mut remaining = exclusive_height;

    for era in 0..params.max_halving_eras {
        if remaining == 0 {
            break;
        }

        let blocks = remaining.min(params.halving_interval_blocks);
        let reward = params.initial_reward_atoms >> era;
        emitted = emitted.saturating_add(blocks.saturating_mul(reward));
        remaining -= blocks;
    }

    emitted.min(params.mining_allocation_atoms)
}

#[cfg(test)]
mod tests {
    use crate::chain::{MAINNET_CANDIDATE, TESTNET};

    use super::*;

    #[test]
    fn reward_halves_each_era_then_stops() {
        assert_eq!(reward_at_height(&TESTNET, 0), TESTNET.initial_reward_atoms);
        assert_eq!(
            reward_at_height(&TESTNET, TESTNET.halving_interval_blocks),
            TESTNET.initial_reward_atoms / 2
        );
        assert_eq!(
            reward_at_height(&TESTNET, TESTNET.halving_interval_blocks * 10),
            0
        );
    }

    #[test]
    fn emission_never_exceeds_cap() {
        let supply = emitted_supply_until_height(&TESTNET, TESTNET.halving_interval_blocks * 20);
        assert!(supply <= TESTNET.mining_allocation_atoms);
    }

    #[test]
    fn ten_era_emission_matches_mining_allocation_with_rounding_dust() {
        let supply = emitted_supply_until_height(&TESTNET, TESTNET.halving_interval_blocks * 10);
        let dust = TESTNET.mining_allocation_atoms - supply;

        assert_eq!(supply, 3_199_999_995_331_200);
        assert_eq!(dust, 4_668_800);
        assert!(dust < TESTNET.initial_reward_atoms);
    }

    #[test]
    fn mainnet_candidate_emission_matches_testnet_schedule() {
        for era in 0..MAINNET_CANDIDATE.max_halving_eras {
            let height = MAINNET_CANDIDATE.halving_interval_blocks * u64::from(era);
            assert_eq!(
                reward_at_height(&MAINNET_CANDIDATE, height),
                reward_at_height(&TESTNET, height)
            );
        }

        assert_eq!(
            emitted_supply_until_height(
                &MAINNET_CANDIDATE,
                MAINNET_CANDIDATE.halving_interval_blocks * 10,
            ),
            emitted_supply_until_height(&TESTNET, TESTNET.halving_interval_blocks * 10)
        );
    }

    #[test]
    fn reward_is_zero_after_final_halving_era() {
        for params in [TESTNET, MAINNET_CANDIDATE] {
            let first_zero_height =
                params.halving_interval_blocks * u64::from(params.max_halving_eras);
            assert_eq!(reward_at_height(&params, first_zero_height), 0);
            assert_eq!(reward_at_height(&params, first_zero_height + 1), 0);
        }
    }
}
