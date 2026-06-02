# RocksDB Storage Migration — Design Spec

**Date:** 2026-06-02
**Status:** Approved
**Scope:** `crates/tensorium-core` + `crates/tensorium-node`

---

## Problem

`ChainState` is serialized as a single JSON file (`state.json`). At 34k+ blocks the file is 215MB and growing linearly. Every new block triggers a full 215MB write. Every node startup reads and parses 215MB. `getblock/N` in the explorer context re-reads the file on each call.

---

## Approach: Drop-in RocksDB Backend (Option A)

Keep every public method signature on `ChainState` identical. Swap the storage layer from `serde_json` + flat file to RocksDB. Three direct field accesses (`state.blocks`) in `main.rs` change to method calls — that is the complete diff to all callers.

---

## RocksDB Schema

### Column Families

**`blocks`**
- Key: block hash (32 bytes, raw)
- Value: `bincode`-encoded `Block`
- Purpose: O(1) lookup by hash — replaces `block_map: HashMap`

**`canonical`**
- Key: block height (8 bytes, big-endian `u64`)
- Value: block hash (32 bytes, raw)
- Purpose: O(1) lookup by height; big-endian ensures RocksDB lexicographic order == numeric order; enables ordered iteration

**`meta`**
- Key `"tip"` → hash of canonical tip (32 bytes)
- Key `"height"` → canonical chain height (8 bytes, `u64` big-endian)
- Key `"chain_id"` → chain ID string (UTF-8)
- Purpose: fast startup reads without scanning CFs

### Encoding

- Blocks: `bincode` — compact binary, ~10–20× smaller than JSON per block
- Keys: raw bytes, no overhead
- Compression: none initially; add Snappy per-CF if disk usage warrants it

---

## Struct Changes

```rust
// BEFORE (in-memory, grows with chain height)
pub struct ChainState {
    pub blocks: Vec<Block>,
    pub block_map: HashMap<String, Block>,
}

// AFTER (O(1) memory regardless of chain height)
pub struct ChainState {
    db: Arc<rocksdb::DB>,
    tip_cache: Option<Block>,   // latest canonical block only
    height_cache: Option<u64>,  // avoids meta read on height()
}
```

### New methods added (all others unchanged)

```rust
pub fn open_db(path: &Path) -> Result<Self, String>   // replaces load_state()
pub fn block_count(&self) -> u64                       // replaces state.blocks.len()
pub fn canonical_blocks_iter(&self) -> impl Iterator<Item = Block> + '_
    // replaces: for block in &state.blocks { ... }
    // streams genesis-first from DB, lazy — no full load into RAM
```

### `ChainState::new()` behavior

`new()` is kept for test ergonomics. In the RocksDB implementation it opens an
in-memory RocksDB instance (using a `tempdir` that is deleted on drop). This
means all existing tests that call `ChainState::new()` continue to compile and
pass without modification.

In production the node uses `ChainState::open_db(path)` — `new()` is never
called in `main.rs`.

### Methods removed from public API

None. All existing method signatures retained.

### Changes to `main.rs` (3 sites, trivial)

```rust
// build_utxo_set: for block in &state.blocks {
for block in state.canonical_blocks_iter() {

// print_status: state.blocks.len()
state.block_count()

// handle_rpc_getblockcount JSON: "blocks": state.blocks.len()
"blocks": state.block_count()
```

`load_state()` and `save_state()` private functions in `main.rs` are replaced:
- `load_state(path)` → `ChainState::open_db(path)`
- `save_state(path, &state)` → removed; RocksDB writes are immediate in `submit_block`

---

## Write Path

`submit_block` uses `rocksdb::WriteBatch` — atomically writes:
1. `CF:blocks` — put `<hash>` → `bincode(block)`
2. `CF:canonical` — put `<height_be>` → `<hash>` (only if new canonical tip)
3. `CF:meta` — put `"tip"` → `<hash>`, put `"height"` → `<height_be>`

