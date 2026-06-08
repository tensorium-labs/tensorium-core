use std::path::{Path, PathBuf};

use rocksdb::{ColumnFamilyDescriptor, Options, WriteBatch, DB};
use tempfile::TempDir;
use thiserror::Error;

use crate::{
    block::{merkle_root, Block, BlockHeader, OutPoint, Transaction},
    chain::ConsensusParams,
    emission::reward_at_height,
    hash::Hash256,
    pow::mine_header,
    storage::{
        decode_block, decode_outpoint, decode_utxo_entry, encode_block, encode_height,
        encode_outpoint, encode_utxo_entry,
        CF_BLOCKS, CF_CANONICAL, CF_META, CF_UTXO, META_HEIGHT, META_TIP, META_UTXO_TIP,
    },
    utxo::{UtxoEntry, UtxoError, UtxoSet},
    validation::{validate_block, ValidationError},
};

fn cf_options() -> Vec<ColumnFamilyDescriptor> {
    vec![
        ColumnFamilyDescriptor::new(CF_BLOCKS,    Options::default()),
        ColumnFamilyDescriptor::new(CF_CANONICAL, Options::default()),
        ColumnFamilyDescriptor::new(CF_META,      Options::default()),
        ColumnFamilyDescriptor::new(CF_UTXO,      Options::default()),
    ]
}

fn open_rocksdb(path: &Path) -> DB {
    try_open_rocksdb(path)
        .unwrap_or_else(|e| panic!("Failed to open RocksDB at {}: {e}", path.display()))
}

