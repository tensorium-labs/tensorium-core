mod accounting;
mod stratum;

use accounting::{PayoutEntry, PayoutLedger};
use serde_json::json;
use std::{
    collections::HashMap,
    env,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tensorium_core::Block;

// ---------------------------------------------------------------------------
// Configuration defaults
// ---------------------------------------------------------------------------

const DEFAULT_POOL_BIND: &str = "0.0.0.0:23336";
const DEFAULT_NODE_RPC: &str = "127.0.0.1:33332";
const DEFAULT_LEDGER_PATH: &str = "pool-ledger.json";
/// Read/write timeout for pool-side HTTP connections (seconds).
const HTTP_TIMEOUT_SECS: u64 = 15;

fn main() {
    if let Err(e) = run() {
        eprintln!("pool error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") | None => serve(),
        Some("stats") => cmd_stats(),
        Some("accounting") => cmd_accounting(),
        Some("custody") => cmd_custody(),
        Some("pending") => {
            let addr = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-pool pending <miner_address>".to_owned())?;
            cmd_pending(addr)
        }
        Some("mark-paid") => {
            let addr = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-pool mark-paid <miner_address>".to_owned())?;
            cmd_mark_paid(addr)
        }
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(cmd) => Err(format!("unknown command: {cmd}")),
    }
}

// ---------------------------------------------------------------------------
// Shared pool state
// ---------------------------------------------------------------------------

struct PoolState {
    /// Node RPC address (host:port).
    node_rpc: String,
    /// Pool treasury wallet address — used as coinbase recipient.
    treasury_address: String,
    /// Operational hot wallet address used to pay miners.
    payout_hot_wallet: Option<String>,
    /// Soft cap for the hot wallet balance, in atoms.
    payout_hot_wallet_max_atoms: Option<u64>,
    /// Path to the persistent payout ledger JSON.
    ledger_path: PathBuf,
    /// Payout ledger (loaded from disk, persisted on each accepted block).
    ledger: PayoutLedger,
    /// Maps block height → miner_address of the last miner to request a template
    /// at that height. Used to attribute an accepted block to the right miner.
    last_miner_for_height: HashMap<u64, String>,
}

impl PoolState {
    fn new(
        node_rpc: String,
        treasury_address: String,
        payout_hot_wallet: Option<String>,
        payout_hot_wallet_max_atoms: Option<u64>,
        ledger_path: PathBuf,
    ) -> Self {
        let ledger = PayoutLedger::load(&ledger_path);
        Self {
            node_rpc,
            treasury_address,
            payout_hot_wallet,
            payout_hot_wallet_max_atoms,
            ledger_path,
            ledger,
            last_miner_for_height: HashMap::new(),
        }
    }

    fn register_miner_for_height(&mut self, height: u64, miner_address: String) {
        self.last_miner_for_height.insert(height, miner_address);
    }

    fn miner_for_height(&self, height: u64) -> Option<&str> {
        self.last_miner_for_height.get(&height).map(String::as_str)
    }

    fn record_block(
        &mut self,
        block_height: u64,
        block_hash: String,
        miner_address: String,
        gross_reward_atoms: u64,
    ) -> Result<(), String> {
        let entry =
            PayoutEntry::new(block_height, block_hash, miner_address, gross_reward_atoms);
        self.ledger.push(entry);
        self.ledger.save(&self.ledger_path)
    }
}

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

fn serve() -> Result<(), String> {
    let bind = env::var("TENSORIUM_POOL_BIND").unwrap_or_else(|_| DEFAULT_POOL_BIND.to_owned());
    let node_rpc = env::var("TENSORIUM_NODE_RPC").unwrap_or_else(|_| DEFAULT_NODE_RPC.to_owned());
    let treasury = env::var("TENSORIUM_POOL_TREASURY").map_err(|_| {
        "TENSORIUM_POOL_TREASURY env var required (pool treasury wallet address)".to_owned()
    })?;
    let payout_hot_wallet = env::var("TENSORIUM_POOL_PAYOUT_HOT_WALLET").ok();
    let payout_hot_wallet_max_atoms = env::var("TENSORIUM_POOL_PAYOUT_HOT_MAX_ATOMS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    let ledger_path = PathBuf::from(
        env::var("TENSORIUM_POOL_LEDGER").unwrap_or_else(|_| DEFAULT_LEDGER_PATH.to_owned()),
    );

    let stratum_bind = std::env::var("TENSORIUM_STRATUM_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3333".to_string());
    let share_diff: u64 = std::env::var("TENSORIUM_POOL_SHARE_DIFF")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_048_576);

    println!("tensorium-pool");
    println!("  bind         = {bind}");
    println!("  node_rpc     = {node_rpc}");
    println!("  treasury     = {treasury}");
    println!(
        "  payout_hot   = {}",
        payout_hot_wallet.as_deref().unwrap_or("<not configured>")
    );
    println!(
        "  payout_cap   = {}",
        payout_hot_wallet_max_atoms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "<not configured>".to_owned())
    );
    println!("  ledger       = {}", ledger_path.display());
    println!("  pool_fee     = {}%", accounting::POOL_FEE_BPS / 100);
    println!("  stratum      = {stratum_bind}");
    println!("  share_diff   = {share_diff}");
    println!();
    println!("Miners connect to {bind} using the same RPC interface as the node.");
    println!("Stratum miners connect to {stratum_bind}.");
    println!("Press Ctrl+C to stop.\n");

    let state = Arc::new(Mutex::new(PoolState::new(
        node_rpc.clone(),
        treasury.clone(),
        payout_hot_wallet,
        payout_hot_wallet_max_atoms,
        ledger_path,
    )));

    // ── Stratum server (TCP port 3333) ──────────────────────────────────────
    let stratum_state = std::sync::Arc::new(std::sync::Mutex::new(
        stratum::StratumState::new(node_rpc.clone(), treasury.clone(), share_diff),
    ));

    {
        let ss   = stratum_state.clone();
        let bind_str = stratum_bind.clone();
        std::thread::spawn(move || stratum::run_stratum_server(ss, &bind_str));
    }

    let listener =
        TcpListener::bind(&bind).map_err(|e| format!("bind {bind}: {e}"))?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                let stratum_state = stratum_state.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, state, stratum_state) {
                        eprintln!("connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<PoolState>>,
    stratum_state: Arc<Mutex<stratum::StratumState>>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(HTTP_TIMEOUT_SECS)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(HTTP_TIMEOUT_SECS)))
        .ok();

    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).map_err(|e| format!("read: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return write_response(&mut stream, 400, "{\"error\":\"bad request\"}");
    }
    let method = parts[0];
    let path = parts[1];

    match (method, path) {
        ("GET", "/health") => {
            write_response(&mut stream, 200, &json!({"ok": true}).to_string())
        }
        ("GET", "/pool/stats") => handle_pool_stats(&mut stream, &state),
        ("GET", "/pool/stratum") => handle_pool_stratum(&mut stream, &stratum_state),
        ("GET", "/pool/accounting") => handle_pool_accounting(&mut stream, &state),
        ("GET", "/pool/custody") => handle_pool_custody(&mut stream, &state),
        ("GET", path) if path.starts_with("/pool/pending/") => {
            let addr = path.trim_start_matches("/pool/pending/");
            handle_pool_pending(&mut stream, addr, &state)
        }
        ("GET", path) if path.starts_with("/getblocktemplate/") => {
            let miner = path.trim_start_matches("/getblocktemplate/");
            handle_get_block_template(&mut stream, miner, &state)
        }
        ("POST", "/submitblock") => {
            let body = extract_body(&request);
            handle_submit_block(&mut stream, body, &state)
        }
        _ => write_response(&mut stream, 404, "{\"error\":\"not found\"}"),
    }
}

