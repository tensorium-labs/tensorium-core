use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tensorium_core::{
    block::{BlockHeader, Transaction},
    chain::{MAINNET_CANDIDATE, TESTNET},
    pow::header_meets_work,
    Block, ChainState, Hash256, Mempool, StateError, UtxoSet,
};

const DEFAULT_STATE_PATH: &str = "tensorium-testnet-state.json";
const DEFAULT_MEMPOOL_PATH: &str = "tensorium-testnet-mempool.json";
const DEFAULT_BAN_PATH: &str = "tensorium-testnet-banlist.json";
const DEFAULT_NONCE_LIMIT: u64 = u64::MAX;
const DEFAULT_RPC_BIND: &str = "127.0.0.1:23332";
const DEFAULT_P2P_BIND: &str = "127.0.0.1:23333";

// Mainnet-candidate defaults (different ports so testnet and mc can coexist)
const DEFAULT_MC_STATE_PATH: &str = "tensorium-mc-state.json";
const DEFAULT_MC_MEMPOOL_PATH: &str = "tensorium-mc-mempool.json";
const DEFAULT_MC_BAN_PATH: &str = "tensorium-mc-banlist.json";
const DEFAULT_MC_RPC_BIND: &str = "127.0.0.1:33332";
const DEFAULT_MC_P2P_BIND: &str = "0.0.0.0:33333";
/// Genesis timestamp for the mainnet-candidate chain (2026-06-01 00:00:00 UTC).
/// All nodes MUST use this exact value to share the same genesis block.
const MC_GENESIS_TIMESTAMP: u64 = 1_780_272_000;
const P2P_PROTOCOL_VERSION: u32 = 1;
/// Maximum blocks returned per GetBlocks response.
const SYNC_BATCH_SIZE: usize = 50;
/// Maximum newline-delimited P2P message size. Keeps malformed peers from
/// growing an unbounded buffer before JSON parsing.
const MAX_P2P_LINE_BYTES: usize = 1_048_576;
/// Maximum concurrent inbound P2P connections. Prevents thread exhaustion
/// under a connection-flood DoS.
const MAX_INBOUND_PEERS: usize = 64;
/// Seconds before a P2P read or write operation times out. Keeps a slow or
/// dead peer from holding a thread indefinitely.
const P2P_IO_TIMEOUT_SECS: u64 = 30;
/// Seconds before an RPC read operation times out. Guards against slow HTTP
/// clients that never finish sending the request.
const RPC_READ_TIMEOUT_SECS: u64 = 10;

// ---------------------------------------------------------------------------
// Peer ban constants
// ---------------------------------------------------------------------------
/// Total score at which a peer becomes banned.
const BAN_THRESHOLD: u32 = 100;
/// How long a ban lasts in seconds (1 hour).
const BAN_DURATION_SECS: u64 = 3_600;
/// Score added per invalid / tampered block.
const SCORE_INVALID_BLOCK: u32 = 20;
/// Score added per invalid transaction (signature failure etc.).
const SCORE_INVALID_TX: u32 = 10;
/// Score added per unparseable P2P message.
const SCORE_INVALID_MSG: u32 = 2;
/// Score added for a bad handshake (wrong chain_id / protocol / version).
const SCORE_BAD_HANDSHAKE: u32 = 100;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    let state_path = state_path_from_env();

    match command {
        "init" => {
            let mut state = ChainState::new();
            // Genesis nonce pre-mined via CUDA (RTX 3060, 369 MH/s, 173.5s).
            // Diff-36 nonce = 64092008986. Verified against expected genesis hash.
            // Users do NOT need to mine genesis — init is instant.
            const GENESIS_NONCE: u64 = 64_092_008_986;
            state
                .init_genesis_nonce(&TESTNET, 1_748_649_600, GENESIS_NONCE)
                .map_err(|err| err.to_string())?;
            save_state(&state_path, &state)?;
            print_status(&state);
        }
        "status" => {
            let state = load_state(&state_path)?;
            print_status(&state);
        }
        "mine-once" => {
            let mut state = load_state(&state_path)?;
            let miner = args.get(2).map(String::as_str).unwrap_or("local-dev-miner");
            state
                .mine_next_block(&TESTNET, now_seconds(), miner, DEFAULT_NONCE_LIMIT)
                .map_err(|err| err.to_string())?;
            save_state(&state_path, &state)?;
            print_status(&state);
        }
        "rpc" => {
            let bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_RPC_BIND);
            serve_rpc(bind, state_path)?;
        }
        "p2p-listen" => {
            let bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_P2P_BIND);
            serve_p2p(bind, state_path)?;
        }
        "p2p-connect" => {
            let peer = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-node p2p-connect <host:port>".to_owned())?;
            connect_peer(peer, &state_path)?;
        }
        "sync" => {
            let peers = configured_peers();
            let peer = args
                .get(2)
                .map(|s| s.as_str())
                .or_else(|| peers.first().map(|s| s.as_str()))
                .ok_or_else(|| {
                    "usage: tensorium-node sync <peer>  (or set TENSORIUM_PEERS; disable built-in seeds with TENSORIUM_NO_DEFAULT_SEEDS=1)".to_owned()
                })?;
            sync_from_peer(peer, &state_path)?;
        }
        "peers" => print_manual_peers(),
        "banlist" => print_banlist(),
        "unban" => {
            let ip = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-node unban <ip>".to_owned())?;
            unban_ip(ip)?;
        }
        "mainnet-candidate" | "mc" => {
            let subcmd = args.get(2).map(String::as_str).unwrap_or("help");
            match subcmd {
                "init" => {
                    let nonce_str = args.get(3).ok_or_else(|| {
                        "usage: tensorium-node mainnet-candidate init <genesis_nonce>".to_owned()
                    })?;
                    let nonce: u64 = nonce_str
                        .parse()
                        .map_err(|_| format!("invalid nonce: {nonce_str}"))?;
                    let mc_state = mc_state_path_from_env();
                    let mut state = ChainState::new();
                    state
                        .init_genesis_nonce(&MAINNET_CANDIDATE, MC_GENESIS_TIMESTAMP, nonce)
                        .map_err(|err| err.to_string())?;
                    save_state(&mc_state, &state)?;
                    println!("mainnet-candidate genesis initialized");
                    print_status(&state);
                }
                "mine-genesis" => {
                    let threads = args
                        .get(3)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or_else(|| {
                            thread::available_parallelism()
                                .map(|n| n.get())
                                .unwrap_or(4)
                        });
                    println!(
                        "Mining mainnet-candidate genesis: diff={} bits, threads={}, timestamp={}",
                        MAINNET_CANDIDATE.initial_leading_zero_bits,
                        threads,
                        MC_GENESIS_TIMESTAMP
                    );
                    println!("This may take hours on CPU — use txmminer-cuda for GPU acceleration.");
                    let nonce = mine_genesis_multithreaded(threads)?;
                    let mut state = ChainState::new();
                    state
                        .init_genesis_nonce(&MAINNET_CANDIDATE, MC_GENESIS_TIMESTAMP, nonce)
                        .map_err(|err| err.to_string())?;
                    let mc_state = mc_state_path_from_env();
                    save_state(&mc_state, &state)?;
                    println!("GENESIS NONCE: {nonce}  (hardcode this in node binary for v1 release)");
                    print_status(&state);
                }
                "status" => {
                    let mc_state = mc_state_path_from_env();
                    let state = load_state(&mc_state)?;
                    print_status(&state);
                }
                _ => print_help_mc(),
            }
        }
        _ => print_help(),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Paths and env helpers
