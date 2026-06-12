use crate::index::Indexer;
use serde_json::{json, Value};
use tensorium_core::assets::AssetKind;
use tensorium_core::hash::Hash256;

/// Pure router: map (method, path) over the current indexer state to (status, body).
/// Read-only. Unknown routes → 404.
pub fn route(idx: &Indexer, method: &str, path: &str) -> (u16, Value) {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("GET", ["status"]) => (
            200,
            json!({
                "last_scanned_height": idx.last_height,
                "scanned_any": idx.scanned_any,
                "assets": idx.state.assets.len(),
            }),
        ),
        ("GET", ["asset", id]) => match parse_id(id).and_then(|k| idx.state.assets.get(&k)) {
            Some(info) => (200, asset_json(id, info)),
            None => (404, json!({ "error": "asset not found" })),
        },
        ("GET", ["nft", id, "owner"]) => {
            match parse_id(id).and_then(|k| idx.state.nft_owner.get(&k)) {
                Some(owner) => (200, json!({ "asset_id": id, "owner": owner })),
                None => (404, json!({ "error": "nft not found" })),
            }
        }
        ("GET", ["balance", addr]) => {
            let mut fts = vec![];
            for ((a, id), bal) in &idx.state.ft_balances {
                if a == addr && *bal > 0 {
                    fts.push(json!({ "asset_id": Hash256(*id).to_hex(), "amount": bal }));
                }
            }
            let nfts: Vec<String> = idx
                .state
                .nft_owner
                .iter()
                .filter(|(_, o)| o.as_str() == *addr)
                .map(|(id, _)| Hash256(*id).to_hex())
                .collect();
            (200, json!({ "address": addr, "fungible": fts, "nfts": nfts }))
        }
        ("GET", ["holders", id]) => match parse_id(id) {
            Some(key) => {
                let mut holders = vec![];
                for ((a, aid), bal) in &idx.state.ft_balances {
                    if *aid == key && *bal > 0 {
                        holders.push(json!({ "address": a, "amount": bal }));
                    }
                }
                if let Some(owner) = idx.state.nft_owner.get(&key) {
                    holders.push(json!({ "address": owner, "amount": 1 }));
                }
                (200, json!({ "asset_id": id, "holders": holders }))
            }
            None => (404, json!({ "error": "bad asset id" })),
        },
        ("GET", ["history", addr]) => {
            let events = idx.history.get(*addr).cloned().unwrap_or_default();
            (200, json!({ "address": addr, "events": events }))
        }
        ("GET", ["assets"]) => {
            let list: Vec<Value> = idx
                .state
                .assets
                .iter()
                .map(|(id, info)| asset_json(&Hash256(*id).to_hex(), info))
                .collect();
            (200, json!({ "assets": list }))
        }
        ("GET", ["outpoint", txid, vout]) => match vout.parse::<u32>() {
            Ok(v) => {
                let key = format!("{txid}:{v}");
                match idx.outpoints.get(&key) {
                    Some(addr) => (200, json!({ "outpoint": key, "address": addr })),
                    None => (404, json!({ "error": "outpoint not indexed" })),
                }
            }
            Err(_) => (400, json!({ "error": "bad vout" })),
        },
        _ => (404, json!({ "error": "not found" })),
    }
}

