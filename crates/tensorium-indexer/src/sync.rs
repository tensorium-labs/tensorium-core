use crate::index::Indexer;
use tensorium_core::block::Block;
use tensorium_core::hash::Hash256;

/// Decide whether the indexer must rebuild from genesis. A rebuild is needed when
/// the indexer has scanned before AND the block hash now reported at its
/// `last_height` differs from the one it recorded (a reorg below the tip).
pub fn needs_rebuild(idx: &Indexer, hash_at_last_height: &str) -> bool {
    idx.scanned_any && idx.last_height > 0 && idx.last_hash != hash_at_last_height
}

/// Take a buried checkpoint every this many blocks (so a shallow reorg rolls
/// back to it instead of rescanning from genesis).
pub const CHECKPOINT_INTERVAL: u64 = 20;
/// Keep the checkpoint at least this many blocks behind the tip, so it is buried
/// below the depth of typical reorgs.
pub const CHECKPOINT_SAFETY: u64 = 5;

/// Drive a scan up to `tip`, fetching blocks via `fetch(height) -> (hash, block)`.
/// On reorg (the block hash at `last_height` changed) the indexer rolls back to
/// the most recent still-canonical checkpoint and re-applies forward; only if no
/// usable checkpoint exists (a reorg deeper than the checkpoint, or none taken
/// yet) does it rebuild from genesis. Returns the number of blocks applied.
pub fn scan<F>(idx: &mut Indexer, tip: u64, mut fetch: F) -> Result<u64, String>
where
    F: FnMut(u64) -> Result<(Hash256, Block), String>,
{
    // Reorg check at the last scanned height.
    if idx.scanned_any && idx.last_height > 0 {
        let (hash, _) = fetch(idx.last_height)?;
        if needs_rebuild(idx, &hash.to_hex()) && !try_rollback(idx, &mut fetch)? {
            *idx = Indexer::default(); // no usable checkpoint → full rebuild
        }
    }

    let start = if idx.scanned_any { idx.last_height + 1 } else { 0 };
    let mut applied = 0;
    for height in start..=tip {
        let (hash, block) = fetch(height)?;
        idx.apply_block(&block, height);
        idx.last_hash = hash.to_hex();
        applied += 1;
        // Refresh the buried checkpoint as we move forward.
        if height % CHECKPOINT_INTERVAL == 0 && height + CHECKPOINT_SAFETY <= tip {
            let snap = idx.snapshot();
            idx.checkpoint = Some((height, idx.last_hash.clone(), snap));
        }
    }
    Ok(applied)
}