// ---------------------------------------------------------------------------

fn state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_STATE_PATH))
}

fn mempool_path_from_env() -> PathBuf {
    env::var("TENSORIUM_MEMPOOL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MEMPOOL_PATH))
}

fn ban_path_from_env() -> PathBuf {
    env::var("TENSORIUM_BANS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BAN_PATH))
}

fn mc_state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_MC_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_MC_STATE_PATH))
}

/// Multi-threaded CPU nonce search for the mainnet-candidate genesis block.
/// Returns the first nonce that satisfies MAINNET_CANDIDATE difficulty.
fn mine_genesis_multithreaded(threads: usize) -> Result<u64, String> {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    // Build a template genesis header (nonce = 0; we'll search the space).
    let header_template = {
        let params = &MAINNET_CANDIDATE;
        BlockHeader {
            version: 1,
            chain_id: params.chain_id.to_owned(),
            height: 0,
            previous_hash: Hash256::ZERO,
            merkle_root: Hash256::ZERO, // not used for PoW check
            timestamp_seconds: MC_GENESIS_TIMESTAMP,
            leading_zero_bits: params.initial_leading_zero_bits,
            nonce: 0,
        }
    };

    let done = Arc::new(AtomicBool::new(false));
    let winner = Arc::new(AtomicU64::new(u64::MAX));
    let total = Arc::new(AtomicU64::new(0));
    let stride = threads as u64;
    let started = std::time::Instant::now();

    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let mut h = header_template.clone();
            let done = done.clone();
            let winner = winner.clone();
            let total = total.clone();
            let start = t as u64;

            thread::spawn(move || {
                let mut nonce = start;
                let mut local = 0u64;
                const FLUSH: u64 = 1_000_000;

                loop {
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    h.nonce = nonce;
                    local += 1;
                    if header_meets_work(&h) {
                        done.store(true, Ordering::SeqCst);
                        total.fetch_add(local, Ordering::Relaxed);
                        winner.store(nonce, Ordering::SeqCst);
                        return;
                    }
                    if local == FLUSH {
                        total.fetch_add(FLUSH, Ordering::Relaxed);
                        local = 0;
                        // Print progress every ~10M hashes per thread
                        let t_hashes = total.load(Ordering::Relaxed);
                        if t_hashes % (10_000_000 * threads as u64) < FLUSH {
                            let elapsed = started.elapsed().as_secs_f64().max(0.001);
                            let mhs = t_hashes as f64 / elapsed / 1e6;
                            eprint!("\r{:.0}M hashes, {:.2} MH/s …", t_hashes / 1_000_000, mhs);
                        }
                    }
                    nonce = match nonce.checked_add(stride) {
                        Some(n) => n,
                        None => {
                            total.fetch_add(local, Ordering::Relaxed);
                            break;
                        }
                    };
                }
            })
        })
        .collect();

    for h in handles {
        h.join().ok();
    }

    let nonce = winner.load(Ordering::SeqCst);
    if nonce == u64::MAX {
        return Err("nonce space exhausted without finding genesis — impossible at diff < 64".to_owned());
    }
    let elapsed = started.elapsed();
    let hashes = total.load(Ordering::Relaxed);
    println!(
        "\nGenesis found!  nonce={}  hashes={}  time={:.1}s",
        nonce,
        hashes,
        elapsed.as_secs_f64()
    );
    Ok(nonce)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs()
}

// ---------------------------------------------------------------------------
// State persistence
// ---------------------------------------------------------------------------

fn load_state(path: &Path) -> Result<ChainState, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut state: ChainState = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    // Populate block_map from the canonical chain for state files written
    // before fork-choice support was added.
    state.ensure_block_map();
    Ok(state)
}

