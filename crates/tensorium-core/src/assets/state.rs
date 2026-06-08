use super::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Fungible,
    NonFungible,
}

#[derive(Clone, Debug)]
pub struct AssetInfo {
    pub kind: AssetKind,
    pub ticker: String,
    pub name: String,
    pub decimals: u8,
    pub supply: u64,
    pub issuer: String,
    pub royalty_bps: u16,
    pub royalty_addr: String,
    pub uri: String,
    pub content_hash: [u8; 32],
    pub mint_height: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Applied,
    Ignored(&'static str),
}

/// Deterministic asset state: reconstructable purely from the canonical chain.
#[derive(Default)]
pub struct AssetState {
    pub assets: HashMap<[u8; 32], AssetInfo>,
    pub ft_balances: HashMap<(String, [u8; 32]), u64>,
    pub nft_owner: HashMap<[u8; 32], String>,
}

impl AssetState {
    pub fn ft_balance(&self, addr: &str, asset_id: &[u8; 32]) -> u64 {
        *self.ft_balances.get(&(addr.to_string(), *asset_id)).unwrap_or(&0)
    }

    /// Apply one op. `txid` = the carrying tx's id (asset_id for ISSUE/NFT_MINT).
    /// `source` = address of the tx's first input. `dest_addr` = resolved address
    /// of the op's `dest_output_index` (only needed for TRANSFER).
    pub fn apply(
        &mut self,
        txid: [u8; 32],
        height: u64,
        source: &str,
        dest_addr: Option<&str>,
        op: &AssetOp,
    ) -> ApplyResult {
        match op {
            AssetOp::Issue(d) => {
                if self.assets.contains_key(&txid) {
                    return ApplyResult::Ignored("asset_id exists");
                }
                if d.decimals > 18 {
                    return ApplyResult::Ignored("decimals too high");
                }
                self.assets.insert(txid, AssetInfo {
                    kind: AssetKind::Fungible,
                    ticker: d.ticker.clone(),
                    name: d.name.clone(),
                    decimals: d.decimals,
                    supply: d.supply,
                    issuer: source.to_string(),
                    royalty_bps: 0,
                    royalty_addr: String::new(),
                    uri: String::new(),
                    content_hash: [0u8; 32],
                    mint_height: height,
                });
                *self.ft_balances.entry((source.to_string(), txid)).or_insert(0) += d.supply;
                ApplyResult::Applied
            }
            _ => ApplyResult::Ignored("not implemented yet"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{AssetOp, IssueData};

    fn issue(ticker: &str, supply: u64) -> AssetOp {
        AssetOp::Issue(IssueData { ticker: ticker.into(), decimals: 8, supply, name: ticker.into(), flags: 0 })
    }

    #[test]
    fn issue_credits_source_full_supply() {
        let mut st = AssetState::default();
        let txid = [1u8; 32];
        assert_eq!(st.apply(txid, 10, "txm1alice", None, &issue("GOLD", 1000)), ApplyResult::Applied);
        assert_eq!(st.ft_balance("txm1alice", &txid), 1000);
        assert_eq!(st.assets.get(&txid).unwrap().ticker, "GOLD");
        // duplicate asset_id ignored
        assert!(matches!(st.apply(txid, 11, "txm1bob", None, &issue("DUP", 5)), ApplyResult::Ignored(_)));
        assert_eq!(st.ft_balance("txm1alice", &txid), 1000);
    }
}