fn parse_id(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn asset_json(id: &str, info: &tensorium_core::assets::AssetInfo) -> Value {
    json!({
        "asset_id": id,
        "kind": match info.kind { AssetKind::Fungible => "ft", AssetKind::NonFungible => "nft" },
        "ticker": info.ticker,
        "name": info.name,
        "decimals": info.decimals,
        "supply": info.supply,
        "issuer": info.issuer,
        "royalty_bps": info.royalty_bps,
        "royalty_addr": info.royalty_addr,
        "uri": info.uri,
        "content_hash": Hash256(info.content_hash).to_hex(),
        "mint_height": info.mint_height,
    })
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Serve the read-only REST API, reading shared indexer state under a mutex.
pub fn serve(bind: &str, idx: Arc<Mutex<Indexer>>) -> Result<(), String> {
    let listener = TcpListener::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    println!("txm-asset-indexer API on http://{bind}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let mut line = req.lines().next().unwrap_or("").split_whitespace();
        let method = line.next().unwrap_or("");
        let path = line.next().unwrap_or("/");
        let (code, body) = {
            let guard = idx.lock().map_err(|e| format!("lock: {e}"))?;
            route(&guard, method, path)
        };
        let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into());
        let status_text = if code == 200 { "OK" } else { "Not Found" };
        let resp = format!(
            "HTTP/1.1 {code} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
            text.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tensorium_core::assets::{AssetOp, IssueData, NftMintData, TransferData};

    fn seeded() -> (Indexer, String) {
        let mut idx = Indexer::default();
        let gold = [1u8; 32];
        idx.state.apply(
            gold, 1, "txm1alice", None,
            &AssetOp::Issue(IssueData { ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0 }),
        );
        idx.state.apply(
            [2u8; 32], 2, "txm1alice", Some("txm1bob"),
            &AssetOp::Transfer(TransferData { asset_id: gold, amount: 400, dest_output_index: 0 }),
        );
        let nft = [7u8; 32];
        idx.state.apply(
            nft, 3, "txm1alice", None,
            &AssetOp::NftMint(NftMintData { collection_id: [0u8; 32], royalty_bps: 500, royalty_addr: "txm1alice".into(), uri: "ipfs://x".into(), content_hash: [9u8; 32] }),
        );
        idx.last_height = 3;
        idx.scanned_any = true;
        (idx, Hash256(gold).to_hex())
    }

    #[test]
    fn status_reports_height_and_count() {
        let (idx, _) = seeded();
        let (code, body) = route(&idx, "GET", "/status");
        assert_eq!(code, 200);
        assert_eq!(body["last_scanned_height"], 3);
        assert_eq!(body["assets"], 2);
    }

    #[test]
    fn asset_and_balance_and_holders() {
        let (idx, gold) = seeded();

        let (code, body) = route(&idx, "GET", &format!("/asset/{gold}"));
        assert_eq!(code, 200);
        assert_eq!(body["ticker"], "GOLD");
        assert_eq!(body["supply"], 1000);

        let (code, body) = route(&idx, "GET", "/balance/txm1bob");
        assert_eq!(code, 200);
        assert_eq!(body["fungible"][0]["amount"], 400);

        let (code, body) = route(&idx, "GET", &format!("/holders/{gold}"));
        assert_eq!(code, 200);
        assert_eq!(body["holders"].as_array().unwrap().len(), 2); // alice 600 + bob 400

        let (code, _) = route(&idx, "GET", "/asset/deadbeef");
        assert_eq!(code, 404);
    }

    #[test]
    fn nft_owner_route() {
        let (idx, _) = seeded();
        let nft = Hash256([7u8; 32]).to_hex();
        let (code, body) = route(&idx, "GET", &format!("/nft/{nft}/owner"));
        assert_eq!(code, 200);
        assert_eq!(body["owner"], "txm1alice");
    }

    #[test]
    fn outpoint_route_resolves_owner_and_404s() {
        use crate::index::Indexer;
        let mut idx = Indexer::default();
        // Seed the outpoints map directly: output "<64 hex>:0" owned by alice.
        let txid_hex = "aa".repeat(32);
        idx.outpoints.insert(format!("{txid_hex}:0"), "txm1alice".to_string());

        let (code, body) = route(&idx, "GET", &format!("/outpoint/{txid_hex}/0"));
        assert_eq!(code, 200);
        assert_eq!(body["address"], "txm1alice");
        assert_eq!(body["outpoint"], format!("{txid_hex}:0"));

        let (code404, _) = route(&idx, "GET", &format!("/outpoint/{txid_hex}/9"));
        assert_eq!(code404, 404);

        let (code400, _) = route(&idx, "GET", &format!("/outpoint/{txid_hex}/notanumber"));
        assert_eq!(code400, 400);
    }

    #[test]
    fn unknown_route_404() {
        let (idx, _) = seeded();
        assert_eq!(route(&idx, "GET", "/nope").0, 404);
        assert_eq!(route(&idx, "POST", "/status").0, 404);
    }
}
