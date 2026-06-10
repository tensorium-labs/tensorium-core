use serde::{Deserialize, Serialize};

pub const COIN: u64 = 100_000_000;
pub const MAX_HALVING_ERAS: u32 = 10;
pub const TOTAL_SUPPLY_COINS: u64 = 33_000_000;
pub const TOTAL_SUPPLY_ATOMS: u64 = TOTAL_SUPPLY_COINS * COIN;
/// Zero premine: the entire max supply is mining-only issuance.
pub const MINING_ALLOCATION_COINS: u64 = TOTAL_SUPPLY_COINS;
pub const MINING_ALLOCATION_ATOMS: u64 = MINING_ALLOCATION_COINS * COIN;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChainNetwork {
    Testnet,
    Mainnet,
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
    /// Height at which difficulty retargeting (`next_leading_zero_bits`) becomes
    /// a consensus-enforced rule. Below this height every block must use the
    /// network's fixed `initial_leading_zero_bits` (preserves validity of all
    /// blocks mined before the fork). `u64::MAX` disables retargeting entirely —
    /// the network stays on fixed difficulty until a real activation height is
    /// chosen and coordinated.
    pub difficulty_retarget_activation_height: u64,
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
            // MAINNET remains GPU-first at 42 bits.
            initial_leading_zero_bits: 20,
            min_leading_zero_bits: 8,
            max_leading_zero_bits: 36,
            difficulty_adjustment_window: 60,
            // Disabled until a real activation height is chosen and coordinated.
            difficulty_retarget_activation_height: u64::MAX,
            // Short maturity retained for the low-difficulty development network.
            coinbase_maturity_blocks: 10,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }

    pub const fn mainnet() -> Self {
        Self {
            network: ChainNetwork::Mainnet,
            chain_id: "tensorium-mainnet",
            target_block_seconds: 60,
            halving_interval_blocks: 2_102_400,
            max_halving_eras: MAX_HALVING_ERAS,
            total_supply_atoms: TOTAL_SUPPLY_ATOMS,
            founder_allocation_atoms: 0,
            mining_allocation_atoms: MINING_ALLOCATION_ATOMS, // = 33M (zero premine)
            genesis_allocations: &[],
            founder_address: "",
            initial_reward_atoms: 785_584_523, // ~7.8558 TXM/block for 33M over 10 eras
            initial_leading_zero_bits: 42,
            min_leading_zero_bits: 34,
            max_leading_zero_bits: 58,
            difficulty_adjustment_window: 60,
            // Active from genesis: this is a fresh chain with no blocks mined
            // under fixed difficulty, so there is no backward-compat concern.
            difficulty_retarget_activation_height: 0,
            coinbase_maturity_blocks: 10,
            max_future_block_time_seconds: 2 * 60 * 60,
            max_block_bytes: 1_000_000,
        }
    }
}

// ── CONSENSUS FREEZE ──────────────────────────────────────────────────────
// Low-difficulty development-network parameters retained for tests and migration drills.
// MAINNET diff (42 bits) remains higher and is unaffected by this configuration.
pub const TESTNET: ConsensusParams = ConsensusParams::testnet();

// MAINNET — TensorHash v1 clean relaunch, tokenomics v2 (2026-06-10)
// chain_id:        tensorium-mainnet
// Algorithm:       TensorHash v1 (memory-hard, GPU-first)
// Initial diff:    42 bits equivalent, retargeting active from genesis (window 60 blocks)
// Genesis ts:      TBD — set at actual launch time
// Genesis nonce:   TBD — re-mine offline (CPU brute-force or GPU miner) before launch
// Pre-mint:        0 (zero premine, mining-only issuance)
// Mining (33M):    ~7.8558 TXM/block, halving every ~4 years, 10 eras, ~40 years
pub const MAINNET: ConsensusParams = ConsensusParams::mainnet();

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
    fn mainnet_tokenomics_match_zero_premine_relaunch_plan() {
        assert_eq!(MAINNET.chain_id, "tensorium-mainnet");
        assert_eq!(MAINNET.target_block_seconds, TESTNET.target_block_seconds);
        // MAINNET uses a 4-year halving era (TESTNET uses 2 years) — different
        // emission schedule for the zero-premine relaunch.
        assert_eq!(MAINNET.halving_interval_blocks, 2_102_400);
        assert_eq!(MAINNET.max_halving_eras, TESTNET.max_halving_eras);
        // Zero premine: no genesis allocations, founder allocation is 0.
        assert_eq!(MAINNET.founder_allocation_atoms, 0);
        assert!(MAINNET.genesis_allocations.is_empty());
        let genesis_total: u64 = MAINNET.genesis_allocations.iter().map(|(_, a)| a).sum();
        assert_eq!(genesis_total, 0);
        // 33M mining allocation with new initial reward.
        assert_eq!(MAINNET.initial_reward_atoms, 785_584_523);
        assert_eq!(MAINNET.coinbase_maturity_blocks, 10);
        assert_supply_split(MAINNET);
    }

    #[test]
    fn mainnet_is_gpu_first_harder_than_reference_network() {
        assert!(MAINNET.initial_leading_zero_bits > TESTNET.initial_leading_zero_bits);
        assert!(MAINNET.min_leading_zero_bits > TESTNET.min_leading_zero_bits);
        assert!(MAINNET.max_leading_zero_bits > TESTNET.max_leading_zero_bits);
        assert!(MAINNET.min_leading_zero_bits <= MAINNET.initial_leading_zero_bits);
        assert!(MAINNET.initial_leading_zero_bits <= MAINNET.max_leading_zero_bits);
    }

    #[test]
    fn mainnet_retargeting_is_active_from_genesis() {
        // Fresh chain — retargeting is enabled from block 0, unlike the old
        // MAINNET_CANDIDATE which kept it disabled (u64::MAX).
        assert_eq!(MAINNET.difficulty_retarget_activation_height, 0);
        assert_eq!(MAINNET.difficulty_adjustment_window, 60);
        assert_eq!(MAINNET.initial_leading_zero_bits, 42);
    }
}