// ---------------------------------------------------------------------------
// Handler: GET /getblocktemplate/<miner>
// ---------------------------------------------------------------------------

fn handle_get_block_template(
    stream: &mut TcpStream,
    miner_address: &str,
    state: &Arc<Mutex<PoolState>>,
) -> Result<(), String> {
    if miner_address.is_empty() {
        return write_response(stream, 400, "{\"error\":\"missing miner address\"}");
    }

    let (node_rpc, treasury_address) = {
        let s = state.lock().unwrap();
        (s.node_rpc.clone(), s.treasury_address.clone())
    };

    // Fetch template from node using the pool treasury address as coinbase recipient.
    let raw = match http_get(
        &node_rpc,
        &format!("/getblocktemplate/{treasury_address}"),
    ) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("node getblocktemplate error: {e}");
            return write_response(
                stream,
                502,
                &json!({"error": format!("node unavailable: {e}")}).to_string(),
            );
        }
    };

    // Extract height from template so we can register miner → height mapping.
    if let Ok(tpl) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(height) = tpl["template"]["header"]["height"].as_u64() {
            state
                .lock()
                .unwrap()
                .register_miner_for_height(height, miner_address.to_owned());
        }
    }

    write_response(stream, 200, &raw)
}

// ---------------------------------------------------------------------------
// Handler: POST /submitblock
// ---------------------------------------------------------------------------

