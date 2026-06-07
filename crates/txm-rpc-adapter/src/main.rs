//! Bitcoin-core-style JSON-RPC adapter for Tensorium (TXM).
//! Lets exchanges (SafeTrade etc.) automate deposits/withdrawals against a
//! running `tensorium-node`, using the standard coin-daemon RPC surface.
mod node;
mod scan;
mod wallet;

use std::env;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use node::Node;
use scan::Ledger;
use wallet::{atoms_to_txm_str, txm_to_atoms, Wallet};

struct Ctx {
    node: Node,
    state: Arc<Mutex<(Wallet, Ledger)>>,
    fee_atoms: u64,
}

fn b64(input: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        out.push(A[(b[0] >> 2) as usize] as char);
        out.push(A[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 { A[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[(b[2] & 0x3f) as usize] as char } else { '=' });
    }
    out
}

fn err(code: i64, msg: impl Into<String>) -> (i64, String) {
    (code, msg.into())
}

fn dispatch(method: &str, params: &Value, ctx: &Ctx) -> Result<Value, (i64, String)> {
    let p = |i: usize| params.get(i);
    match method {
        "getblockcount" => Ok(json!(ctx.node.block_count().map_err(|e| err(-1, e))?)),

        "getbestblockhash" => {
            let tip = ctx.node.block_count().map_err(|e| err(-1, e))?;
            let (hash, _) = ctx.node.block_at(tip).map_err(|e| err(-1, e))?;
            Ok(json!(hash))
        }

        "getblockhash" => {
            let h = p(0).and_then(|v| v.as_u64()).ok_or(err(-8, "height required"))?;
            let (hash, _) = ctx.node.block_at(h).map_err(|e| err(-1, e))?;
            Ok(json!(hash))
        }

        "getblock" => {
            // accept a block hash (resolve via scanned heights) or a height number
            let arg = p(0).ok_or(err(-8, "hash/height required"))?;
            let height = if let Some(h) = arg.as_u64() {
                h
            } else if let Some(hs) = arg.as_str() {
                let st = ctx.state.lock().unwrap();
                *st.1.heights.iter().find(|(_, hh)| hh.as_str() == hs).map(|(ht, _)| ht)
                    .ok_or(err(-5, "block hash not in scanned range; use getblockhash(height)"))?
            } else {
                return Err(err(-8, "bad arg"));
            };
            let (hash, block) = ctx.node.block_at(height).map_err(|e| err(-1, e))?;
            Ok(json!({ "hash": hash, "height": height, "tx": block.get("transactions") }))
        }

        "getblockchaininfo" => {
            let tip = ctx.node.block_count().map_err(|e| err(-1, e))?;
            let (hash, _) = ctx.node.block_at(tip).map_err(|e| err(-1, e))?;
            Ok(json!({ "chain": "main", "blocks": tip, "headers": tip, "bestblockhash": hash, "verificationprogress": 1.0 }))
        }

        "getnetworkinfo" => Ok(json!({ "version": 10000, "subversion": "/tensorium-rpc-adapter:0.1.0/", "connections": 1, "networkactive": true })),
        "getwalletinfo" => {
            let st = ctx.state.lock().unwrap();
            let bal = st.0.balance_atoms(&ctx.node).map_err(|e| err(-1, e))?;
            Ok(json!({ "walletversion": 1, "balance": amount_num(bal), "txcount": st.1.entries.len() }))
        }

        "getnewaddress" => {
            let mut st = ctx.state.lock().unwrap();
            Ok(json!(st.0.new_address()))
        }

        "validateaddress" => {
            let a = p(0).and_then(|v| v.as_str()).ok_or(err(-8, "address required"))?;
            let valid = tensorium_core::script::standard::p2pkh_from_address(a).is_ok();
            let mine = ctx.state.lock().unwrap().0.is_mine(a);
            Ok(json!({ "isvalid": valid, "address": a, "ismine": mine }))
        }

        "getbalance" => {
            let st = ctx.state.lock().unwrap();
            let bal = st.0.balance_atoms(&ctx.node).map_err(|e| err(-1, e))?;
            Ok(amount_num(bal))
        }

        "sendtoaddress" => {
            let dest = p(0).and_then(|v| v.as_str()).ok_or(err(-8, "address required"))?;
            let amt = amount_to_atoms(p(1).ok_or(err(-8, "amount required"))?)?;
            let mut st = ctx.state.lock().unwrap();
            let txid = st.0.send(&ctx.node, dest, amt, ctx.fee_atoms).map_err(|e| err(-4, e))?;
            Ok(json!(txid))
        }

        "settxfee" => Ok(json!(true)), // fixed fee; accept for compatibility
        "estimatesmartfee" | "estimatefee" => Ok(json!({ "feerate": amount_num(ctx.fee_atoms) })),

        "listsinceblock" => {
            let bh = p(0).and_then(|v| v.as_str()).unwrap_or("");
            let tip = ctx.node.block_count().map_err(|e| err(-1, e))?;
            let (tiphash, _) = ctx.node.block_at(tip).map_err(|e| err(-1, e))?;
            let st = ctx.state.lock().unwrap();
            let txs: Vec<Value> = st.1.since_block(bh).iter().map(|e| entry_json(e, tip, &st.1)).collect();
            Ok(json!({ "transactions": txs, "lastblock": tiphash }))
        }

        "listtransactions" => {
            let count = p(1).and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let skip = p(2).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let tip = ctx.node.block_count().map_err(|e| err(-1, e))?;
            let st = ctx.state.lock().unwrap();
            let n = st.1.entries.len();
            let txs: Vec<Value> = st.1.entries.iter().rev().skip(skip).take(count)
                .map(|e| entry_json(e, tip, &st.1)).collect();
            let _ = n;
            Ok(json!(txs))
        }

        "gettransaction" => {
            let txid = p(0).and_then(|v| v.as_str()).ok_or(err(-8, "txid required"))?;
            let tip = ctx.node.block_count().map_err(|e| err(-1, e))?;
            let st = ctx.state.lock().unwrap();
            let matches: Vec<&scan::LedgerEntry> = st.1.entries.iter().filter(|e| e.txid == txid).collect();
            if matches.is_empty() {
                return Err(err(-5, "transaction not found in wallet ledger"));
            }
            let amount: u64 = matches.iter().map(|e| e.amount_atoms).sum();
            let conf = st.1.confirmations(matches[0], tip);
            let details: Vec<Value> = matches.iter().map(|e| entry_json(e, tip, &st.1)).collect();
            Ok(json!({ "txid": txid, "amount": amount_num(amount), "confirmations": conf, "details": details }))
        }

        "help" => Ok(json!("Tensorium RPC adapter: getblockcount getblockhash getblock getbestblockhash getblockchaininfo getnetworkinfo getwalletinfo getnewaddress validateaddress getbalance sendtoaddress listsinceblock listtransactions gettransaction estimatesmartfee settxfee")),
        "uptime" => Ok(json!(0)),

        _ => Err(err(-32601, format!("method not found: {method}"))),
    }
}

fn amount_num(atoms: u64) -> Value {
    json!(atoms_to_txm_str(atoms).parse::<f64>().unwrap_or(0.0))
}

fn amount_to_atoms(v: &Value) -> Result<u64, (i64, String)> {
    let s = match v {
        Value::Number(n) => format!("{:.8}", n.as_f64().unwrap_or(0.0)),
        Value::String(s) => s.clone(),
        _ => return Err(err(-8, "bad amount")),
    };
    txm_to_atoms(&s).map_err(|e| err(-3, e))
}

fn entry_json(e: &scan::LedgerEntry, tip: u64, l: &Ledger) -> Value {
    json!({
        "txid": e.txid,
        "address": e.address,
        "category": e.category,
        "amount": amount_num(e.amount_atoms),
        "vout": e.vout,
        "confirmations": l.confirmations(e, tip),
        "blockhash": e.block_hash,
        "blockheight": e.block_height,
        "time": e.time,
    })
}

fn main() {
    let bind = env::var("ADAPTER_BIND").unwrap_or_else(|_| "127.0.0.1:8332".into());
    let node = Node::new(&env::var("NODE_RPC").unwrap_or_else(|_| "http://127.0.0.1:33332".into()));
    let user = env::var("RPC_USER").unwrap_or_default();
    let pass = env::var("RPC_PASS").unwrap_or_default();
    let fee_atoms = env::var("FEE_ATOMS").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000u64);
    let wallet_path = env::var("WALLET_PATH").unwrap_or_else(|_| "adapter-wallet.json".into());
    let ledger_path = env::var("LEDGER_PATH").unwrap_or_else(|_| "adapter-ledger.json".into());
    let poll_ms: u64 = env::var("SCAN_POLL_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);

    let state = Arc::new(Mutex::new((Wallet::load(wallet_path.into()), Ledger::load(ledger_path.into()))));
    let expected_auth = if user.is_empty() { None } else { Some(format!("Basic {}", b64(format!("{user}:{pass}").as_bytes()))) };

    // Background deposit scanner.
    {
        let node = node.clone();
        let state = state.clone();
        thread::spawn(move || loop {
            {
                let mut st = state.lock().unwrap();
                let (w, l) = &mut *st;
                if let Err(e) = l.scan(&node, w) {
                    eprintln!("[scan] {e}");
                }
            }
            thread::sleep(Duration::from_millis(poll_ms));
        });
    }

    let ctx = Ctx { node, state, fee_atoms };
    let server = tiny_http::Server::http(&bind).expect("bind adapter");
    eprintln!("[adapter] JSON-RPC on {bind} (auth: {})", if expected_auth.is_some() { "on" } else { "OFF" });

    for mut req in server.incoming_requests() {
        // Auth
        if let Some(exp) = &expected_auth {
            let ok = req.headers().iter().any(|h| h.field.equiv("Authorization") && h.value.as_str() == exp);
            if !ok {
                let _ = req.respond(tiny_http::Response::from_string("Unauthorized").with_status_code(401));
                continue;
            }
        }
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
        let v: Value = serde_json::from_str(&body).unwrap_or(json!({}));
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = v.get("params").cloned().unwrap_or(json!([]));
        let resp = match dispatch(method, &params, &ctx) {
            Ok(result) => json!({ "result": result, "error": Value::Null, "id": id }),
            Err((code, message)) => json!({ "result": Value::Null, "error": { "code": code, "message": message }, "id": id }),
        };
        let r = tiny_http::Response::from_string(resp.to_string())
            .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
        let _ = req.respond(r);
    }
}

#[cfg(test)]
mod tests {
    use super::wallet::{atoms_to_txm_str, txm_to_atoms, SAT};

    #[test]
    fn amount_roundtrip() {
        assert_eq!(txm_to_atoms("1").unwrap(), SAT);
        assert_eq!(txm_to_atoms("0.5").unwrap(), 50_000_000);
        assert_eq!(txm_to_atoms("1.23456789").unwrap(), 123_456_789);
        assert_eq!(txm_to_atoms("0").unwrap(), 0);
        assert_eq!(atoms_to_txm_str(SAT), "1.00000000");
        assert_eq!(atoms_to_txm_str(123_456_789), "1.23456789");
        assert_eq!(atoms_to_txm_str(50_000_000), "0.50000000");
    }

    #[test]
    fn amount_rejects_too_many_decimals() {
        assert!(txm_to_atoms("1.123456789").is_err());
    }

    #[test]
    fn b64_basic() {
        assert_eq!(super::b64(b"user:pass"), "dXNlcjpwYXNz");
    }
}
