use std::path::Path;

use crate::block::Block;
use crate::state::ChainState;
use crate::storage::{encode_block, encode_height, CF_BLOCKS, CF_CANONICAL, CF_META, META_HEIGHT, META_TIP};

/// One-time migration: read blocks from a legacy JSON state file and write
/// them into a new RocksDB at `db_path`.
/// Only the canonical `blocks` array is migrated; fork branches are dropped.
pub fn migrate_json_to_rocksdb(json_path: &Path, db_path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(json_path)
        .map_err(|e| format!("cannot read {}: {e}", json_path.display()))?;

    #[derive(serde::Deserialize)]
    struct OldState {
        blocks: Vec<Block>,
    }

    let old: OldState = serde_json::from_str(&raw)
        .map_err(|e| format!("JSON parse error: {e}"))?;

    if old.blocks.is_empty() {
        return Err("source JSON has no blocks".into());
    }

    let mut state = ChainState::open_db(db_path)?;

    {
        use rocksdb::WriteBatch;
        let mut batch = WriteBatch::default();
        for block in &old.blocks {
            let hash         = block.hash();
            let blocks_cf    = state.db_handle().cf_handle(CF_BLOCKS).expect("blocks CF");
            let canonical_cf = state.db_handle().cf_handle(CF_CANONICAL).expect("canonical CF");
            batch.put_cf(blocks_cf,    &hash.0,                             encode_block(block));
            batch.put_cf(canonical_cf, &encode_height(block.header.height), &hash.0);
        }
        let tip      = old.blocks.last().unwrap();
        let tip_hash = tip.hash();
        let meta_cf  = state.db_handle().cf_handle(CF_META).expect("meta CF");
        batch.put_cf(meta_cf, META_TIP,    &tip_hash.0);
        batch.put_cf(meta_cf, META_HEIGHT, &encode_height(tip.header.height));
        state.db_handle().write(batch).map_err(|e| format!("RocksDB write: {e}"))?;
    }

    state.reload_caches_pub();
    println!(
        "[migration] {} blocks migrated, tip height={}",
        old.blocks.len(),
        state.height().unwrap_or(0)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::TEST_PARAMS;
    use crate::state::ChainState;

    #[test]
    fn migration_roundtrip() {
        let dir       = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("test-state.json");

        // Build a 2-block chain and collect canonical blocks.
        let original_blocks: Vec<Block> = {
            let mut state = ChainState::new();
            state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
            state.mine_next_block(&TEST_PARAMS, 1_700_000_060, "miner", 1_000_000).unwrap();
            state.canonical_blocks_iter().collect()
        };

        // Serialize to the old JSON format: { "blocks": [...], "block_map": {} }
        let blocks_json: Vec<serde_json::Value> = original_blocks
            .iter()
            .map(|b| serde_json::to_value(b).unwrap())
            .collect();
        let fixture = serde_json::json!({ "blocks": blocks_json, "block_map": {} });
        std::fs::write(&json_path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();

        // Migrate.
        let db_path = dir.path().join("test-state.db");
        migrate_json_to_rocksdb(&json_path, &db_path).unwrap();

        // Open migrated DB and verify.
        let state    = ChainState::open_db(&db_path).unwrap();
        let migrated: Vec<Block> = state.canonical_blocks_iter().collect();
        assert_eq!(state.height(), Some(1));
        assert_eq!(migrated.len(), original_blocks.len());
        for (a, b) in original_blocks.iter().zip(migrated.iter()) {
            assert_eq!(a.hash(), b.hash(), "block hashes must match after migration");
        }
    }
}