fn handle_submit_block(
    stream: &mut TcpStream,
    body: &str,
    state: &Arc<Mutex<PoolState>>,
) -> Result<(), String> {
    // Parse the submitted block.
    let block: Block = match serde_json::from_str(body) {
        Ok(b) => b,
        Err(e) => {
            return write_response(
                stream,
                400,
                &json!({"error": format!("invalid block: {e}")}).to_string(),
            )
        }
    };

    let block_height = block.header.height;
    let node_rpc = state.lock().unwrap().node_rpc.clone();

    // Forward to node.
    let node_resp = match http_post(&node_rpc, "/submitblock", body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("node submitblock error: {e}");
            return write_response(
                stream,
                502,
                &json!({"error": format!("node unavailable: {e}")}).to_string(),
            );
        }
    };

    // Check node acceptance.
    let accepted = serde_json::from_str::<serde_json::Value>(&node_resp)
        .ok()
        .and_then(|v| v["accepted"].as_bool())
        .unwrap_or(false);

    if accepted {
        // Extract gross reward from coinbase output (first tx, first output).
        let gross_reward_atoms = block
            .transactions
            .first()
            .and_then(|tx| tx.outputs.first())
            .map(|o| o.value_atoms)
            .unwrap_or(0);

        // Compute block hash for the ledger entry.
        let block_hash = block.hash().to_hex();

        // Look up which miner was working on this height.
        let miner_address = {
            let s = state.lock().unwrap();
            s.miner_for_height(block_height)
                .unwrap_or("unknown")
                .to_owned()
        };

        // Record payout accounting.
        if let Err(e) = state.lock().unwrap().record_block(
            block_height,
            block_hash,
            miner_address.clone(),
            gross_reward_atoms,
        ) {
            eprintln!("accounting error: {e}");
        } else {
            let fee = gross_reward_atoms * accounting::POOL_FEE_BPS / 10_000;
            let net = gross_reward_atoms.saturating_sub(fee);
            println!(
                "block accepted  height={}  miner={}  gross={}  fee={}  net={}",
                block_height, miner_address, gross_reward_atoms, fee, net
            );
        }
    }

    write_response(stream, 200, &node_resp)
}

// ---------------------------------------------------------------------------
// Pool management handlers
// ---------------------------------------------------------------------------

fn handle_pool_stats(stream: &mut TcpStream, state: &Arc<Mutex<PoolState>>) -> Result<(), String> {
    let stats = state.lock().unwrap().ledger.stats();
    write_response(stream, 200, &serde_json::to_string(&stats).unwrap())
}

fn handle_pool_stratum(
    stream: &mut TcpStream,
    stratum_state: &Arc<Mutex<stratum::StratumState>>,
) -> Result<(), String> {
    let body = stratum_state.lock().unwrap().stats_json().to_string();
    write_response(stream, 200, &body)
}

fn handle_pool_accounting(
    stream: &mut TcpStream,
    state: &Arc<Mutex<PoolState>>,
) -> Result<(), String> {
    let entries = state.lock().unwrap().ledger.entries.clone();
    let body = serde_json::to_string_pretty(&entries).unwrap_or_default();
    write_response(stream, 200, &body)
}

fn handle_pool_custody(stream: &mut TcpStream, state: &Arc<Mutex<PoolState>>) -> Result<(), String> {
    let s = state.lock().unwrap();
    let body = json!({
        "treasury_address": s.treasury_address,
        "payout_hot_wallet": s.payout_hot_wallet,
        "payout_hot_wallet_max_atoms": s.payout_hot_wallet_max_atoms,
        "ledger_path": s.ledger_path,
    });
    write_response(stream, 200, &body.to_string())
}

fn handle_pool_pending(
    stream: &mut TcpStream,
    miner_address: &str,
    state: &Arc<Mutex<PoolState>>,
) -> Result<(), String> {
    let pending = state.lock().unwrap().ledger.pending_atoms(miner_address);
    let body = json!({
        "miner_address": miner_address,
        "pending_net_atoms": pending,
    });
    write_response(stream, 200, &body.to_string())
}

// ---------------------------------------------------------------------------
// CLI commands (read-only, work on ledger file directly)
// ---------------------------------------------------------------------------

fn cmd_stats() -> Result<(), String> {
    let ledger = load_ledger_from_env();
    let stats = ledger.stats();
    println!("{}", serde_json::to_string_pretty(&stats).unwrap());
    Ok(())
}

fn cmd_accounting() -> Result<(), String> {
    let ledger = load_ledger_from_env();
    println!("{}", serde_json::to_string_pretty(&ledger.entries).unwrap());
    Ok(())
}

