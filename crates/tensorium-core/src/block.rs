use serde::{Deserialize, Serialize};

use crate::hash::Hash256;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Hash256,
    pub payload: Vec<u8>,
}

impl Transaction {
    pub fn coinbase(height: u64, reward_atoms: u64, miner: &str) -> Self {
        let payload = format!("coinbase:{height}:{reward_atoms}:{miner}").into_bytes();
        Self {
            id: Hash256::double_sha256(&payload),
            payload,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub chain_id: String,
    pub height: u64,
    pub previous_hash: Hash256,
    pub merkle_root: Hash256,
    pub timestamp_seconds: u64,
    pub leading_zero_bits: u8,
    pub nonce: u64,
}

impl BlockHeader {
    pub fn hash(&self) -> Hash256 {
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(self.chain_id.as_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.previous_hash.0);
        bytes.extend_from_slice(&self.merkle_root.0);
        bytes.extend_from_slice(&self.timestamp_seconds.to_le_bytes());
        bytes.push(self.leading_zero_bits);
        bytes.extend_from_slice(&self.nonce.to_le_bytes());
        Hash256::double_sha256(&bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
        }
    }

    pub fn genesis(chain_id: &str, timestamp_seconds: u64, leading_zero_bits: u8) -> Self {
        let tx = Transaction::coinbase(0, 0, "genesis");
        let merkle_root = merkle_root(core::slice::from_ref(&tx));
        Self {
            header: BlockHeader {
                version: 1,
                chain_id: chain_id.to_owned(),
                height: 0,
                previous_hash: Hash256::ZERO,
                merkle_root,
                timestamp_seconds,
                leading_zero_bits,
                nonce: 0,
            },
            transactions: vec![tx],
        }
    }

    pub fn hash(&self) -> Hash256 {
        self.header.hash()
    }
}

pub fn merkle_root(transactions: &[Transaction]) -> Hash256 {
    if transactions.is_empty() {
        return Hash256::ZERO;
    }

    let mut layer: Vec<Hash256> = transactions.iter().map(|tx| tx.id).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity((layer.len() + 1) / 2);
        for pair in layer.chunks(2) {
            let left = pair[0];
            let right = *pair.get(1).unwrap_or(&left);
            let mut bytes = Vec::with_capacity(64);
            bytes.extend_from_slice(&left.0);
            bytes.extend_from_slice(&right.0);
            next.push(Hash256::double_sha256(&bytes));
        }
        layer = next;
    }

    layer[0]
}