fn save_state(path: &Path, state: &ChainState) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(state)
        .map_err(|err| format!("failed to serialize chain state: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Load mempool, or return an empty one if the file does not exist yet.
fn load_mempool(path: &Path) -> Mempool {
    let Ok(raw) = fs::read_to_string(path) else {
        return Mempool::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| Mempool::new())
}

fn save_mempool(path: &Path, mempool: &Mempool) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(mempool)
        .map_err(|err| format!("failed to serialize mempool: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Build a full UTXO set by replaying all blocks in `state`.
fn build_utxo_set(state: &ChainState) -> Result<UtxoSet, String> {
    let mut utxos = UtxoSet::new();
    for block in &state.blocks {
        utxos
            .apply_block(&TESTNET, block)
            .map_err(|err| format!("UTXO apply failed: {err}"))?;
    }
    Ok(utxos)
}

// ---------------------------------------------------------------------------
// Ban list — persistent peer reputation tracking
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BanEntry {
    /// Accumulated violation score.
    score: u32,
    /// Unix timestamp after which the ban expires; `None` means not yet banned.
    banned_until: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BanList {
    entries: std::collections::HashMap<String, BanEntry>,
}

impl BanList {
    fn is_banned(&self, ip: &str, now: u64) -> bool {
        self.entries
            .get(ip)
            .and_then(|e| e.banned_until)
            .map_or(false, |until| until > now)
    }

    /// Add `score` to `ip`'s tally.  Returns `true` when a new ban is imposed.
    fn record(&mut self, ip: &str, score: u32, now: u64) -> bool {
        let entry = self.entries.entry(ip.to_owned()).or_default();
        entry.score = entry.score.saturating_add(score);
        let already_banned = entry.banned_until.map_or(false, |u| u > now);
        if !already_banned && entry.score >= BAN_THRESHOLD {
            entry.banned_until = Some(now + BAN_DURATION_SECS);
            return true;
        }
        false
    }

    fn unban(&mut self, ip: &str) {
        self.entries.remove(ip);
    }

    fn prune_expired(&mut self, now: u64) {
        // Keep entries with no ban yet (score accumulation only) and entries
        // with an active ban.  Only remove entries whose ban has expired —
        // using map_or(false, …) would accidentally wipe sub-threshold score
        // entries, preventing peers from ever reaching the ban threshold.
        self.entries
            .retain(|_, e| e.banned_until.map_or(true, |u| u > now));
    }
}

fn load_banlist(path: &Path) -> BanList {
    let Ok(raw) = fs::read_to_string(path) else {
        return BanList::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_banlist(path: &Path, banlist: &BanList) {
    if let Ok(raw) = serde_json::to_string_pretty(banlist) {
        let _ = fs::write(path, raw);
    }
}

/// Record a violation for `ip` and persist.  Returns `true` if a ban was
/// just imposed so the caller can close the connection.
fn record_violation(ban_path: &Path, ip: &str, score: u32) -> bool {
    let mut banlist = load_banlist(ban_path);
    let now = now_seconds();
    banlist.prune_expired(now);
    let banned = banlist.record(ip, score, now);
    if banned {
        eprintln!(
            "p2p ban imposed on {ip} \
             (score>={BAN_THRESHOLD} threshold, duration={BAN_DURATION_SECS}s)"
        );
    }
    save_banlist(ban_path, &banlist);
    banned
}

/// Extract just the IP (no port) from a connected stream.
fn peer_ip(stream: &TcpStream) -> String {
    stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// Returns `true` when a block rejection reason is worth penalising.
/// `AlreadyKnown` and `UnknownParent` are not the peer's fault.
fn is_bannable_block_error(reason: &str) -> bool {
    !reason.contains("already known") && !reason.contains("parent")
}

/// Returns `true` when a tx rejection reason is worth penalising.
fn is_bannable_tx_error(reason: &str) -> bool {
    reason.contains("signature") || reason.contains("invalid")
}

fn print_banlist() {
    let banlist = load_banlist(&ban_path_from_env());
    let now = now_seconds();
    if banlist.entries.is_empty() {
        println!("no peers in ban list");
        return;
    }
    for (ip, entry) in &banlist.entries {
        let status = match entry.banned_until {
            Some(until) if until > now => {
                let secs_left = until - now;
                format!("BANNED (expires in {secs_left}s)")
            }
            Some(_) => "expired".to_owned(),
            None => format!("score={} (not banned yet)", entry.score),
        };
        println!("{ip}: {status}  score={}", entry.score);
    }
}

fn unban_ip(ip: &str) -> Result<(), String> {
    let ban_path = ban_path_from_env();
    let mut banlist = load_banlist(&ban_path);
    if banlist.entries.contains_key(ip) {
        banlist.unban(ip);
        save_banlist(&ban_path, &banlist);
        println!("unbanned {ip}");
    } else {
        println!("{ip} was not in the ban list");
    }
    Ok(())
}

fn print_status(state: &ChainState) {
    let Some(tip) = state.tip() else {
        println!("chain_id={} height=empty", TESTNET.chain_id);
        return;
    };
    println!(
        "chain_id={} height={} tip={} difficulty_bits={} blocks={}",
        tip.header.chain_id,
        tip.header.height,
        tip.hash(),
        tip.header.leading_zero_bits,
        state.blocks.len()
    );
}

fn print_help() {
    println!("tensorium-node <command>");
    println!();
    println!("commands:");
    println!("  init                 create local testnet genesis state");
    println!("  status               show local chain status");
    println!("  mine-once [miner]    mine one block and persist it");
    println!("  rpc [bind]           start localhost HTTP RPC server");
    println!("  p2p-listen [bind]    listen for peer connections and messages");
    println!("  p2p-connect <peer>   connect to a peer for diagnostics");
    println!("  sync [peer]          pull missing blocks from a peer");
    println!("  peers                print manual peers from TENSORIUM_PEERS");
    println!("  banlist              show peer ban list");
    println!("  unban <ip>           remove a peer from the ban list");
    println!();
    println!("rpc endpoints:");
    println!("  GET  /health");
    println!("  GET  /getblockcount");
    println!("  GET  /getdifficulty");
    println!("  GET  /getblock/<height>");
    println!("  GET  /getblocktemplate/<miner>   (includes mempool txs)");
    println!("  POST /submitblock                 (broadcasts to peers, cleans mempool)");
    println!("  POST /sendrawtransaction          (validates, pools, broadcasts to peers)");
    println!("  GET  /getmempoolinfo");
    println!("  GET  /getbanlist");
    println!("  GET  /unban/<ip>                  (remove ban)");
    println!();
    println!("env:");
    println!("  TENSORIUM_STATE      state file path, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_MEMPOOL    mempool file path, default {DEFAULT_MEMPOOL_PATH}");
    println!("  TENSORIUM_BANS       ban list file path, default {DEFAULT_BAN_PATH}");
    println!("  TENSORIUM_PEERS      comma-separated peers to broadcast to (overrides built-in seeds)");
    println!("  TENSORIUM_NO_DEFAULT_SEEDS=1  disable built-in seed list");
    println!("  TENSORIUM_NODE_ID    node identity string");
    println!("  TENSORIUM_RPC_ALLOW_PUBLIC=1  allow non-loopback RPC bind");
}

fn print_help_mc() {
    println!("tensorium-node mainnet-candidate <subcommand>\n");
    println!("subcommands:");
    println!("  init <genesis_nonce>    initialize mc state with a pre-mined genesis nonce");
    println!("  mine-genesis [threads]  CPU-mine the mc genesis nonce (may take hours; use GPU)");
    println!("  status                  show mc chain status");
    println!();
    println!("mainnet-candidate params:");
    println!("  chain_id       = {}", MAINNET_CANDIDATE.chain_id);
    println!("  initial_diff   = {} bits (2^{} hashes/block expected)", MAINNET_CANDIDATE.initial_leading_zero_bits, MAINNET_CANDIDATE.initial_leading_zero_bits);
    println!("  target_block   = {}s ({}min)", MAINNET_CANDIDATE.target_block_seconds, MAINNET_CANDIDATE.target_block_seconds / 60);
    println!("  halving        = every {} blocks (~{} years)", MAINNET_CANDIDATE.halving_interval_blocks, MAINNET_CANDIDATE.halving_interval_blocks / 525_600);
    println!("  genesis_ts     = {MC_GENESIS_TIMESTAMP}  (2026-06-01 00:00:00 UTC)");
    println!("  rpc_default    = {DEFAULT_MC_RPC_BIND}");
    println!("  p2p_default    = {DEFAULT_MC_P2P_BIND}");
    println!();
    println!("env:");
    println!("  TENSORIUM_MC_STATE    mc state file path, default {DEFAULT_MC_STATE_PATH}");
    println!();
    println!("NOTE: Full mainnet-candidate RPC/P2P daemon support is planned.");
    println!("      Genesis nonce must be GPU-mined before mainnet-candidate chain launch.");
}

// ---------------------------------------------------------------------------
// P2P message protocol
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct P2pHello {
    protocol: String,
    version: u32,
    chain_id: String,
    node_id: String,
    height: u64,
    tip_hash: Hash256,
}

/// Messages exchanged after the initial hello handshake.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum P2pMsg {
    // --- block propagation ---
    NewBlock {
        block: Box<Block>,
    },
    Ack {
        height: u64,
        hash: Hash256,
    },
    Reject {
        reason: String,
    },
    // --- transaction propagation ---
    NewTx {
        tx: Box<Transaction>,
    },
    TxAck {
        txid: Hash256,
    },
    TxReject {
        txid: Hash256,
        reason: String,
    },
    // --- chain sync ---
    /// Request up to SYNC_BATCH_SIZE blocks starting at `from_height`.
    GetBlocks {
        from_height: u64,
    },
    /// Response to GetBlocks; empty vec means "no more blocks".
    Blocks {
        blocks: Vec<Block>,
    },
}

/// Read one newline-terminated line from `stream` byte-by-byte.
fn read_p2p_line(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err("peer closed connection".to_owned()),
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
                if buf.len() > MAX_P2P_LINE_BYTES {
                    return Err(format!(
                        "p2p message too large: {} bytes exceeds limit {}",
                        buf.len(),
                        MAX_P2P_LINE_BYTES
                    ));
                }
            }
            Err(err) => return Err(format!("read from peer: {err}")),
        }
    }
    String::from_utf8(buf).map_err(|err| format!("p2p invalid utf8: {err}"))
}

fn write_p2p_line<T: Serialize>(stream: &mut TcpStream, value: &T) -> Result<(), String> {
    let mut raw =
        serde_json::to_vec(value).map_err(|err| format!("failed to encode p2p message: {err}"))?;
    raw.push(b'\n');
    stream
        .write_all(&raw)
        .map_err(|err| format!("failed to write p2p message: {err}"))
}

fn local_hello(state: &ChainState) -> P2pHello {
    let node_id =
        env::var("TENSORIUM_NODE_ID").unwrap_or_else(|_| format!("node-{}", now_seconds()));
    let (height, tip_hash) = state
        .tip()
        .map(|tip| (tip.header.height, tip.hash()))
        .unwrap_or((0, Hash256::ZERO));
    P2pHello {
        protocol: "tensorium-p2p".to_owned(),
        version: P2P_PROTOCOL_VERSION,
        chain_id: TESTNET.chain_id.to_owned(),
        node_id,
        height,
        tip_hash,
    }
}

fn validate_hello(hello: &P2pHello) -> Result<(), String> {
    if hello.protocol != "tensorium-p2p" {
        return Err(format!("unsupported P2P protocol: {}", hello.protocol));
    }
    if hello.version != P2P_PROTOCOL_VERSION {
        return Err(format!("unsupported P2P version: {}", hello.version));
    }
    if hello.chain_id != TESTNET.chain_id {
        return Err(format!(
            "wrong chain_id: {} (expected {})",
            hello.chain_id, TESTNET.chain_id
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// P2P server — accepts inbound connections and processes messages
// ---------------------------------------------------------------------------

fn serve_p2p(bind: &str, state_path: PathBuf) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    println!("tensorium P2P listening on {bind}");
    let ban_path = ban_path_from_env();
    let peer_count = Arc::new(AtomicUsize::new(0));

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let remote_ip = peer_ip(&stream);

                // Enforce inbound connection limit before spawning a thread.
                let current = peer_count.load(Ordering::Relaxed);
                if current >= MAX_INBOUND_PEERS {
                    eprintln!(
                        "p2p connection limit reached ({current}/{MAX_INBOUND_PEERS}), \
                         refusing ip={remote_ip}"
                    );
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    continue;
                }

                // Reject connections from banned peers before spawning a thread.
                let banlist = load_banlist(&ban_path);
                if banlist.is_banned(&remote_ip, now_seconds()) {
                    eprintln!("p2p refused banned peer ip={remote_ip}");
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    continue;
                }

                // Apply I/O timeouts so a slow or dead peer does not hold a
                // thread forever.
                let timeout = Some(Duration::from_secs(P2P_IO_TIMEOUT_SECS));
                let _ = stream.set_read_timeout(timeout);
                let _ = stream.set_write_timeout(timeout);

                let path = state_path.clone();
                let bans = ban_path.clone();
                let count = Arc::clone(&peer_count);
                count.fetch_add(1, Ordering::Relaxed);
                thread::spawn(move || {
                    if let Err(err) = handle_p2p_connection(&mut stream, &path, &bans) {
                        eprintln!("p2p connection error from {remote_ip}: {err}");
                    }
                    count.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(err) => eprintln!("p2p accept error: {err}"),
        }
    }
    Ok(())
}

/// Full lifecycle of a single inbound P2P connection.
///
/// Enforces the peer ban policy:
/// - Wrong handshake (chain_id / protocol / version) → SCORE_BAD_HANDSHAKE (instant ban)
/// - Unparseable message → SCORE_INVALID_MSG
/// - Invalid block (bad PoW / tampered) → SCORE_INVALID_BLOCK
/// - Invalid transaction (bad signature etc.) → SCORE_INVALID_TX
///
/// The connection is closed as soon as a ban is imposed.
fn handle_p2p_connection(
    stream: &mut TcpStream,
    state_path: &Path,
    ban_path: &Path,
) -> Result<(), String> {
    let remote_ip = peer_ip(stream);
    let mempool_path = mempool_path_from_env();

    // --- handshake ---
    let line = match read_p2p_line(stream) {
        Ok(l) => l,
        Err(err) => {
            record_violation(ban_path, &remote_ip, SCORE_INVALID_MSG);
            return Err(format!("read hello from {remote_ip}: {err}"));
        }
    };

    let remote: P2pHello = match serde_json::from_str(&line) {
        Ok(h) => h,
        Err(err) => {
            record_violation(ban_path, &remote_ip, SCORE_INVALID_MSG);
            return Err(format!("parse hello from {remote_ip}: {err}"));
        }
    };

    if let Err(err) = validate_hello(&remote) {
        // Wrong chain_id, version, or protocol — potentially an attacker or
        // a node on the wrong network.  Instant ban.
        record_violation(ban_path, &remote_ip, SCORE_BAD_HANDSHAKE);
        return Err(format!(
            "handshake rejected from {remote_ip} ({}): {err}",
            remote.node_id
        ));
    }

    let state = load_state(state_path)?;
    write_p2p_line(stream, &local_hello(&state))?;

    println!(
        "p2p accepted peer={} ip={remote_ip} chain_id={} height={} tip={}",
        remote.node_id, remote.chain_id, remote.height, remote.tip_hash
    );

    // --- message loop ---
    loop {
        let line = match read_p2p_line(stream) {
            Ok(line) => line,
            Err(_) => break,
        };

        let msg: P2pMsg = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(err) => {
                eprintln!(
                    "p2p invalid message from {} (ip={remote_ip}): {err}",
                    remote.node_id
                );
                if record_violation(ban_path, &remote_ip, SCORE_INVALID_MSG) {
                    break; // newly banned — close connection
                }
                continue;
            }
        };

        match msg {
            P2pMsg::NewBlock { block } => {
                match accept_peer_block(state_path, *block) {
                    Ok((height, hash)) => {
                        println!(
                            "p2p accepted block from {} height={height} hash={hash}",
                            remote.node_id
                        );
                        let _ = write_p2p_line(stream, &P2pMsg::Ack { height, hash });
                    }
                    Err(ref reason) if !is_bannable_block_error(reason) => {
                        // AlreadyKnown / UnknownParent — not the peer's fault.
                        let _ = write_p2p_line(
                            stream,
                            &P2pMsg::Reject {
                                reason: reason.clone(),
                            },
                        );
                    }
                    Err(reason) => {
                        eprintln!(
                            "p2p rejected block from {} (ip={remote_ip}): {reason}",
                            remote.node_id
                        );
                        let _ = write_p2p_line(
                            stream,
                            &P2pMsg::Reject {
                                reason: reason.clone(),
                            },
                        );
                        if record_violation(ban_path, &remote_ip, SCORE_INVALID_BLOCK) {
                            break; // newly banned
                        }
                    }
                }
            }
            P2pMsg::NewTx { tx } => {
                let txid = tx.id;
                match accept_peer_tx(&state_path, &mempool_path, *tx) {
                    Ok(()) => {
                        println!("p2p accepted tx from {} txid={txid}", remote.node_id);
                        let _ = write_p2p_line(stream, &P2pMsg::TxAck { txid });
                    }
                    Err(ref reason) if !is_bannable_tx_error(reason) => {
                        // AlreadyKnown / missing UTXO — not necessarily hostile.
                        let _ = write_p2p_line(
                            stream,
                            &P2pMsg::TxReject {
                                txid,
                                reason: reason.clone(),
                            },
                        );
                    }
                    Err(reason) => {
                        eprintln!(
                            "p2p rejected tx from {} (ip={remote_ip}): {reason}",
                            remote.node_id
                        );
                        let _ = write_p2p_line(
                            stream,
                            &P2pMsg::TxReject {
                                txid,
                                reason: reason.clone(),
                            },
                        );
                        if record_violation(ban_path, &remote_ip, SCORE_INVALID_TX) {
                            break; // newly banned
                        }
                    }
                }
            }
            P2pMsg::GetBlocks { from_height } => {
                let batch = match load_state(state_path) {
                    Ok(state) => state
                        .blocks
                        .iter()
                        .filter(|b| b.header.height >= from_height)
                        .take(SYNC_BATCH_SIZE)
                        .cloned()
                        .collect::<Vec<_>>(),
                    Err(err) => {
                        eprintln!("getblocks: load state error: {err}");
                        vec![]
                    }
                };
                let count = batch.len();
                let _ = write_p2p_line(stream, &P2pMsg::Blocks { blocks: batch });
                println!(
                    "p2p getblocks from={from_height} sent={count} to {}",
                    remote.node_id
                );
            }
            P2pMsg::Ack { .. }
            | P2pMsg::Reject { .. }
            | P2pMsg::TxAck { .. }
            | P2pMsg::TxReject { .. }
            | P2pMsg::Blocks { .. } => {
                // Response-type messages should never be sent to a listener.
                eprintln!(
                    "p2p unexpected response-type message from {} (ip={remote_ip})",
                    remote.node_id
                );
                if record_violation(ban_path, &remote_ip, SCORE_INVALID_MSG) {
                    break;
                }
            }
        }
    }

    println!("p2p disconnected peer={} ip={remote_ip}", remote.node_id);
    Ok(())
}

fn accept_peer_block(state_path: &Path, block: Block) -> Result<(u64, Hash256), String> {
    let block_height = block.header.height;
    let block_hash = block.hash();
    let mut state = load_state(state_path)?;

    match state.submit_block(&TESTNET, block.clone(), now_seconds()) {
        Ok(_) => {}
        Err(StateError::AlreadyKnown) => {
            // We already have this block — acknowledge without re-saving.
            return Ok((block_height, block_hash));
        }
        Err(err) => return Err(err.to_string()),
    }

    save_state(state_path, &state)?;

    // Clean mempool of transactions confirmed by this block.
    let mempool_path = mempool_path_from_env();
    let mut mempool = load_mempool(&mempool_path);
    mempool.remove_confirmed(&block);
    let _ = save_mempool(&mempool_path, &mempool);

    Ok((block_height, block_hash))
}

fn accept_peer_tx(state_path: &Path, mempool_path: &Path, tx: Transaction) -> Result<(), String> {
    let state = load_state(state_path)?;
    let utxos = build_utxo_set(&state)?;
    let tip_height = state.height().unwrap_or(0);
    let mut mempool = load_mempool(mempool_path);
    mempool
        .add(&utxos, &TESTNET, tx, tip_height)
        .map_err(|err| err.to_string())?;
    save_mempool(mempool_path, &mempool)
}

// ---------------------------------------------------------------------------
// P2P client — push a block or transaction to a single peer
// ---------------------------------------------------------------------------

fn push_block_to_peer(peer: &str, block: &Block, state: &ChainState) -> Result<u64, String> {
    let mut stream = TcpStream::connect(peer).map_err(|err| format!("connect {peer}: {err}"))?;

    write_p2p_line(&mut stream, &local_hello(state))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote)?;

    write_p2p_line(
        &mut stream,
        &P2pMsg::NewBlock {
            block: Box::new(block.clone()),
        },
    )?;

    let line = read_p2p_line(&mut stream)?;
    let response: P2pMsg =
        serde_json::from_str(&line).map_err(|err| format!("parse response from {peer}: {err}"))?;

    match response {
        P2pMsg::Ack { height, .. } => Ok(height),
        P2pMsg::Reject { reason } => Err(format!("block rejected by {peer}: {reason}")),
        other => Err(format!("unexpected response from {peer}: {other:?}")),
    }
}

fn push_tx_to_peer(peer: &str, tx: &Transaction, state: &ChainState) -> Result<Hash256, String> {
    let mut stream = TcpStream::connect(peer).map_err(|err| format!("connect {peer}: {err}"))?;

    write_p2p_line(&mut stream, &local_hello(state))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote)?;

    write_p2p_line(
        &mut stream,
        &P2pMsg::NewTx {
            tx: Box::new(tx.clone()),
        },
    )?;

    let line = read_p2p_line(&mut stream)?;
    let response: P2pMsg =
        serde_json::from_str(&line).map_err(|err| format!("parse response from {peer}: {err}"))?;

    match response {
        P2pMsg::TxAck { txid } => Ok(txid),
        P2pMsg::TxReject { reason, .. } => Err(format!("tx rejected by {peer}: {reason}")),
        other => Err(format!("unexpected response from {peer}: {other:?}")),
    }
}

/// Broadcast a block to every configured peer.  Per-peer errors are logged.
fn broadcast_block_to_peers(block: &Block, state: &ChainState) {
    let peers = configured_peers();
    for peer in &peers {
        match push_block_to_peer(peer, block, state) {
            Ok(height) => println!("broadcast block to {peer} accepted height={height}"),
            Err(err) => eprintln!("broadcast block to {peer} failed: {err}"),
        }
    }
}

/// Broadcast a transaction to every configured peer.  Per-peer errors are logged.
fn broadcast_tx_to_peers(tx: &Transaction, state: &ChainState) {
    let peers = configured_peers();
    for peer in &peers {
        match push_tx_to_peer(peer, tx, state) {
            Ok(txid) => println!("broadcast tx to {peer} accepted txid={txid}"),
            Err(err) => eprintln!("broadcast tx to {peer} failed: {err}"),
        }
    }
}

/// Built-in testnet seed nodes. Used when TENSORIUM_PEERS is unset and
/// TENSORIUM_NO_DEFAULT_SEEDS is not set.
const DEFAULT_SEEDS: &[&str] = &["157.230.44.162:23333"];

fn configured_peers() -> Vec<String> {
    let raw = env::var("TENSORIUM_PEERS").unwrap_or_default();
    let manual: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if !manual.is_empty() {
        return manual;
    }
    // Fall back to built-in seeds unless operator explicitly opts out.
    if env::var("TENSORIUM_NO_DEFAULT_SEEDS").is_ok() {
        return vec![];
    }
    DEFAULT_SEEDS.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// p2p-connect — diagnostic handshake
// ---------------------------------------------------------------------------

fn connect_peer(peer: &str, state_path: &Path) -> Result<(), String> {
    let state = load_state(state_path)?;
    let mut stream =
        TcpStream::connect(peer).map_err(|err| format!("failed to connect to {peer}: {err}"))?;

    write_p2p_line(&mut stream, &local_hello(&state))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello: {err}"))?;
    validate_hello(&remote)?;

    println!(
        "connected peer={} chain_id={} height={} tip={}",
        peer, remote.chain_id, remote.height, remote.tip_hash
    );
    Ok(())
}

fn print_manual_peers() {
    let peers = configured_peers();
    if peers.is_empty() {
        println!("manual_peers=[]");
        return;
    }
    for peer in &peers {
        println!("{peer}");
    }
}

/// Download all blocks that `peer` has but we do not.
///
/// Prerequisites:
/// - `init` must have been run first so we share the same genesis.
/// - `peer` must be running `p2p-listen`.
///
/// Blocks are fetched in batches of SYNC_BATCH_SIZE, validated against our
/// local chain, and persisted after each successful batch.
fn sync_from_peer(peer: &str, state_path: &Path) -> Result<(), String> {
    let mut state = load_state(state_path)?;
    let our_height = state.height().unwrap_or(0);

    // --- handshake ---
    let mut stream =
        TcpStream::connect(peer).map_err(|err| format!("failed to connect to {peer}: {err}"))?;

    write_p2p_line(&mut stream, &local_hello(&state))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote)?;

    if remote.height <= our_height {
        println!(
            "already up to date: our_height={our_height} peer_height={}",
            remote.height
        );
        return Ok(());
    }

    println!(
        "sync start: peer={peer} peer_height={} our_height={our_height}",
        remote.height
    );

    let mut synced: usize = 0;
    let mut current_height = our_height;

    // --- fetch loop ---
    loop {
        let from = current_height + 1;
        write_p2p_line(&mut stream, &P2pMsg::GetBlocks { from_height: from })?;

        let line = read_p2p_line(&mut stream)?;
        let response: P2pMsg = serde_json::from_str(&line)
            .map_err(|err| format!("parse sync response from {peer}: {err}"))?;

        let P2pMsg::Blocks { blocks } = response else {
            return Err(format!("unexpected message during sync (expected Blocks)"));
        };

        if blocks.is_empty() {
            break;
        }

        let batch_count = blocks.len();
        for block in blocks {
            let height = block.header.height;
            match state.submit_block(&TESTNET, block, now_seconds()) {
                Ok(_) => {}
                Err(StateError::AlreadyKnown) => {} // resume after interrupted sync
                Err(err) => return Err(format!("sync failed at height {height}: {err}")),
            }
            current_height = height;
        }

        save_state(state_path, &state)?;
        synced += batch_count;
        println!("  synced +{batch_count} blocks  height={current_height}  total_synced={synced}");

        if current_height >= remote.height {
            break;
        }
    }

    println!("sync complete: tip={current_height} synced={synced} blocks from {peer}");
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP RPC server
// ---------------------------------------------------------------------------

fn serve_rpc(bind: &str, state_path: PathBuf) -> Result<(), String> {
    ensure_safe_rpc_bind(bind)?;
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    println!("tensorium RPC listening on http://{bind}");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                // Guard against slow HTTP clients that never finish sending.
                let _ = stream
                    .set_read_timeout(Some(Duration::from_secs(RPC_READ_TIMEOUT_SECS)));
                if let Err(err) = handle_rpc_stream(&mut stream, &state_path) {
                    let response = RpcError {
                        error: err.to_string(),
                    };
                    let _ = write_json_response(&mut stream, 500, &response);
                }
            }
            Err(err) => eprintln!("rpc connection error: {err}"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// RPC rate-limit strategy notes
// ---------------------------------------------------------------------------
// The RPC server is single-threaded: it handles one request at a time.
// This is intentional — it serialises state access and naturally throttles
// throughput without extra locking.
//
// By default, RPC binds to 127.0.0.1 only (enforced by ensure_safe_rpc_bind).
// Localhost exposure does not require further rate limiting.
//
// If TENSORIUM_RPC_ALLOW_PUBLIC=1 is set to expose RPC on a public interface,
// the operator MUST place a reverse proxy (e.g. nginx) in front with:
//   - per-IP request rate limiting  (limit_req_zone / limit_req)
//   - connection concurrency limit  (limit_conn)
//   - allowed methods whitelist     (GET and POST only)
// Failing to do so risks amplification attacks that exhaust disk I/O via
// repeated getblock / getblocktemplate calls.
//
// The RPC_READ_TIMEOUT_SECS guard prevents a single slow HTTP client from
// holding the server thread indefinitely, but it is not a substitute for
// nginx-level rate limiting on public endpoints.

fn ensure_safe_rpc_bind(bind: &str) -> Result<(), String> {
    let host = bind
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(bind)
        .trim_matches(['[', ']']);
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    let explicitly_allowed = env::var("TENSORIUM_RPC_ALLOW_PUBLIC")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !loopback && !explicitly_allowed {
        return Err(format!(
            "refusing public RPC bind {bind}; use 127.0.0.1 or set TENSORIUM_RPC_ALLOW_PUBLIC=1"
        ));
    }

    Ok(())
}

fn handle_rpc_stream(stream: &mut TcpStream, state_path: &Path) -> Result<(), String> {
    let mut buffer = [0u8; 65_536];
    let bytes_read = stream
        .read(&mut buffer)
        .map_err(|err| format!("failed to read request: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let parsed = parse_http_request(&request).ok_or_else(|| "invalid HTTP request".to_owned())?;
    let mempool_path = mempool_path_from_env();

    match (parsed.method.as_str(), parsed.path.as_str()) {
        ("GET", "/health") => write_json_response(stream, 200, &json!({ "ok": true })),

        ("GET", "/getblockcount") => {
            let state = load_state(state_path)?;
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": TESTNET.chain_id,
                    "height": state.height(),
                    "blocks": state.blocks.len(),
                }),
            )
        }

        ("GET", "/getdifficulty") => {
            let state = load_state(state_path)?;
            let Some(tip) = state.tip() else {
                return write_json_response(stream, 404, &RpcError::new("chain state is empty"));
            };
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": tip.header.chain_id,
                    "height": tip.header.height,
                    "leading_zero_bits": tip.header.leading_zero_bits,
                }),
            )
        }

        ("GET", path) if path.starts_with("/getblock/") => {
            let height = path
                .trim_start_matches("/getblock/")
                .parse::<u64>()
                .map_err(|err| format!("invalid block height: {err}"))?;
            let state = load_state(state_path)?;
            let Some(block) = state
                .blocks
                .iter()
                .find(|block| block.header.height == height)
            else {
                return write_json_response(stream, 404, &RpcError::new("block not found"));
            };
            write_json_response(
                stream,
                200,
                &json!({
                    "hash": block.hash(),
                    "block": block,
                }),
            )
        }

        ("GET", path) if path.starts_with("/getblocktemplate/") => {
            let miner = path.trim_start_matches("/getblocktemplate/");
            if miner.is_empty() {
                return write_json_response(stream, 404, &RpcError::new("missing miner address"));
            }
            let state = load_state(state_path)?;
            let mempool = load_mempool(&mempool_path);
            let extra_txs = mempool.select_for_block();
            let block = state
                .candidate_block_with_mempool(&TESTNET, now_seconds(), miner, extra_txs)
                .map_err(|err| err.to_string())?;
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": TESTNET.chain_id,
                    "height": block.header.height,
                    "previous_hash": block.header.previous_hash,
                    "leading_zero_bits": block.header.leading_zero_bits,
                    "tx_count": block.transactions.len(),
                    "template": block,
                }),
            )
        }

        ("POST", "/submitblock") => {
            let block: Block = match serde_json::from_str(parsed.body) {
                Ok(b) => b,
                Err(err) => {
                    return write_json_response(
                        stream,
                        400,
                        &RpcError::new(&format!("invalid block: {err}")),
                    )
                }
            };
            let mut state = load_state(state_path)?;

            let accepted = match state.submit_block(&TESTNET, block.clone(), now_seconds()) {
                Ok(b) => b,
                Err(StateError::AlreadyKnown) => {
                    return write_json_response(
                        stream,
                        200,
                        &json!({ "accepted": true, "height": block.header.height, "hash": block.hash(), "note": "already known" }),
                    );
                }
                Err(err) => return Err(err.to_string()),
            };

            let height = accepted.header.height;
            let hash = accepted.hash();

            save_state(state_path, &state)?;

            // Remove confirmed transactions from mempool.
            let mut mempool = load_mempool(&mempool_path);
            mempool.remove_confirmed(&accepted);
            let _ = save_mempool(&mempool_path, &mempool);

            // Broadcast to configured peers.
            broadcast_block_to_peers(&accepted, &state);

            write_json_response(
                stream,
                200,
                &json!({
                    "accepted": true,
                    "height": height,
                    "hash": hash,
                    "canonical": state.tip_hash() == hash,
                }),
            )
        }

        ("POST", "/sendrawtransaction") => {
            let tx: Transaction = match serde_json::from_str(parsed.body) {
                Ok(t) => t,
                Err(err) => {
                    return write_json_response(
                        stream,
                        400,
                        &RpcError::new(&format!("invalid transaction: {err}")),
                    )
                }
            };
            let txid = tx.id;
            let state = load_state(state_path)?;
            let utxos = build_utxo_set(&state)?;
            let tip_height = state.height().unwrap_or(0);

            let mut mempool = load_mempool(&mempool_path);
            mempool
                .add(&utxos, &TESTNET, tx.clone(), tip_height)
                .map_err(|err| err.to_string())?;
            save_mempool(&mempool_path, &mempool)?;

            // Broadcast to configured peers
            broadcast_tx_to_peers(&tx, &state);

            write_json_response(
                stream,
                200,
                &json!({
                    "accepted": true,
                    "txid": txid,
                    "mempool_size": mempool.len(),
                }),
            )
        }

        ("GET", "/getmempoolinfo") => {
            let mempool = load_mempool(&mempool_path);
            let txids: Vec<String> = mempool.pending.keys().cloned().collect();
            write_json_response(
                stream,
                200,
                &json!({
                    "count": mempool.len(),
                    "txids": txids,
                }),
            )
        }

        ("GET", "/getbanlist") => {
            let ban_path = ban_path_from_env();
            let banlist = load_banlist(&ban_path);
            let now = now_seconds();
            let entries: Vec<_> = banlist
                .entries
                .iter()
                .map(|(ip, e)| {
                    let banned = e.banned_until.map_or(false, |u| u > now);
                    let secs_left = e.banned_until.filter(|&u| u > now).map(|u| u - now);
                    json!({
                        "ip": ip,
                        "score": e.score,
                        "banned": banned,
                        "secs_remaining": secs_left,
                    })
                })
                .collect();
            write_json_response(
                stream,
                200,
                &json!({ "count": entries.len(), "entries": entries }),
            )
        }

        ("GET", path) if path.starts_with("/unban/") => {
            let ip = path.trim_start_matches("/unban/");
            if ip.is_empty() {
                return write_json_response(stream, 404, &RpcError::new("missing ip"));
            }
            let ban_path = ban_path_from_env();
            let mut banlist = load_banlist(&ban_path);
            let was_present = banlist.entries.contains_key(ip);
            banlist.unban(ip);
            save_banlist(&ban_path, &banlist);
            write_json_response(
                stream,
                200,
                &json!({ "unbanned": ip, "was_present": was_present }),
            )
        }

        _ => write_json_response(stream, 404, &RpcError::new("unknown RPC endpoint")),
    }
}

