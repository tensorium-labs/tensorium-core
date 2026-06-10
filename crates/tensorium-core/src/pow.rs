use crate::{block::BlockHeader, hash::Hash256};

pub fn hash_meets_work(hash: Hash256, leading_zero_bits: u8) -> bool {
    hash.leading_zero_bits() >= u32::from(leading_zero_bits)
}

/// Checks whether `header` satisfies its declared `leading_zero_bits` target
/// under TensorHash v1, given the dataset `epoch_seed` for its epoch.
pub fn header_meets_work(header: &BlockHeader, epoch_seed: Hash256) -> bool {
    hash_meets_work(header.pow_hash(epoch_seed), header.leading_zero_bits)
}

/// Brute-force nonce search (used by tests and the node's CPU devnet mining
/// path — TEST_PARAMS difficulty only). Production GPU mining lives in
/// `tools/tensorium-miner`.
pub fn mine_header(mut header: BlockHeader, epoch_seed: Hash256, max_nonce: u64) -> Option<BlockHeader> {
    for nonce in 0..=max_nonce {
        header.nonce = nonce;
        if header_meets_work(&header, epoch_seed) {
            return Some(header);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_zero_work_check_is_monotonic() {
        let hash = Hash256([0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(hash_meets_work(hash, 16));
        assert!(!hash_meets_work(hash, 17));
    }

    #[test]
    fn mine_header_finds_a_satisfying_nonce_at_low_difficulty() {
        let header = BlockHeader {
            version: 1,
            chain_id: "tensorium-testnet-0".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1_700_000_000,
            leading_zero_bits: 8,
            nonce: 0,
        };
        let mined = mine_header(header.clone(), Hash256::ZERO, 100_000)
            .expect("difficulty 8 should be found within 100k nonces");
        assert!(header_meets_work(&mined, Hash256::ZERO));
    }
}
