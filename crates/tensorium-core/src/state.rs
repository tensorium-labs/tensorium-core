use std::path::{Path, PathBuf};

use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use thiserror::Error;

use crate::{
    block::{Block, BlockHeader, Transaction},
    chain::ConsensusParams,
    emission::reward_at_height,
    hash::Hash256,
    pow::mine_header,
    storage::{
        decode_block, encode_block, encode_height,
        CF_BLOCKS, CF_CANONICAL, CF_META, META_HEIGHT, META_TIP,
    },
    validation::{validate_block, ValidationError},
};

fn cf_options() -> Vec<ColumnFamilyDescriptor> {
    vec![
        ColumnFamilyDescriptor::new(CF_BLOCKS,    Options::default()),
        ColumnFamilyDescriptor::new(CF_CANONICAL, Options::default()),
        ColumnFamilyDescriptor::new(CF_META,      Options::default()),
    ]
}

fn open_rocksdb(path: &Path) -> DB {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    DB::open_cf_descriptors(&opts, path, cf_options())
        .unwrap_or_else(|e| panic!("Failed to open RocksDB at {}: {e}", path.display()))
}

pub struct ChainState {
    db:           DB,
    _tmpdir:      Option<TempDir>,
    tip_cache:    Option<Block>,
    height_cache: Option<u64>,
}