struct ParsedHttpRequest<'a> {
    method: String,
    path: String,
    body: &'a str,
}

fn parse_http_request(request: &str) -> Option<ParsedHttpRequest<'_>> {
    let request_line = request.lines().next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let body = request.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    Some(ParsedHttpRequest { method, path, body })
}

fn write_json_response<T: Serialize>(
    stream: &mut TcpStream,
    status_code: u16,
    body: &T,
) -> Result<(), String> {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let body = serde_json::to_string_pretty(body)
        .map_err(|err| format!("failed to serialize RPC response: {err}"))?;
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| format!("failed to write response: {err}"))
}

#[derive(Serialize)]
struct RpcError {
    error: String,
}

impl RpcError {
    fn new(error: &str) -> Self {
        Self {
            error: error.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- RPC bind guard ---

    #[test]
    fn rpc_bind_allows_loopback_by_default() {
        assert_eq!(ensure_safe_rpc_bind("127.0.0.1:23332"), Ok(()));
        assert_eq!(ensure_safe_rpc_bind("localhost:23332"), Ok(()));
    }

    #[test]
    fn rpc_bind_rejects_public_host_by_default() {
        assert!(ensure_safe_rpc_bind("0.0.0.0:23332").is_err());
    }

    // --- BanList unit tests ---

    #[test]
    fn ban_score_persists_below_threshold() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        // Accumulate score that stays below the ban threshold.
        bl.record("1.2.3.4", SCORE_INVALID_BLOCK, now); // 20
        bl.record("1.2.3.4", SCORE_INVALID_BLOCK, now); // 40
        bl.record("1.2.3.4", SCORE_INVALID_BLOCK, now); // 60
        assert!(!bl.is_banned("1.2.3.4", now));
        // prune_expired must NOT wipe entries that have score but no active ban.
        bl.prune_expired(now + 7200);
        assert!(
            bl.entries.contains_key("1.2.3.4"),
            "sub-threshold score entry must survive prune_expired"
        );
        assert_eq!(bl.entries["1.2.3.4"].score, 3 * SCORE_INVALID_BLOCK);
    }

    #[test]
    fn ban_score_accumulates_to_threshold() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        // 5 × SCORE_INVALID_BLOCK = 100 = BAN_THRESHOLD → ban on 5th call.
        for i in 0..4 {
            let banned = bl.record("2.3.4.5", SCORE_INVALID_BLOCK, now);
            assert!(!banned, "should not ban before threshold on call {i}");
        }
        let banned = bl.record("2.3.4.5", SCORE_INVALID_BLOCK, now);
        assert!(banned, "should impose ban exactly at threshold");
        assert!(bl.is_banned("2.3.4.5", now));
    }

