//! TXM asset overlay protocol (TXM20 fungible tokens + NFTs).
//! Asset operations ride inside ordinary TXM transactions as `OP_RETURN`
//! metadata; balances/ownership are a deterministic function of the chain.
//! This module is pure (no I/O) — shared by the wallet and the indexer.
pub mod codec;
pub mod state;

pub use codec::{decode_op, encode_op, extract_asset_op};
pub use state::{ApplyResult, AssetInfo, AssetKind, AssetState};

/// One asset operation, decoded from a `TXMA` OP_RETURN payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetOp {
    Issue(IssueData),
    NftMint(NftMintData),
    Transfer(TransferData),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueData {
    pub ticker: String,   // ≤ 8 bytes
    pub decimals: u8,     // ≤ 18
    pub supply: u64,
    pub name: String,     // ≤ 32 bytes
    pub flags: u8,        // bit0 = mintable (reserved)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NftMintData {
    pub collection_id: [u8; 32], // all-zero = standalone NFT
    pub royalty_bps: u16,        // ≤ 10000
    pub royalty_addr: String,    // creator payout (may be empty = no royalty)
    pub uri: String,             // ≤ 200 bytes
    pub content_hash: [u8; 32],  // SHA-256 of the media
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferData {
    pub asset_id: [u8; 32],      // txid of the ISSUE/NFT_MINT
    pub amount: u64,             // NFT: must be 1
    pub dest_output_index: u8,   // output whose address receives the asset
}

#[derive(Debug, PartialEq, Eq)]
pub enum AssetError {
    BadMagic,
    BadVersion,
    UnknownOpcode,
    Truncated,
    TooLarge,
    FieldTooLong,
    BadRoyalty,
}

/// Protocol constants.
pub const MAGIC: &[u8; 4] = b"TXMA";
pub const VERSION: u8 = 0x01;
pub const OP_ISSUE: u8 = 0x01;
pub const OP_NFT_MINT: u8 = 0x02;
pub const OP_TRANSFER: u8 = 0x03;
/// Must fit a single OP_RETURN data push.
pub const MAX_PAYLOAD: usize = 520;

#[cfg(test)]
mod tests_e2e;
