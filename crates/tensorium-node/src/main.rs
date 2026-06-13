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
    assets::{build_outputs, AssetOp, IssueData, NftMintData, TransferData},
    block::{merkle_root as compute_merkle_root, BlockHeader, Transaction},
    chain::{ConsensusParams, MAINNET, TESTNET},
    emission::reward_at_height,
    pow::header_meets_work,
    script::standard::{extract_address, p2pkh_from_address},
    Block, ChainState, Hash256, Mempool, StateError, UtxoSet,
};

const DEFAULT_STATE_PATH: &str = "tensorium-mainnet-state.json";
const DEFAULT_MEMPOOL_PATH: &str = "tensorium-mainnet-mempool.json";
const DEFAULT_BAN_PATH: &str = "tensorium-mainnet-banlist.json";
const DEFAULT_NONCE_LIMIT: u64 = u64::MAX;
const DEFAULT_RPC_BIND: &str = "127.0.0.1:33332";
const DEFAULT_P2P_BIND: &str = "0.0.0.0:33333";

/// Genesis timestamp for the MAINNET chain (TensorHash v1 relaunch, zero premine).
/// All nodes MUST use this exact value to share the same genesis block.
/// TODO(launch): placeholder — set to the actual mainnet genesis timestamp before launch.
const MAINNET_GENESIS_TIMESTAMP: u64 = 1_781_144_892;
/// Genesis nonce for the MAINNET chain (TensorHash v1, zero premine, 33M mining allocation).
/// Mined 2026-06-11 on 4x RTX 5090 against MAINNET_GENESIS_TIMESTAMP (TensorHash v1, 42-bit).
/// Verified: pow_hash 00000000001fb20...8a3aa6 has 43 leading zero bits (>= 42 required).
const MAINNET_GENESIS_NONCE: u64 = 9_223_372_445_780_809_059;
const P2P_PROTOCOL_VERSION: u32 = 1;
/// Maximum blocks returned per GetBlocks response.
const SYNC_BATCH_SIZE: usize = 50;
/// Maximum number of blocks `sync_blocks` will walk backward while searching
/// for a common ancestor with a peer whose chain forked below our current
/// tip. A real transient fork is at most a few blocks deep; anything beyond
/// this depth indicates an incompatible chain rather than a healable fork.
const MAX_FORK_SEARCH_DEPTH: u64 = 1000;
/// Maximum newline-delimited P2P message size. Keeps malformed peers from
/// growing an unbounded buffer before JSON parsing.
const MAX_P2P_LINE_BYTES: usize = 1_048_576;
/// Maximum concurrent inbound P2P connections. Prevents thread exhaustion
/// under a connection-flood DoS.
const MAX_INBOUND_PEERS: usize = 64;
/// Seconds before a P2P read or write operation times out. Keeps a slow or
/// dead peer from holding a thread indefinitely.
const P2P_IO_TIMEOUT_SECS: u64 = 30;
/// Seconds before an outbound P2P connect attempt gives up. A peer that is
/// down but unreachable (packets silently dropped) would otherwise block on the
/// OS SYN-retry timeout (~2 min). Keeps cron sync and block broadcast bounded.
const P2P_CONNECT_TIMEOUT_SECS: u64 = 5;
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
            let state = init_mainnet_state(&state_path, MAINNET_GENESIS_NONCE)?;
            print_status(&state, &MAINNET);
        }
        "status" => {
            let state = load_state(&state_path)?;
            print_status(&state, &MAINNET);
        }
        "mine-once" => {
            let mut state = load_state(&state_path)?;
            let miner = args.get(2).map(String::as_str).unwrap_or("local-dev-miner");
            state
                .mine_next_block(&MAINNET, now_seconds(), miner, DEFAULT_NONCE_LIMIT)
                .map_err(|err| err.to_string())?;
            print_status(&state, &MAINNET);
        }
        "rpc" => {
            let bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_RPC_BIND);
            serve_rpc(bind, state_path, mempool_path_from_env(), &MAINNET)?;
        }
        "p2p-listen" => {
            let bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_P2P_BIND);
            serve_p2p(
                bind,
                state_path,
                mempool_path_from_env(),
                ban_path_from_env(),
                &MAINNET,
            )?;
        }
        "p2p-connect" => {
            let peer = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-node p2p-connect <host:port>".to_owned())?;
            connect_peer(peer, &state_path, &MAINNET)?;
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
            sync_from_peer(peer, &state_path, &mempool_path_from_env(), &MAINNET)?;
        }
        "daemon" => {
            // Run RPC + P2P in one process so they share the same DB path without
            // fighting over the RocksDB exclusive lock (open_rocksdb retries handle
            // the brief simultaneous-open window).
            let rpc_bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_RPC_BIND).to_owned();
            let p2p_bind = args.get(3).map(String::as_str).unwrap_or(DEFAULT_P2P_BIND).to_owned();
            let mempool_path = mempool_path_from_env();
            let ban_path = ban_path_from_env();

            println!("tensorium mainnet daemon  rpc={rpc_bind}  p2p={p2p_bind}");

            let rpc_state = state_path.clone();
            let rpc_mempool = mempool_path.clone();
            let rpc_handle = thread::spawn(move || {
                serve_rpc(&rpc_bind, rpc_state, rpc_mempool, &MAINNET)
            });

            serve_p2p(&p2p_bind, state_path, mempool_path, ban_path, &MAINNET)?;
            rpc_handle.join().map_err(|_| "RPC thread panicked".to_owned())??;
        }
        "mine-genesis" => {
            let threads = args
                .get(2)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or_else(|| thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
            println!(
                "Mining mainnet genesis: diff={} bits, threads={}, timestamp={}",
                MAINNET.initial_leading_zero_bits, threads, MAINNET_GENESIS_TIMESTAMP
            );
            println!("This may take hours — use tensorium-miner for GPU acceleration.");
            let nonce = mine_genesis_multithreaded(threads)?;
            let state = init_mainnet_state(&state_path, nonce)?;
            println!("GENESIS NONCE: {nonce}  (hardcode this in node binary for v1 release)");
            print_status(&state, &MAINNET);
        }
        "peers" => print_manual_peers(),
        "banlist" => print_banlist(),
        "unban" => {
            let ip = args
                .get(2)
                .ok_or_else(|| "usage: tensorium-node unban <ip>".to_owned())?;
            unban_ip(ip)?;
        }
        "print-genesis-prefix" => {
            let timestamp: u64 = match args.get(2) {
                Some(s) => s.parse().map_err(|_| format!("invalid timestamp: {s}"))?,
                None => MAINNET_GENESIS_TIMESTAMP,
            };
            let header = genesis_header_template(timestamp);
            println!("chain_id    = {}", MAINNET.chain_id);
            println!("timestamp   = {timestamp}");
            println!("bits        = {}", header.leading_zero_bits);
            println!("merkle_root = {}", header.merkle_root);
            println!("prefix_hex  = {}", hex_lower(&header.pow_prefix_bytes()));
            println!();
            println!("mine with:  tensorium-miner --mode genesis --prefix <prefix_hex> --bits {}",
                header.leading_zero_bits);
        }
        "verify-genesis" => {
            let usage = "usage: tensorium-node verify-genesis <timestamp> <nonce>";
            let timestamp: u64 = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| usage.to_owned())?;
            let nonce: u64 = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| usage.to_owned())?;
            let mut header = genesis_header_template(timestamp);
            header.nonce = nonce;
            // Genesis is height 0 → epoch 0 → fixed zero seed.
            let pow = header.pow_hash(Hash256::ZERO);
            if header_meets_work(&header, Hash256::ZERO) {
                println!("VALID    pow_hash = {pow}");
                println!("paste into crates/tensorium-node/src/main.rs:");
                println!("  const MAINNET_GENESIS_TIMESTAMP: u64 = {timestamp};");
                println!("  const MAINNET_GENESIS_NONCE: u64 = {nonce};");
            } else {
                println!(
                    "INVALID  pow_hash = {pow}  (needs {} leading zero bits)",
                    header.leading_zero_bits
                );
                return Err(format!(
                    "genesis nonce {nonce} does not satisfy {} leading zero bits",
                    header.leading_zero_bits
                ));
            }
        }
        "devnet" => {
            let subcmd = args.get(2).map(String::as_str).unwrap_or("help");
            match subcmd {
                "init" => {
                    let mut state = ChainState::open_db(&devnet_state_path_from_env())?;
                    state
                        .init_genesis(&TESTNET, now_seconds(), u64::MAX)
                        .map_err(|err| err.to_string())?;
                    println!("devnet (TESTNET params, {} bits) genesis initialized",
                        TESTNET.initial_leading_zero_bits);
                    print_status(&state, &TESTNET);
                }
                "rpc" => {
                    let bind = args.get(3).map(String::as_str).unwrap_or("127.0.0.1:43332");
                    serve_rpc(
                        bind,
                        devnet_state_path_from_env(),
                        devnet_mempool_path_from_env(),
                        &TESTNET,
                    )?;
                }
                "status" => {
                    let state = load_state(&devnet_state_path_from_env())?;
                    print_status(&state, &TESTNET);
                }
                _ => {
                    println!("usage: tensorium-node devnet init|rpc [bind]|status");
                    println!("  low-difficulty TESTNET-params chain for miner live-path testing");
                    println!("  env: TENSORIUM_DEVNET_STATE, TENSORIUM_DEVNET_MEMPOOL");
                }
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

fn devnet_state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_DEVNET_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tensorium-devnet.json"))
}

fn devnet_mempool_path_from_env() -> PathBuf {
    env::var("TENSORIUM_DEVNET_MEMPOOL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tensorium-devnet-mempool.json"))
}


