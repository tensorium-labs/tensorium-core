pub mod migration;

use crate::block::Block;

pub const CF_BLOCKS:    &str = "blocks";
pub const CF_CANONICAL: &str = "canonical";
pub const CF_META:      &str = "meta";

pub const META_TIP:      &[u8] = b"tip";
pub const META_HEIGHT:   &[u8] = b"height";
pub const META_CHAIN_ID: &[u8] = b"chain_id";

/// Encode block height as 8-byte big-endian (lexicographic == numeric order).
pub fn encode_height(h: u64) -> [u8; 8] {
    h.to_be_bytes()
}

pub fn decode_height(b: &[u8]) -> u64 {
    let arr: [u8; 8] = b.try_into().expect("height key must be 8 bytes");
    u64::from_be_bytes(arr)
}

/// Encode a Block to bytes using bincode.
pub fn encode_block(block: &Block) -> Vec<u8> {
    bincode::serialize(block).expect("Block serialization must not fail")
}

/// Decode a Block from bytes.
pub fn decode_block(bytes: &[u8]) -> Block {
    bincode::deserialize(bytes).expect("Block deserialization must not fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_roundtrip() {
        for h in [0u64, 1, 100, u64::MAX] {
            assert_eq!(decode_height(&encode_height(h)), h);
        }
    }

    #[test]
    fn height_keys_sort_numerically() {
        let keys: Vec<[u8; 8]> = (0u64..5).map(encode_height).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "big-endian keys must be in ascending order");
    }
}
