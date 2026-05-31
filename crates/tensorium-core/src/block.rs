use serde::{Deserialize, Serialize};

use crate::hash::Hash256;

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
    pub address: String,
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
                address: miner.to_owned(),
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
        bytes.extend_from_slice(output.address.as_bytes());
    }
    bytes.extend_from_slice(payload);
    Hash256::double_sha256(&bytes)
}
