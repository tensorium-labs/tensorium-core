# Persistent UTXO Set + Block-Accept Validation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `ChainState::submit_block` reject any block whose transactions violate UTXO rules (inflated coinbase, double-spend, bad signature, output>input, immature-coinbase spend) before it is adopted as canonical, by maintaining a persistent UTXO set in RocksDB.

**Architecture:** A new `CF_UTXO` column family holds the UTXO set; `META_UTXO_TIP` records which block's post-state it reflects. On a tip extension, `submit_block` validates the block by seeding a small `UtxoSet` with its referenced inputs and running the existing `apply_block`, then writes the UTXO delta atomically with the canonical pointer. On a reorg it replays `apply_block` along the new branch into a fresh set and rewrites `CF_UTXO`. A cheap `ensure_utxo_synced` check (run at the top of `submit_block`) rebuilds the set once after upgrade and self-heals any inconsistency.

**Tech Stack:** Rust, RocksDB (`rust-rocksdb`), `bincode` serialization, `thiserror`. All work is in `crates/tensorium-core`.

**Spec:** `docs/superpowers/specs/2026-06-07-persistent-utxo-accept-validation-design.md`

---

## File Structure

- `crates/tensorium-core/src/storage/mod.rs` — add `CF_UTXO`, `META_UTXO_TIP`, and `encode_outpoint` / `decode_outpoint` / `encode_utxo_entry` / `decode_utxo_entry` with round-trip tests.
- `crates/tensorium-core/src/state.rs` — register `CF_UTXO` in `cf_options()`; add `StateError::Utxo`; add UTXO read/replay/persist helpers, `ensure_utxo_synced`, and the `submit_block` extension + reorg integration; add tests.
- `crates/tensorium-core/src/utxo.rs` — unchanged (`apply_block` reused as-is). `UtxoEntry` and `UtxoSet` are already `pub`.

---

## Task 1: Storage encoders for UTXO entries

**Files:**
- Modify: `crates/tensorium-core/src/storage/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/tensorium-core/src/storage/mod.rs`:

```rust
    #[test]
    fn outpoint_roundtrip() {
        use crate::block::OutPoint;
        use crate::hash::Hash256;
        let op = OutPoint { txid: Hash256([7u8; 32]), output_index: 0x01020304 };
        let encoded = encode_outpoint(&op);
        assert_eq!(encoded.len(), 36);
        assert_eq!(decode_outpoint(&encoded), op);
    }

    #[test]
    fn utxo_entry_roundtrip() {
        use crate::block::TxOutput;
        use crate::utxo::UtxoEntry;
        let entry = UtxoEntry {
            output: TxOutput { value_atoms: 11_902_795_81, script_pubkey: vec![0xde, 0xad, 0xbe, 0xef] },
            created_height: 1234,
            coinbase: true,
        };
        let bytes = encode_utxo_entry(&entry);
        assert_eq!(decode_utxo_entry(&bytes), entry);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-core storage:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function encode_outpoint` / `encode_utxo_entry` in this scope.

- [ ] **Step 3: Write the minimal implementation**

In `crates/tensorium-core/src/storage/mod.rs`, add the imports and constants near the existing ones:

```rust
use crate::block::OutPoint;
use crate::utxo::UtxoEntry;
```

Add to the existing `pub const` block:

```rust
pub const CF_UTXO: &str = "utxo";
pub const META_UTXO_TIP: &[u8] = b"utxo_tip";
```

Add the four helpers (after `decode_block`):

```rust
/// Encode an outpoint as a 36-byte key: txid (32) || output_index (4, big-endian).
pub fn encode_outpoint(outpoint: &OutPoint) -> [u8; 36] {
    let mut key = [0u8; 36];
    key[..32].copy_from_slice(&outpoint.txid.0);
    key[32..].copy_from_slice(&outpoint.output_index.to_be_bytes());
    key
}

/// Decode a 36-byte outpoint key.
pub fn decode_outpoint(bytes: &[u8]) -> OutPoint {
    let txid_bytes: [u8; 32] = bytes[..32].try_into().expect("outpoint key must be 36 bytes");
    let index_bytes: [u8; 4] = bytes[32..36].try_into().expect("outpoint key must be 36 bytes");
    OutPoint {
        txid: crate::hash::Hash256(txid_bytes),
        output_index: u32::from_be_bytes(index_bytes),
    }
}

/// Encode a UTXO entry to bytes using bincode.
pub fn encode_utxo_entry(entry: &UtxoEntry) -> Vec<u8> {
    bincode::serialize(entry).expect("UtxoEntry serialization must not fail")
}

/// Decode a UTXO entry from bytes.
pub fn decode_utxo_entry(bytes: &[u8]) -> UtxoEntry {
    bincode::deserialize(bytes).expect("UtxoEntry deserialization must not fail")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tensorium-core storage:: 2>&1 | tail -20`