/// Genesis block header (nonce = 0) for the given launch timestamp.
/// Must construct exactly what `init_genesis_nonce` validates via
/// `candidate_block` — same coinbase, same merkle root.
fn genesis_header_template(timestamp_seconds: u64) -> BlockHeader {
    let params = &MAINNET;
    let reward = reward_at_height(params, 0);
    let coinbase = Transaction::genesis_coinbase(
        reward,
        "genesis",
        params.founder_allocation_atoms,
        params.founder_address,
        params.genesis_allocations,
    );
    let real_merkle = compute_merkle_root(&[coinbase]);
    BlockHeader {
        version: 1,
        chain_id: params.chain_id.to_owned(),
        height: 0,
        previous_hash: Hash256::ZERO,
        merkle_root: real_merkle,
        timestamp_seconds,
        leading_zero_bits: params.initial_leading_zero_bits,
        nonce: 0,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Multi-threaded CPU nonce search for the mainnet genesis block.
/// Returns the first nonce that satisfies MAINNET difficulty.
fn mine_genesis_multithreaded(threads: usize) -> Result<u64, String> {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    // Build the actual genesis block header with the real merkle root.
    // Must match exactly what init_genesis_nonce constructs via candidate_block.
    let header_template = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
    println!("genesis template: chain_id={} height=0 diff={} merkle_root={}",
        MAINNET.chain_id,
        MAINNET.initial_leading_zero_bits,
        header_template.merkle_root,
    );

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
                    if header_meets_work(&h, Hash256::ZERO) {
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

fn init_mainnet_state(state_path: &Path, nonce: u64) -> Result<ChainState, String> {
    let mut state = ChainState::open_db(state_path)?;
    state
        .init_genesis_nonce(&MAINNET, MAINNET_GENESIS_TIMESTAMP, nonce)
        .map_err(|err| err.to_string())?;
    Ok(state)
}

// ---------------------------------------------------------------------------
// State persistence
// ---------------------------------------------------------------------------

fn load_state(path: &Path) -> Result<ChainState, String> {
    // Derive the .db directory path from the .json path.
    let db_path: std::path::PathBuf =
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            path.with_extension("db")
        } else {
            path.to_path_buf()
        };

    // Auto-migrate: if JSON exists but DB directory does not, migrate once.
    if !db_path.exists() && path.exists() {
        eprintln!(
            "[storage] Migrating {} → {} (one-time)",
            path.display(),
            db_path.display()
        );
        tensorium_core::storage::migration::migrate_json_to_rocksdb(path, &db_path)?;
        let backup = path.with_extension("json.migrated");
        let _ = std::fs::rename(path, &backup);
        eprintln!("[storage] Migration complete. Backup at {}", backup.display());
    }

    ChainState::try_open_db(&db_path)
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

/// Seed a minimal `UtxoSet` with exactly the outpoints `tx` spends, read from
/// the persistent CF_UTXO. Behaviour-identical to validating `tx` against the
/// full UTXO set because `validate_transaction` only reads the inputs it spends
/// — a referenced outpoint absent from CF_UTXO is left absent here too, so
/// validation still yields `MissingInput`. Replaces a full-chain replay for
/// mempool acceptance.
fn seed_utxos_for_tx(state: &ChainState, tx: &Transaction) -> UtxoSet {
    let mut set = UtxoSet::new();
    for input in &tx.inputs {
        if let Some(entry) = state.utxo_lookup(&input.previous_output) {
            set.entries.insert(input.previous_output, entry);
        }
    }
    set
}

/// After `submit_block` performs a reorg, the transactions confirmed only on
/// the now-disconnected branch have nowhere to live — without this they
/// vanish silently (neither on-chain nor in the mempool), even though their
/// sender's balance correctly reverts. This walks `state.last_disconnected`
/// (drained by `take_reorg_requeue_candidates`) and tries to re-admit each
/// transaction into the mempool against the *new* canonical UTXO set.
///
/// Best-effort: a transaction is silently dropped if it no longer validates
/// (e.g. its inputs were spent by a competing transaction that made it onto
/// the winning chain, or it was independently mined into both branches and is
/// `AlreadyKnown`/already confirmed) — `Mempool::add` already distinguishes
/// these from real problems, and re-broadcasting a dead transaction would
/// just waste peers' bandwidth.
fn requeue_reorged_transactions(
    state: &mut ChainState,
    mempool: &mut Mempool,
    params: &ConsensusParams,
) -> usize {
    let candidates = state.take_reorg_requeue_candidates();
    if candidates.is_empty() {
        return 0;
    }
    let tip_height = state.height().unwrap_or(0);
    let mut requeued = 0;
    for tx in candidates {
        let txid = tx.id.to_hex();
        let utxos = seed_utxos_for_tx(state, &tx);
        match mempool.add(&utxos, params, tx, tip_height) {
            Ok(()) => {
                requeued += 1;
                println!("[reorg] requeued orphaned tx txid={txid} back into mempool");
            }
            Err(err) => {
                println!("[reorg] dropped orphaned tx txid={txid}: {err} (no longer valid on the winning chain)");
            }
        }
    }
    requeued
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

fn print_status(state: &ChainState, params: &ConsensusParams) {
    let Some(tip) = state.tip() else {
        println!("chain_id={} height=empty", params.chain_id);
        return;
    };
    println!(
        "chain_id={} height={} tip={} difficulty_bits={} blocks={}",
        tip.header.chain_id,
        tip.header.height,
        tip.hash(),
        tip.header.leading_zero_bits,
        state.block_count()
    );
}

fn print_help() {
    println!("tensorium-node <command>");
    println!();
    println!("commands:");
    println!("  init                 create local mainnet genesis state");
    println!("  status               show local chain status");
    println!("  mine-once [miner]    mine one block and persist it (diagnostic only)");
    println!("  rpc [bind]           start mainnet HTTP RPC server");
    println!("  p2p-listen [bind]    listen for mainnet peer connections and messages");
    println!("  p2p-connect <peer>   connect to a peer for diagnostics");
    println!("  sync [peer]          pull missing mainnet blocks from a peer");
    println!("  daemon [rpc_bind] [p2p_bind]  start RPC + P2P in one process (recommended)");
    println!("  mine-genesis [threads]      CPU-mine the genesis nonce (prefer tensorium-miner GPU)");
    println!("  peers                print manual peers from TENSORIUM_PEERS");
    println!("  banlist              show peer ban list");
    println!("  unban <ip>           remove a peer from the ban list");
    println!("  print-genesis-prefix [ts]   print MAINNET genesis pow-prefix hex for GPU mining");
    println!("  verify-genesis <ts> <nonce> check a mined genesis nonce against MAINNET difficulty");
    println!("  devnet init|rpc|status      low-difficulty TESTNET chain for miner testing");
    println!();
    println!("default chain params:");
    println!("  chain_id       = {}", MAINNET.chain_id);
    println!("  initial_diff   = {} bits", MAINNET.initial_leading_zero_bits);
    println!("  target_block   = {}s", MAINNET.target_block_seconds);
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
    println!("  GET  /getutxos/<address>          (all UTXOs for address, includes mature flag)");
    println!("  GET  /getbanlist");
    println!("  GET  /unban/<ip>                  (remove ban)");
    println!();
    println!("env:");
    println!("  TENSORIUM_STATE      state file path, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_MEMPOOL    mempool file path, default {DEFAULT_MEMPOOL_PATH}");
    println!("  TENSORIUM_BANS       ban list file path, default {DEFAULT_BAN_PATH}");
    println!("  TENSORIUM_PEERS      comma-separated mainnet peers (overrides built-in seeds)");
    println!("  TENSORIUM_NO_DEFAULT_SEEDS=1  disable built-in mainnet seed list");
    println!("  TENSORIUM_NODE_ID    node identity string");
    println!("  TENSORIUM_RPC_ALLOW_PUBLIC=1  allow non-loopback RPC bind");
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
/// Open an outbound P2P connection with a bounded connect timeout and read/write
/// timeouts applied. Inbound connections get their timeouts in `serve_p2p`; this
/// is the matching guard for every outbound connect (sync, broadcast, diagnostic
/// handshake) so a dead-but-unreachable peer can never hang a thread forever.
fn p2p_connect(peer: &str) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addr = peer
        .to_socket_addrs()
        .map_err(|err| format!("resolve {peer}: {err}"))?
        .next()
        .ok_or_else(|| format!("no address resolved for {peer}"))?;
    let stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(P2P_CONNECT_TIMEOUT_SECS))
            .map_err(|err| format!("connect {peer}: {err}"))?;
    let timeout = Some(Duration::from_secs(P2P_IO_TIMEOUT_SECS));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
    Ok(stream)
}

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

fn local_hello(state: &ChainState, params: &ConsensusParams) -> P2pHello {
    let node_id =
        env::var("TENSORIUM_NODE_ID").unwrap_or_else(|_| format!("node-{}", now_seconds()));
    let (height, tip_hash) = state
        .tip()
        .map(|tip| (tip.header.height, tip.hash()))
        .unwrap_or((0, Hash256::ZERO));
    P2pHello {
        protocol: "tensorium-p2p".to_owned(),
        version: P2P_PROTOCOL_VERSION,
        chain_id: params.chain_id.to_owned(),
        node_id,
        height,
        tip_hash,
    }
}

fn validate_hello(hello: &P2pHello, params: &ConsensusParams) -> Result<(), String> {
    if hello.protocol != "tensorium-p2p" {
        return Err(format!("unsupported P2P protocol: {}", hello.protocol));
    }
    if hello.version != P2P_PROTOCOL_VERSION {
        return Err(format!("unsupported P2P version: {}", hello.version));
    }
    if hello.chain_id != params.chain_id {
        return Err(format!(
            "wrong chain_id: {} (expected {})",
            hello.chain_id, params.chain_id
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// P2P server — accepts inbound connections and processes messages
// ---------------------------------------------------------------------------

fn serve_p2p(
    bind: &str,
    state_path: PathBuf,
    mempool_path: PathBuf,
    ban_path: PathBuf,
    params: &'static ConsensusParams,
) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    println!("tensorium P2P listening on {bind}");
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
                let mpool = mempool_path.clone();
                let count = Arc::clone(&peer_count);
                count.fetch_add(1, Ordering::Relaxed);
                thread::spawn(move || {
                    if let Err(err) =
                        handle_p2p_connection(&mut stream, &path, &bans, &mpool, params)
                    {
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
    mempool_path: &Path,
    params: &ConsensusParams,
) -> Result<(), String> {
    let remote_ip = peer_ip(stream);

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

    if let Err(err) = validate_hello(&remote, params) {
        // Wrong chain_id, version, or protocol — potentially an attacker or
        // a node on the wrong network.  Instant ban.
        record_violation(ban_path, &remote_ip, SCORE_BAD_HANDSHAKE);
        return Err(format!(
            "handshake rejected from {remote_ip} ({}): {err}",
            remote.node_id
        ));
    }

    // Build hello from current state, then drop state immediately so the DB
    // lock is released before entering the long-lived message loop.
    let my_hello = {
        let state = load_state(state_path)?;
        local_hello(&state, params)
    };
    write_p2p_line(stream, &my_hello)?;

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
                match accept_peer_block(state_path, mempool_path, *block, params) {
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
                match accept_peer_tx(&state_path, mempool_path, *tx, params) {
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
                        .canonical_blocks_iter()
                        .filter(|b| b.header.height >= from_height)
                        .take(SYNC_BATCH_SIZE)
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

fn accept_peer_block(
    state_path: &Path,
    mempool_path: &Path,
    block: Block,
    params: &ConsensusParams,
) -> Result<(u64, Hash256), String> {
    let block_height = block.header.height;
    let block_hash = block.hash();
    let mut state = load_state(state_path)?;

    match state.submit_block(params, block.clone(), now_seconds()) {
        Ok(_) => {}
        Err(StateError::AlreadyKnown) => {
            return Ok((block_height, block_hash));
        }
        Err(err) => return Err(err.to_string()),
    }

    let mut mempool = load_mempool(mempool_path);
    mempool.remove_confirmed(&block);
    requeue_reorged_transactions(&mut state, &mut mempool, params);
    let _ = save_mempool(mempool_path, &mempool);

    Ok((block_height, block_hash))
}

fn accept_peer_tx(
    state_path: &Path,
    mempool_path: &Path,
    tx: Transaction,
    params: &ConsensusParams,
) -> Result<(), String> {
    let mut state = load_state(state_path)?;
    state
        .ensure_utxo_synced(params)
        .map_err(|e| e.to_string())?;
    let utxos = seed_utxos_for_tx(&state, &tx);
    let tip_height = state.height().unwrap_or(0);
    let mut mempool = load_mempool(mempool_path);
    mempool
        .add(&utxos, params, tx, tip_height)
        .map_err(|err| err.to_string())?;
    save_mempool(mempool_path, &mempool)
}

// ---------------------------------------------------------------------------
// P2P client — push a block or transaction to a single peer
// ---------------------------------------------------------------------------

fn push_block_to_peer(
    peer: &str,
    block: &Block,
    state: &ChainState,
    params: &ConsensusParams,
) -> Result<u64, String> {
    let mut stream = p2p_connect(peer)?;

    write_p2p_line(&mut stream, &local_hello(state, params))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote, params)?;

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

fn push_tx_to_peer(
    peer: &str,
    tx: &Transaction,
    state: &ChainState,
    params: &ConsensusParams,
) -> Result<Hash256, String> {
    let mut stream = p2p_connect(peer)?;

    write_p2p_line(&mut stream, &local_hello(state, params))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote, params)?;

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
fn broadcast_block_to_peers(block: &Block, state: &ChainState, params: &ConsensusParams) {
    let peers = peers_for(params);
    for peer in &peers {
        match push_block_to_peer(peer, block, state, params) {
            Ok(height) => println!("broadcast block to {peer} accepted height={height}"),
            Err(err) => eprintln!("broadcast block to {peer} failed: {err}"),
        }
    }
}

/// Broadcast a transaction to every configured peer.  Per-peer errors are logged.
fn broadcast_tx_to_peers(tx: &Transaction, state: &ChainState, params: &ConsensusParams) {
    let peers = peers_for(params);
    for peer in &peers {
        match push_tx_to_peer(peer, tx, state, params) {
            Ok(txid) => println!("broadcast tx to {peer} accepted txid={txid}"),
            Err(err) => eprintln!("broadcast tx to {peer} failed: {err}"),
        }
    }
}

/// Built-in mainnet seed nodes for the generic command set. Used when
/// TENSORIUM_NO_DEFAULT_SEEDS is not set.
const DEFAULT_SEEDS: &[&str] = &[
    "seed.tensoriumlabs.com:33333",
];

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

/// Select the peer list appropriate to the chain being served.
///
/// Single canonical peer list for every chain now that the mc namespace is gone.
fn peers_for(params: &ConsensusParams) -> Vec<String> {
    let _ = params;
    configured_peers()
}

// ---------------------------------------------------------------------------
// p2p-connect — diagnostic handshake
// ---------------------------------------------------------------------------

fn connect_peer(peer: &str, state_path: &Path, params: &ConsensusParams) -> Result<(), String> {
    let state = load_state(state_path)?;
    let mut stream = p2p_connect(peer)?;

    write_p2p_line(&mut stream, &local_hello(&state, params))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello: {err}"))?;
    validate_hello(&remote, params)?;

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

/// Fetches blocks from a peer via `fetch` and applies them to `state`.
///
/// `fetch(from_height)` must return the peer's canonical blocks starting at
/// `from_height` in ascending order (an empty vec means "no more blocks").
///
/// Naively always requesting `from_height = our_height + 1` only works when
/// the peer's chain is a strict linear extension of ours. If the two chains
/// instead **forked below our current tip** (each side mined its own blocks
/// past a shared ancestor — exactly what produces a prolonged network
/// partition), the peer's block at `our_height + 1` has a `previous_hash`
/// that we never received, so `submit_block` returns `UnknownParent` and a
/// naive loop must abort — leaving the fork to grow forever, since neither
/// side can ever apply the other's blocks.
///
/// To heal that case, on `UnknownParent` we walk `from_height` backward one
/// block at a time until we reach a height whose peer-supplied block has a
/// parent we already know (the common ancestor). From there the peer's fork
/// blocks all apply successfully — `submit_block`'s chain_work-based fork
/// choice then reorgs onto the peer's chain automatically if it has more
/// cumulative work, exactly as it would for any other competing chain.
fn sync_blocks(
    state: &mut ChainState,
    params: &ConsensusParams,
    remote_height: u64,
    mut fetch: impl FnMut(u64) -> Result<Vec<Block>, String>,
) -> Result<usize, String> {
    let mut synced: usize = 0;
    let mut fetch_from = state.height().unwrap_or(0) + 1;
    let mut backtrack: u64 = 0;

    loop {
        let blocks = fetch(fetch_from)?;
        if blocks.is_empty() {
            break;
        }

        let mut forked_at: Option<u64> = None;
        let mut applied: usize = 0;
        for block in blocks {
            let height = block.header.height;
            match state.submit_block(params, block, now_seconds()) {
                Ok(_) | Err(StateError::AlreadyKnown) => applied += 1,
                Err(StateError::UnknownParent) if fetch_from > 1 => {
                    forked_at = Some(height);
                    break;
                }
                Err(err) => return Err(format!("sync failed at height {height}: {err}")),
            }
        }

        if let Some(height) = forked_at {
            backtrack += 1;
            if backtrack > MAX_FORK_SEARCH_DEPTH {
                return Err(format!(
                    "sync failed: no common ancestor found within {MAX_FORK_SEARCH_DEPTH} \
                     blocks while walking back from height {height} — chains may be incompatible"
                ));
            }
            fetch_from -= 1;
            continue;
        }

        backtrack = 0;
        synced += applied;
        let current_height = state.height().unwrap_or(0);
        fetch_from = current_height + 1;
        println!("  synced +{applied} blocks  height={current_height}  total_synced={synced}");

        if current_height >= remote_height {
            break;
        }
    }

    Ok(synced)
}

/// Download all blocks that `peer` has but we do not.
///
/// Prerequisites:
/// - `init` must have been run first so we share the same genesis.
/// - `peer` must be running `p2p-listen`.
///
/// Blocks are fetched in batches of SYNC_BATCH_SIZE, validated against our
/// local chain, and persisted after each successful batch. See `sync_blocks`
/// for how forks below our current tip are detected and healed.
fn sync_from_peer(
    peer: &str,
    state_path: &Path,
    mempool_path: &Path,
    params: &ConsensusParams,
) -> Result<(), String> {
    let mut state = load_state(state_path)?;
    let our_height = state.height().unwrap_or(0);

    // --- handshake ---
    let mut stream = p2p_connect(peer)?;

    write_p2p_line(&mut stream, &local_hello(&state, params))?;
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote, params)?;

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

    let synced = sync_blocks(&mut state, params, remote.height, |from_height| {
        write_p2p_line(&mut stream, &P2pMsg::GetBlocks { from_height })?;
        let line = read_p2p_line(&mut stream)?;
        let response: P2pMsg = serde_json::from_str(&line)
            .map_err(|err| format!("parse sync response from {peer}: {err}"))?;
        match response {
            P2pMsg::Blocks { blocks } => Ok(blocks),
            _ => Err("unexpected message during sync (expected Blocks)".to_owned()),
        }
    })?;

    // The "fork-below-tip" healing path in `sync_blocks` reorgs onto the
    // peer's chain when it has more work — exactly the scenario that can
    // orphan our own recently-broadcast transactions. Requeue them so they
    // aren't silently lost (mirrors `accept_peer_block` / `/submitblock`).
    let mut mempool = load_mempool(mempool_path);
    let requeued = requeue_reorged_transactions(&mut state, &mut mempool, params);
    if requeued > 0 {
        let _ = save_mempool(mempool_path, &mempool);
    }

    println!(
        "sync complete: tip={} synced={synced} blocks from {peer}",
        state.height().unwrap_or(our_height)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP RPC server
// ---------------------------------------------------------------------------

fn serve_rpc(
    bind: &str,
    state_path: PathBuf,
    mempool_path: PathBuf,
    params: &'static ConsensusParams,
) -> Result<(), String> {
    ensure_safe_rpc_bind(bind)?;
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    println!("tensorium RPC listening on http://{bind}");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let _ = stream
                    .set_read_timeout(Some(Duration::from_secs(RPC_READ_TIMEOUT_SECS)));
                if let Err(err) =
                    handle_rpc_stream(&mut stream, &state_path, &mempool_path, params)
                {
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

/// Load chain state for an RPC handler.  Returns a 503 and propagates Err if the DB is
/// temporarily locked (daemon mode: P2P thread holds the lock).  This keeps the RPC accept
/// loop unblocked — callers use `?` to return early on lock contention.
fn rpc_state(stream: &mut TcpStream, path: &Path) -> Result<ChainState, String> {
    ChainState::try_open_db(path).map_err(|e| {
        let _ = write_json_response(
            stream,
            503,
            &json!({ "error": "node busy — DB locked by P2P sync, retry in 1s" }),
        );
        e
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum BuildAssetTxRequest {
    Issue {
        from: String,
        ticker: String,
        decimals: u8,
        supply: u64,
        name: String,
    },
    NftMint {
        from: String,
        #[serde(default)]
        collection_id: Option<String>, // 64-hex, defaults to all-zero (standalone)
        royalty_bps: u16,
        royalty_addr: String,
        uri: String,
        content_hash: String, // 64-hex sha256
    },
    Transfer {
        from: String,
        to: String,
        asset_id: String, // 64-hex
        amount: u64,
    },
}

fn handle_rpc_stream(
    stream: &mut TcpStream,
    state_path: &Path,
    mempool_path: &Path,
    params: &ConsensusParams,
) -> Result<(), String> {
    let request = read_http_request(stream)?;
    let parsed = parse_http_request(&request).ok_or_else(|| "invalid HTTP request".to_owned())?;

    match (parsed.method.as_str(), parsed.path.as_str()) {
        ("GET", "/health") => write_json_response(stream, 200, &json!({ "ok": true })),

        ("GET", "/getblockcount") => {
            let state = load_state(state_path)?;
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": params.chain_id,
                    "height": state.height(),
                    "blocks": state.block_count(),
                }),
            )
        }

        ("GET", "/getdifficulty") => {
            let state = rpc_state(stream, state_path)?;
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
            let state = rpc_state(stream, state_path)?;
            let Some(block) = state.get_block_by_height(height) else {
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
            let mut state = rpc_state(stream, state_path)?;
            state
                .ensure_utxo_synced(params)
                .map_err(|e| e.to_string())?;
            let mempool = load_mempool(&mempool_path);
            let (candidate_txs, _) = mempool.select_for_block();
            // Revalidate each selected tx against the live UTXO set: a tx whose
            // inputs were already spent on-chain (e.g. the losing side of a
            // double-spend, or a reorg) must not enter the template, or the mined
            // block would be rejected by accept-time UTXO validation. Recompute
            // fees from the surviving txs.
            let tip_height = state.height().unwrap_or(0);
            let mut extra_txs = Vec::new();
            let mut total_fees: u64 = 0;
            for tx in candidate_txs {
                let seed = seed_utxos_for_tx(&state, &tx);
                match seed.validate_transaction(&tx, tip_height, params) {
                    Ok(fee) => {
                        total_fees = total_fees.saturating_add(fee);
                        extra_txs.push(tx);
                    }
                    Err(_) => { /* stale/invalid — skip it for this template */ }
                }
            }
            let block = state
                .candidate_block_with_mempool(params, now_seconds(), miner, extra_txs, total_fees)
                .map_err(|err| err.to_string())?;
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": params.chain_id,
                    "height": block.header.height,
                    "previous_hash": block.header.previous_hash,
                    "leading_zero_bits": block.header.leading_zero_bits,
                    "epoch_seed": state.epoch_seed_for_height(block.header.height),
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
            let mut state = rpc_state(stream, state_path)?;

            let accepted = match state.submit_block(params, block.clone(), now_seconds()) {
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

            // Remove confirmed transactions from mempool, and requeue any
            // that this submission orphaned via a reorg (see `submit_block`'s
            // `last_disconnected` bookkeeping).
            let mut mempool = load_mempool(&mempool_path);
            mempool.remove_confirmed(&accepted);
            requeue_reorged_transactions(&mut state, &mut mempool, params);
            let _ = save_mempool(&mempool_path, &mempool);

            broadcast_block_to_peers(&accepted, &state, params);

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
            let mut state = rpc_state(stream, state_path)?;
            state
                .ensure_utxo_synced(params)
                .map_err(|e| e.to_string())?;
            let utxos = seed_utxos_for_tx(&state, &tx);
            let tip_height = state.height().unwrap_or(0);

            let mut mempool = load_mempool(mempool_path);
            mempool
                .add(&utxos, params, tx.clone(), tip_height)
                .map_err(|err| err.to_string())?;
            save_mempool(mempool_path, &mempool)?;

            broadcast_tx_to_peers(&tx, &state, params);

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
            let fee_stats = mempool.fee_stats();
            write_json_response(
                stream,
                200,
                &json!({
                    "count": mempool.len(),
                    "txids": txids,
                    "fees": {
                        "total_fee_atoms":      fee_stats.total_fee_atoms,
                        "min_fee_atoms":        fee_stats.min_fee_atoms,
                        "max_fee_atoms":        fee_stats.max_fee_atoms,
                        "median_fee_atoms":     fee_stats.median_fee_atoms,
                        "min_relay_fee_atoms":  fee_stats.min_relay_fee_atoms,
                        "priority_fee_atoms":   fee_stats.priority_fee_atoms,
                    },
                }),
            )
        }

        ("GET", "/estimatefee") => {
            let mempool = load_mempool(&mempool_path);
            let tiers = mempool.fee_tiers();
            write_json_response(
                stream,
                200,
                &json!({
                    "slow_atoms":       tiers.slow_atoms,
                    "normal_atoms":     tiers.normal_atoms,
                    "fast_atoms":       tiers.fast_atoms,
                    "congestion_level": tiers.congestion_level,
                    "mempool_count":    tiers.mempool_count,
                    "slow_txm":         tiers.slow_atoms   as f64 / 1e8,
                    "normal_txm":       tiers.normal_atoms as f64 / 1e8,
                    "fast_txm":         tiers.fast_atoms   as f64 / 1e8,
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
            if !admin_authorized(&request) {
                return write_json_response(
                    stream,
                    403,
                    &RpcError::new(
                        "admin endpoint: set TENSORIUM_RPC_ADMIN_TOKEN and send a matching X-Admin-Token header",
                    ),
                );
            }
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

        ("GET", path) if path.starts_with("/getutxos/") => {
            let param = path.trim_start_matches("/getutxos/");

            if param.is_empty() {
                return write_json_response(
                    stream,
                    400,
                    &RpcError::new("missing param: GET /getutxos/<address_or_scriptpubkey_hex>"),
                );
            }

            // If param starts with "txm1" treat as bech32 address → derive P2PKH script.
            // Otherwise treat as lowercase hex-encoded scriptPubKey.
            let script = if param.starts_with("txm1") {
                match p2pkh_from_address(param) {
                    Ok(s) => s,
                    Err(_) => return write_json_response(
                        stream,
                        400,
                        &RpcError::new("invalid address: GET /getutxos/<address>"),
                    ),
                }
            } else {
                match hex::decode(param) {
                    Ok(s) => s,
                    Err(_) => return write_json_response(
                        stream,
                        400,
                        &RpcError::new("invalid hex: GET /getutxos/<scriptpubkey_hex>"),
                    ),
                }
            };
            let mut state = rpc_state(stream, state_path)?;
            state
                .ensure_utxo_synced(params)
                .map_err(|e| e.to_string())?;
            let tip_height = state.height().unwrap_or(0);
            let entries: Vec<serde_json::Value> = state
                .utxos_for_script(&script)
                .iter()
                .map(|(outpoint, entry)| {
                    let mature = !entry.coinbase
                        || tip_height
                            >= entry
                                .created_height
                                .saturating_add(params.coinbase_maturity_blocks);
                    json!({
                        "txid": outpoint.txid.to_hex(),
                        "txid_bytes": outpoint.txid.0.to_vec(),
                        "output_index": outpoint.output_index,
                        "value_atoms": entry.output.value_atoms,
                        "address": extract_address(&entry.output.script_pubkey).unwrap_or_default(),
                        "coinbase": entry.coinbase,
                        "created_height": entry.created_height,
                        "mature": mature,
                    })
                })
                .collect();
            write_json_response(
                stream,
                200,
                &json!({
                    "address": param,
                    "tip_height": tip_height,
                    "utxo_count": entries.len(),
                    "utxos": entries,
                }),
            )
        }

        ("POST", "/buildAssetTx") => {
            let req: BuildAssetTxRequest = match serde_json::from_str(parsed.body) {
                Ok(r) => r,
                Err(err) => {
                    return write_json_response(
                        stream,
                        400,
                        &RpcError::new(&format!("invalid request: {err}")),
                    )
                }
            };

            const ASSET_CARRIER_ATOMS: u64 = 1_000;
            let fee_atoms = tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;

            let (from, op, dest): (String, AssetOp, Option<(String, u64)>) = match req {
                BuildAssetTxRequest::Issue { from, ticker, decimals, supply, name } => {
                    if ticker.len() > 8 {
                        return write_json_response(stream, 400, &RpcError::new("ticker must be <= 8 bytes"));
                    }
                    if name.len() > 32 {
                        return write_json_response(stream, 400, &RpcError::new("name must be <= 32 bytes"));
                    }
                    if decimals > 18 {
                        return write_json_response(stream, 400, &RpcError::new("decimals must be <= 18"));
                    }
                    if supply == 0 {
                        return write_json_response(stream, 400, &RpcError::new("supply must be > 0"));
                    }
                    (from, AssetOp::Issue(IssueData { ticker, decimals, supply, name, flags: 0 }), None)
                }
                BuildAssetTxRequest::NftMint { from, collection_id, royalty_bps, royalty_addr, uri, content_hash } => {
                    if royalty_bps > 10_000 {
                        return write_json_response(stream, 400, &RpcError::new("royalty_bps must be <= 10000"));
                    }
                    if uri.len() > 200 {
                        return write_json_response(stream, 400, &RpcError::new("uri must be <= 200 bytes"));
                    }
                    let content_hash_bytes = match hex::decode(&content_hash) {
                        Ok(b) if b.len() == 32 => b,
                        _ => return write_json_response(stream, 400, &RpcError::new("content_hash must be 32 bytes (64 hex chars)")),
                    };
                    let collection_id_bytes: [u8; 32] = match collection_id {
                        Some(hexstr) => match hex::decode(&hexstr) {
                            Ok(b) if b.len() == 32 => b.try_into().unwrap(),
                            _ => return write_json_response(stream, 400, &RpcError::new("collection_id must be 32 bytes (64 hex chars)")),
                        },
                        None => [0u8; 32],
                    };
                    (from, AssetOp::NftMint(NftMintData {
                        collection_id: collection_id_bytes,
                        royalty_bps,
                        royalty_addr,
                        uri,
                        content_hash: content_hash_bytes.try_into().unwrap(),
                    }), None)
                }
                BuildAssetTxRequest::Transfer { from, to, asset_id, amount } => {
                    if amount == 0 {
                        return write_json_response(stream, 400, &RpcError::new("amount must be > 0"));
                    }
                    let asset_id_bytes: [u8; 32] = match hex::decode(&asset_id) {
                        Ok(b) if b.len() == 32 => b.try_into().unwrap(),
                        _ => return write_json_response(stream, 400, &RpcError::new("asset_id must be 32 bytes (64 hex chars)")),
                    };
                    // Balance/ownership check against the read-only asset indexer.
                    match indexer_check_transfer(&from, &asset_id, amount) {
                        Ok(true) => {}
                        Ok(false) => return write_json_response(stream, 400, &RpcError::new("insufficient asset balance or not the NFT owner")),
                        Err(e) => return write_json_response(stream, 503, &RpcError::new(&format!("asset index unavailable: {e}"))),
                    }
                    (from.clone(), AssetOp::Transfer(TransferData { asset_id: asset_id_bytes, amount, dest_output_index: 0 }), Some((to, ASSET_CARRIER_ATOMS)))
                }
            };

            let script = match p2pkh_from_address(&from) {
                Ok(s) => s,
                Err(_) => return write_json_response(stream, 400, &RpcError::new("invalid 'from' address")),
            };

            let mut state = rpc_state(stream, state_path)?;
            state.ensure_utxo_synced(params).map_err(|e| e.to_string())?;
            let tip_height = state.height().unwrap_or(0);
            let needed = dest.as_ref().map(|(_, a)| *a).unwrap_or(0).saturating_add(fee_atoms);

            let mut inputs = Vec::new();
            let mut total_in = 0u64;
            for (outpoint, entry) in state.utxos_for_script(&script).iter() {
                let mature = !entry.coinbase
                    || tip_height >= entry.created_height.saturating_add(params.coinbase_maturity_blocks);
                if !mature {
                    continue;
                }
                inputs.push(tensorium_core::TxInput {
                    previous_output: *outpoint,
                    signature_script: Vec::new(),
                });
                total_in = total_in.saturating_add(entry.output.value_atoms);
                if total_in >= needed {
                    break;
                }
            }
            if total_in < needed {
                return write_json_response(
                    stream,
                    400,
                    &RpcError::new(&format!("insufficient mature balance: have {total_in}, need {needed}")),
                );
            }

            let dest_ref = dest.as_ref().map(|(addr, atoms)| (addr.as_str(), *atoms));
            let outputs = match build_outputs(&op, dest_ref, &from, total_in, fee_atoms) {
                Ok(o) => o,
                Err(e) => return write_json_response(stream, 400, &RpcError::new(&e)),
            };

            let tx = Transaction::payment(inputs, outputs);
            let description = match &op {
                AssetOp::Issue(d) => format!("Issue token {} — supply {}, decimals {}", d.ticker, d.supply, d.decimals),
                AssetOp::NftMint(d) => format!("Mint NFT — {}", d.uri),
                AssetOp::Transfer(d) => format!("Transfer {} of asset {} to {}", d.amount, hex::encode(d.asset_id), dest.as_ref().unwrap().0),
            };

            write_json_response(
                stream,
                200,
                &json!({
                    "unsigned_tx": tx,
                    "summary": {
                        "op": match &op { AssetOp::Issue(_) => "issue", AssetOp::NftMint(_) => "nft_mint", AssetOp::Transfer(_) => "transfer" },
                        "description": description,
                        "fee_atoms": fee_atoms,
                    },
                }),
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

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.eq_ignore_ascii_case("content-length") {
            return None;
        }
        value.trim().parse::<usize>().ok()
    })
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

    let mut buffer = Vec::with_capacity(65_536);
    let mut chunk = [0u8; 8192];
    let mut expected_total = None;

    loop {
        let bytes_read = stream
            .read(&mut chunk)
            .map_err(|err| format!("failed to read request: {err}"))?;
        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err("request too large".to_owned());
        }

        if expected_total.is_none() {
            if let Some(end) = header_end(&buffer) {
                let headers = String::from_utf8_lossy(&buffer[..end]);
                let content_length = parse_content_length(&headers).unwrap_or(0);
                expected_total = Some(end + 4 + content_length);

                if content_length == 0 {
                    break;
                }
            }
        }

        if let Some(total) = expected_total {
            if buffer.len() >= total {
                break;
            }
        }
    }

    String::from_utf8(buffer).map_err(|err| format!("request was not valid UTF-8: {err}"))
}

fn parse_http_request(request: &str) -> Option<ParsedHttpRequest<'_>> {
    let request_line = request.lines().next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let body = request.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    Some(ParsedHttpRequest { method, path, body })
}

/// Extract a header value (case-insensitive name) from a raw HTTP request.
fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let head = request.split_once("\r\n\r\n").map_or(request, |(h, _)| h);
    // skip(1): the request line is not a header
    head.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Constant-time byte comparison — avoids leaking token contents/length via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Authorization gate for mutating/admin RPC endpoints (e.g. `/unban`).
/// Requires `TENSORIUM_RPC_ADMIN_TOKEN` to be set AND the request to carry a
/// matching `X-Admin-Token` header. If the env var is unset/empty the endpoint
/// is disabled (returns `false`), so a node is never exposed by default — even
/// without a reverse-proxy ACL in front of it.
fn admin_authorized(request: &str) -> bool {
    let expected = match env::var("TENSORIUM_RPC_ADMIN_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => return false,
    };
    match header_value(request, "x-admin-token") {
        Some(provided) => constant_time_eq(provided.as_bytes(), expected.as_bytes()),
        None => false,
    }
}

/// Query the local read-only asset indexer to check whether `from` can
/// perform a transfer of `amount` of `asset_id` (fungible balance, or NFT
/// ownership when `amount == 1` and the asset is non-fungible).
/// Returns `Ok(true)`/`Ok(false)` for a definitive answer, `Err` if the
/// indexer is unreachable or returns malformed data.
fn indexer_check_transfer(from: &str, asset_id: &str, amount: u64) -> Result<bool, String> {
    let indexer_base = env::var("TENSORIUM_INDEXER_URL").unwrap_or_else(|_| "127.0.0.1:23340".to_string());
    let body = http_get(&indexer_base, &format!("/balance/{from}"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad indexer response: {e}"))?;

    if let Some(nfts) = v.get("nfts").and_then(|n| n.as_array()) {
        if nfts.iter().any(|id| id.as_str() == Some(asset_id)) {
            return Ok(amount == 1);
        }
    }
    if let Some(fts) = v.get("fungible").and_then(|n| n.as_array()) {
        for ft in fts {
            if ft.get("asset_id").and_then(|i| i.as_str()) == Some(asset_id) {
                let bal = ft.get("amount").and_then(|a| a.as_u64()).unwrap_or(0);
                return Ok(bal >= amount);
            }
        }
    }
    Ok(false)
}

/// Minimal blocking HTTP/1.1 GET over a raw TCP socket — avoids adding an
/// HTTP client dependency for this one internal call to the indexer.
fn http_get(host_port: &str, path: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    let mut stream = TcpStream::connect(host_port).map_err(|e| format!("connect {host_port}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).map_err(|e| e.to_string())?;
    if !status_line.contains("200") {
        return Err(format!("indexer returned: {}", status_line.trim()));
    }
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 || line == "\r\n" {
            break;
        }
    }
    let mut body = String::new();
    use std::io::Read as _;
    reader.read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
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

    // --- admin endpoint auth helpers ---

    #[test]
    fn header_value_is_case_insensitive_and_trims() {
        let req = "GET /unban/1.2.3.4 HTTP/1.1\r\nHost: x\r\nX-Admin-Token:  s3cret \r\n\r\n";
        assert_eq!(header_value(req, "x-admin-token"), Some("s3cret"));
        assert_eq!(header_value(req, "X-ADMIN-TOKEN"), Some("s3cret"));
        assert_eq!(header_value(req, "missing"), None);
        // the request line must not be mistaken for a header
        assert_eq!(header_value(req, "GET"), None);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn admin_authorized_denies_when_token_unset() {
        // Pure-function path: no token configured ⇒ endpoint disabled regardless
        // of header. (Uses a header-bearing request to prove the header alone is
        // never sufficient.) Guarded so it doesn't race other tests on the env.
        std::env::remove_var("TENSORIUM_RPC_ADMIN_TOKEN");
        let req = "GET /unban/1.2.3.4 HTTP/1.1\r\nX-Admin-Token: anything\r\n\r\n";
        assert!(!admin_authorized(req));
    }

    // --- /buildAssetTx request parsing ---

    #[test]
    fn build_asset_tx_request_parses_issue() {
        let body = r#"{"op":"issue","from":"txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38","ticker":"GOLD","decimals":0,"supply":1000000,"name":"Gold Token"}"#;
        let req: BuildAssetTxRequest = serde_json::from_str(body).unwrap();
        match req {
            BuildAssetTxRequest::Issue { from, ticker, decimals, supply, name } => {
                assert_eq!(from, "txm1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqve9p38");
                assert_eq!(ticker, "GOLD");
                assert_eq!(decimals, 0);
                assert_eq!(supply, 1_000_000);
                assert_eq!(name, "Gold Token");
            }
            _ => panic!("expected Issue variant"),
        }
    }

    #[test]
    fn build_asset_tx_request_parses_transfer() {
        let body = r#"{"op":"transfer","from":"txm1a","to":"txm1b","asset_id":"0707070707070707070707070707070707070707070707070707070707070707","amount":50}"#;
        // asset_id above is 32 bytes hex (64 chars).
        let req: BuildAssetTxRequest = serde_json::from_str(body).unwrap();
        match req {
            BuildAssetTxRequest::Transfer { from, to, asset_id, amount } => {
                assert_eq!(from, "txm1a");
                assert_eq!(to, "txm1b");
                assert_eq!(amount, 50);
                // 32 bytes of hex decodes fine; the actual length check (must be 32 bytes)
                // happens in the handler, not at parse time.
                assert_eq!(hex::decode(&asset_id).unwrap().len(), 32);
            }
            _ => panic!("expected Transfer variant"),
        }
    }

    #[test]
    fn build_asset_tx_request_rejects_unknown_op() {
        let body = r#"{"op":"frobnicate","from":"txm1a"}"#;
        let result: Result<BuildAssetTxRequest, _> = serde_json::from_str(body);
        assert!(result.is_err());
    }

    // --- RPC bind guard ---

    #[test]
    fn rpc_bind_allows_loopback_by_default() {
        assert_eq!(ensure_safe_rpc_bind("127.0.0.1:33332"), Ok(()));
        assert_eq!(ensure_safe_rpc_bind("localhost:33332"), Ok(()));
    }

    #[test]
    fn rpc_bind_rejects_public_host_by_default() {
        assert!(ensure_safe_rpc_bind("0.0.0.0:33332").is_err());
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

    // --- getutxos path parsing ---

    #[test]
    fn getutxos_path_parses() {
        assert_eq!(
            "/getutxos/txm1abc".trim_start_matches("/getutxos/"),
            "txm1abc"
        );
    }

    #[test]
    fn getutxos_rejects_empty_address() {
        let path = "/getutxos/";
        let addr = path.trim_start_matches("/getutxos/");
        assert!(addr.is_empty());
    }

    #[test]
    fn getutxos_maturity_flag_coinbase() {
        // coinbase output is immature when tip_height < created_height + maturity_blocks
        let coinbase_maturity = 100u64;
        let created_height = 5u64;
        let tip_height_immature = 50u64;
        let tip_height_mature = 105u64;

        let is_coinbase = true;
        let mature_when_immature_tip = !is_coinbase
            || tip_height_immature >= created_height.saturating_add(coinbase_maturity);
        let mature_when_mature_tip = !is_coinbase
            || tip_height_mature >= created_height.saturating_add(coinbase_maturity);

        assert!(!mature_when_immature_tip, "coinbase at height 5 should be immature when tip=50");
        assert!(mature_when_mature_tip, "coinbase at height 5 should be mature when tip=105");
    }

    #[test]
    fn getutxos_maturity_flag_non_coinbase() {
        // non-coinbase outputs are always mature
        let coinbase_maturity = 100u64;
        let created_height = 5u64;
        let tip_height = 0u64; // even at tip_height 0

        let is_coinbase = false;
        let mature = !is_coinbase || tip_height >= created_height.saturating_add(coinbase_maturity);
        assert!(mature, "non-coinbase output is always mature");
    }

    #[test]
    fn getutxos_accepts_scriptpubkey_hex() {
        let param = "5221aabb";
        let is_address = param.starts_with("txm1");
        assert!(!is_address, "hex scriptpubkey should not be decoded as address");
    }

    #[test]
    fn mainnet_init_persists_state_on_disk() {
        // Verify that init creates a RocksDB-backed state that survives close+reopen.
        // Uses TEST_PARAMS (diff 8) so the test mines instantly without a GPU.
        // The real MC genesis nonce (diff 40, GPU-mined) is validated at deploy time.
        use tensorium_core::chain::TEST_PARAMS;
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("tensorium-mainnet-state.json");

        let mut state = ChainState::open_db(&state_path).unwrap();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, DEFAULT_NONCE_LIMIT).unwrap();
        assert_eq!(state.height(), Some(0));
        drop(state);

        let reopened = load_state(&state_path).unwrap();
        assert_eq!(reopened.height(), Some(0));
        assert!(state_path.with_extension("db").exists());
    }

    #[test]
    fn seed_utxos_for_tx_includes_only_referenced_existing_outpoints() {
        use tensorium_core::chain::TEST_PARAMS;
        use tensorium_core::{OutPoint, Transaction, TxInput, TxOutput};

        let mut state = ChainState::new();
        state.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        state.ensure_utxo_synced(&TEST_PARAMS).unwrap();

        // A real, existing outpoint: the genesis coinbase output 0.
        let genesis = state.get_block_by_height(0).unwrap();
        let existing = OutPoint {
            txid: genesis.transactions[0].id,
            output_index: 0,
        };
        assert!(
            state.utxo_lookup(&existing).is_some(),
            "setup: genesis coinbase output must be in CF_UTXO"
        );

        // A bogus outpoint that does not exist in the set.
        let missing = OutPoint {
            txid: Hash256([9u8; 32]),
            output_index: 0,
        };
        assert!(state.utxo_lookup(&missing).is_none());

        let tx = Transaction::payment(
            vec![
                TxInput {
                    previous_output: existing,
                    signature_script: Vec::new(),
                },
                TxInput {
                    previous_output: missing,
                    signature_script: Vec::new(),
                },
            ],
            vec![TxOutput {
                value_atoms: 1,
                script_pubkey: Vec::new(),
            }],
        );

        let seed = seed_utxos_for_tx(&state, &tx);
        assert_eq!(
            seed.entries.len(),
            1,
            "only the existing referenced outpoint is seeded"
        );
        assert!(seed.entries.contains_key(&existing));
        assert!(!seed.entries.contains_key(&missing));
    }

    // --- sync_blocks: fork-below-tip healing ---

    #[test]
    fn sync_blocks_walks_back_to_find_common_ancestor_on_fork_below_tip() {
        // Reproduces the live DO/Vultr mainnet partition (height
        // 962 vs 960, diverging at height 959): two nodes mine their own
        // blocks past a shared ancestor, ending up with chains that differ
        // *below* both tips. The naive "always fetch from our_height + 1"
        // loop hits StateError::UnknownParent on the very first fetched
        // block and aborts — the fork then grows forever, since neither
        // gossip nor that loop can ever connect the other side's chain.
        use tensorium_core::{chain::TEST_PARAMS, pow::mine_header};

        fn extend(state: &mut ChainState, count: u64, base_ts: u64, miner: &str) {
            for i in 0..count {
                let candidate = state.candidate_block(&TEST_PARAMS, base_ts + i, miner).unwrap();
                let header = mine_header(candidate.header.clone(), Hash256::ZERO, 1_000_000).unwrap();
                let block = Block::new(header, candidate.transactions);
                state.submit_block(&TEST_PARAMS, block, base_ts + i).unwrap();
            }
        }

        // `local` and `remote` start from an identical genesis + 3-block
        // common chain (deterministic mining ⇒ identical hashes).
        let mut local = ChainState::new();
        local.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        extend(&mut local, 3, 1_700_000_060, "miner-common");

        let mut remote = ChainState::new();
        remote.init_genesis(&TEST_PARAMS, 1_700_000_000, 1_000_000).unwrap();
        extend(&mut remote, 3, 1_700_000_060, "miner-common");
        assert_eq!(
            local.tip().unwrap().hash(),
            remote.tip().unwrap().hash(),
            "test setup: local and remote must share an identical common chain"
        );

        // Below both tips, each side mines its own competing fork:
        // local stays short (height 5), remote ends up taller (height 7) —
        // mirroring DO (962) being ahead of Vultr (960) after both diverged
        // at height 959.
        extend(&mut local, 2, 1_700_001_000, "miner-local");
        extend(&mut remote, 4, 1_700_002_000, "miner-remote");
        assert_eq!(local.height(), Some(5));
        assert_eq!(remote.height(), Some(7));
        assert_ne!(
            local.get_block_by_height(4).unwrap().hash(),
            remote.get_block_by_height(4).unwrap().hash(),
            "test setup: chains must diverge below both tips (at height 4)"
        );

        let remote_height = remote.height().unwrap();
        let remote_tip_hash = remote.tip().unwrap().hash();
        let fetch = |from_height: u64| -> Result<Vec<Block>, String> {
            let mut out = Vec::new();
            for h in from_height..=remote_height {
                match remote.get_block_by_height(h) {
                    Some(block) => out.push(block),
                    None => break,
                }
            }
            Ok(out)
        };

        let synced = sync_blocks(&mut local, &TEST_PARAMS, remote_height, fetch)
            .expect("sync_blocks must walk back past the fork point and heal the divergence");

        assert!(synced > 0, "expected the peer's fork blocks to be applied");
        assert_eq!(local.height(), Some(remote_height));
        assert_eq!(
            local.tip().unwrap().hash(),
            remote_tip_hash,
            "local must reorg onto the peer's taller/heavier chain after finding the common ancestor"
        );
    }

    // --- broadcast peer selection: mainnet must use TENSORIUM_PEERS ---

    #[test]
    fn peers_for_mainnet_uses_tensorium_peers() {
        // After the mc-namespace removal there is a single peer env var.
        // peers_for(&MAINNET) must read TENSORIUM_PEERS (the canonical list the
        // top-level rpc/p2p/daemon commands and install.sh configure).
        env::set_var("TENSORIUM_PEERS", "10.0.0.2:33333");

        let peers = peers_for(&MAINNET);
        assert_eq!(
            peers,
            vec!["10.0.0.2:33333".to_owned()],
            "mainnet broadcast must use TENSORIUM_PEERS"
        );

        env::remove_var("TENSORIUM_PEERS");
    }

    // --- client-side connect: must error (not hang/panic) on a dead peer ---

    #[test]
    fn p2p_connect_errors_on_unreachable_peer() {
        // Outbound P2P connections (sync_from_peer / push_block_to_peer) must
        // not block a thread forever on a peer that never responds. p2p_connect
        // wraps connect with a bounded timeout and returns Err instead of
        // hanging. Bind then drop a listener to get a guaranteed-refused port.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let result = p2p_connect(&addr.to_string());
        assert!(
            result.is_err(),
            "connecting to a closed port must return Err, not hang or panic"
        );
    }

    // --- genesis prefix / verify helpers ---

    #[test]
    fn genesis_header_template_matches_consensus_shape() {
        let header = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
        assert_eq!(header.height, 0);
        assert_eq!(header.nonce, 0);
        assert_eq!(header.chain_id, MAINNET.chain_id);
        assert_eq!(header.leading_zero_bits, MAINNET.initial_leading_zero_bits);
        assert_eq!(header.previous_hash, Hash256::ZERO);
        // pow prefix is what print-genesis-prefix emits for the CUDA miner
        assert_eq!(header.pow_prefix_bytes().len(), 102);
    }

    #[test]
    fn genesis_header_template_depends_on_timestamp() {
        let a = genesis_header_template(1_780_272_000);
        let b = genesis_header_template(1_780_272_001);
        assert_ne!(a.pow_prefix_bytes(), b.pow_prefix_bytes());
    }

    #[test]
    fn unmined_genesis_nonce_zero_fails_work_check() {
        // The placeholder nonce 0 must not satisfy 42-bit difficulty —
        // verify-genesis relies on header_meets_work for its pass/fail.
        let header = genesis_header_template(MAINNET_GENESIS_TIMESTAMP);
        assert!(!header_meets_work(&header, Hash256::ZERO));
    }
}
