use crate::index::{HistoryEntry, Indexer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tensorium_core::assets::{AssetInfo, AssetKind};
use tensorium_core::hash::Hash256;

/// Serializable mirror of `Indexer` with hex-encoded keys (JSON-safe).
#[derive(Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub last_height: u64,
    pub last_hash: String,
    pub scanned_any: bool,
    /// asset_id hex -> info
    pub assets: HashMap<String, AssetInfoJson>,
    /// "<addr>|<asset_hex>" -> balance
    pub ft_balances: HashMap<String, u64>,
    /// asset_id hex -> owner address
    pub nft_owner: HashMap<String, String>,
    /// "<txid_hex>:<vout>" -> address
    pub outpoints: HashMap<String, String>,
    /// address -> events
    pub history: HashMap<String, Vec<HistoryEntry>>,
}

#[derive(Serialize, Deserialize)]
pub struct AssetInfoJson {
    pub kind: String, // "ft" | "nft"
    pub ticker: String,
    pub name: String,
    pub decimals: u8,
    pub supply: u64,
    pub issuer: String,
    pub royalty_bps: u16,
    pub royalty_addr: String,
    pub uri: String,
    pub content_hash: String, // hex
    pub mint_height: u64,
}

fn hex32(b: &[u8; 32]) -> String {
    Hash256(*b).to_hex()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let bytes = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok());
    let mut out = [0u8; 32];
    let mut n = 0;
    for b in bytes {
        let b = b?;
        if n >= 32 {
            return None;
        }
        out[n] = b;
        n += 1;
    }
    if n == 32 {
        Some(out)
    } else {
        None
    }
}

impl Snapshot {
    /// Build a snapshot from live indexer state.
    pub fn from_indexer(idx: &Indexer) -> Self {
        let mut assets = HashMap::new();
        for (id, info) in &idx.state.assets {
            assets.insert(
                hex32(id),
                AssetInfoJson {
                    kind: match info.kind {
                        AssetKind::Fungible => "ft".into(),
                        AssetKind::NonFungible => "nft".into(),
                    },
                    ticker: info.ticker.clone(),
                    name: info.name.clone(),
                    decimals: info.decimals,
                    supply: info.supply,
                    issuer: info.issuer.clone(),
                    royalty_bps: info.royalty_bps,
                    royalty_addr: info.royalty_addr.clone(),
                    uri: info.uri.clone(),
                    content_hash: hex32(&info.content_hash),
                    mint_height: info.mint_height,
                },
            );
        }
        let mut ft_balances = HashMap::new();
        for ((addr, id), bal) in &idx.state.ft_balances {
            ft_balances.insert(format!("{addr}|{}", hex32(id)), *bal);
        }
        let nft_owner =
            idx.state.nft_owner.iter().map(|(id, o)| (hex32(id), o.clone())).collect();

        Snapshot {
            last_height: idx.last_height,
            last_hash: idx.last_hash.clone(),
            scanned_any: idx.scanned_any,
            assets,
            ft_balances,
            nft_owner,
            outpoints: idx.outpoints.clone(),
            history: idx.history.clone(),
        }
    }

    /// Rehydrate indexer state from this snapshot.
    pub fn apply_to(self, idx: &mut Indexer) {
        idx.last_height = self.last_height;
        idx.last_hash = self.last_hash;
        idx.scanned_any = self.scanned_any;
        idx.outpoints = self.outpoints;
        idx.history = self.history;
        for (id_hex, info) in self.assets {
            let Some(id) = unhex32(&id_hex) else { continue };
            idx.state.assets.insert(
                id,
                AssetInfo {
                    kind: if info.kind == "nft" { AssetKind::NonFungible } else { AssetKind::Fungible },
                    ticker: info.ticker,
                    name: info.name,
                    decimals: info.decimals,
                    supply: info.supply,
                    issuer: info.issuer,
                    royalty_bps: info.royalty_bps,
                    royalty_addr: info.royalty_addr,
                    uri: info.uri,
                    content_hash: unhex32(&info.content_hash).unwrap_or([0u8; 32]),
                    mint_height: info.mint_height,
                },
            );
        }
        for (key, bal) in self.ft_balances {
            if let Some((addr, id_hex)) = key.rsplit_once('|') {
                if let Some(id) = unhex32(id_hex) {
                    idx.state.ft_balances.insert((addr.to_string(), id), bal);
                }
            }
        }
        for (id_hex, owner) in self.nft_owner {
            if let Some(id) = unhex32(&id_hex) {
                idx.state.nft_owner.insert(id, owner);
            }
        }
    }
}

/// Atomically write the snapshot to `path` (tmp + rename).
pub fn save(idx: &Indexer, path: &str) -> Result<(), String> {
    let snap = Snapshot::from_indexer(idx);
    let json = serde_json::to_string(&snap).map_err(|e| format!("serialize: {e}"))?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp}->{path}: {e}"))
}

/// Load a snapshot from `path` into a fresh indexer. Missing file → empty indexer.
pub fn load(path: &str) -> Result<Indexer, String> {
    let mut idx = Indexer::default();
    match std::fs::read_to_string(path) {
        Ok(json) => {
            let snap: Snapshot =
                serde_json::from_str(&json).map_err(|e| format!("parse snapshot: {e}"))?;
            snap.apply_to(&mut idx);
            Ok(idx)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(idx),
        Err(e) => Err(format!("read {path}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tensorium_core::assets::{AssetOp, IssueData};

    #[test]
    fn snapshot_roundtrip_preserves_balances() {
        let mut idx = Indexer::default();
        let txid = [3u8; 32];
        idx.state.apply(
            txid,
            5,
            "txm1alice",
            None,
            &AssetOp::Issue(IssueData {
                ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
            }),
        );
        idx.outpoints.insert("deadbeef:0".into(), "txm1alice".into());
        idx.last_height = 5;
        idx.last_hash = "abc".into();
        idx.scanned_any = true;

        let json = serde_json::to_string(&Snapshot::from_indexer(&idx)).unwrap();
        let snap: Snapshot = serde_json::from_str(&json).unwrap();
        let mut restored = Indexer::default();
        snap.apply_to(&mut restored);

        assert_eq!(restored.last_height, 5);
        assert_eq!(restored.last_hash, "abc");
        assert_eq!(restored.state.ft_balance("txm1alice", &txid), 1000);
        assert_eq!(restored.state.assets.get(&txid).unwrap().ticker, "GOLD");
        assert_eq!(restored.outpoints.get("deadbeef:0").unwrap(), "txm1alice");
    }

    #[test]
    fn load_missing_file_is_empty_indexer() {
        let idx = load("/tmp/does-not-exist-txm-indexer.json").unwrap();
        assert!(!idx.scanned_any);
        assert_eq!(idx.last_height, 0);
    }
}