Expected: PASS (4 storage tests: 2 existing + 2 new).

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/storage/mod.rs
git commit -m "feat(storage): UTXO column family constants and outpoint/entry codecs"
```

---

## Task 2: Register CF_UTXO and add UTXO read helpers + StateError::Utxo

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core utxo_cf_put_get_roundtrip 2>&1 | tail -20`
Expected: FAIL — `no method named utxo_put_direct` / `utxo_get`, and the `utxo` CF is not opened.

- [ ] **Step 3: Write the minimal implementation**

In `crates/tensorium-core/src/state.rs`, extend the storage import to include the new symbols:

```rust
    storage::{
        decode_block, decode_utxo_entry, encode_block, encode_height, encode_outpoint,
        encode_utxo_entry, CF_BLOCKS, CF_CANONICAL, CF_META, CF_UTXO, META_HEIGHT,
        META_TIP, META_UTXO_TIP,
    },
```

Add `use crate::{block::OutPoint, utxo::{UtxoEntry, UtxoError, UtxoSet}};` to the existing imports (merge into the existing `use crate::{ ... }` block; `Block`, `BlockHeader`, `Transaction`, `merkle_root` are already imported).

Register the CF in `cf_options()`:

```rust
        ColumnFamilyDescriptor::new(CF_UTXO,      Options::default()),
```

Add the `StateError::Utxo` variant:

```rust
    #[error(transparent)]
    Utxo(#[from] UtxoError),
```

Add the read/write helpers inside `impl ChainState` (next to the other private DB helpers):

```rust
    fn utxo_get(&self, outpoint: &OutPoint) -> Option<UtxoEntry> {
        let cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        self.db
            .get_cf(cf, encode_outpoint(outpoint))
            .expect("DB read")
            .map(|b| decode_utxo_entry(&b))
    }

    /// Test/helper: write a single UTXO entry directly (no batch).
    fn utxo_put_direct(&self, outpoint: &OutPoint, entry: &UtxoEntry) {
        let cf = self.db.cf_handle(CF_UTXO).expect("utxo CF");
        self.db
            .put_cf(cf, encode_outpoint(outpoint), encode_utxo_entry(entry))
            .expect("DB put");
    }

    fn read_meta_utxo_tip(&self) -> Option<Hash256> {
        let cf = self.db.cf_handle(CF_META).expect("meta CF");
        self.db
            .get_cf(cf, META_UTXO_TIP)
            .expect("meta read")
            .and_then(|b| b.as_slice().try_into().ok().map(Hash256))
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core utxo_cf_put_get_roundtrip 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Run the full core suite to confirm no regressions**

Run: `cargo test -p tensorium-core 2>&1 | grep "test result"`
Expected: all pass (existing 94 + 1 new).

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "feat(state): open CF_UTXO, add UTXO read helpers and StateError::Utxo"
```

---

## Task 3: Replay/persist helpers + ensure_utxo_synced (migration)

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tensorium-core ensure_utxo_synced_builds_set_from_canonical_chain 2>&1 | tail -20`
Expected: FAIL — `no method named ensure_utxo_synced`.

- [ ] **Step 3: Write the minimal implementation**

Add these methods inside `impl ChainState` in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tensorium-core ensure_utxo_synced_builds_set_from_canonical_chain 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "feat(state): replay/rewrite UTXO helpers and ensure_utxo_synced migration"
```

---

