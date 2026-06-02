use serde::{Deserialize, Serialize};

pub const COIN: u64 = 100_000_000;
pub const MAX_HALVING_ERAS: u32 = 10;
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;
/// Total genesis pre-mint (all allocation buckets combined).
pub const GENESIS_PRE_MINT_COINS: u64 = 8_000_000;
pub const MINING_ALLOCATION_COINS: u64 = 25_000_000;
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN;
/// Kept for backward-compat; equals GENESIS_PRE_MINT_COINS * COIN.
pub const FOUNDER_ALLOCATION_ATOMS: u64 = GENESIS_PRE_MINT_COINS * COIN;
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;

/// Genesis allocation buckets — (address, atoms). All minted at block 0.
/// Founder 1M | Liquidity pool 3M | Bridge reserve 2M | Ecosystem 2M = 8M total.
pub const MC_GENESIS_ALLOCATIONS: &[(&str, u64)] = &[
    ("txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d", 1_000_000 * COIN), // founder
    ("txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p", 3_000_000 * COIN), // liquidity pool
    ("txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq", 2_000_000 * COIN), // bridge reserve
    ("txm1jwz2nvfajy84kyypzxp0pq8n5vrwahu6yny9hf", 2_000_000 * COIN), // ecosystem/treasury
];

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
    /// Total genesis pre-mint atoms (sum of all genesis_allocations).
    pub founder_allocation_atoms: u64,
    pub mining_allocation_atoms: u64,
    /// Genesis allocation buckets: (address, atoms). Applied at block 0 only.
    /// Empty slice means no genesis pre-mint for this network.
    #[serde(skip)]
    pub genesis_allocations: &'static [(&'static str, u64)],
    /// Kept for backward compat; ignored when genesis_allocations is non-empty.
    pub founder_address: &'static str,
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
            total_supply_atoms: 33_000_000 * COIN,
            founder_allocation_atoms: 1_000_000 * COIN, // testnet keeps 1M founder
            mining_allocation_atoms: 32_000_000 * COIN, // testnet keeps 32M mining
            genesis_allocations: &[],
            founder_address: "",
            initial_reward_atoms: 1_523_557_865,
            // Legacy low-difficulty network retained for CPU development and migration drills.
            // Mainnet candidate remains GPU-first at 40 bits.
            initial_leading_zero_bits: 20,
            min_leading_zero_bits: 8,
            max_leading_zero_bits: 36,
            difficulty_adjustment_window: 60,
            // Short maturity retained for the low-difficulty development network.
            coinbase_maturity_blocks: 10,
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
            founder_allocation_atoms: FOUNDER_ALLOCATION_ATOMS, // = 8M total pre-mint
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS,   // = 25M
            genesis_allocations: MC_GENESIS_ALLOCATIONS,
            founder_address: "",
            initial_reward_atoms: 1_190_279_581,                // 11.9027... TXM/block for 25M over 10 eras
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
// Low-difficulty development-network parameters retained for tests and migration drills.
// MC diff (40 bits) remains higher and is unaffected by this configuration.
pub const TESTNET: ConsensusParams = ConsensusParams::testnet();

// MAINNET_CANDIDATE — tokenomics v2 (2026-06-02)
// chain_id:        tensorium-mainnet-candidate-0
// Initial diff:    40 bits (GPU-first, RTX 3060+)
// Genesis ts:      1_780_272_000 (2026-06-01 00:00:00 UTC)
// Genesis nonce:   TBD — re-mine after tokenomics update
// Pre-mint (8M):   founder 1M | liquidity 3M | bridge 2M | ecosystem 2M
// Mining (25M):    11.9027... TXM/block, 10 eras, ~20 years
pub const MAINNET_CANDIDATE: ConsensusParams = ConsensusParams::mainnet_candidate();

/// Low-difficulty params for unit tests — mines instantly (difficulty 8 = 256 hashes avg).
pub const TEST_PARAMS: ConsensusParams = ConsensusParams {
    network: ChainNetwork::Testnet,
    chain_id: "tensorium-testnet-0",
    initial_leading_zero_bits: 8,
    min_leading_zero_bits: 4,
    max_leading_zero_bits: 16,
    founder_address: "",
    genesis_allocations: &[],
    ..ConsensusParams::testnet()
};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_supply_split(params: ConsensusParams) {
        assert_eq!(params.total_supply_atoms, 33_000_000 * COIN);
        assert_eq!(
            params.founder_allocation_atoms + params.mining_allocation_atoms,
            params.total_supply_atoms,
            "pre-mint + mining must equal total supply"
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
        assert!(TESTNET.genesis_allocations.is_empty());
        assert_eq!(TESTNET.coinbase_maturity_blocks, 10);
        assert_eq!(TESTNET.max_future_block_time_seconds, 2 * 60 * 60);
        assert_supply_split(TESTNET);
    }

    #[test]
    fn mainnet_candidate_tokenomics_match_reference_supply_plan() {
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
        // MC has different reward (25M mining) and different genesis allocations
        assert_eq!(MAINNET_CANDIDATE.initial_reward_atoms, 1_190_279_581);
        assert_eq!(MAINNET_CANDIDATE.genesis_allocations.len(), 4);
        let genesis_total: u64 = MAINNET_CANDIDATE.genesis_allocations.iter().map(|(_, a)| a).sum();
        assert_eq!(genesis_total, 8_000_000 * COIN);
        assert_supply_split(MAINNET_CANDIDATE);
    }

    #[test]
    fn mainnet_is_gpu_first_harder_than_reference_network() {
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
