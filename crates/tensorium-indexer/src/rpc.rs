use serde::Deserialize;
use tensorium_core::block::Block;
use tensorium_core::hash::Hash256;

/// Shape of `GET /getblock/<h>` → `{ "hash": <hash>, "block": <Block> }`.
#[derive(Deserialize)]
struct BlockResponse {
    hash: Hash256,
    block: Block,
}

/// Shape of `GET /getblockcount` → `{ "height": <u64|null>, ... }`.
#[derive(Deserialize)]
struct CountResponse {
    height: Option<u64>,
}

/// Pure: parse a `/getblock` JSON body into (block hash, block).
pub fn parse_block_response(body: &str) -> Result<(Hash256, Block), String> {
    let r: BlockResponse =
        serde_json::from_str(body).map_err(|e| format!("getblock parse: {e}"))?;
    Ok((r.hash, r.block))
}

/// Pure: parse a `/getblockcount` JSON body into the tip height (None = empty chain).
pub fn parse_count_response(body: &str) -> Result<Option<u64>, String> {
    let r: CountResponse =
        serde_json::from_str(body).map_err(|e| format!("getblockcount parse: {e}"))?;
    Ok(r.height)
}

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// HTTP RPC client for a Tensorium node. `addr` is `host:port` (e.g. `127.0.0.1:33332`).
pub struct NodeRpc {
    pub addr: String,
}

impl NodeRpc {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    fn get(&self, path: &str) -> Result<String, String> {
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: {}\r\nconnection: close\r\n\r\n", self.addr);
        let mut stream = TcpStream::connect(&self.addr)
            .map_err(|e| format!("RPC connect {}: {e}", self.addr))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("RPC timeout: {e}"))?;
        stream.write_all(request.as_bytes()).map_err(|e| format!("RPC write: {e}"))?;
        let mut response = String::new();
        stream.read_to_string(&mut response).map_err(|e| format!("RPC read: {e}"))?;
        let (head, body) =
            response.split_once("\r\n\r\n").ok_or_else(|| "invalid HTTP response".to_owned())?;
        if !head.starts_with("HTTP/1.1 200") {
            return Err(format!("RPC error ({path}): {body}"));
        }
        Ok(body.to_owned())
    }

    /// Tip height, or None if the chain is empty.
    pub fn block_count(&self) -> Result<Option<u64>, String> {
        parse_count_response(&self.get("/getblockcount")?)
    }

    /// Fetch block at `height` → (block hash, block).
    pub fn block_at(&self, height: u64) -> Result<(Hash256, Block), String> {
        parse_block_response(&self.get(&format!("/getblock/{height}"))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tensorium_core::block::Transaction;

    #[test]
    fn parses_block_response_roundtrip() {
        let block = sample_block();
        let hash = Hash256([5u8; 32]);
        let body = json!({ "hash": hash, "block": block }).to_string();
        let (got_hash, got_block) = parse_block_response(&body).unwrap();
        assert_eq!(got_hash, hash);
        assert_eq!(got_block.header.height, block.header.height);
        assert_eq!(got_block.transactions.len(), block.transactions.len());
    }

    #[test]
    fn parses_count_response() {
        assert_eq!(parse_count_response(r#"{"height":1911,"blocks":1912}"#).unwrap(), Some(1911));
        assert_eq!(parse_count_response(r#"{"height":null}"#).unwrap(), None);
        assert!(parse_count_response("not json").is_err());
    }

    fn sample_block() -> Block {
        let coinbase = Transaction::coinbase(10, 1190, "txm1miner");
        let header = tensorium_core::block::BlockHeader {
            version: 1,
            chain_id: "tensorium-mainnet-candidate-0".into(),
            height: 10,
            previous_hash: Hash256([1u8; 32]),
            merkle_root: Hash256([2u8; 32]),
            timestamp_seconds: 1_780_000_000,
            leading_zero_bits: 40,
            nonce: 42,
        };
        Block::new(header, vec![coinbase])
    }
}