    #[test]
    fn bad_handshake_triggers_instant_ban() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        let banned = bl.record("3.4.5.6", SCORE_BAD_HANDSHAKE, now);
        assert!(banned, "SCORE_BAD_HANDSHAKE must equal BAN_THRESHOLD");
        assert!(bl.is_banned("3.4.5.6", now));
    }

    #[test]
    fn expired_ban_is_pruned() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        bl.record("4.5.6.7", BAN_THRESHOLD, now);
        assert!(bl.is_banned("4.5.6.7", now));
        // After the ban duration, the entry should be cleaned up.
        bl.prune_expired(now + BAN_DURATION_SECS + 1);
        assert!(
            !bl.entries.contains_key("4.5.6.7"),
            "expired ban entry must be removed by prune_expired"
        );
    }

    #[test]
    fn active_ban_is_not_pruned() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        bl.record("5.6.7.8", BAN_THRESHOLD, now);
        // Prune before the ban expires — entry must remain.
        bl.prune_expired(now + BAN_DURATION_SECS / 2);
        assert!(
            bl.entries.contains_key("5.6.7.8"),
            "active ban entry must survive prune_expired"
        );
        assert!(bl.is_banned("5.6.7.8", now));
    }

    #[test]
    fn unban_removes_entry() {
        let mut bl = BanList::default();
        let now = 1_000_000u64;
        bl.record("6.7.8.9", BAN_THRESHOLD, now);
        assert!(bl.is_banned("6.7.8.9", now));
        bl.unban("6.7.8.9");
        assert!(!bl.entries.contains_key("6.7.8.9"));
        assert!(!bl.is_banned("6.7.8.9", now));
    }
}