## Task 4: submit_block extension-path UTXO validation (the headline fix)

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-core submit_block_rejects_inflated_coinbase_extension submit_block_accepts_valid_extension_and_tracks_utxo 2>&1 | tail -25`
Expected: FAIL — the inflated-coinbase block is currently accepted (no UTXO check), and `read_meta_utxo_tip` is not updated by `submit_block`.

- [ ] **Step 3: Write the minimal implementation**

In `crates/tensorium-core/src/state.rs`, add a helper that validates a block against the persistent UTXO and a helper that emits its delta:

```rust
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
```

Now rewrite the adoption branch of `submit_block`. Replace the existing block:

```rust
        if new_work > old_work {
            let new_canonical = self.build_canonical_chain(block_hash);
            let height = new_canonical.last().map(|b| b.header.height).unwrap_or(0);
            let old_height = self.height_cache.unwrap_or(0);
            let mut batch = WriteBatch::default();
            let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
            let meta_cf      = self.db.cf_handle(CF_META).expect("meta CF");
            for b in &new_canonical {
                batch.put_cf(canonical_cf, &encode_height(b.header.height), &b.hash().0);
            }
            // A reorg can replace a taller chain with a shorter one that has
            // more cumulative work (retargeting makes this possible). The old
            // chain's canonical entries at heights beyond the new tip would
            // otherwise survive as orphaned tail entries — canonical_blocks_iter
            // has no upper bound, so it would yield [new chain][stale old tail],
            // a sequence whose transactions don't connect (apply_block fails
            // with "spends an output that does not exist"). Prune them here.
            for h in (height + 1)..=old_height {
                batch.delete_cf(canonical_cf, &encode_height(h));
            }
            batch.put_cf(meta_cf, META_TIP,    &block_hash.0);
            batch.put_cf(meta_cf, META_HEIGHT, &encode_height(height));
            self.write_batch(batch);
            self.tip_cache    = Some(block.clone());
            self.height_cache = Some(height);
        }
        Ok(block)