/// Try to open RocksDB, retrying on transient lock contention.
/// Returns Err if the DB is still locked after ~10 s (30 attempts).
/// Used everywhere that must not panic when another thread holds the lock
/// (RPC handler, P2P sync threads, etc.).
pub fn try_open_rocksdb(path: &Path) -> Result<DB, String> {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let mut wait_ms = 10u64;
    for _ in 0..30 {
        match DB::open_cf_descriptors(&opts, path, cf_options()) {
            Ok(db) => return Ok(db),
            Err(e) if e.to_string().contains("temporarily unavailable")
                   || e.to_string().contains("lock") =>
            {
                std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                wait_ms = (wait_ms * 2).min(500);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!("DB at {} is locked by another thread", path.display()))
}

pub struct ChainState {
    db:           DB,
    _tmpdir:      Option<TempDir>,
    tip_cache:    Option<Block>,
    height_cache: Option<u64>,
    /// Blocks disconnected from the canonical chain by the most recent
    /// `submit_block` reorg (empty when the last accepted block simply
    /// extended the tip, or didn't change it at all). Non-coinbase
    /// transactions inside these blocks no longer have a home on the
    /// winning chain and should be re-queued into the mempool — see
    /// `take_reorg_requeue_candidates`. Without this, a transaction that
    /// happens to land in a block that later loses a natural fork vanishes
    /// silently instead of returning to the sender's mempool.
    last_disconnected: Vec<Block>,
}

impl ChainState {
    /// Create an in-memory (tempdir) instance — for tests only.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let db  = open_rocksdb(dir.path());
        ChainState { db, _tmpdir: Some(dir), tip_cache: None, height_cache: None, last_disconnected: Vec::new() }
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
        let mut s = ChainState { db, _tmpdir: None, tip_cache: None, height_cache: None, last_disconnected: Vec::new() };
        s.reload_caches();
        Ok(s)
    }

    /// Like `open_db` but returns Err instead of panicking if DB is locked.
    /// Used by the RPC handler so it can return 503 without blocking the accept loop.
    pub fn try_open_db(path: &Path) -> Result<Self, String> {
        let db_path: PathBuf = if path.extension().map(|e| e == "json").unwrap_or(false) {
            path.with_extension("db")
        } else {
            path.to_path_buf()
        };
        let db = try_open_rocksdb(&db_path)?;
        let mut s = ChainState { db, _tmpdir: None, tip_cache: None, height_cache: None, last_disconnected: Vec::new() };
        s.reload_caches();
        Ok(s)
    }

    /// Drains the non-coinbase transactions from blocks that the most recent
    /// `submit_block` reorg knocked off the canonical chain. Coinbase
    /// transactions are excluded — their reward is simply forfeited when a
    /// block is orphaned (normal PoW behaviour, nothing to requeue).
    ///
    /// Callers should try to re-admit each returned transaction into the
    /// mempool against the *new* canonical UTXO set: some may now conflict
    /// with a transaction that made it into the winning chain (e.g. the same
    /// transaction was independently mined into both branches, or its inputs
    /// were spent by a competing transaction) and should be dropped silently
    /// in that case — `Mempool::add` already reports those as errors.
    pub fn take_reorg_requeue_candidates(&mut self) -> Vec<Transaction> {
        std::mem::take(&mut self.last_disconnected)
            .into_iter()
            .flat_map(|b| b.transactions.into_iter())
            .filter(|tx| !tx.is_coinbase())
            .collect()
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
    #[error(transparent)]
    Utxo(#[from] UtxoError),
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

    fn utxo_get(&self, outpoint: &OutPoint) -> Option<UtxoEntry> {
        let cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        self.db
            .get_cf(cf, encode_outpoint(outpoint))
            .expect("DB read")
            .map(|b| decode_utxo_entry(&b))
    }

    /// Test/helper: write a single UTXO entry directly (no batch).
    #[cfg(test)]
    fn utxo_put_direct(&self, outpoint: &OutPoint, entry: &UtxoEntry) {
        let cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        self.db
            .put_cf(cf, encode_outpoint(outpoint), encode_utxo_entry(entry))
            .expect("DB put");
    }

    /// Scan the persistent UTXO set, returning every entry whose output
    /// `script_pubkey` equals `script`. Replaces a full-chain replay for
    /// address/script queries (e.g. `/getutxos`). O(set size), not O(chain).
    pub fn utxos_for_script(&self, script: &[u8]) -> Vec<(OutPoint, UtxoEntry)> {
        let cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        self.db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .filter_map(|r| {
                let (k, v) = r.expect("utxo iterator read");
                let entry = decode_utxo_entry(&v);
                if entry.output.script_pubkey == script {
                    Some((decode_outpoint(&k), entry))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Look up a single outpoint in the persistent UTXO set.
    /// Public so the node can seed a minimal `UtxoSet` for mempool acceptance
    /// without replaying the whole chain.
    pub fn utxo_lookup(&self, outpoint: &OutPoint) -> Option<UtxoEntry> {
        self.utxo_get(outpoint)
    }

    fn read_meta_utxo_tip(&self) -> Option<Hash256> {
        let cf = self.db.cf_handle(CF_META).expect("meta CF");
        self.db
            .get_cf(cf, META_UTXO_TIP)
            .expect("meta read")
            .and_then(|b| b.as_slice().try_into().ok().map(Hash256))
    }

    /// Return the canonical block at `height`, if present.
    pub fn get_block_by_height(&self, height: u64) -> Option<Block> {
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
        let mut block = candidate_block(params, None, timestamp_seconds, "genesis", vec![], 0);
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
        Ok(candidate_block(params, Some(parent), timestamp_seconds, miner, vec![], 0))
    }

    /// Like `candidate_block` but includes `extra_txs` after the coinbase.
    /// Like `candidate_block` but includes mempool transactions after the coinbase.
    /// `total_fees` is the sum of all fees in `extra_txs` — callers must pre-calculate
    /// this (e.g. from `Mempool::select_for_block()`) so the coinbase is credited correctly.
    pub fn candidate_block_with_mempool(
        &self,
        params: &ConsensusParams,
        timestamp_seconds: u64,
        miner: &str,
        extra_txs: Vec<Transaction>,
        total_fees: u64,
    ) -> Result<Block, StateError> {
        let parent = self.tip().ok_or(StateError::MissingGenesis)?;
        Ok(candidate_block(params, Some(parent), timestamp_seconds, miner, extra_txs, total_fees))
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
        // Make sure the persistent UTXO set reflects the current tip before we
        // validate against it (rebuilds once after upgrade; cheap when synced).
        self.ensure_utxo_synced(params)?;
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

        // Reset reorg bookkeeping for this call — only the reorg branch below
        // repopulates it, so a fast-path extension or a stored-but-losing side
        // block both correctly report "nothing to requeue".
        self.last_disconnected = Vec::new();

        if new_work > old_work {
            if parent_hash == old_tip_hash {
                // Fast path: the block extends the current canonical tip. The
                // persistent UTXO set already reflects the parent, so validate
                // against it and apply the block's delta.
                self.validate_block_utxo(params, &block)?;
                let height = block.header.height;
                let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
                let meta_cf = self.db.cf_handle(CF_META).expect("meta CF");
                let mut batch = WriteBatch::default();
                batch.put_cf(canonical_cf, &encode_height(height), &block_hash.0);
                self.apply_utxo_delta_to_batch(&mut batch, &block);
                batch.put_cf(meta_cf, META_TIP, &block_hash.0);
                batch.put_cf(meta_cf, META_HEIGHT, &encode_height(height));
                batch.put_cf(meta_cf, META_UTXO_TIP, &block_hash.0);
                self.write_batch(batch);
                self.tip_cache = Some(block.clone());
                self.height_cache = Some(height);
            } else {
                // Reorg: rebuild the UTXO set along the new canonical chain. The
                // replay validates every block on the branch; if any is invalid
                // the reorg is rejected and the old chain + UTXO are preserved.
                let new_canonical = self.build_canonical_chain(block_hash);
                let set = self.replay_utxo(params, &new_canonical)?;
                let height = new_canonical.last().map(|b| b.header.height).unwrap_or(0);
                let old_height = self.height_cache.unwrap_or(0);

                // Diff against the chain we're replacing: every block on the
                // old chain past the fork point is being disconnected, and its
                // (non-coinbase) transactions need to find their way back into
                // the mempool — see `take_reorg_requeue_candidates`.
                let old_canonical = self.build_canonical_chain(old_tip_hash);
                let fork_index = old_canonical.iter().zip(new_canonical.iter())
                    .take_while(|(a, b)| a.hash() == b.hash())
                    .count();
                self.last_disconnected = old_canonical[fork_index..].to_vec();
                let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
                let meta_cf = self.db.cf_handle(CF_META).expect("meta CF");
                let mut batch = WriteBatch::default();
                for b in &new_canonical {
                    batch.put_cf(canonical_cf, &encode_height(b.header.height), &b.hash().0);
                }
                // Prune stale tail entries when the winning chain is shorter
                // (see commit 842b2b8).
                for h in (height + 1)..=old_height {
                    batch.delete_cf(canonical_cf, &encode_height(h));
                }
                batch.put_cf(meta_cf, META_TIP, &block_hash.0);
                batch.put_cf(meta_cf, META_HEIGHT, &encode_height(height));
                self.rewrite_utxo_into_batch(&mut batch, &set, &block_hash);
                self.write_batch(batch);
                self.tip_cache = Some(block.clone());
                self.height_cache = Some(height);
            }
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

    /// Replay `apply_block` over `chain` (genesis-first order) into a fresh set.
    fn replay_utxo(&self, params: &ConsensusParams, chain: &[Block]) -> Result<UtxoSet, StateError> {
        let mut set = UtxoSet::new();
        for block in chain {
            set.apply_block(params, block)?;
        }
        Ok(set)
    }

    /// Delete every existing CF_UTXO entry, then insert all entries of `set`,
    /// and stamp META_UTXO_TIP — all in `batch`.
    fn rewrite_utxo_into_batch(&self, batch: &mut WriteBatch, set: &UtxoSet, utxo_tip: &Hash256) {
        let utxo_cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        let meta_cf = self.db.cf_handle(CF_META).expect("meta CF");
        let existing: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(utxo_cf, rocksdb::IteratorMode::Start)
            .filter_map(|r| r.ok().map(|(k, _)| k))
            .collect();
        for k in existing {
            batch.delete_cf(utxo_cf, k);
        }
        for (op, entry) in &set.entries {
            batch.put_cf(utxo_cf, encode_outpoint(op), encode_utxo_entry(entry));
        }
        batch.put_cf(meta_cf, META_UTXO_TIP, &utxo_tip.0);
    }

    /// Validate a block's transactions against the persistent UTXO set by
    /// seeding a temporary set with exactly the inputs the block references and
    /// running the canonical `apply_block` rules.
    fn validate_block_utxo(&self, params: &ConsensusParams, block: &Block) -> Result<(), StateError> {
        let mut seed = UtxoSet::new();
        for tx in block.transactions.iter().skip(1) {
            for input in &tx.inputs {
                if let Some(entry) = self.utxo_get(&input.previous_output) {
                    seed.entries.insert(input.previous_output, entry);
                }
                // absent input → apply_block returns MissingInput
            }
        }
        seed.apply_block(params, block)?;
        Ok(())
    }

    /// Append a single block's UTXO delta (spend inputs, create outputs) to `batch`.
    fn apply_utxo_delta_to_batch(&self, batch: &mut WriteBatch, block: &Block) {
        let utxo_cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        for tx in block.transactions.iter().skip(1) {
            for input in &tx.inputs {
                batch.delete_cf(utxo_cf, encode_outpoint(&input.previous_output));
            }
        }
        for tx in &block.transactions {
            for (index, output) in tx.outputs.iter().enumerate() {
                if output.script_pubkey.first() == Some(&crate::script::OP_RETURN) {
                    continue;
                }
                let outpoint = OutPoint { txid: tx.id, output_index: index as u32 };
                let entry = UtxoEntry {
                    output: output.clone(),
                    created_height: block.header.height,
                    coinbase: tx.is_coinbase(),
                };
                batch.put_cf(utxo_cf, encode_outpoint(&outpoint), encode_utxo_entry(&entry));
            }
        }
    }

    /// Expose raw DB handle — used by migration only.
    pub(crate) fn db_handle(&self) -> &DB { &self.db }

    /// Public wrapper for reload_caches — used by migration.
    pub(crate) fn reload_caches_pub(&mut self) { self.reload_caches(); }

    /// Ensure the persistent UTXO set reflects the current canonical tip.
    /// Cheap (one meta read) when already in sync; rebuilds from the canonical
    /// chain when META_UTXO_TIP is absent or stale (first upgrade / crash heal).
    pub fn ensure_utxo_synced(&mut self, params: &ConsensusParams) -> Result<(), StateError> {
        let tip = match self.tip_cache.as_ref() {
            Some(b) => b.hash(),
            None => return Ok(()), // empty chain — nothing to build
        };
        if self.read_meta_utxo_tip() == Some(tip) {
            return Ok(());
        }
        let chain: Vec<Block> = self.canonical_blocks_iter().collect();
        let set = self.replay_utxo(params, &chain)?;
        let mut batch = WriteBatch::default();
        self.rewrite_utxo_into_batch(&mut batch, &set, &tip);
        self.write_batch(batch);
        Ok(())
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
    let block = candidate_block(params, parent, timestamp_seconds, miner, vec![], 0);
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
    total_fees: u64,
) -> Block {
    let height = parent.map_or(0, |block| block.header.height + 1);
    let previous_hash = parent.map_or(Hash256::ZERO, Block::hash);
    // Miner earns block reward + all transaction fees included in this block.
    let reward = reward_at_height(params, height).saturating_add(total_fees);
    let coinbase_tx = if height == 0 && (!params.genesis_allocations.is_empty() || !params.founder_address.is_empty()) {
        Transaction::genesis_coinbase(
            reward, miner,
            params.founder_allocation_atoms, params.founder_address,
            params.genesis_allocations,
        )
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
        let mut orphan = candidate_block(&TEST_PARAMS, None, 1_700_000_060, "miner", vec![], 0);
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
            0,
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
            0,
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
            0,
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

    /// Regression test for the "UTXO apply failed: transaction spends an
    /// output that does not exist" payout bug observed on mainnet.
    ///
    /// Root cause: when a reorg replaces a TALLER chain with a SHORTER one
    /// that has more cumulative work (possible because of difficulty
    /// retargeting), `submit_block` overwrote canonical entries for
    /// `0..=new_height` but left the old chain's entries at
    /// `new_height+1..=old_height` in place. `canonical_blocks_iter()` has no
    /// upper bound, so it would then yield the new chain followed by the old
    /// chain's orphaned tail — a non-contiguous sequence whose blocks don't
    /// chain together, causing `build_utxo_set`'s replay to fail.
    #[test]
    fn submit_block_prunes_stale_canonical_entries_on_reorg_to_shorter_chain() {
        let mut state = ChainState::new();
        state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        let genesis = state.get_block_by_height(0).unwrap();

        // Chain A: five low-difficulty blocks (height 5) extending genesis —
        // becomes canonical first since it's the only chain.
        let mut parent = genesis.clone();
        let mut chain_a = Vec::new();
        for i in 0..5u64 {
            let c = candidate_block(
                &TEST_PARAMS,
                Some(&parent),
                1_700_000_060 + i,
                "miner-a",
                vec![],
                0,
            );
            let h = mine_header(c.header.clone(), 1_000_000).unwrap();
            let block = Block::new(h, c.transactions);
            state
                .submit_block(&TEST_PARAMS, block.clone(), 1_700_000_060 + i)
                .unwrap();
            parent = block.clone();
            chain_a.push(block);
        }
        assert_eq!(state.height(), Some(5));
        assert_eq!(state.tip().unwrap().hash(), chain_a[4].hash());

        // Chain B: a single block directly off genesis, mined at much higher
        // difficulty. Its cumulative work (genesis@8 + 1 block@16 = 256 +
        // 65536 = 65792) exceeds chain A's (genesis@8 + 5 blocks@8 = 1536),
        // even though it is far shorter — exactly the scenario that produced
        // the 936→901 reorg observed on the Vultr node.
        let mut c_b1 = candidate_block(
            &TEST_PARAMS,
            Some(&genesis),
            1_700_000_500,
            "miner-b",
            vec![],
            0,
        );
        c_b1.header.leading_zero_bits = 16;
        let h_b1 = mine_header(c_b1.header.clone(), 10_000_000).unwrap();
        let b1 = Block::new(h_b1, c_b1.transactions);

        state
            .submit_block(&TEST_PARAMS, b1.clone(), 1_700_000_500)
            .unwrap();

        // The shorter-but-heavier chain must win the reorg.
        assert_eq!(
            state.height(),
            Some(1),
            "shorter chain with more cumulative work must become canonical"
        );
        assert_eq!(state.tip().unwrap().hash(), b1.hash());

        // Canonical iteration must yield exactly the new chain — not the old
        // chain's orphaned tail at heights 2..=5.
        let canon: Vec<Block> = state.canonical_blocks_iter().collect();
        assert_eq!(
            canon.iter().map(|b| b.hash()).collect::<Vec<_>>(),
            vec![genesis.hash(), b1.hash()],
            "stale canonical entries from the replaced chain must be pruned"
        );

        // Heights beyond the new tip must not resolve to stale entries.
        assert!(state.get_block_by_height(2).is_none());
        assert!(state.get_block_by_height(5).is_none());

        // The old chain's blocks remain stored (so a future reorg back to a
        // heavier extension of chain A is still possible) — they're just no
        // longer indexed as canonical.
        assert!(state.get_block_by_hash(chain_a[4].hash()).is_some());
    }

    #[test]
    fn submit_block_rejects_inflated_coinbase_extension() {
        use crate::chain::TEST_PARAMS;
        use crate::emission::reward_at_height;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        let genesis = state.get_block_by_height(0).unwrap();

        // Build a height-1 block whose coinbase pays double the scheduled reward.
        let mut c = candidate_block(&TEST_PARAMS, Some(&genesis), 1_700_000_060, "miner", vec![], 0);
        let inflated = reward_at_height(&TEST_PARAMS, 1).saturating_mul(2);
        c.transactions[0] = Transaction::coinbase(1, inflated, "attacker");
        c.header.merkle_root = merkle_root(&c.transactions);
        c.header.nonce = 0;
        let header = mine_header(c.header.clone(), 10_000_000).unwrap();
        let bad = Block::new(header, c.transactions);

        let result = state.submit_block(&TEST_PARAMS, bad.clone(), 1_700_000_060);
        assert!(matches!(result, Err(StateError::Utxo(_))), "inflated coinbase must be rejected, got {result:?}");
        // The block must NOT have become canonical.
        assert_eq!(state.height(), Some(0));
        assert_ne!(state.tip().unwrap().hash(), bad.hash());
    }

    #[test]
    fn submit_block_accepts_valid_extension_and_tracks_utxo() {
        use crate::chain::TEST_PARAMS;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        let candidate = state.candidate_block(&TEST_PARAMS, 1_700_000_060, "miner").unwrap();
        let header = mine_header(candidate.header.clone(), 1_000_000).unwrap();
        let block = Block::new(header, candidate.transactions);
        state.submit_block(&TEST_PARAMS, block.clone(), 1_700_000_060).unwrap();

        assert_eq!(state.height(), Some(1));
        // The new coinbase output is now in the persistent UTXO set, and the
        // persisted tip stamp tracks the new block.
        assert_eq!(state.read_meta_utxo_tip(), Some(block.hash()));
        let coinbase = &block.transactions[0];
        let op = crate::block::OutPoint { txid: coinbase.id, output_index: 0 };
        assert!(state.utxo_get(&op).is_some(), "coinbase output must be tracked in CF_UTXO");
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
    fn utxo_cf_put_get_roundtrip() {
        use crate::block::{OutPoint, TxOutput};
        use crate::hash::Hash256;
        use crate::utxo::UtxoEntry;
        let state = ChainState::new();
        let op = OutPoint { txid: Hash256([3u8; 32]), output_index: 2 };
        let entry = UtxoEntry {
            output: TxOutput { value_atoms: 500, script_pubkey: vec![1, 2, 3] },
            created_height: 9,
            coinbase: false,
        };
        state.utxo_put_direct(&op, &entry);
        assert_eq!(state.utxo_get(&op), Some(entry));
        assert_eq!(state.utxo_get(&OutPoint { txid: Hash256([9u8; 32]), output_index: 0 }), None);
    }

    #[test]
    fn ensure_utxo_synced_builds_set_from_canonical_chain() {
        use crate::chain::TEST_PARAMS;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.mine_next_block(&TEST_PARAMS, 1_700_000_060, "miner", 1_000_000).unwrap();
        state.mine_next_block(&TEST_PARAMS, 1_700_000_120, "miner", 1_000_000).unwrap();

        // mine_next_block does not maintain CF_UTXO, so it starts empty/out of sync.
        assert!(state.read_meta_utxo_tip().is_none());

        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        // After sync, META_UTXO_TIP matches the tip and the persisted set equals
        // a from-scratch replay of the canonical chain.
        assert_eq!(state.read_meta_utxo_tip(), Some(state.tip_hash()));
        let chain: Vec<Block> = state.canonical_blocks_iter().collect();
        let mut expected = UtxoSet::new();
        for b in &chain {
            expected.apply_block(&TEST_PARAMS, b).unwrap();
        }
        for (op, entry) in &expected.entries {
            assert_eq!(state.utxo_get(op).as_ref(), Some(entry));
        }
        // And a second call is a cheap no-op (still synced).
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();
        assert_eq!(state.read_meta_utxo_tip(), Some(state.tip_hash()));
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

    #[test]
    fn submit_block_rejects_double_spend_in_extension() {
        use crate::block::{OutPoint, TxInput, TxOutput};
        use crate::chain::TEST_PARAMS;
        use crate::script::standard::p2pkh_from_address;
        use crate::wallet::WalletKeypair;

        let keypair = WalletKeypair::generate();
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        // Height 1: coinbase to our address.
        let mut c1 = candidate_block(&TEST_PARAMS, Some(&state.tip().unwrap().clone()), 1_700_000_060, "x", vec![], 0);
        let cb1 = Transaction::coinbase(1, crate::emission::reward_at_height(&TEST_PARAMS, 1), keypair.address.as_str());
        c1.transactions[0] = cb1.clone();
        c1.header.merkle_root = merkle_root(&c1.transactions);
        let h1 = mine_header(c1.header.clone(), 10_000_000).unwrap();
        let b1 = Block::new(h1, c1.transactions);
        state.submit_block(&TEST_PARAMS, b1, 1_700_000_060).unwrap();

        // Advance past coinbase maturity so the coinbase is spendable.
        let mut ts = 1_700_000_120;
        for _ in 0..TEST_PARAMS.coinbase_maturity_blocks {
            let cand = state.candidate_block(&TEST_PARAMS, ts, "x").unwrap();
            let hh = mine_header(cand.header.clone(), 10_000_000).unwrap();
            state.submit_block(&TEST_PARAMS, Block::new(hh, cand.transactions), ts).unwrap();
            ts += 60;
        }

        // Build a block that spends the same coinbase output twice.
        let outpoint = OutPoint { txid: cb1.id, output_index: 0 };
        let mut spend = |val: u64| {
            let mut tx = Transaction::payment(
                vec![TxInput { previous_output: outpoint, signature_script: Vec::new() }],
                vec![TxOutput { value_atoms: val, script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap() }],
            );
            keypair.sign_transaction(&mut tx).unwrap();
            tx
        };
        let next_h = state.height().unwrap() + 1;
        let coinbase = Transaction::coinbase(next_h, crate::emission::reward_at_height(&TEST_PARAMS, next_h), "x");
        let txs = vec![coinbase, spend(10), spend(10)];
        let header = BlockHeader {
            version: 1,
            chain_id: TEST_PARAMS.chain_id.to_owned(),
            height: next_h,
            previous_hash: state.tip().unwrap().hash(),
            merkle_root: merkle_root(&txs),
            timestamp_seconds: ts,
            leading_zero_bits: TEST_PARAMS.initial_leading_zero_bits,
            nonce: 0,
        };
        let mined = mine_header(header, 10_000_000).unwrap();
        let bad = Block::new(mined, txs);

        let before = state.height();
        let result = state.submit_block(&TEST_PARAMS, bad, ts);
        assert!(matches!(result, Err(StateError::Utxo(_))), "double-spend must be rejected, got {result:?}");
        assert_eq!(state.height(), before, "rejected block must not advance the tip");
    }

    #[test]
    fn reorg_to_invalid_branch_is_rejected_and_preserves_state() {
        use crate::chain::TEST_PARAMS;
        use crate::emission::reward_at_height;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        let genesis = state.get_block_by_height(0).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        // Canonical: one valid block off genesis.
        let c1 = candidate_block(&TEST_PARAMS, Some(&genesis), 1_700_000_060, "miner-a", vec![], 0);
        let h1 = mine_header(c1.header.clone(), 10_000_000).unwrap();
        let good = Block::new(h1, c1.transactions);
        state.submit_block(&TEST_PARAMS, good.clone(), 1_700_000_060).unwrap();
        let good_utxo_tip = state.read_meta_utxo_tip();

        // Competing branch: a single higher-difficulty block off genesis whose
        // coinbase is inflated. It has more work, so fork choice would adopt it —
        // but the UTXO replay must reject it.
        let mut s1 = candidate_block(&TEST_PARAMS, Some(&genesis), 1_700_000_061, "miner-b", vec![], 0);
        s1.transactions[0] = Transaction::coinbase(1, reward_at_height(&TEST_PARAMS, 1).saturating_mul(5), "attacker");
        s1.header.leading_zero_bits = 16;
        s1.header.merkle_root = merkle_root(&s1.transactions);
        let h2 = mine_header(s1.header.clone(), 10_000_000).unwrap();
        let evil = Block::new(h2, s1.transactions);

        let result = state.submit_block(&TEST_PARAMS, evil.clone(), 1_700_000_061);
        assert!(matches!(result, Err(StateError::Utxo(_))), "invalid heavier branch must be rejected, got {result:?}");
        // Old canonical chain and UTXO tip are untouched.
        assert_eq!(state.tip().unwrap().hash(), good.hash());
        assert_eq!(state.height(), Some(1));
        assert_eq!(state.read_meta_utxo_tip(), good_utxo_tip);
    }

    /// Regression test for the "300k transfer vanished" mainnet incident
    /// (2026-06-08): a transaction confirmed only on a branch that later lost
    /// a natural fork disappeared completely — neither on-chain nor back in
    /// the mempool — even though its sender's balance correctly reverted.
    ///
    /// `submit_block` must record the transactions of any block it disconnects
    /// during a reorg so the node layer can requeue them into the mempool
    /// (see `take_reorg_requeue_candidates` and `requeue_reorged_transactions`
    /// in `tensorium-node`).
    #[test]
    fn reorg_requeues_orphaned_transactions_for_remempool() {
        use crate::block::{OutPoint, TxInput, TxOutput};
        use crate::chain::TEST_PARAMS;
        use crate::script::standard::p2pkh_from_address;
        use crate::wallet::WalletKeypair;

        let sender = WalletKeypair::generate();
        let receiver = WalletKeypair::generate();
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        // Height 1: coinbase to `sender`, then mature it by extending the chain.
        let mut c1 = candidate_block(&TEST_PARAMS, Some(&state.tip().unwrap().clone()), 1_700_000_060, "x", vec![], 0);
        let cb1 = Transaction::coinbase(1, crate::emission::reward_at_height(&TEST_PARAMS, 1), sender.address.as_str());
        c1.transactions[0] = cb1.clone();
        c1.header.merkle_root = merkle_root(&c1.transactions);
        let h1 = mine_header(c1.header.clone(), 10_000_000).unwrap();
        state.submit_block(&TEST_PARAMS, Block::new(h1, c1.transactions), 1_700_000_060).unwrap();

        let mut ts = 1_700_000_120;
        for _ in 0..TEST_PARAMS.coinbase_maturity_blocks {
            let cand = state.candidate_block(&TEST_PARAMS, ts, "x").unwrap();
            let hh = mine_header(cand.header.clone(), 10_000_000).unwrap();
            state.submit_block(&TEST_PARAMS, Block::new(hh, cand.transactions), ts).unwrap();
            ts += 60;
        }

        // Fork point: the common ancestor both competing branches extend.
        let fork_parent = state.tip().unwrap().clone();
        let fork_height = fork_parent.header.height;

        // The transaction that's about to get caught in the crossfire: spends
        // the now-mature coinbase, sending part of it to `receiver`.
        let outpoint = OutPoint { txid: cb1.id, output_index: 0 };
        let mut payment = Transaction::payment(
            vec![TxInput { previous_output: outpoint, signature_script: Vec::new() }],
            vec![TxOutput { value_atoms: 1_000_000, script_pubkey: p2pkh_from_address(receiver.address.as_str()).unwrap() }],
        );
        sender.sign_transaction(&mut payment).unwrap();
        let payment_txid = payment.id.to_hex();

        let block_at = |height: u64, parent_hash: Hash256, miner: &str, extra: Vec<Transaction>, ts: u64| {
            let coinbase = Transaction::coinbase(height, crate::emission::reward_at_height(&TEST_PARAMS, height), miner);
            let mut txs = vec![coinbase];
            txs.extend(extra);
            let header = BlockHeader {
                version: 1,
                chain_id: TEST_PARAMS.chain_id.to_owned(),
                height,
                previous_hash: parent_hash,
                merkle_root: merkle_root(&txs),
                timestamp_seconds: ts,
                leading_zero_bits: TEST_PARAMS.initial_leading_zero_bits,
                nonce: 0,
            };
            let mined = mine_header(header, 10_000_000).unwrap();
            Block::new(mined, txs)
        };

        // Branch A: includes `payment` and becomes canonical first.
        ts += 60;
        let block_a = block_at(fork_height + 1, fork_parent.hash(), "miner-a", vec![payment.clone()], ts);
        state.submit_block(&TEST_PARAMS, block_a.clone(), ts).unwrap();
        assert_eq!(state.tip().unwrap().hash(), block_a.hash(), "branch A becomes canonical first");
        // Fast-path extension — nothing to requeue.
        assert!(state.take_reorg_requeue_candidates().is_empty());

        // Branch B: a competing block at the same height, WITHOUT `payment` —
        // equal work, so A stays canonical for now...
        ts += 60;
        let block_b1 = block_at(fork_height + 1, fork_parent.hash(), "miner-b", vec![], ts);
        state.submit_block(&TEST_PARAMS, block_b1.clone(), ts).unwrap();
        assert_eq!(state.tip().unwrap().hash(), block_a.hash(), "equal-work side block must not yet reorg");
        assert!(state.take_reorg_requeue_candidates().is_empty());

        // ...until B is extended one block further, giving it strictly more
        // cumulative work and triggering a reorg that orphans `block_a`.
        ts += 60;
        let block_b2 = block_at(fork_height + 2, block_b1.hash(), "miner-b", vec![], ts);
        state.submit_block(&TEST_PARAMS, block_b2.clone(), ts).unwrap();
        assert_eq!(state.tip().unwrap().hash(), block_b2.hash(), "heavier branch B wins the reorg");

        // `payment` was confirmed only on the now-orphaned `block_a` — it must
        // come back as a requeue candidate so the node can re-admit it to the
        // mempool instead of letting it vanish.
        let requeued = state.take_reorg_requeue_candidates();
        assert_eq!(requeued.len(), 1, "exactly the orphaned payment should be requeued, got {requeued:?}");
        assert_eq!(requeued[0].id.to_hex(), payment_txid, "the orphaned payment transaction must be returned for requeue");

        // One-shot: draining again returns nothing until the next reorg.
        assert!(state.take_reorg_requeue_candidates().is_empty());
    }

    #[test]
    fn utxos_for_script_returns_only_matching_entries() {
        use crate::chain::TEST_PARAMS;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        let mut ts = 1_700_000_060;
        for _ in 0..4 {
            let cand = state.candidate_block(&TEST_PARAMS, ts, "miner").unwrap();
            let header = mine_header(cand.header.clone(), 1_000_000).unwrap();
            state.submit_block(&TEST_PARAMS, Block::new(header, cand.transactions), ts).unwrap();
            ts += 60;
        }

        // Expected = a from-scratch replay of the canonical chain.
        let chain: Vec<Block> = state.canonical_blocks_iter().collect();
        let mut expected = UtxoSet::new();
        for b in &chain {
            expected.apply_block(&TEST_PARAMS, b).unwrap();
        }

        // Pick a script that actually has entries.
        let script = expected
            .entries
            .values()
            .next()
            .expect("at least one utxo")
            .output
            .script_pubkey
            .clone();
        let expected_for_script: std::collections::HashMap<OutPoint, UtxoEntry> = expected
            .entries
            .iter()
            .filter(|(_, e)| e.output.script_pubkey == script)
            .map(|(op, e)| (*op, e.clone()))
            .collect();
        assert!(!expected_for_script.is_empty(), "test needs a non-empty script set");

        let got = state.utxos_for_script(&script);
        assert_eq!(
            got.len(),
            expected_for_script.len(),
            "utxos_for_script returned wrong count"
        );
        for (op, entry) in &got {
            assert_eq!(
                expected_for_script.get(op),
                Some(entry),
                "mismatched/extra utxo {op:?}"
            );
        }

        // A script with no entries returns empty.
        assert!(
            state.utxos_for_script(b"no-such-script").is_empty(),
            "unknown script must yield no entries"
        );
    }

    #[test]
    fn incremental_utxo_matches_full_replay_after_extensions() {
        use crate::chain::TEST_PARAMS;
        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        let mut ts = 1_700_000_060;
        for _ in 0..6 {
            let cand = state.candidate_block(&TEST_PARAMS, ts, "miner").unwrap();
            let header = mine_header(cand.header.clone(), 1_000_000).unwrap();
            state.submit_block(&TEST_PARAMS, Block::new(header, cand.transactions), ts).unwrap();
            ts += 60;
        }

        // The incrementally maintained CF_UTXO must equal a from-scratch replay.
        let chain: Vec<Block> = state.canonical_blocks_iter().collect();
        let mut expected = UtxoSet::new();
        for b in &chain {
            expected.apply_block(&TEST_PARAMS, b).unwrap();
        }
        // Every expected entry is present and equal.
        for (op, entry) in &expected.entries {
            assert_eq!(state.utxo_get(op).as_ref(), Some(entry), "missing/mismatched utxo {op:?}");
        }
        // And there are no extra entries: counts match.
        let cf = state.db.cf_handle(CF_UTXO).expect("utxo CF");
        let persisted_count = state.db.iterator_cf(cf, rocksdb::IteratorMode::Start).count();
        assert_eq!(persisted_count, expected.entries.len(), "persistent UTXO has extra/missing entries");
    }
}
