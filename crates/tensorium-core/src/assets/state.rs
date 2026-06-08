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
            AssetOp::Transfer(d) => {
                let Some(info) = self.assets.get(&d.asset_id) else {
                    return ApplyResult::Ignored("unknown asset");
                };
                let Some(dest) = dest_addr else {
                    return ApplyResult::Ignored("bad dest output");
                };
                match info.kind {
                    AssetKind::Fungible => {
                        if d.amount == 0 {
                            return ApplyResult::Ignored("zero amount");
                        }
                        let bal = self.ft_balance(source, &d.asset_id);
                        if bal < d.amount {
                            return ApplyResult::Ignored("insufficient balance");
                        }
                        *self.ft_balances.get_mut(&(source.to_string(), d.asset_id)).unwrap() -= d.amount;
                        *self.ft_balances.entry((dest.to_string(), d.asset_id)).or_insert(0) += d.amount;
                        ApplyResult::Applied
                    }
                    AssetKind::NonFungible => {
                        if d.amount != 1 {
                            return ApplyResult::Ignored("nft amount must be 1");
                        }
                        if self.nft_owner.get(&d.asset_id).map(|s| s.as_str()) != Some(source) {
                            return ApplyResult::Ignored("not nft owner");
                        }
                        self.nft_owner.insert(d.asset_id, dest.to_string());
                        ApplyResult::Applied
                    }
                }
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

    use crate::assets::TransferData;

    fn transfer(asset_id: [u8; 32], amount: u64) -> AssetOp {
        AssetOp::Transfer(TransferData { asset_id, amount, dest_output_index: 0 })
    }

    #[test]
    fn transfer_ft_debits_source_credits_dest() {
        let mut st = AssetState::default();
        let txid = [1u8; 32];
        st.apply(txid, 1, "txm1alice", None, &issue("GOLD", 1000));
        // move 300 alice -> bob
        assert_eq!(
            st.apply([2u8; 32], 2, "txm1alice", Some("txm1bob"), &transfer(txid, 300)),
            ApplyResult::Applied
        );
        assert_eq!(st.ft_balance("txm1alice", &txid), 700);
        assert_eq!(st.ft_balance("txm1bob", &txid), 300);
        // over-balance ignored, state unchanged
        assert!(matches!(
            st.apply([3u8; 32], 3, "txm1alice", Some("txm1bob"), &transfer(txid, 99999)),
            ApplyResult::Ignored(_)
        ));
        assert_eq!(st.ft_balance("txm1alice", &txid), 700);
        // unknown asset ignored
        assert!(matches!(
            st.apply([4u8; 32], 4, "txm1alice", Some("txm1bob"), &transfer([8u8; 32], 1)),
            ApplyResult::Ignored(_)
        ));
    }
}