/// Roll the indexer back to its checkpoint if that checkpoint is still on the
/// canonical chain. Returns true on a successful rollback, false if there is no
/// checkpoint or it was itself orphaned (caller then rebuilds from genesis).
fn try_rollback<F>(idx: &mut Indexer, fetch: &mut F) -> Result<bool, String>
where
    F: FnMut(u64) -> Result<(Hash256, Block), String>,
{
    let Some((cp_height, cp_hash, snap)) = idx.checkpoint.clone() else {
        return Ok(false);
    };
    let (canon, _) = fetch(cp_height)?;
    if canon.to_hex() == cp_hash {
        idx.restore(snap);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tensorium_core::block::{Block, BlockHeader, Transaction};

    fn block(height: u64, prev: u8) -> (Hash256, Block) {
        let header = BlockHeader {
            version: 1,
            chain_id: "test".into(),
            height,
            previous_hash: Hash256([prev; 32]),
            merkle_root: Hash256([0u8; 32]),
            timestamp_seconds: 0,
            leading_zero_bits: 0,
            nonce: 0,
        };
        // Deterministic synthetic hash per (height, prev).
        let hash = Hash256([height as u8 ^ prev; 32]);
        (hash, Block::new(header, vec![Transaction::coinbase(height, 1, "txm1m")]))
    }

    #[test]
    fn scans_forward_then_incrementally() {
        let chain: HashMap<u64, (Hash256, Block)> =
            (0..=3).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();

        let applied = scan(&mut idx, 2, |h| Ok(chain[&h].clone())).unwrap();
        assert_eq!(applied, 3); // heights 0,1,2
        assert_eq!(idx.last_height, 2);

        // Tip advances to 3: only the new block is applied.
        let applied = scan(&mut idx, 3, |h| Ok(chain[&h].clone())).unwrap();
        assert_eq!(applied, 1);
        assert_eq!(idx.last_height, 3);
    }

    #[test]
    fn reorg_below_tip_triggers_full_rebuild() {
        // First scan on chain A.
        let chain_a: HashMap<u64, (Hash256, Block)> =
            (0..=2).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();
        scan(&mut idx, 2, |h| Ok(chain_a[&h].clone())).unwrap();
        assert_eq!(idx.last_height, 2);

        // Chain B replaces history (different prev byte → different hashes).
        let chain_b: HashMap<u64, (Hash256, Block)> =
            (0..=2).map(|h| (h, block(h, 9))).collect();
        let applied = scan(&mut idx, 2, |h| Ok(chain_b[&h].clone())).unwrap();
        assert_eq!(applied, 3); // rebuilt from 0 (too short for a checkpoint)
        assert_eq!(idx.last_hash, chain_b[&2].0.to_hex());
    }

    #[test]
    fn shallow_reorg_rolls_back_to_checkpoint_not_genesis() {
        // Chain A: heights 0..=60, all prev=1.
        let chain_a: HashMap<u64, (Hash256, Block)> =
            (0..=60).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();
        scan(&mut idx, 60, |h| Ok(chain_a[&h].clone())).unwrap();
        assert_eq!(idx.last_height, 60);
        // A buried checkpoint exists (latest multiple of 20 with +5 <= 60 → 40).
        let (cp_h, _, _) = idx.checkpoint.clone().expect("checkpoint taken");
        assert_eq!(cp_h, 40);

        // Chain B forks only at height 58 (0..=57 identical hashes, 58..=60 differ).
        let chain_b: HashMap<u64, (Hash256, Block)> = (0..=60)
            .map(|h| (h, block(h, if h >= 58 { 9 } else { 1 })))
            .collect();
        let mut fetches = 0u64;
        let applied = scan(&mut idx, 60, |h| {
            fetches += 1;
            Ok(chain_b[&h].clone())
        })
        .unwrap();
        // Rolled back to checkpoint 40 and re-applied 41..=60 = 20 blocks (NOT 61).
        assert_eq!(applied, 20);
        assert!(fetches < 30, "should not rescan from genesis (fetched {fetches})");
        assert_eq!(idx.last_height, 60);
        assert_eq!(idx.last_hash, chain_b[&60].0.to_hex());
    }

    #[test]
    fn reorg_deeper_than_checkpoint_falls_back_to_full_rebuild() {
        let chain_a: HashMap<u64, (Hash256, Block)> =
            (0..=60).map(|h| (h, block(h, 1))).collect();
        let mut idx = Indexer::default();
        scan(&mut idx, 60, |h| Ok(chain_a[&h].clone())).unwrap();
        assert_eq!(idx.checkpoint.clone().unwrap().0, 40);

        // Fork at height 30 — below the checkpoint at 40, so the checkpoint is
        // itself orphaned → full rebuild from genesis.
        let chain_b: HashMap<u64, (Hash256, Block)> = (0..=60)
            .map(|h| (h, block(h, if h >= 30 { 9 } else { 1 })))
            .collect();
        let applied = scan(&mut idx, 60, |h| Ok(chain_b[&h].clone())).unwrap();
        assert_eq!(applied, 61); // rebuilt from 0
        assert_eq!(idx.last_hash, chain_b[&60].0.to_hex());
    }
}
