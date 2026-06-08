//! txm-asset-indexer — Layer 2 of the TXM asset marketplace.
//!
//! Env / args:
//!   TXM_INDEXER_RPC   node RPC host:port      (default 127.0.0.1:33332)
//!   TXM_INDEXER_BIND  REST API bind           (default 127.0.0.1:23340)
//!   TXM_INDEXER_DB    snapshot path           (default ./txm-asset-index.json)
//!   TXM_INDEXER_POLL  poll seconds            (default 10)
mod api;
mod index;
mod rpc;
mod store;
mod sync;

use rpc::NodeRpc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn main() -> Result<(), String> {
    let rpc_addr = env_or("TXM_INDEXER_RPC", "127.0.0.1:33332");
    let bind = env_or("TXM_INDEXER_BIND", "127.0.0.1:23340");
    let db = env_or("TXM_INDEXER_DB", "./txm-asset-index.json");
    let poll: u64 = env_or("TXM_INDEXER_POLL", "10").parse().unwrap_or(10);

    let idx = Arc::new(Mutex::new(store::load(&db)?));
    println!("loaded snapshot: last_height={}", idx.lock().unwrap().last_height);

    // Scanner thread.
    let scan_idx = Arc::clone(&idx);
    let rpc = NodeRpc::new(rpc_addr.clone());
    let db_path = db.clone();
    thread::spawn(move || loop {
        if let Err(e) = tick(&scan_idx, &rpc, &db_path) {
            eprintln!("scan tick error: {e}");
        }
        thread::sleep(Duration::from_secs(poll));
    });

    // REST API on the main thread.
    api::serve(&bind, idx)
}

/// One scan cycle: fetch tip, scan to it, persist if anything changed.
fn tick(idx: &Arc<Mutex<index::Indexer>>, rpc: &NodeRpc, db: &str) -> Result<(), String> {
    let Some(tip) = rpc.block_count()? else {
        return Ok(()); // empty chain
    };
    let mut guard = idx.lock().map_err(|e| format!("lock: {e}"))?;
    if guard.scanned_any && guard.last_height >= tip {
        return Ok(());
    }
    let applied = sync::scan(&mut guard, tip, |h| rpc.block_at(h))?;
    if applied > 0 {
        store::save(&guard, db)?;
        println!("scanned to height {} (+{applied})", guard.last_height);
    }
    Ok(())
}
