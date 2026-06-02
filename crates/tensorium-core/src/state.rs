use std::path::{Path, PathBuf};

use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch, DB};
use tempfile::TempDir;
use thiserror::Error;

use crate::{
    block::{merkle_root, Block, BlockHeader, Transaction},
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
    // ── Private DB helpers ──────────────────────────────────────────────────

    fn put_block_batch(&self, batch: &mut WriteBatch, block: &Block) {
        let blocks_cf    = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
        let meta_cf      = self.db.cf_handle(CF_META).expect("meta CF");
        let hash = block.hash();
        batch.put_cf(blocks_cf,    &hash.0,                               encode_block(block));
        batch.put_cf(canonical_cf, &encode_height(block.header.height),   &hash.0);
        batch.put_cf(meta_cf,      META_TIP,                              &hash.0);
        batch.put_cf(meta_cf,      META_HEIGHT,                           &encode_height(block.header.height));
        batch.put_cf(meta_cf,      crate::storage::META_CHAIN_ID,         block.header.chain_id.as_bytes());
    }

    fn write_batch(&self, batch: WriteBatch) {
        self.db.write(batch).expect("RocksDB write must not fail");
    }

    pub(crate) fn get_block_by_hash(&self, hash: Hash256) -> Option<Block> {
        let cf = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        self.db.get_cf(cf, &hash.0).expect("DB read").map(|b| decode_block(&b))
    }

    pub(crate) fn get_block_by_height(&self, height: u64) -> Option<Block> {
        let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
        let blocks_cf    = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        let hash_bytes = self.db.get_cf(canonical_cf, &encode_height(height)).ok()??;
        let hash = Hash256(hash_bytes.as_slice().try_into().ok()?);
        self.db.get_cf(blocks_cf, &hash.0).ok()?.map(|b| decode_block(&b))
    }

    fn block_known(&self, hash: &Hash256) -> bool {
        let cf = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        self.db.get_cf(cf, &hash.0).expect("DB read").is_some()
    }

    // ── Public methods (Tasks 5-7 will fill these in) ───────────────────────

    pub fn init_genesis(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        max_nonce: u64,
    ) -> Result<&Block, StateError> {
        if self.height_cache.is_some() {
            return Err(StateError::GenesisAlreadyExists);
        }
        let block = mine_candidate_block(params, None, timestamp_seconds, "genesis", max_nonce)?;
        validate_block(params, None, &block, timestamp_seconds)?;
        let mut batch = WriteBatch::default();
        self.put_block_batch(&mut batch, &block);
        self.write_batch(batch);
        self.tip_cache    = Some(block);
        self.height_cache = Some(0);
        Ok(self.tip_cache.as_ref().unwrap())
    }

    /// Initialize genesis using a pre-computed nonce (no CPU mining required).
    /// Used for GPU-first chains where genesis was mined offline via CUDA.
    pub fn init_genesis_nonce(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        genesis_nonce: u64,
    ) -> Result<&Block, StateError> {
        if self.height_cache.is_some() {
            return Err(StateError::GenesisAlreadyExists);
        }
        let mut block = candidate_block(params, None, timestamp_seconds, "genesis", vec![]);
        block.header.nonce = genesis_nonce;
        if !crate::pow::header_meets_work(&block.header) {
            return Err(StateError::MiningFailed);
        }
        validate_block(params, None, &block, timestamp_seconds)?;
        let mut batch = WriteBatch::default();
        self.put_block_batch(&mut batch, &block);
        self.write_batch(batch);
        self.tip_cache    = Some(block);
        self.height_cache = Some(0);
        Ok(self.tip_cache.as_ref().unwrap())
    }

    pub fn mine_next_block(
        &mut self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        miner: &str,
        max_nonce: u64,
    ) -> Result<&Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?.clone();
        let block = mine_candidate_block(params, Some(&parent), timestamp_seconds, miner, max_nonce)?;
        validate_block(params, Some(&parent), &block, timestamp_seconds)?;
        let height = block.header.height;
        let mut batch = WriteBatch::default();
        self.put_block_batch(&mut batch, &block);
        self.write_batch(batch);
        self.tip_cache    = Some(block);
        self.height_cache = Some(height);
        Ok(self.tip_cache.as_ref().unwrap())
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
    /// stored in RocksDB).  The canonical chain is updated only when the new
    /// chain's cumulative work exceeds the current best chain.
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
        let block_hash = block.hash();
        if self.block_known(&block_hash) {
            return Err(StateError::AlreadyKnown);
        }
        let parent_hash = block.header.previous_hash;
        let parent = self.get_block_by_hash(parent_hash).ok_or(StateError::UnknownParent)?;
        validate_block(params, Some(&parent), &block, now_seconds)?;

        // Always store the block.
        let blocks_cf = self.db.cf_handle(CF_BLOCKS).expect("blocks CF");
        self.db.put_cf(blocks_cf, &block_hash.0, encode_block(&block)).expect("DB put");

        // Fork choice.
        let old_tip_hash = self.tip_hash();
        let new_work     = self.chain_work(block_hash);
        let old_work     = self.chain_work(old_tip_hash);

        if new_work > old_work {
            let new_canonical = self.build_canonical_chain(block_hash);
            let height = new_canonical.last().map(|b| b.header.height).unwrap_or(0);
            let mut batch = WriteBatch::default();
            let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
            let meta_cf      = self.db.cf_handle(CF_META).expect("meta CF");
            for b in &new_canonical {
                batch.put_cf(canonical_cf, &encode_height(b.header.height), &b.hash().0);
            }
            batch.put_cf(meta_cf, META_TIP,    &block_hash.0);
            batch.put_cf(meta_cf, META_HEIGHT, &encode_height(height));
            self.write_batch(batch);
            self.tip_cache    = Some(block.clone());
            self.height_cache = Some(height);
        }
        Ok(block)
    }

    fn chain_work(&self, mut tip_hash: Hash256) -> u128 {
        let mut work = 0u128;
        loop {
            match self.get_block_by_hash(tip_hash) {
                None    => break,
                Some(b) => {
                    work = work.saturating_add(1u128 << b.header.leading_zero_bits);
                    if b.header.previous_hash == Hash256::ZERO { break; }
                    tip_hash = b.header.previous_hash;
                }
            }
        }
        work
    }

    fn build_canonical_chain(&self, tip_hash: Hash256) -> Vec<Block> {
        let mut chain = Vec::new();
        let mut cur = tip_hash;
        loop {
            match self.get_block_by_hash(cur) {
                None    => break,
                Some(b) => {
                    let prev = b.header.previous_hash;
                    chain.push(b);
                    if prev == Hash256::ZERO { break; }
                    cur = prev;
                }
            }
        }
        chain.reverse();
        chain
    }

    /// Expose raw DB handle — used by migration only.
    pub(crate) fn db_handle(&self) -> &DB { &self.db }

    /// Public wrapper for reload_caches — used by migration.
    pub(crate) fn reload_caches_pub(&mut self) { self.reload_caches(); }
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
        assert_eq!(state.block_count(), 1);

        state
            .mine_next_block(&TEST_PARAMS, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        assert_eq!(state.height(), Some(1));
        let genesis = state.get_block_by_height(0).unwrap();
        let block1  = state.get_block_by_height(1).unwrap();
        assert_eq!(block1.header.previous_hash, genesis.hash());
        assert_eq!(state.block_count(), 2);
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

        // Both blocks are in the DB.
        assert!(state.get_block_by_hash(block_a.hash()).is_some());
        assert!(state.get_block_by_hash(block_b.hash()).is_some());
        assert_eq!(
            state.block_count(),
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
        let genesis = state.get_block_by_height(0).unwrap();

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
        assert_eq!(state.block_count(), 3);
        assert_eq!(state.get_block_by_height(1).unwrap().hash(), side_block.hash());
        assert_eq!(state.get_block_by_height(2).unwrap().hash(), side_extension.hash());
        assert!(state.get_block_by_hash(canonical_block.hash()).is_some());
    }

    #[test]
    fn put_and_get_block() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        let tip = state.tip().unwrap().clone();
        assert_eq!(state.get_block_by_hash(tip.hash()), Some(tip));
    }

    #[test]
    fn get_block_by_height_returns_genesis() {
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        let got = state.get_block_by_height(0).expect("genesis at height 0");
        assert_eq!(got.header.height, 0);
    }

    #[test]
    fn state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        {
            let mut state = ChainState::open_db(&db_path).unwrap();
            state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
            state.mine_next_block(&TEST_PARAMS, 1_700_000_060, "miner", 1_000_000).unwrap();
            assert_eq!(state.height(), Some(1));
        }
        // state dropped here — DB closed

        let state = ChainState::open_db(&db_path).unwrap();
        assert_eq!(state.height(), Some(1));
        assert_eq!(state.get_block_by_height(0).unwrap().header.height, 0);
        assert_eq!(state.get_block_by_height(1).unwrap().header.height, 1);
    }
}
