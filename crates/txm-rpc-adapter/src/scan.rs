//! Block scanner → wallet-transaction ledger for deposit detection.
//! Walks blocks from a checkpoint, recording outputs paid to managed addresses.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tensorium_core::script::standard::extract_address;

use crate::node::Node;
use crate::wallet::Wallet;

#[derive(Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub txid: String,
    pub address: String,
    pub category: String, // "receive"
    pub amount_atoms: u64,
    pub vout: u32,
    pub block_height: u64,
    pub block_hash: String,
    pub time: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Ledger {
    pub last_height: u64,
    pub entries: Vec<LedgerEntry>,
    /// height -> block hash (for listsinceblock)
    pub heights: HashMap<u64, String>,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    seen: std::collections::HashSet<String>, // txid:vout dedupe
}

fn json_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_u64().map(|b| b as u8)).collect(),
        _ => Vec::new(),
    }
}

impl Ledger {
    pub fn load(path: PathBuf) -> Self {
        let mut l: Ledger = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        l.path = path;
        for e in &l.entries {
            l.seen.insert(format!("{}:{}", e.txid, e.vout));
        }
        l
    }

    fn save(&self) {
        if let Ok(s) = serde_json::to_string(&self) {
            let _ = fs::write(&self.path, s);
        }
    }

    /// Scan new blocks up to the current tip, recording deposits to managed addrs.
    pub fn scan(&mut self, node: &Node, wallet: &Wallet) -> Result<u64, String> {
        let tip = node.block_count()?;
        let start = if self.last_height == 0 && self.entries.is_empty() {
            // first run: don't rescan all history unless explicitly checkpointed
            self.last_height.max(0)
        } else {
            self.last_height + 1
        };
        let mut new_deposits = 0u64;
        for h in start..=tip {
            let (hash, block) = node.block_at(h)?;
            self.heights.insert(h, hash.clone());
            if let Some(txs) = block.get("transactions").and_then(|t| t.as_array()) {
                for tx in txs {
                    let txid = tx.get("id").map(crate::node::txid_to_hex).unwrap_or_default();
                    let time = block
                        .get("header")
                        .and_then(|hd| hd.get("timestamp_seconds"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    if let Some(outs) = tx.get("outputs").and_then(|o| o.as_array()) {
                        for (vout, out) in outs.iter().enumerate() {
                            let spk = out.get("script_pubkey").map(json_bytes).unwrap_or_default();
                            let amount = out.get("value_atoms").and_then(|a| a.as_u64()).unwrap_or(0);
                            if let Some(addr) = extract_address(&spk) {
                                if wallet.is_mine(&addr) {
                                    let key = format!("{txid}:{vout}");
                                    if self.seen.insert(key) {
                                        self.entries.push(LedgerEntry {
                                            txid: txid.clone(),
                                            address: addr,
                                            category: "receive".into(),
                                            amount_atoms: amount,
                                            vout: vout as u32,
                                            block_height: h,
                                            block_hash: hash.clone(),
                                            time,
                                        });
                                        new_deposits += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            self.last_height = h;
        }
        self.save();
        Ok(new_deposits)
    }

    pub fn confirmations(&self, entry: &LedgerEntry, tip: u64) -> u64 {
        tip.saturating_sub(entry.block_height) + 1
    }

    /// Entries with block_height strictly greater than the given block hash's height.
    /// Unknown/empty hash → all entries.
    pub fn since_block(&self, block_hash: &str) -> Vec<&LedgerEntry> {
        let from_height = self
            .heights
            .iter()
            .find(|(_, h)| h.as_str() == block_hash)
            .map(|(height, _)| *height);
        match from_height {
            Some(fh) => self.entries.iter().filter(|e| e.block_height > fh).collect(),
            None => self.entries.iter().collect(),
        }
    }
}