fn cmd_custody() -> Result<(), String> {
    let treasury = env::var("TENSORIUM_POOL_TREASURY")
        .unwrap_or_else(|_| "<not configured>".to_owned());
    let payout_hot_wallet = env::var("TENSORIUM_POOL_PAYOUT_HOT_WALLET")
        .unwrap_or_else(|_| "<not configured>".to_owned());
    let payout_hot_wallet_max_atoms = env::var("TENSORIUM_POOL_PAYOUT_HOT_MAX_ATOMS")
        .unwrap_or_else(|_| "<not configured>".to_owned());
    let ledger_path = env::var("TENSORIUM_POOL_LEDGER")
        .unwrap_or_else(|_| DEFAULT_LEDGER_PATH.to_owned());

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "treasury_address": treasury,
            "payout_hot_wallet": payout_hot_wallet,
            "payout_hot_wallet_max_atoms": payout_hot_wallet_max_atoms,
            "ledger_path": ledger_path,
        }))
        .unwrap()
    );
    Ok(())
}

fn cmd_pending(miner_address: &str) -> Result<(), String> {
    let ledger = load_ledger_from_env();
    let pending = ledger.pending_atoms(miner_address);
    println!("miner={miner_address}  pending_net_atoms={pending}");
    Ok(())
}

fn cmd_mark_paid(miner_address: &str) -> Result<(), String> {
    let path = PathBuf::from(
        env::var("TENSORIUM_POOL_LEDGER").unwrap_or_else(|_| DEFAULT_LEDGER_PATH.to_owned()),
    );
    let mut ledger = PayoutLedger::load(&path);
    ledger.mark_paid(miner_address);
    ledger.save(&path)?;
    println!("marked all entries for {miner_address} as paid");
    Ok(())
}

fn load_ledger_from_env() -> PayoutLedger {
    let path = PathBuf::from(
        env::var("TENSORIUM_POOL_LEDGER").unwrap_or_else(|_| DEFAULT_LEDGER_PATH.to_owned()),
    );
    PayoutLedger::load(&path)
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

fn print_help() {
    println!("tensorium-pool — reference mining pool (5% fee)\n");
    println!("commands:");
    println!("  serve         start pool HTTP server (default)");
    println!("  stats         print ledger summary");
    println!("  accounting    print full payout ledger");
    println!("  custody       print configured treasury/hot-wallet metadata");
    println!("  pending <addr>     pending payout for miner");
    println!("  mark-paid <addr>   mark miner payouts as sent");
    println!();
    println!("env:");
    println!("  TENSORIUM_POOL_BIND      pool listen address, default {DEFAULT_POOL_BIND}");
    println!("  TENSORIUM_NODE_RPC       upstream node RPC, default {DEFAULT_NODE_RPC}");
    println!("  TENSORIUM_POOL_TREASURY  pool treasury wallet address (required)");
    println!("  TENSORIUM_POOL_PAYOUT_HOT_WALLET   operational payout hot wallet address");
    println!("  TENSORIUM_POOL_PAYOUT_HOT_MAX_ATOMS  soft cap for payout hot wallet balance");
    println!("  TENSORIUM_POOL_LEDGER    payout ledger JSON path, default {DEFAULT_LEDGER_PATH}");
    println!();
    println!("pool fee: {}%  ({} bps)", accounting::POOL_FEE_BPS / 100, accounting::POOL_FEE_BPS);
    println!();
    println!("miners: point txmminer-cuda (GPU) at pool bind address instead of node RPC.");
    println!("  txmminer (CPU) is dev/diagnostic only — cannot mine at mainnet difficulty.");
    println!("example:");
    println!("  TENSORIUM_POOL_TREASURY=txm1treasury... TENSORIUM_POOL_PAYOUT_HOT_WALLET=txm1hot... tensorium-pool serve");
    println!("  txmminer-cuda <pool_host:23336> <miner_wallet_address>");
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|e| format!("write response: {e}"))
}

fn extract_body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

fn http_get(rpc: &str, path: &str) -> Result<String, String> {
    send_http(
        rpc,
        &format!("GET {path} HTTP/1.1\r\nhost: {rpc}\r\nconnection: close\r\n\r\n"),
    )
}

fn http_post(rpc: &str, path: &str, body: &str) -> Result<String, String> {
    send_http(
        rpc,
        &format!(
            "POST {path} HTTP/1.1\r\nhost: {rpc}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn send_http(rpc: &str, request: &str) -> Result<String, String> {
    let mut stream =
        TcpStream::connect(rpc).map_err(|e| format!("connect {rpc}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(HTTP_TIMEOUT_SECS)))
        .ok();
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("send: {e}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("read: {e}"))?;

    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or("invalid HTTP response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("RPC error: {body}"));
    }
    Ok(body.to_owned())
}
