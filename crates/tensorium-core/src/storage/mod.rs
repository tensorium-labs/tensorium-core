pub mod migration;

use crate::block::Block;
use crate::block::OutPoint;
use crate::utxo::UtxoEntry;

pub const CF_BLOCKS:    &str = "blocks";
pub const CF_CANONICAL: &str = "canonical";
pub const CF_META:      &str = "meta";
pub const CF_UTXO:      &str = "utxo";

pub const META_TIP:      &[u8] = b"tip";
pub const META_HEIGHT:   &[u8] = b"height";
pub const META_CHAIN_ID: &[u8] = b"chain_id";
pub const META_UTXO_TIP: &[u8] = b"utxo_tip";

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
}