impl ChainState {
    /// Create an in-memory (tempdir) instance — for tests only.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let db  = open_rocksdb(dir.path());
        ChainState { db, _tmpdir: Some(dir), tip_cache: None, height_cache: None }
    }

    /// Open (or create) a persistent RocksDB at `path`.
    /// If `path` ends in `.json` the DB lives at `path` with `.json` replaced by `.db`.
    pub fn open_db(path: &Path) -> Result<Self, String> {
        let db_path: PathBuf = if path.extension().map(|e| e == "json").unwrap_or(false) {
            path.with_extension("db")
        } else {
            path.to_path_buf()
        };
        let db = open_rocksdb(&db_path);
        let mut s = ChainState { db, _tmpdir: None, tip_cache: None, height_cache: None };
        s.reload_caches();
        Ok(s)
    }

    fn reload_caches(&mut self) {
        let meta_cf   = self.db.cf_handle(CF_META).expect("meta CF");
        let blocks_cf = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");

        let tip_bytes = match self.db.get_cf(meta_cf, META_TIP).expect("meta tip read") {
            Some(b) => b,
            None    => return,
        };
        let hash = Hash256(tip_bytes.as_slice().try_into().expect("32-byte hash"));
        let block_bytes = match self.db.get_cf(blocks_cf, &hash.0).expect("block read") {
            Some(b) => b,
            None    => return,
        };
        let block = decode_block(&block_bytes);
        self.height_cache = Some(block.header.height);
        self.tip_cache    = Some(block);
    }

    pub fn height(&self) -> Option<u64> {
        self.height_cache
    }

    pub fn tip(&self) -> Option<&Block> {
        self.tip_cache.as_ref()
    }

    pub fn tip_hash(&self) -> Hash256 {
        self.tip().map(|b| b.hash()).unwrap_or(Hash256::ZERO)
    }

    pub fn block_count(&self) -> usize {
        self.height_cache.map(|h| h as usize + 1).unwrap_or(0)
    }

    pub fn canonical_blocks_iter(&self) -> impl Iterator<Item = Block> + '_ {
        let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
        let blocks_cf    = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        self.db
            .iterator_cf(canonical_cf, rocksdb::IteratorMode::Start)
            .filter_map(move |r| {
                let (_k, hash_bytes) = r.expect("iterator read");
                let hash = Hash256(hash_bytes.as_ref().try_into().ok()?);
                let block_bytes = self.db.get_cf(blocks_cf, &hash.0).ok()??;
                Some(decode_block(&block_bytes))
            })
    }
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

    /// Initialize genesis using a pre-computed nonce (no CPU mining required).
    /// Used for GPU-first chains where genesis was mined offline via CUDA.
    pub fn init_genesis_nonce(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        genesis_nonce: u64,
    ) -> Result<&Block, StateError> {
        if !self.blocks.is_empty() {
            return Err(StateError::GenesisAlreadyExists);
        }
        let mut block = candidate_block(params, None, timestamp_seconds, "genesis", vec![]);
        block.header.nonce = genesis_nonce;
        // Verify the pre-computed nonce actually satisfies difficulty
        if !crate::pow::header_meets_work(&block.header) {
            return Err(StateError::MiningFailed);
        }
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
        let block =
            mine_candidate_block(params, Some(&parent), timestamp_seconds, miner, max_nonce)?;
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
        Ok(candidate_block(
            params,
            Some(parent),
            timestamp_seconds,
            miner,
            vec![],
        ))
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
        Ok(candidate_block(
            params,
            Some(parent),
            timestamp_seconds,
            miner,
            extra_txs,
        ))
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
                let ancestor_height = self.common_ancestor_height(block_hash, old_tip_hash);
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
    let coinbase_tx = if height == 0 && !params.founder_address.is_empty() {
        Transaction::genesis_coinbase(reward, miner, params.founder_allocation_atoms, params.founder_address)
    } else {
        Transaction::coinbase(height, reward, miner)
    };
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
    use crate::{chain::TEST_PARAMS, pow::mine_header};

    use super::*;

    #[test]
    fn initializes_genesis_then_mines_next_block() {
        let mut state = ChainState::new();
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
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
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

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
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

        let candidate = state
            .candidate_block(&TEST_PARAMS, 1_700_000_060, "miner")
            .unwrap();
        let mined_header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
        let block = Block::new(mined_header, candidate.transactions);

        state
            .submit_block(&TEST_PARAMS, block.clone(), 1_700_000_060)
            .unwrap();
        assert_eq!(
            state.submit_block(&TEST_PARAMS, block, 1_700_000_060),
            Err(StateError::AlreadyKnown)
        );
    }

    #[test]
    fn submit_block_rejects_unknown_parent() {
        let mut state = ChainState::new();
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

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
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();

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
        assert_eq!(
            state.tip().unwrap().hash(),
            block_a.hash(),
            "first-seen stays canonical on equal work"
        );

        // Both blocks are in block_map.
        assert!(state.block_map.contains_key(&block_a.hash().to_hex()));
        assert!(state.block_map.contains_key(&block_b.hash().to_hex()));
        assert_eq!(
            state.blocks.len(),
            2,
            "canonical chain has genesis + block_a"
        );
    }

    #[test]
    fn fork_choice_reorgs_to_chain_with_more_work() {
        let mut state = ChainState::new();
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        let genesis = state.blocks[0].clone();

        let c1 = candidate_block(
            &TEST_PARAMS,
            Some(&genesis),
            1_700_000_060,
            "miner-a",
            vec![],
        );
        let h1 = mine_header(c1.header.clone(), 10_000_000).unwrap();
        let canonical_block = Block::new(h1, c1.transactions);
        state
            .submit_block(&TEST_PARAMS, canonical_block.clone(), 1_700_000_060)
            .unwrap();
        assert_eq!(state.tip().unwrap().hash(), canonical_block.hash());

        let s1 = candidate_block(
            &TEST_PARAMS,
            Some(&genesis),
            1_700_000_061,
            "miner-b",
            vec![],
        );
        let h2 = mine_header(s1.header.clone(), 10_000_000).unwrap();
        let side_block = Block::new(h2, s1.transactions);
        state
            .submit_block(&TEST_PARAMS, side_block.clone(), 1_700_000_061)
            .unwrap();
        assert_eq!(
            state.tip().unwrap().hash(),
            canonical_block.hash(),
            "equal-work side block must not reorg the tip"
        );

        let s2 = candidate_block(
            &TEST_PARAMS,
            Some(&side_block),
            1_700_000_120,
            "miner-b",
            vec![],
        );
        let h3 = mine_header(s2.header.clone(), 10_000_000).unwrap();
        let side_extension = Block::new(h3, s2.transactions);
        state
            .submit_block(&TEST_PARAMS, side_extension.clone(), 1_700_000_120)
            .unwrap();

        assert_eq!(state.tip().unwrap().hash(), side_extension.hash());
        assert_eq!(state.blocks.len(), 3);
        assert_eq!(state.blocks[1].hash(), side_block.hash());
        assert_eq!(state.blocks[2].hash(), side_extension.hash());
        assert!(state
            .block_map
            .contains_key(&canonical_block.hash().to_hex()));
    }

    #[test]
    fn ensure_block_map_migrates_old_state() {
        // Simulate a state file loaded without block_map (default empty).
        let mut state = ChainState::new();
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        // Manually clear block_map to simulate old state file.
        state.block_map.clear();
        assert!(state.block_map.is_empty());

        // ensure_block_map should repopulate from blocks.
        state.ensure_block_map();
        assert_eq!(state.block_map.len(), 1);
    }
}
