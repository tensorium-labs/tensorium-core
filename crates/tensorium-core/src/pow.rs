use crate::{block::BlockHeader, hash::Hash256};

pub fn hash_meets_work(hash: Hash256, leading_zero_bits: u8) -> bool {
    hash.leading_zero_bits() >= u32::from(leading_zero_bits)
}

pub fn header_meets_work(header: &BlockHeader) -> bool {
    hash_meets_work(header.hash(), header.leading_zero_bits)
}

pub fn mine_header(mut header: BlockHeader, max_nonce: u64) -> Option<BlockHeader> {
    for nonce in 0..=max_nonce {
        header.nonce = nonce;
        if header_meets_work(&header) {
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
}
