use crate::index::Indexer;
use tensorium_core::block::Block;
use tensorium_core::hash::Hash256;

/// Decide whether the indexer must rebuild from genesis. A rebuild is needed when
/// the indexer has scanned before AND the block hash now reported at its
/// `last_height` differs from the one it recorded (a reorg below the tip).
pub fn needs_rebuild(idx: &Indexer, hash_at_last_height: &str) -> bool {
    idx.scanned_any && idx.last_height > 0 && idx.last_hash != hash_at_last_height
}

/// Drive a scan up to `tip`, fetching blocks via `fetch(height) -> (hash, block)`.
/// On reorg (detected at `last_height`) the indexer is reset and rebuilt from 0.
/// Returns the number of blocks applied this call.
pub fn scan<F>(idx: &mut Indexer, tip: u64, mut fetch: F) -> Result<u64, String>
where
    F: FnMut(u64) -> Result<(Hash256, Block), String>,
{
    // Reorg check at the last scanned height.
    if idx.scanned_any && idx.last_height > 0 {
        let (hash, _) = fetch(idx.last_height)?;
        if needs_rebuild(idx, &hash.to_hex()) {
            *idx = Indexer::default();
        }
    }

    let start = if idx.scanned_any { idx.last_height + 1 } else { 0 };
    let mut applied = 0;
    for height in start..=tip {
        let (hash, block) = fetch(height)?;
        idx.apply_block(&block, height);
        idx.last_hash = hash.to_hex();
        applied += 1;
    }
    Ok(applied)
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
        assert_eq!(applied, 3); // rebuilt from 0
        assert_eq!(idx.last_hash, chain_b[&2].0.to_hex());
    }
}
