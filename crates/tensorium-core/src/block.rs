use serde::{Deserialize, Serialize};

use crate::hash::Hash256;
use crate::script::standard::p2pkh_from_address;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct OutPoint {
    pub txid: Hash256,
    pub output_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxInput {
    pub previous_output: OutPoint,
    pub signature_script: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxOutput {
    pub value_atoms: u64,
    pub script_pubkey: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Hash256,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub payload: Vec<u8>,
}

impl Transaction {
    pub fn coinbase(height: u64, reward_atoms: u64, miner: &str) -> Self {
        let payload = format!("coinbase:{height}:{reward_atoms}:{miner}").into_bytes();
        let outputs = if reward_atoms == 0 {
            Vec::new()
        } else {
            vec![TxOutput {
                value_atoms: reward_atoms,
                script_pubkey: p2pkh_from_address(miner)
                    .unwrap_or_default(),
            }]
        };
        let id = transaction_id(&[], &outputs, &payload);
        Self {
            id,
            inputs: Vec::new(),
            outputs,
            payload,
        }
    }

    /// Genesis-only coinbase: mining reward to `miner` PLUS founder allocation
    /// to `founder_addr`. Only used at height 0 when founder_address is set.
    /// The payload is identical to a normal coinbase so `is_coinbase()` returns true.
    pub fn genesis_coinbase(
        reward_atoms: u64,
        miner: &str,
        founder_atoms: u64,
        founder_addr: &str,
        genesis_allocations: &[(&str, u64)],
    ) -> Self {
        let payload = format!("coinbase:0:{reward_atoms}:{miner}").into_bytes();
        let mut outputs = Vec::new();
        if reward_atoms > 0 {
            outputs.push(TxOutput {
                value_atoms: reward_atoms,
                script_pubkey: p2pkh_from_address(miner).unwrap_or_default(),
            });
        }
        // If genesis_allocations is provided, use it for pre-mint distribution.
        // Otherwise fall back to legacy single-founder output.
        if !genesis_allocations.is_empty() {
            for (addr, atoms) in genesis_allocations {
                if *atoms > 0 {
                    outputs.push(TxOutput {
                        value_atoms: *atoms,
                        script_pubkey: p2pkh_from_address(addr).unwrap_or_default(),
                    });
                }
            }
        } else if founder_atoms > 0 && !founder_addr.is_empty() {
            outputs.push(TxOutput {
                value_atoms: founder_atoms,
                script_pubkey: p2pkh_from_address(founder_addr).unwrap_or_default(),
            });
        }
        let id = transaction_id(&[], &outputs, &payload);
        Self {
            id,
            inputs: Vec::new(),
            outputs,
            payload,
        }
    }

    pub fn is_coinbase(&self) -> bool {
        self.inputs.is_empty() && self.payload.starts_with(b"coinbase:")
    }

    pub fn total_output_atoms(&self) -> u64 {
        self.outputs
            .iter()
            .fold(0u64, |sum, output| sum.saturating_add(output.value_atoms))
    }

    pub fn payment(inputs: Vec<TxInput>, outputs: Vec<TxOutput>) -> Self {
        let payload = b"payment:v1".to_vec();
        let id = transaction_id(&inputs, &outputs, &payload);
        Self {
            id,
            inputs,
            outputs,
            payload,
        }
    }

    pub fn refresh_id(&mut self) {
        self.id = transaction_id(&self.inputs, &self.outputs, &self.payload);
    }

    pub fn signature_hash(&self) -> Hash256 {
        let unsigned_inputs: Vec<TxInput> = self
            .inputs
            .iter()
            .map(|input| TxInput {
                previous_output: input.previous_output,
                signature_script: Vec::new(),
            })
            .collect();
        transaction_id(&unsigned_inputs, &self.outputs, &self.payload)
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

    /// Serialized header bytes excluding `nonce` — the nonce-independent
    /// prefix fed into TensorHash's `pow_hash`. Mirrors the field order of
    /// `hash()` minus the trailing nonce bytes.
    fn pow_prefix_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(120);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(self.chain_id.as_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.previous_hash.0);
        bytes.extend_from_slice(&self.merkle_root.0);
        bytes.extend_from_slice(&self.timestamp_seconds.to_le_bytes());
        bytes.push(self.leading_zero_bits);
        bytes
    }

    /// TensorHash v1 proof-of-work hash for this header, given the dataset
    /// epoch seed for the epoch containing `self.height`. Used only by
    /// `pow::header_meets_work` — chain linkage, merkle roots, and storage
    /// keys continue to use `hash()` (double-SHA256), unaffected by this.
    pub fn pow_hash(&self, epoch_seed: Hash256) -> Hash256 {
        let prefix = self.pow_prefix_bytes();
        Hash256(tensorium_tensorhash::pow_hash(&prefix, self.nonce, &epoch_seed.0))
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

fn transaction_id(inputs: &[TxInput], outputs: &[TxOutput], payload: &[u8]) -> Hash256 {
    let mut bytes = Vec::new();
    for input in inputs {
        bytes.extend_from_slice(&input.previous_output.txid.0);
        bytes.extend_from_slice(&input.previous_output.output_index.to_le_bytes());
        bytes.extend_from_slice(&input.signature_script);
    }
    for output in outputs {
        bytes.extend_from_slice(&output.value_atoms.to_le_bytes());
        bytes.extend_from_slice(&output.script_pubkey);
    }
    bytes.extend_from_slice(payload);
    Hash256::double_sha256(&bytes)
}

#[cfg(test)]
mod pow_hash_tests {
    use super::*;

    #[test]
    fn pow_hash_differs_from_id_hash() {
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
        assert_ne!(header.hash(), header.pow_hash(Hash256::ZERO));
    }

    #[test]
    fn pow_hash_changes_with_nonce() {
        let mut header = BlockHeader {
            version: 1,
            chain_id: "tensorium-testnet-0".to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            timestamp_seconds: 1_700_000_000,
            leading_zero_bits: 8,
            nonce: 0,
        };
        let h0 = header.pow_hash(Hash256::ZERO);
        header.nonce = 1;
        let h1 = header.pow_hash(Hash256::ZERO);
        assert_ne!(h0, h1);
    }
}
