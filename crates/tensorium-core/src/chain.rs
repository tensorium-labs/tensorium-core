use serde::{Deserialize, Serialize};

pub const COIN: u64 = 100_000_000;
pub const MAX_HALVING_ERAS: u32 = 10;
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;
pub const FOUNDER_ALLOCATION_COINS: u64 = 1_000_000;
pub const MINING_ALLOCATION_COINS: u64 = 32_000_000;
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN;
pub const FOUNDER_ALLOCATION_ATOMS: u64 = FOUNDER_ALLOCATION_COINS * COIN;
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChainNetwork {
    Testnet,
    MainnetCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConsensusParams {
    pub network: ChainNetwork,
    pub chain_id: &'static str,
    pub target_block_seconds: u64,
    pub halving_interval_blocks: u64,
    pub max_halving_eras: u32,
    pub total_supply_atoms: u64,
    pub founder_allocation_atoms: u64,
    pub mining_allocation_atoms: u64,
    pub initial_reward_atoms: u64,
    pub initial_leading_zero_bits: u8,
    pub min_leading_zero_bits: u8,
    pub max_leading_zero_bits: u8,
    pub difficulty_adjustment_window: u64,
    pub coinbase_maturity_blocks: u64,
    pub max_future_block_time_seconds: u64,
    pub max_block_bytes: u64,
}

impl ConsensusParams {
    pub const fn blocks_per_year(target_block_seconds: u64) -> u64 {
        365 * 24 * 60 * 60 / target_block_seconds
    }

    pub const fn testnet() -> Self {
        Self {
            network: ChainNetwork::Testnet,
            chain_id: "tensorium-testnet-0",
            target_block_seconds: 60,
            halving_interval_blocks: 1_051_200,
            max_halving_eras: MAX_HALVING_ERAS,
            total_supply_atoms: TOTAL_SUPPLY_ATOMS,
            founder_allocation_atoms: FOUNDER_ALLOCATION_ATOMS,
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS,
            initial_reward_atoms: 1_523_557_865,
            initial_leading_zero_bits: 36,
            min_leading_zero_bits: 28,
            max_leading_zero_bits: 48,
            difficulty_adjustment_window: 60,
            coinbase_maturity_blocks: 100,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }

    pub const fn mainnet_candidate() -> Self {
        Self {
            network: ChainNetwork::MainnetCandidate,
            chain_id: "tensorium-mainnet-candidate-0",
            target_block_seconds: 60,
            halving_interval_blocks: 1_051_200,
            max_halving_eras: MAX_HALVING_ERAS,
            total_supply_atoms: TOTAL_SUPPLY_ATOMS,
            founder_allocation_atoms: FOUNDER_ALLOCATION_ATOMS,
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS,
            initial_reward_atoms: 1_523_557_865,
            initial_leading_zero_bits: 40,
            min_leading_zero_bits: 32,
            max_leading_zero_bits: 56,
            difficulty_adjustment_window: 120,
            coinbase_maturity_blocks: 100,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }
}

// ── CONSENSUS FREEZE ──────────────────────────────────────────────────────
// TESTNET parameters are FROZEN as of Phase 6 (2025-05-31).
// Do NOT change TESTNET fields without a chain reset and version bump.
// Difficulty 36 bits — requires GPU mining (RTX 3060+)
// Genesis nonce: see main.rs GENESIS_NONCE_36 (pre-mined via CUDA on RTX 3060)
// These parameters are locked for the GPU-first testnet phase.
pub const TESTNET: ConsensusParams = ConsensusParams::testnet();

// MAINNET_CANDIDATE parameters are FROZEN as of Phase 7E (2026-05-31).
// chain_id: tensorium-mainnet-candidate-0
// Initial difficulty: 40 bits (requires RTX 3060+ or equivalent GPU).
// Genesis timestamp: 1_780_272_000 (2026-06-01 00:00:00 UTC) — TBD until launch.
// Genesis nonce: TBD — must be GPU-mined before mainnet-candidate chain launch.
// Do NOT change these fields without bumping chain_id and issuing a breaking notice.
pub const MAINNET_CANDIDATE: ConsensusParams = ConsensusParams::mainnet_candidate();

/// Low-difficulty params for unit tests — mines instantly (difficulty 8 = 256 hashes avg).
pub const TEST_PARAMS: ConsensusParams = ConsensusParams {
    network: ChainNetwork::Testnet,
    chain_id: "tensorium-testnet-0",
    initial_leading_zero_bits: 8,
    min_leading_zero_bits: 4,
    max_leading_zero_bits: 16,
    ..ConsensusParams::testnet()
};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_supply_split(params: ConsensusParams) {
        assert_eq!(params.total_supply_atoms, 33_000_000 * COIN);
        assert_eq!(params.founder_allocation_atoms, 1_000_000 * COIN);
        assert_eq!(params.mining_allocation_atoms, 32_000_000 * COIN);
        assert_eq!(
            params.founder_allocation_atoms + params.mining_allocation_atoms,
            params.total_supply_atoms
        );
    }

    #[test]
    fn halving_interval_is_two_years_for_one_minute_blocks() {
        assert_eq!(
            TESTNET.halving_interval_blocks,
            2 * ConsensusParams::blocks_per_year(60)
        );
    }

    #[test]
    fn testnet_tokenomics_match_phase_7_readiness_plan() {
        assert_eq!(TESTNET.chain_id, "tensorium-testnet-0");
        assert_eq!(TESTNET.target_block_seconds, 60);
        assert_eq!(TESTNET.max_halving_eras, 10);
        assert_eq!(TESTNET.initial_reward_atoms, 1_523_557_865);
        assert_eq!(TESTNET.coinbase_maturity_blocks, 100);
        assert_eq!(TESTNET.max_future_block_time_seconds, 2 * 60 * 60);
        assert_supply_split(TESTNET);
    }

    #[test]
    fn mainnet_candidate_tokenomics_match_testnet_supply_plan() {
        assert_eq!(MAINNET_CANDIDATE.chain_id, "tensorium-mainnet-candidate-0");
        assert_eq!(
            MAINNET_CANDIDATE.target_block_seconds,
            TESTNET.target_block_seconds
        );
        assert_eq!(
            MAINNET_CANDIDATE.halving_interval_blocks,
            TESTNET.halving_interval_blocks
        );
        assert_eq!(MAINNET_CANDIDATE.max_halving_eras, TESTNET.max_halving_eras);
        assert_eq!(
            MAINNET_CANDIDATE.initial_reward_atoms,
            TESTNET.initial_reward_atoms
        );
        assert_supply_split(MAINNET_CANDIDATE);
    }

    #[test]
    fn mainnet_is_gpu_first_harder_than_testnet() {
        assert!(MAINNET_CANDIDATE.initial_leading_zero_bits > TESTNET.initial_leading_zero_bits);
        assert!(MAINNET_CANDIDATE.min_leading_zero_bits > TESTNET.min_leading_zero_bits);
        assert!(MAINNET_CANDIDATE.max_leading_zero_bits > TESTNET.max_leading_zero_bits);
        assert!(
            MAINNET_CANDIDATE.min_leading_zero_bits <= MAINNET_CANDIDATE.initial_leading_zero_bits
        );
        assert!(
            MAINNET_CANDIDATE.initial_leading_zero_bits <= MAINNET_CANDIDATE.max_leading_zero_bits
        );
    }
}
