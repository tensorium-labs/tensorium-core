//! Multi-address hot keystore + balance + withdrawal building, for exchange use.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tensorium_core::{
    block::{OutPoint, Transaction, TxInput, TxOutput},
    hash::Hash256,
    script::standard::{p2pkh_from_address, p2pkh_script_sig},
    WalletKeypair,
};

use crate::node::Node;

#[derive(Default, Serialize, Deserialize)]
pub struct Keystore {
    /// address -> private key hex
    pub keys: HashMap<String, String>,
    /// the change address (a managed address); created on first use
    pub change: Option<String>,
}

pub struct Wallet {
    path: PathBuf,
    store: Keystore,
}

pub const SAT: u64 = 100_000_000;

/// Parse a decimal TXM amount into atoms (integer-safe: no float rounding drift).
pub fn txm_to_atoms(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if frac.len() > 8 {
        return Err("too many decimals (max 8)".into());
    }
    let int: u64 = int.parse().map_err(|_| "bad integer part".to_string())?;
    let mut frac_padded = frac.to_string();
    while frac_padded.len() < 8 {
        frac_padded.push('0');
    }
    let frac: u64 = if frac_padded.is_empty() { 0 } else { frac_padded.parse().map_err(|_| "bad fraction".to_string())? };
    int.checked_mul(SAT).and_then(|a| a.checked_add(frac)).ok_or("amount overflow".into())
}

/// Render atoms as an 8-decimal string (Bitcoin-style amount).
pub fn atoms_to_txm_str(atoms: u64) -> String {
    format!("{}.{:08}", atoms / SAT, atoms % SAT)
}

impl Wallet {
    pub fn load(path: PathBuf) -> Self {
        let store = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, store }
    }

    fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.store) {
            let _ = fs::write(&self.path, s);
        }
    }

    /// Generate a fresh deposit address, persist its key.
    pub fn new_address(&mut self) -> String {
        let kp = WalletKeypair::generate();
        let addr = kp.address.as_str().to_string();
        self.store.keys.insert(addr.clone(), kp.private_key_hex);
        if self.store.change.is_none() {
            self.store.change = Some(addr.clone());
        }
        self.save();
        addr
    }

    pub fn addresses(&self) -> Vec<String> {
        self.store.keys.keys().cloned().collect()
    }

    pub fn is_mine(&self, addr: &str) -> bool {
        self.store.keys.contains_key(addr)
    }

    fn change_address(&mut self) -> String {
        if let Some(c) = &self.store.change {
            return c.clone();
        }
        self.new_address()
    }

    /// Total mature spendable balance (atoms) across all managed addresses.
    pub fn balance_atoms(&self, node: &Node) -> Result<u64, String> {
        let mut total = 0u64;
        for addr in self.store.keys.keys() {
            for u in node.utxos(addr)? {
                if u.mature {
                    total = total.saturating_add(u.value_atoms);
                }
            }
        }
        Ok(total)
    }

    /// Build, sign and broadcast a payment to `dest` for `amount_atoms` + `fee_atoms`.
    /// Selects mature UTXOs across all managed addresses; change returns to the
    /// wallet's change address. Returns the txid hex.
    pub fn send(
        &mut self,
        node: &Node,
        dest: &str,
        amount_atoms: u64,
        fee_atoms: u64,
    ) -> Result<String, String> {
        let needed = amount_atoms.checked_add(fee_atoms).ok_or("amount overflow")?;
        // Gather mature UTXOs (remember which address each belongs to → its key).
        let mut selected: Vec<(OutPoint, String)> = Vec::new(); // (outpoint, owner addr)
        let mut sum = 0u64;
        'outer: for addr in self.store.keys.keys().cloned().collect::<Vec<_>>() {
            for u in node.utxos(&addr)? {
                if !u.mature {
                    continue;
                }
                let txid = Hash256(
                    u.txid_bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "bad txid len".to_string())?,
                );
                selected.push((OutPoint { txid, output_index: u.output_index }, addr.clone()));
                sum = sum.saturating_add(u.value_atoms);
                if sum >= needed {
                    break 'outer;
                }
            }
        }
        if sum < needed {
            return Err(format!(
                "insufficient funds: have {}, need {}",
                atoms_to_txm_str(sum),
                atoms_to_txm_str(needed)
            ));
        }

        let dest_spk = p2pkh_from_address(dest).map_err(|_| format!("invalid address: {dest}"))?;
        let change_addr = self.change_address();
        let change_spk = p2pkh_from_address(&change_addr).map_err(|_| "bad change addr".to_string())?;

        let inputs: Vec<TxInput> = selected
            .iter()
            .map(|(op, _)| TxInput { previous_output: *op, signature_script: Vec::new() })
            .collect();
        let mut outputs = vec![TxOutput { value_atoms: amount_atoms, script_pubkey: dest_spk }];
        let change = sum - needed;
        if change > 0 {
            outputs.push(TxOutput { value_atoms: change, script_pubkey: change_spk });
        }

        let mut tx = Transaction::payment(inputs, outputs);
        let sig_hash = tx.signature_hash();
        // Sign each input with the key controlling its source address.
        for (i, (_, owner)) in selected.iter().enumerate() {
            let pk_hex = self.store.keys.get(owner).ok_or("missing key for input")?;
            let kp = WalletKeypair::from_private_key_hex(pk_hex).map_err(|e| format!("key: {e:?}"))?;
            let der = kp.sign_hash(&sig_hash).map_err(|e| format!("sign: {e:?}"))?;
            let pubkey = hex::decode(&kp.public_key_hex).map_err(|_| "bad pubkey hex".to_string())?;
            tx.inputs[i].signature_script = p2pkh_script_sig(&der, &pubkey);
        }
        tx.refresh_id();

        let raw = serde_json::to_string(&tx).map_err(|e| format!("serialize: {e}"))?;
        node.send_raw(&raw)?;
        Ok(tx.id.to_hex())
    }
}
