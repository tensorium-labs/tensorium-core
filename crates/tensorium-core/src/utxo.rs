use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    block::{Block, OutPoint, Transaction, TxOutput},
    chain::ConsensusParams,
    emission::reward_at_height,
    script::vm::{execute, ScriptContext},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub output: TxOutput,
    pub created_height: u64,
    pub coinbase: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UtxoSet {
    pub entries: HashMap<OutPoint, UtxoEntry>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum UtxoError {
    #[error("transaction spends an output that does not exist")]
    MissingInput,
    #[error("transaction spends the same output more than once")]
    DuplicateInput,
    #[error("coinbase transaction is missing")]
    MissingCoinbase,
    #[error("coinbase output exceeds block reward plus transaction fees")]
    CoinbaseOutputTooHigh,
    #[error("transaction signature is invalid")]
    InvalidSignature,
    #[error("coinbase output is not mature enough to spend")]
    ImmatureCoinbaseSpend,
    #[error("transaction outputs exceed inputs (would inflate supply)")]
    OutputExceedsInput,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn total_atoms(&self) -> u64 {
        self.entries.values().fold(0u64, |sum, entry| {
            sum.saturating_add(entry.output.value_atoms)
        })
    }

    /// Validate that `tx` can be spent given the current UTXO set.
    /// Returns the implicit fee (sum_inputs − sum_outputs) on success.
    /// Does not mutate the set — safe to call for mempool acceptance checks.
    pub fn validate_transaction(
        &self,
        tx: &Transaction,
        tip_height: u64,
        params: &ConsensusParams,
    ) -> Result<u64, UtxoError> {
        let mut seen: HashSet<OutPoint> = HashSet::new();
        let mut input_sum = 0u64;
        for input in &tx.inputs {
            if !seen.insert(input.previous_output) {
                return Err(UtxoError::DuplicateInput);
            }
            let entry = self
                .entries
                .get(&input.previous_output)
                .ok_or(UtxoError::MissingInput)?;
            if entry.coinbase
                && tip_height
                    < entry
                        .created_height
                        .saturating_add(params.coinbase_maturity_blocks)
            {
                return Err(UtxoError::ImmatureCoinbaseSpend);
            }
            let ctx = ScriptContext {
                sig_hash: tx.signature_hash(),
                block_height: tip_height,
            };
            let ok = execute(&input.signature_script, &entry.output.script_pubkey, &ctx)
                .map_err(|_| UtxoError::InvalidSignature)?;
            if !ok {
                return Err(UtxoError::InvalidSignature);
            }
            input_sum = input_sum.saturating_add(entry.output.value_atoms);
        }
        let output_sum = tx
            .outputs
            .iter()
            .fold(0u64, |s, o| s.saturating_add(o.value_atoms));
        if output_sum > input_sum {
            return Err(UtxoError::OutputExceedsInput);
        }
        Ok(input_sum.saturating_sub(output_sum))
    }

    pub fn apply_block(
        &mut self,
        params: &ConsensusParams,
        block: &Block,
    ) -> Result<(), UtxoError> {
        let coinbase = block
            .transactions
            .first()
            .ok_or(UtxoError::MissingCoinbase)?;

        // ── Phase 1: read-only — validate non-coinbase txs and sum fees ──────
        // `seen` spans the whole block (not just one transaction) so that two
        // different transactions in the same block cannot both claim the same
        // outpoint — entries aren't removed from `self.entries` until Phase 3,
        // so a per-transaction set alone would miss this intra-block double-spend.
        let mut seen: HashSet<OutPoint> = HashSet::new();
        let mut total_fees = 0u64;
        for tx in block.transactions.iter().skip(1) {
            let mut input_sum = 0u64;
            for input in &tx.inputs {
                if !seen.insert(input.previous_output) {
                    return Err(UtxoError::DuplicateInput);
                }
                let entry = self
                    .entries
                    .get(&input.previous_output)
                    .ok_or(UtxoError::MissingInput)?;
                if entry.coinbase
                    && block.header.height
                        < entry
                            .created_height
                            .saturating_add(params.coinbase_maturity_blocks)
                {
                    return Err(UtxoError::ImmatureCoinbaseSpend);
                }
                let ctx = ScriptContext {
                    sig_hash: tx.signature_hash(),
                    block_height: block.header.height,
                };
                let ok = execute(&input.signature_script, &entry.output.script_pubkey, &ctx)
                    .map_err(|_| UtxoError::InvalidSignature)?;
                if !ok {
                    return Err(UtxoError::InvalidSignature);
                }
                input_sum = input_sum.saturating_add(entry.output.value_atoms);
            }
            let output_sum = tx
                .outputs
                .iter()
                .fold(0u64, |s, o| s.saturating_add(o.value_atoms));
            if output_sum > input_sum {
                return Err(UtxoError::OutputExceedsInput);
            }
            total_fees = total_fees.saturating_add(input_sum.saturating_sub(output_sum));
        }

        // ── Phase 2: coinbase limit = block reward + all tx fees ─────────────
        let has_genesis_premint = block.header.height == 0
            && (!params.genesis_allocations.is_empty() || !params.founder_address.is_empty());
        let premint_atoms: u64 = if !params.genesis_allocations.is_empty() {
            params.genesis_allocations.iter().map(|(_, a)| a).sum()
        } else {
            params.founder_allocation_atoms
        };
        let max_coinbase_atoms = if has_genesis_premint {
            reward_at_height(params, 0)
                .saturating_add(premint_atoms)
                .saturating_add(total_fees)
        } else {
            reward_at_height(params, block.header.height).saturating_add(total_fees)
        };
        if coinbase.total_output_atoms() > max_coinbase_atoms {
            return Err(UtxoError::CoinbaseOutputTooHigh);
        }

        // ── Phase 3: apply changes (spend inputs, create outputs) ─────────────
        for tx in block.transactions.iter().skip(1) {
            for input in &tx.inputs {
                self.entries.remove(&input.previous_output);
            }
        }
        for tx in &block.transactions {
            for (index, output) in tx.outputs.iter().enumerate() {
                // OP_RETURN outputs are unspendable — never add to UTXO set
                if output.script_pubkey.first() == Some(&crate::script::OP_RETURN) {
                    continue;
                }
                self.entries.insert(
                    OutPoint {
                        txid: tx.id,
                        output_index: index as u32,
                    },
                    UtxoEntry {
                        output: output.clone(),
                        created_height: block.header.height,
                        coinbase: tx.is_coinbase(),
                    },
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        block::{merkle_root, BlockHeader, Transaction, TxInput, TxOutput},
        chain::TEST_PARAMS,
        hash::Hash256,
        script::standard::p2pkh_from_address,
        state::ChainState,
        wallet::WalletKeypair,
    };

    use super::*;

    #[test]
    fn tracks_coinbase_outputs() {
        let mut state = ChainState::new();
        let mut utxos = UtxoSet::new();

        let genesis = state
            .init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000)
            .unwrap();
        utxos.apply_block(&TEST_PARAMS, genesis).unwrap();
        assert_eq!(utxos.total_atoms(), reward_at_height(&TEST_PARAMS, 0));

        let next = state
            .mine_next_block(&TEST_PARAMS, 1_700_000_060, "test-miner", 1_000_000)
            .unwrap();
        utxos.apply_block(&TEST_PARAMS, next).unwrap();
        assert_eq!(
            utxos.total_atoms(),
            reward_at_height(&TEST_PARAMS, 0) + reward_at_height(&TEST_PARAMS, 1)
        );
    }

    #[test]
    fn rejects_immature_coinbase_spend() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();
        let coinbase = Transaction::coinbase(1, 100, keypair.address.as_str());
        let coinbase_block = test_block(1, vec![coinbase.clone()]);
        utxos.apply_block(&TEST_PARAMS, &coinbase_block).unwrap();

        let mut spend = Transaction::payment(
            vec![TxInput {
                previous_output: crate::block::OutPoint {
                    txid: coinbase.id,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 100,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut spend).unwrap();
        let block = test_block(2, vec![Transaction::coinbase(2, 100, "miner"), spend]);

        assert_eq!(
            utxos.apply_block(&TEST_PARAMS, &block),
            Err(UtxoError::ImmatureCoinbaseSpend)
        );
    }

    #[test]
    fn rejects_output_exceeds_input() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();
        let coinbase = Transaction::coinbase(1, 1_000, keypair.address.as_str());
        let cb_block = test_block(1, vec![coinbase.clone()]);
        utxos.apply_block(&TEST_PARAMS, &cb_block).unwrap();

        // Try to spend 1000 atoms but output 2000 (inflation attempt)
        let mut inflate = Transaction::payment(
            vec![TxInput {
                previous_output: crate::block::OutPoint {
                    txid: coinbase.id,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 2_000,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut inflate).unwrap();

        // validate_transaction should reject it
        assert_eq!(
            utxos.validate_transaction(&inflate, 1_000, &TEST_PARAMS),
            Err(UtxoError::OutputExceedsInput)
        );
    }

    #[test]
    fn validate_transaction_returns_fee() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();
        let coinbase = Transaction::coinbase(1, 1_000, keypair.address.as_str());
        let cb_block = test_block(1, vec![coinbase.clone()]);
        utxos.apply_block(&TEST_PARAMS, &cb_block).unwrap();

        // Send 900, keep 100 as fee
        let mut tx = Transaction::payment(
            vec![TxInput {
                previous_output: crate::block::OutPoint {
                    txid: coinbase.id,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 900,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut tx).unwrap();

        let fee = utxos.validate_transaction(&tx, 1_000, &TEST_PARAMS).unwrap();
        assert_eq!(fee, 100);
    }

    #[test]
    fn coinbase_can_claim_tx_fees() {
        let keypair = WalletKeypair::generate();
        let mut utxos = UtxoSet::new();

        // Seed the UTXO set directly with a mature non-coinbase entry so we
        // avoid the coinbase maturity window (10 blocks) in this unit test.
        let fake_outpoint = crate::block::OutPoint {
            txid: crate::hash::Hash256([0xab; 32]),
            output_index: 0,
        };
        utxos.entries.insert(
            fake_outpoint,
            UtxoEntry {
                output: TxOutput {
                    value_atoms: 1_000,
                    script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
                },
                created_height: 0,
                coinbase: false,
            },
        );

        // Payment tx: input=1000, output=900 → fee=100
        let mut pay_tx = Transaction::payment(
            vec![TxInput {
                previous_output: fake_outpoint,
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 900,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut pay_tx).unwrap();

        // Miner coinbase claims block reward + 100 fee
        let block_reward = reward_at_height(&TEST_PARAMS, 2);
        let coinbase2 = Transaction::coinbase(2, block_reward + 100, "miner");

        // Must succeed: coinbase = reward + fee is allowed
        let block = test_block(2, vec![coinbase2, pay_tx]);
        utxos.apply_block(&TEST_PARAMS, &block).unwrap();
    }

    fn test_block(height: u64, transactions: Vec<Transaction>) -> Block {
        let merkle_root = merkle_root(&transactions);
        Block::new(
            BlockHeader {
                version: 1,
                chain_id: TEST_PARAMS.chain_id.to_owned(),
                height,
                previous_hash: Hash256::ZERO,
                merkle_root,
                timestamp_seconds: 1_700_000_000 + height,
                leading_zero_bits: TEST_PARAMS.initial_leading_zero_bits,
                nonce: 0,
            },
            transactions,
        )
    }
}
