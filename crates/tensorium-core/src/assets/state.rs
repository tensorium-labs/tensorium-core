use super::*;
use std::collections::HashMap;

// Scaffold stubs — replaced wholesale in Task 5 (AssetState + apply).
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

#[derive(Default)]
pub struct AssetState {
    pub assets: HashMap<[u8; 32], AssetInfo>,
    pub ft_balances: HashMap<(String, [u8; 32]), u64>,
    pub nft_owner: HashMap<[u8; 32], String>,
}
