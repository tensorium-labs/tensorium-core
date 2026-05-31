use std::collections::{HashMap, HashSet};

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

/// All validated blocks indexed by hash hex (canonical + stale forks).
type BlockMap = HashMap<String, Block>;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChainState {
    /// Canonical chain in genesis-first order; `blocks[0]` is genesis.
    pub blocks: Vec<Block>,
    /// Every validated block, keyed by its hash hex.  Canonical blocks appear
    /// here too.  Old state files without this field get it populated lazily
    /// from `blocks` the first time `submit_block` is called.
    #[serde(default)]
    pub block_map: BlockMap,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum StateError {
    #[error("chain state already has a genesis block")]
    GenesisAlreadyExists,
    #[error("chain state has no genesis block")]
    MissingGenesis,
    #[error("mining failed before nonce limit")]
    MiningFailed,
    #[error("block is already known")]
    AlreadyKnown,
    #[error("block's parent is not known")]
    UnknownParent,
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

impl ChainState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Populate `block_map` from the canonical `blocks` Vec.
    ///
    /// Call this after deserializing state files that pre-date `block_map`.
    /// Safe to call multiple times; does nothing when the map is already
    /// populated.
    pub fn ensure_block_map(&mut self) {
        if self.block_map.is_empty() && !self.blocks.is_empty() {
            for block in &self.blocks {
                self.block_map.insert(block.hash().to_hex(), block.clone());
            }
        }
    }

    pub fn height(&self) -> Option<u64> {
        self.tip().map(|block| block.header.height)
    }

    pub fn tip(&self) -> Option<&Block> {
        self.blocks.last()
    }

    /// Hash of the current canonical tip, or `Hash256::ZERO` when empty.
    pub fn tip_hash(&self) -> Hash256 {
        self.tip().map(|b| b.hash()).unwrap_or(Hash256::ZERO)
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
        let hash_hex = block.hash().to_hex();
        let block_copy = block.clone();
        self.blocks.push(block);
        self.block_map.insert(hash_hex, block_copy);

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
        let hash_hex = block.hash().to_hex();
        let block_copy = block.clone();
        self.blocks.push(block);
        self.block_map.insert(hash_hex, block_copy);

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

    /// Like `candidate_block` but includes `extra_txs` after the coinbase.
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

    /// Accept a block from a miner or a peer, applying the fork-choice rule.
    ///
    /// The block is validated against its direct parent (which must already be
    /// in `block_map`).  The canonical chain (`self.blocks`) is updated only
    /// when the new chain's cumulative work exceeds the current best chain.
    ///
    /// Returns the validated block on success.  Returns `AlreadyKnown` if the
    /// block was seen before (not an error in practice — callers should ignore
    /// it).
    pub fn submit_block(
        &mut self,
        params: &ConsensusParams,
        block: Block,
        now_seconds: u64,
    ) -> Result<Block, StateError> {
        self.ensure_block_map();

        let block_hash = block.hash();
        let block_hash_hex = block_hash.to_hex();

        if self.block_map.contains_key(&block_hash_hex) {
            return Err(StateError::AlreadyKnown);
        }

        // Validate against the block's own parent (not necessarily the tip).
        let parent = self
            .block_map
            .get(&block.header.previous_hash.to_hex())
            .ok_or(StateError::UnknownParent)?
            .clone();

        validate_block(params, Some(&parent), &block, now_seconds)?;

        // Persist the validated block before fork-choice comparison.
        let old_tip_hash = self.tip_hash();
        let old_tip_height = self.height().unwrap_or(0);
        self.block_map.insert(block_hash_hex, block.clone());

        // Fork choice: the chain with the most cumulative work wins.
        let new_work = self.chain_work(block_hash);
        let current_work = self.chain_work(old_tip_hash);

        if new_work > current_work {
            let is_direct_extension = block.header.previous_hash == old_tip_hash;

            if !is_direct_extension {
                // Reorg: find common ancestor depth for logging.
                let ancestor_height =
                    self.common_ancestor_height(block_hash, old_tip_hash);
                let reorg_depth = old_tip_height.saturating_sub(ancestor_height);
                eprintln!(
                    "fork-choice: reorg depth={reorg_depth} \
                     old_tip={old_tip_hash} new_tip={block_hash}"
                );
            }

            self.blocks = self.build_canonical_chain(block_hash);
        }
        // else: block is on a side chain with equal or less work; it is stored
        // in block_map but the canonical chain is unchanged.

        Ok(block)
    }

    // -------------------------------------------------------------------------
    // Fork-choice internals
    // -------------------------------------------------------------------------

    /// Cumulative PoW work for the chain whose tip is at `tip_hash`.
    ///
    /// Work per block = 2^leading_zero_bits (represents expected hashes needed).
    fn chain_work(&self, tip_hash: Hash256) -> u128 {
        let mut work = 0u128;
        let mut current = tip_hash;
        loop {
            let block = match self.block_map.get(&current.to_hex()) {
                Some(b) => b,
                None => break,
            };
            work = work.saturating_add(1u128 << block.header.leading_zero_bits);
            if block.header.previous_hash == Hash256::ZERO {
                break;
            }
            current = block.header.previous_hash;
        }
        work
    }

    /// Reconstruct a genesis-first Vec<Block> by following parent links from
    /// `tip_hash`.
    fn build_canonical_chain(&self, tip_hash: Hash256) -> Vec<Block> {
        let mut chain = Vec::new();
        let mut current = tip_hash;
        loop {
            let block = match self.block_map.get(&current.to_hex()) {
                Some(b) => b.clone(),
                None => break,
            };
            let prev = block.header.previous_hash;
            chain.push(block);
            if prev == Hash256::ZERO {
                break;
            }
            current = prev;
        }
        chain.reverse();
        chain
    }

    /// Height of the deepest block shared by both chains.
    fn common_ancestor_height(&self, tip_a: Hash256, tip_b: Hash256) -> u64 {
        // Collect all hashes on chain A.
        let mut chain_a: HashSet<String> = HashSet::new();
        let mut cur = tip_a;
        loop {
            let hex = cur.to_hex();
            chain_a.insert(hex.clone());
            let block = match self.block_map.get(&hex) {
                Some(b) => b,
                None => break,
            };
            if block.header.previous_hash == Hash256::ZERO {
                break;
            }
            cur = block.header.previous_hash;
        }

        // Walk chain B until we land on a block that is also on chain A.
        let mut cur = tip_b;
        loop {
            if chain_a.contains(&cur.to_hex()) {
                return self
                    .block_map
                    .get(&cur.to_hex())
                    .map(|b| b.header.height)
                    .unwrap_or(0);
            }
            let block = match self.block_map.get(&cur.to_hex()) {
                Some(b) => b,
                None => break,
            };
            if block.header.previous_hash == Hash256::ZERO {
                break;
            }
            cur = block.header.previous_hash;
        }
        0
    }
}

// -----------------------------------------------------------------------------
// Private helpers
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{chain::{TESTNET, TEST_PARAMS}, pow::mine_header};

    use super::*;

    #[test]
    fn initializes_genesis_then_mines_next_block() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        assert_eq!(state.height(), Some(0));
        assert_eq!(state.block_map.len(), 1);

        state
            .mine_next_block(&TEST_PARAMS, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        assert_eq!(state.height(), Some(1));
        assert_eq!(state.blocks[1].header.previous_hash, state.blocks[0].hash());
        assert_eq!(state.block_map.len(), 2);
    }

    #[test]
    fn builds_candidate_then_accepts_mined_submit() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();

        let candidate = state
            .candidate_block(&TEST_PARAMS, 1_700_000_060, "template-miner")
            .unwrap();
        let mined_header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
        let mined_block = Block::new(mined_header, candidate.transactions);

        state
            .submit_block(&TEST_PARAMS, mined_block, 1_700_000_060)
            .unwrap();
        assert_eq!(state.height(), Some(1));
    }

    #[test]
    fn submit_block_rejects_already_known() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();

        let candidate = state
            .candidate_block(&TEST_PARAMS, 1_700_000_060, "miner")
            .unwrap();
        let mined_header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
        let block = Block::new(mined_header, candidate.transactions);

        state.submit_block(&TEST_PARAMS, block.clone(), 1_700_000_060).unwrap();
        assert_eq!(
            state.submit_block(&TEST_PARAMS, block, 1_700_000_060),
            Err(StateError::AlreadyKnown)
        );
    }

    #[test]
    fn submit_block_rejects_unknown_parent() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();

        // Build a block whose previous_hash points to an unknown block.
        let mut orphan = candidate_block(&TEST_PARAMS, None, 1_700_000_060, "miner", vec![]);
        orphan.header.previous_hash = Hash256([0xff; 32]); // unknown parent
        orphan.header.height = 1;

        assert_eq!(
            state.submit_block(&TEST_PARAMS, orphan, 1_700_000_060),
            Err(StateError::UnknownParent)
        );
    }

    /// Two miners produce competing blocks at height 1.  The one with more work
    /// (higher leading_zero_bits that still passes validation) becomes the tip.
    ///
    /// We manually craft two valid blocks extending genesis with the same
    /// leading_zero_bits, then verify the first-accepted one wins on tie.
    #[test]
    fn fork_choice_keeps_first_seen_on_equal_work() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();

        // Mine two competing blocks at height 1.
        let c1 = state
            .candidate_block(&TEST_PARAMS, 1_700_000_060, "miner-a")
            .unwrap();
        let h1 = mine_header(c1.header.clone(), 10_000_000).unwrap();
        let block_a = Block::new(h1, c1.transactions.clone());

        let c2 = state
            .candidate_block(&TEST_PARAMS, 1_700_000_061, "miner-b")
            .unwrap();
        let h2 = mine_header(c2.header.clone(), 10_000_000).unwrap();
        let block_b = Block::new(h2, c2.transactions.clone());

        // Accept block_a first — it becomes canonical tip.
        state
            .submit_block(&TEST_PARAMS, block_a.clone(), 1_700_000_060)
            .unwrap();
        assert_eq!(state.tip().unwrap().hash(), block_a.hash());

        // block_b has equal work — first-seen (block_a) stays canonical.
        state
            .submit_block(&TEST_PARAMS, block_b.clone(), 1_700_000_061)
            .unwrap();
        assert_eq!(state.tip().unwrap().hash(), block_a.hash(), "first-seen stays canonical on equal work");

        // Both blocks are in block_map.
        assert!(state.block_map.contains_key(&block_a.hash().to_hex()));
        assert!(state.block_map.contains_key(&block_b.hash().to_hex()));
        assert_eq!(state.blocks.len(), 2, "canonical chain has genesis + block_a");
    }

    #[test]
    fn ensure_block_map_migrates_old_state() {
        // Simulate a state file loaded without block_map (default empty).
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        // Manually clear block_map to simulate old state file.
        state.block_map.clear();
        assert!(state.block_map.is_empty());

        // ensure_block_map should repopulate from blocks.
        state.ensure_block_map();
        assert_eq!(state.block_map.len(), 1);
    }
}