One ~1KB atomic write per block replaces one ~215MB file write.

---

## Migration

### Path convention

| Chain | Old JSON | New RocksDB dir |
|---|---|---|
| testnet | `tensorium-testnet-state.json` | `tensorium-testnet-state.db/` |
| MC | `tensorium-mc-state.json` | `tensorium-mc-state.db/` |
| `$TXM_STATE` | `<value>` | `<value minus .json>.db/` |

### Logic in `ChainState::open_db(path)`

1. If `<path>.db/` exists → open RocksDB (normal operation, fast)
2. Else if `<path>.json` exists → migrate:
   a. Parse JSON into old `ChainState` struct (one-time, ~5–10s for 34k blocks)
   b. Batch-write all blocks to RocksDB
   c. Rename `state.json` → `state.json.migrated` (backup, do not delete)
   d. Return open DB
3. Else → create empty RocksDB (fresh node)

### Rollback

A `--json-state` CLI flag forces the legacy JSON path for one release. Operator
rebuilds or passes the flag if RocksDB migration causes issues. Remove the flag
in the release after.

---

## File Layout

```
crates/tensorium-core/src/
  lib.rs                  — re-export ChainState (unchanged)
  state.rs                — ChainState public API impl (DB-backed, same signatures)
  storage/
    mod.rs                — internal StorageBackend trait
    rocksdb.rs            — RocksDB column-family helpers, encode/decode
    migration.rs          — JSON → RocksDB one-time migration
  block.rs                — unchanged
  utxo.rs                 — unchanged
  (all other files)       — unchanged
```

---

## Dependencies

```toml
# crates/tensorium-core/Cargo.toml
[dependencies]
rocksdb  = { version = "0.22", features = ["snappy"] }
bincode  = "1.3"
```

`bincode` is already in the Rust ecosystem; `rocksdb` wraps the C++ RocksDB library via a well-maintained crate. No other new dependencies.

---

## Test Plan

All tests live in `crates/tensorium-core/src/storage/tests.rs` plus existing `state.rs` tests.

**New tests:**

| Test | What it verifies |
|---|---|
| `empty_db_open` | Fresh DB: `height()` = None, `tip()` = None |
| `genesis_persists` | `init_genesis_nonce` → close → reopen → `tip()` correct |
| `submit_block_persists` | Write N blocks → close → reopen → `height()` = N |
| `get_block_by_height` | `canonical_blocks_iter()` returns genesis-first order |
| `fork_choice_reorg_rocksdb` | Reorg updates canonical CF + meta correctly |
| `migration_roundtrip` | Load JSON fixture → migrate → all blocks + hashes match |
| `already_known_error` | `submit_block` duplicate → `StateError::AlreadyKnown` |
| `unknown_parent_error` | Orphan block → `StateError::UnknownParent` |

**Existing tests (must all still pass):**
All 37 tests in `cargo test --workspace` — storage is transparent to them.
Existing `state.rs` unit tests use `ChainState` via `open_db` pointing to a
`tempdir` created in the test setup.

---

## Performance Expectations

| Metric | Before | After |
|---|---|---|
| Write per block | ~215MB (full file) | ~1KB (3 RocksDB keys) |
| Startup load | ~3–5s (JSON parse 215MB) | ~10ms (read 2 meta keys) |
| `getblock/N` | ~2–4s (full file read) | ~1ms (2 DB lookups) |
| Memory at 34k blocks | ~500MB heap | ~5MB (tip cache only) |
| Memory at 1M blocks | ~15GB (unusable) | ~5MB (unchanged) |

---

## What is NOT in scope

- Mempool storage (stays JSON, small)
- Banlist storage (stays JSON, small)
- Pool ledger (separate process)
- UTXO set storage (separate future work)
- RocksDB compaction tuning (default settings acceptable for Phase 10)