```

with:

```rust
        if new_work > old_work {
            let canonical_cf = self.db.cf_handle(CF_CANONICAL).expect("canonical CF");
            let meta_cf = self.db.cf_handle(CF_META).expect("meta CF");

            if parent_hash == old_tip_hash {
                // Fast path: the block extends the current canonical tip. The
                // persistent UTXO set already reflects the parent, so validate
                // against it and apply the block's delta.
                self.validate_block_utxo(params, &block)?;
                let height = block.header.height;
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
```

At the very top of `submit_block`, immediately after the `let block_hash = block.hash();` / `AlreadyKnown` check and before `validate_block`, ensure the UTXO set is in sync so the extension fast-path has a correct parent set:

```rust
        // Make sure the persistent UTXO set reflects the current tip before we
        // validate against it (rebuilds once after upgrade; cheap when synced).
        self.ensure_utxo_synced(params)?;
```

Note: `parent_hash` and `old_tip_hash` are already bound earlier in `submit_block` (`let parent_hash = block.header.previous_hash;` and `let old_tip_hash = self.tip_hash();`). Keep those bindings.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tensorium-core submit_block_rejects_inflated_coinbase_extension submit_block_accepts_valid_extension_and_tracks_utxo 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 5: Run the full core suite (watch the existing fork-choice tests)**

Run: `cargo test -p tensorium-core 2>&1 | grep -E "test result|FAILED"`
Expected: all pass. The existing `fork_choice_*` and `submit_block_prunes_stale_canonical_entries_on_reorg_to_shorter_chain` tests still pass (they mine valid coinbases, so UTXO validation accepts them).

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "feat(state): validate UTXO consequences before adopting a block

submit_block now rejects inflated-coinbase / double-spend / bad-signature
blocks before they become canonical, via the persistent UTXO set. Extension
applies the block delta; reorg rebuilds via replay."
```

---

## Task 5: Reorg + double-spend + signature coverage

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tensorium-core submit_block_rejects_double_spend_in_extension reorg_to_invalid_branch_is_rejected_and_preserves_state 2>&1 | tail -25`
Expected: With Task 4 already implemented these should PASS. If either FAILS, the implementation from Task 4 has a gap — fix `submit_block` (do not weaken the test). The point of writing them separately is to prove the double-spend and reorg-rejection paths explicitly.

Note: if `submit_block_rejects_double_spend_in_extension` passes immediately because Task 4 already covers it, that is expected (Task 4 wired the same validation path). Keep the test — it documents the double-spend guarantee distinctly from the coinbase case.

- [ ] **Step 3: Implementation**

No new production code expected — Task 4's `validate_block_utxo` (extension) and `replay_utxo` (reorg) already cover these. If a test fails, the cause is in the Task 4 code; correct it there.

- [ ] **Step 4: Run the full core suite**

Run: `cargo test -p tensorium-core 2>&1 | grep -E "test result|FAILED"`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "test(state): cover double-spend rejection and invalid-reorg preservation"
```

---

## Task 6: UTXO-equals-replay invariant + full workspace green

**Files:**
- Modify: `crates/tensorium-core/src/state.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/tensorium-core/src/state.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it passes (proves the invariant)**

Run: `cargo test -p tensorium-core incremental_utxo_matches_full_replay_after_extensions 2>&1 | tail -20`
Expected: PASS. (This test guards against drift between the extension delta and the replay. If it FAILS, the delta logic in `apply_utxo_delta_to_batch` diverges from `apply_block` phase 3 — fix the delta, not the test.)

- [ ] **Step 3: Run the entire workspace suite + fmt check on touched files**

Run: `cargo test --workspace 2>&1 | grep -E "test result|FAILED|error"`
Expected: all `test result: ok`, 0 failed. (Node/pool/wallet suites unaffected.)

Run: `cargo fmt -p tensorium-core --check 2>&1 | head -5`
Expected: no diff in the files this plan touched (`storage/mod.rs`, `state.rs`). Pre-existing unrelated diffs elsewhere are not introduced by this plan — do NOT run a global `cargo fmt`.

- [ ] **Step 4: Commit**

```bash
git add crates/tensorium-core/src/state.rs
git commit -m "test(state): pin incremental UTXO == full replay invariant"
```

---

## Task 7: Deploy to both VPS (consensus change — together)

**Files:** none (deployment).

This task is operational; run it only after Tasks 1–6 are merged and pushed to `main`. Follow the established workflow (local → git → VPS).

- [ ] **Step 1: Push to GitHub**

```bash
git push origin main
```

- [ ] **Step 2: Build + deploy on DO (157.230.44.162)**

Over SSH (password auth), with `export PATH="$HOME/.cargo/bin:$PATH"` first. Do NOT pipe `cargo build` through `tail` under `set -e`.

```bash
cd /root/tensorium-core
git pull --ff-only origin main
cargo build --release -p tensorium-node     # must print "Finished"
test -f target/release/tensorium-node || { echo BUILD FAILED; exit 1; }
TS=$(date +%Y%m%d-%H%M%S)
cp /usr/local/bin/tensorium-node /usr/local/bin/tensorium-node.bak-pre-utxo-$TS
cp target/release/tensorium-node /usr/local/bin/tensorium-node.new
mv -f /usr/local/bin/tensorium-node.new /usr/local/bin/tensorium-node
systemctl restart tensorium-mc
```

- [ ] **Step 3: Verify DO migration + health**

```bash
sleep 10
systemctl is-active tensorium-mc                 # active
journalctl -u tensorium-mc -n 30 --no-pager      # no panic, daemon up
curl -s 127.0.0.1:33332/getblockcount            # height advancing
curl -s 127.0.0.1:33332/getutxos/txm13vgxzj5ulrfhe7x0mlzxg0q6veq42tkku4g3jr | head -c 300
```

Expected: service active, `/getutxos` returns a clean set (the first `submit_block` after restart triggers the one-time UTXO build; if no block has arrived yet, trigger awareness by waiting for the next pool block, then re-check). Height keeps climbing.

- [ ] **Step 4: Build + deploy on Vultr (139.180.137.144)** — identical steps to Step 2, then the Step 3 verification.

- [ ] **Step 5: Cross-node verification**

```bash
# from local
curl -s https://mc-rpc.tensoriumlabs.com/getblockcount    # DO height
curl -s https://mc-rpc2.tensoriumlabs.com/getblockcount   # Vultr height — must match DO
```

Confirm: both nodes at the same height with identical chain, a pool payout cycle completes (`journalctl -u tensorium-pool | grep payout`), and no `UTXO apply failed` lines appear.

- [ ] **Step 6: Update memory** with the deployment result, binary md5s, and backup names.

---

## Self-Review Notes

- **Spec coverage:** storage CF + codecs (Task 1 ↔ spec §1), reuse apply_block via seeding (Task 4 `validate_block_utxo` ↔ §2), submit_block extension/reorg/not-heavier (Task 4 ↔ §3), migration on sync (Task 3 ↔ §4), error handling fail-closed (Tasks 4–5 ↔ Error Handling), all eight test cases (Tasks 1,3,4,5,6 ↔ Testing), deployment (Task 7 ↔ Deployment). The follow-on query-path switch (§5) is intentionally **out of scope** for this plan.
- **Type consistency:** `encode_outpoint`/`decode_outpoint`/`encode_utxo_entry`/`decode_utxo_entry` (storage) used identically in state.rs; `validate_block_utxo`, `apply_utxo_delta_to_batch`, `replay_utxo`, `rewrite_utxo_into_batch`, `ensure_utxo_synced`, `read_meta_utxo_tip`, `utxo_get`, `utxo_put_direct` defined in Tasks 2–4 and referenced consistently thereafter; `StateError::Utxo(#[from] UtxoError)` enables the `?` mapping in `replay_utxo`/`validate_block_utxo`.
- **No placeholders:** every code step shows complete code; commands have expected output.
