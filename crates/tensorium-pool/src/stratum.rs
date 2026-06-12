//! Tensorium Stratum Protocol v1 — TCP mining pool server.
//! Port 3333 (alongside existing HTTP pool on port 23336).
//!
//! Vardiff: per-worker share difficulty auto-adjusts every 60 s to keep each
//! miner in the 15–60 shares/minute target band.  Adjustment step is ±1 bit
//! (×2 or ÷2 difficulty).  Bounds: 16 bit (min) … 40 bit (max, 2 below
//! network diff so a valid block always beats a share).

use crate::accounting::{PayoutEntry, PayoutLedger, ShareRecord};
use serde_json::{json, Value};
use tensorium_core::{block::BlockHeader, hash::Hash256};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// ── Vardiff constants ─────────────────────────────────────────────────────────

/// Check / possibly adjust share difficulty every N seconds.
pub const VARDIFF_WINDOW_SECS: u64 = 60;
/// Minimum accepted shares per minute — below this, difficulty drops.
pub const VARDIFF_TARGET_MIN: u64 = 15;
/// Maximum accepted shares per minute — above this, difficulty rises.
pub const VARDIFF_TARGET_MAX: u64 = 60;
/// Minimum per-worker share difficulty (bits of leading zeros required).
pub const VARDIFF_MIN_BITS: u8 = 16;
/// Maximum per-worker share difficulty.  2 bits below MAINNET network diff
/// (42) so there is always a gap between "valid share" and "valid block".
pub const VARDIFF_MAX_BITS: u8 = 40;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct StratumJob {
    pub job_id:          String,
    pub chain_id:        String,
    pub height:          u64,
    pub previous_hash:   [u8; 32],
    pub merkle_root:     [u8; 32],
    pub epoch_seed:      [u8; 32],
    pub timestamp:       u64,
    pub difficulty_bits: u8,
    pub version:         u32,
}

#[derive(Clone, Debug)]
pub struct WorkerSession {
    pub connection_id:            String,
    pub worker_name:              String,
    pub wallet_address:           String,
    pub peer_addr:                String,
    pub authorized_at_unix:       u64,
    pub last_seen_at_unix:        u64,
    pub accepted_shares:          u64,
    pub rejected_shares:          u64,
    pub last_submit_result:       String,
    /// Current per-worker share difficulty (leading-zero bits).
    pub share_diff_bits:          u8,
    /// Shares accepted in the current vardiff window.
    pub vardiff_window_shares:    u64,
    /// Unix timestamp when the current vardiff window started.
    pub vardiff_window_start:     u64,
}

pub struct StratumState {
    pub current_job:        Option<StratumJob>,
    /// Global initial share difficulty (bits).  New workers start here.
    pub share_diff_bits:    u8,
    pub node_rpc:           String,
    pub treasury:           String,
    /// connection_id → worker session
    pub workers:            HashMap<String, WorkerSession>,
    pub shares_accepted:    u64,
    pub shares_rejected:    u64,
    pub blocks_found:       u64,
    /// connection_id → job sender (cleaned up on disconnect)
    pub job_senders:        HashMap<String, std::sync::mpsc::Sender<StratumJob>>,
    /// job_id → raw template JSON (last 2 jobs, for stale-share lookup)
    pub job_template_cache: HashMap<String, String>,
    /// Shared payout ledger — same instance as the HTTP pool
    pub ledger:             Arc<Mutex<PayoutLedger>>,
    pub ledger_path:        PathBuf,
}

impl StratumState {
    pub fn new(
        node_rpc:     String,
        treasury:     String,
        share_diff:   u64,
        ledger:       Arc<Mutex<PayoutLedger>>,
        ledger_path:  PathBuf,
    ) -> Self {
        Self {
            current_job: None,
            share_diff_bits: diff_to_bits(share_diff),
            node_rpc,
            treasury,
            workers: HashMap::new(),
            shares_accepted: 0,
            shares_rejected: 0,
            blocks_found: 0,
            job_senders: HashMap::new(),
            job_template_cache: HashMap::new(),
            ledger,
            ledger_path,
        }
    }

    /// Stats for HTTP /pool/stratum endpoint.
    pub fn stats_json(&self) -> Value {
        let active_workers: Vec<Value> = self
            .workers
            .values()
            .map(|w| {
                json!({
                    "connection_id":      w.connection_id,
                    "worker_name":        w.worker_name,
                    "wallet_address":     w.wallet_address,
                    "peer_addr":          w.peer_addr,
                    "authorized_at_unix": w.authorized_at_unix,
                    "last_seen_at_unix":  w.last_seen_at_unix,
                    "accepted_shares":    w.accepted_shares,
                    "rejected_shares":    w.rejected_shares,
                    "last_submit_result": w.last_submit_result,
                    "share_diff_bits":    w.share_diff_bits,
                    "share_diff":         bits_to_diff(w.share_diff_bits),
                })
            })
            .collect();

        json!({
            "stratum_workers":      self.job_senders.len(),
            "authorized_workers":   self.workers.len(),
            "stratum_port":         3333,
            "initial_share_diff_bits": self.share_diff_bits,
            "shares_accepted":      self.shares_accepted,
            "shares_rejected":      self.shares_rejected,
            "blocks_found":         self.blocks_found,
            "active_workers":       active_workers,
            "vardiff": {
                "window_secs":  VARDIFF_WINDOW_SECS,
                "target_min":   VARDIFF_TARGET_MIN,
                "target_max":   VARDIFF_TARGET_MAX,
                "min_bits":     VARDIFF_MIN_BITS,
                "max_bits":     VARDIFF_MAX_BITS,
            },
        })
    }
}

// ── Diff ↔ bits helpers ───────────────────────────────────────────────────────

/// Convert a raw diff value (e.g. 1_048_576) to the number of leading-zero
/// bits it represents (e.g. 20).  Uses floor(log2).
pub fn diff_to_bits(diff: u64) -> u8 {
    if diff <= 1 { return 0; }
    let mut d = diff;
    let mut b = 0u8;
    while d > 1 { d >>= 1; b += 1; }
    b
}

/// Convert bits back to the raw diff threshold (2^bits).
pub fn bits_to_diff(bits: u8) -> u64 {
    1u64 << bits
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn leading_zero_bits(hash: &[u8; 32]) -> u8 {
    let mut bits = 0u8;
    for &byte in hash.iter() {
        if byte == 0 { bits += 8; } else { bits += byte.leading_zeros() as u8; break; }
    }
    bits
}

// ── Consensus header builder ──────────────────────────────────────────────────

/// Build the consensus `BlockHeader` for a job + nonce. Using the consensus
/// type means the pool's PoW check and ledger id-hash can never drift from
/// the node's serialization.
fn job_header(job: &StratumJob, nonce: u64) -> BlockHeader {
    BlockHeader {
        version: job.version,
        chain_id: job.chain_id.clone(),
        height: job.height,
        previous_hash: Hash256(job.previous_hash),
        merkle_root: Hash256(job.merkle_root),
        timestamp_seconds: job.timestamp,
        leading_zero_bits: job.difficulty_bits,
        nonce,
    }
}

// ── Nonce helpers ─────────────────────────────────────────────────────────────

/// Parse a little-endian hex nonce sent by the miner (LSB first).
/// e.g. nonce=1000 (0x3E8) → "e803000000000000"
fn le_hex_to_u64(s: &str) -> Option<u64> {
    let s = s.trim_start_matches("0x");
    let mut bytes = [0u8; 8];
    let pairs = s.len() / 2;
    for i in 0..pairs.min(8) {
        bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(u64::from_le_bytes(bytes))
}

// ── Share validation ───────────────────────────────────────────────────────────

/// Returns `Some((leading_zeros, is_share, is_block))` or `None` if the nonce
/// cannot be parsed.  Uses the per-worker share diff (in bits).
fn validate_share(
    job:              &StratumJob,
    nonce_hex:        &str,
    worker_diff_bits: u8,
) -> Option<(u8, bool, bool)> {
    let nonce  = le_hex_to_u64(nonce_hex)?;
    let hash   = job_header(job, nonce).pow_hash(Hash256(job.epoch_seed));
    let zeros  = leading_zero_bits(&hash.0);
    let is_share = zeros >= worker_diff_bits;
    let is_block = zeros >= job.difficulty_bits;
    Some((zeros, is_share, is_block))
}

// ── Hex helpers ────────────────────────────────────────────────────────────────

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

#[allow(dead_code)]
fn hex_to_bytes32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 { return None; }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i*2..i*2+2], 16).ok()?;
    }
    Some(out)
}

// ── Gross reward from cached template ─────────────────────────────────────────

fn gross_from_template(raw: &str) -> u64 {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v["template"]["transactions"][0]["outputs"][0]["value_atoms"].as_u64()
        })
        .unwrap_or(0)
}

// ── Job fetch from node ────────────────────────────────────────────────────────

/// Fetch a job from the node.  Returns `(job, raw_template_json)` on success.
pub fn fetch_job(node_rpc: &str, treasury: &str) -> Option<(StratumJob, String)> {
    let url  = format!("http://{}/getblocktemplate/{}", node_rpc, treasury);
    let resp = http_get_body(&url)?;
    parse_job_response(&resp)
}

/// Parse a /getblocktemplate response body into a StratumJob.
/// Returns None (and logs) when the node does not send `epoch_seed` —
/// such a node predates TensorHash v1 and cannot be pool-mined against.
fn parse_job_response(resp: &str) -> Option<(StratumJob, String)> {
    let v: Value = serde_json::from_str(resp).ok()?;

    let hdr       = v["template"]["header"].as_object()?;
    let chain_id  = hdr["chain_id"].as_str()?.to_string();
    let height    = hdr["height"].as_u64()?;
    let diff_bits = hdr["leading_zero_bits"].as_u64()? as u8;
    let timestamp = hdr["timestamp_seconds"].as_u64()?;
    let version   = hdr["version"].as_u64().unwrap_or(1) as u32;
    let prev      = parse_byte_array(hdr.get("previous_hash")?)?;
    let mroot     = parse_byte_array(hdr.get("merkle_root")?)?;

    // Top-level field, added by the node in Phase A2.
    let epoch_seed = match v.get("epoch_seed").and_then(parse_byte_array) {
        Some(seed) => seed,
        None => {
            eprintln!(
                "[stratum] node template has no epoch_seed — node too old \
                 for TensorHash v1, upgrade tensorium-node"
            );
            return None;
        }
    };

    let ms     = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_millis();
    let job_id = format!("h{}-{}", height, ms);

    let job = StratumJob { job_id, chain_id, height, previous_hash: prev,
                           merkle_root: mroot, epoch_seed, timestamp,
                           difficulty_bits: diff_bits, version };
    Some((job, resp.to_string()))
}

fn parse_byte_array(v: &Value) -> Option<[u8; 32]> {
    let arr = v.as_array()?;
    if arr.len() < 32 { return None; }
    let mut out = [0u8; 32];
    for (i, x) in arr.iter().enumerate().take(32) {
        out[i] = x.as_u64()? as u8;
    }
    Some(out)
}

fn http_get_body(url: &str) -> Option<String> {
    use std::io::Read;
    let without_scheme = url.strip_prefix("http://")?;
    let slash  = without_scheme.find('/')?;
    let hp     = &without_scheme[..slash];
    let path   = &without_scheme[slash..];
    let colon  = hp.rfind(':')?;
    let host   = &hp[..colon];
    let port   = &hp[colon + 1..];
    let mut conn = TcpStream::connect(format!("{}:{}", host, port)).ok()?;
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    write!(conn, "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
           path, host, port).ok()?;
    let mut resp = String::new();
    conn.read_to_string(&mut resp).ok()?;
    Some(resp.split("\r\n\r\n").nth(1)?.to_string())
}

// ── Submit block to node ────────────────────────────────────────────────────────

/// Submit a found block to the node.  Uses the cached raw template JSON so the
/// full coinbase transaction is preserved — only the nonce field is updated.
/// Returns `true` if the node accepted the block.
fn submit_block(node_rpc: &str, job: &StratumJob, nonce: u64, raw_template: Option<&str>) -> bool {
    use std::io::Read;
    let colon = match node_rpc.rfind(':') { Some(c) => c, None => return false };
    let host = &node_rpc[..colon];
    let port = &node_rpc[colon + 1..];
    eprintln!("[stratum] BLOCK! height={} nonce={} job={}", job.height, nonce, job.job_id);

    // Node's /submitblock expects a Block struct directly: {"header":{...},"transactions":[...]}
    // The cached template has an outer "template" wrapper that must be stripped.
    let body = if let Some(raw) = raw_template {
        if let Ok(mut v) = serde_json::from_str::<Value>(raw) {
            // Set nonce inside template, then unwrap the "template" layer.
            v["template"]["header"]["nonce"] = json!(nonce);
            if let Some(inner) = v.get("template").cloned() {
                inner.to_string()
            } else {
                build_fallback_body(job, nonce)
            }
        } else { build_fallback_body(job, nonce) }
    } else { build_fallback_body(job, nonce) };

    let mut conn = match TcpStream::connect(format!("{}:{}", host, port)) {
        Ok(c) => c, Err(_) => return false,
    };
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let req = format!(
        "POST /submitblock HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        host, port, body.len(), body
    );
    if conn.write_all(req.as_bytes()).is_err() { return false; }
    let mut resp = String::new();
    conn.read_to_string(&mut resp).ok();
    // Parse response body (after HTTP headers) and check accepted field.
    let body = resp.split("\r\n\r\n").nth(1).unwrap_or(&resp);
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["accepted"].as_bool())
        .unwrap_or(false)
}

fn build_fallback_body(job: &StratumJob, nonce: u64) -> String {
    // Node /submitblock expects Block directly (no "template" wrapper).
    json!({
        "header": {
            "chain_id": job.chain_id,
            "height": job.height,
            "previous_hash": job.previous_hash.to_vec(),
            "merkle_root": job.merkle_root.to_vec(),
            "timestamp_seconds": job.timestamp,
            "leading_zero_bits": job.difficulty_bits,
            "version": job.version,
            "nonce": nonce
        },
        "transactions": []
    }).to_string()
}

// ── Mining.notify builder ─────────────────────────────────────────────────────

/// Build a `mining.notify` message.  `worker_diff` is the per-worker share
/// difficulty (used by the miner to know what hash target to aim for).
fn notify_msg(job: &StratumJob, worker_diff: u64) -> Value {
    json!({
        "id": null, "method": "mining.notify",
        "params": {
            "job_id":           job.job_id,
            "chain_id":         job.chain_id,
            "height":           job.height,
            "previous_hash":    bytes_to_hex(&job.previous_hash),
            "merkle_root":      bytes_to_hex(&job.merkle_root),
            "epoch_seed":       bytes_to_hex(&job.epoch_seed),
            "timestamp":        job.timestamp,
            "difficulty_bits":  job.difficulty_bits,
            "share_difficulty": worker_diff,
            "clean_jobs":       true,
        }
    })
}

// ── Per-connection handler ─────────────────────────────────────────────────────

fn send_line(w: &mut TcpStream, msg: &Value) -> bool {
    let mut line = msg.to_string();
    line.push('\n');
    w.write_all(line.as_bytes()).is_ok()
}

fn handle_stratum_conn(
    stream:        TcpStream,
    state:         Arc<Mutex<StratumState>>,
    job_rx:        std::sync::mpsc::Receiver<StratumJob>,
    connection_id: String,
) {
    let peer_addr = stream.peer_addr()
        .map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());

    let cleanup = |state: &Arc<Mutex<StratumState>>, connection_id: &str, peer_addr: &str| {
        let mut s = state.lock().unwrap();
        s.workers.remove(connection_id);
        s.job_senders.remove(connection_id);
        eprintln!("[stratum] disconnected {}", peer_addr);
    };

    // Short read timeout so we can send proactive pings between shares.
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => {
            cleanup(&state, &connection_id, &peer_addr);
            return;
        }
    };
    let reader = BufReader::new(stream);

    let mut authorized  = false;
    let mut wallet_addr = String::new();
    let mut last_job_id = String::new();
    let mut last_ping   = Instant::now();

    for line_res in reader.lines() {
        // ── Drain any pending new-job notifications ────────────────────────
        while let Ok(job) = job_rx.try_recv() {
            if authorized {
                let diff = {
                    let s = state.lock().unwrap();
                    s.workers.get(&connection_id)
                        .map(|w| bits_to_diff(w.share_diff_bits))
                        .unwrap_or(bits_to_diff(s.share_diff_bits))
                };
                if !send_line(&mut writer, &notify_msg(&job, diff)) {
                    cleanup(&state, &connection_id, &peer_addr);
                    return;
                }
                last_job_id = job.job_id.clone();
            }
        }

        let line = match line_res {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                if authorized && last_ping.elapsed().as_secs() >= 5 {
                    if !send_line(&mut writer, &json!({"id":null,"method":"mining.ping","params":[]})) {
                        cleanup(&state, &connection_id, &peer_addr);
                        return;
                    }
                    last_ping = Instant::now();
                }
                continue;
            }
            Err(_) => break,
        };
        if line.is_empty() { continue; }

        let msg: Value = match serde_json::from_str(&line) { Ok(v) => v, Err(_) => continue };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id     = msg["id"].clone();

        match method.as_str() {
            // ── mining.subscribe ──────────────────────────────────────────
            "mining.subscribe" => {
                let resp = json!({ "id": id, "result": {
                    "session_id": format!("s{}", SystemTime::now()
                        .duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()),
                    "protocol":   "tensorium-stratum/1",
                    "nonce_bits": 64,
                }, "error": null });
                if !send_line(&mut writer, &resp) { break; }
            }

            // ── mining.authorize ──────────────────────────────────────────
            "mining.authorize" => {
                let auth   = msg["params"][0].as_str().unwrap_or("").to_string();
                let parts: Vec<&str> = auth.splitn(2, '.').collect();
                wallet_addr = parts[0].to_string();
                let wname  = parts.get(1).copied().unwrap_or("default").to_string();

                let (init_bits, maybe_job, init_diff) = {
                    let mut s = state.lock().unwrap();
                    let bits = s.share_diff_bits;
                    s.workers.insert(connection_id.clone(), WorkerSession {
                        connection_id:         connection_id.clone(),
                        worker_name:           wname,
                        wallet_address:        wallet_addr.clone(),
                        peer_addr:             peer_addr.clone(),
                        authorized_at_unix:    unix_now(),
                        last_seen_at_unix:     unix_now(),
                        accepted_shares:       0,
                        rejected_shares:       0,
                        last_submit_result:    "authorized".to_string(),
                        share_diff_bits:       bits,
                        vardiff_window_shares: 0,
                        vardiff_window_start:  unix_now(),
                    });
                    (bits, s.current_job.clone(), bits_to_diff(bits))
                };
                authorized = true;

                // 1. auth OK
                if !send_line(&mut writer, &json!({"id":id,"result":true,"error":null})) { break; }
                // 2. set_difficulty (initial)
                if !send_line(&mut writer,
                    &json!({"id":null,"method":"mining.set_difficulty","params":[init_diff]})) { break; }
                // 3. current job
                if let Some(job) = maybe_job {
                    if !send_line(&mut writer, &notify_msg(&job, init_diff)) { break; }
                    last_job_id = job.job_id.clone();
                }
                eprintln!("[stratum] authorized {} @ {}  initial_diff={}bits",
                          auth, peer_addr, init_bits);
            }

            // ── mining.submit ─────────────────────────────────────────────
            "mining.submit" => {
                if !authorized { continue; }
                let params    = &msg["params"];
                let job_id    = params["job_id"].as_str().unwrap_or("").to_string();
                let nonce_hex = params["nonce"].as_str().unwrap_or("0").to_string();

                // Fetch current job + per-worker diff in one lock.
                let (job_opt, worker_diff_bits) = {
                    let s = state.lock().unwrap();
                    let bits = s.workers.get(&connection_id)
                        .map(|w| w.share_diff_bits)
                        .unwrap_or(s.share_diff_bits);
                    (s.current_job.clone(), bits)
                };

                let result = if let Some(ref job) = job_opt {
                    // Allow current job_id or previous (1 stale tolerated).
                    let stale = job.job_id != job_id && job_id != last_job_id;
                    if stale {
                        let mut s = state.lock().unwrap();
                        s.shares_rejected += 1;
                        if let Some(w) = s.workers.get_mut(&connection_id) {
                            w.rejected_shares += 1;
                            w.last_submit_result = "rejected-stale".to_string();
                        }
                        send_line(&mut writer,
                            &json!({"id":id,"result":"rejected","error":"stale"}));
                        continue;
                    }

                    match validate_share(job, &nonce_hex, worker_diff_bits) {
                        Some((_, true, is_block)) => {
                            // ── Block found ──────────────────────────────
                            if is_block {
                                let nonce = le_hex_to_u64(&nonce_hex).unwrap_or(0);
                                let (node_rpc, raw_tpl, ledger_arc, ledger_path, wname) = {
                                    let s = state.lock().unwrap();
                                    let raw = s.job_template_cache.get(&job.job_id)
                                        .or_else(|| s.job_template_cache.get(&job_id))
                                        .cloned();
                                    let wname = s.workers.get(&connection_id)
                                        .map(|w| w.worker_name.clone())
                                        .unwrap_or_else(|| "unknown".to_string());
                                    (s.node_rpc.clone(), raw, s.ledger.clone(), s.ledger_path.clone(), wname)
                                };

                                let accepted = submit_block(&node_rpc, job, nonce, raw_tpl.as_deref());
                                if accepted {
                                    let block_hash = bytes_to_hex(&job_header(job, nonce).hash().0);
                                    let gross = raw_tpl.as_deref()
                                        .map(gross_from_template).unwrap_or(0);
                                    if gross == 0 {
                                        eprintln!("[stratum] WARNING: could not read gross reward for height={}", job.height);
                                    }

                                    // ── PPLNS reward split ────────────────────────────
                                    // Include the winning share in the window first so
                                    // the block finder is also credited proportionally.
                                    let winning_share = ShareRecord {
                                        wallet_address:  wallet_addr.clone(),
                                        worker_name:     wname,
                                        share_diff_bits: worker_diff_bits,
                                        submitted_at_unix: unix_now(),
                                    };
                                    let mut ledger = ledger_arc.lock().unwrap();
                                    ledger.push_share(winning_share);

                                    let splits = ledger.pplns_split(gross, &wallet_addr);
                                    let n_miners = splits.len();
                                    for (addr, miner_gross) in splits {
                                        let entry = PayoutEntry::new(
                                            job.height, block_hash.clone(), addr, miner_gross);
                                        ledger.push(entry);
                                    }
                                    if let Err(e) = ledger.save(&ledger_path) {
                                        eprintln!("[stratum] ledger save error: {e}");
                                    } else {
                                        let fee = gross * crate::accounting::POOL_FEE_BPS / 10_000;
                                        eprintln!("[stratum] BLOCK ACCEPTED height={} gross={} fee={} net={} split_to={}",
                                            job.height, gross, fee, gross.saturating_sub(fee), n_miners);
                                    }
                                }
                                state.lock().unwrap().blocks_found += 1;
                                eprintln!("[stratum] BLOCK by {} nonce={}", wallet_addr, nonce_hex);
                            }

                            // ── Update worker stats + vardiff ─────────────
                            let vardiff_update = {
                                let mut s = state.lock().unwrap();
                                s.shares_accepted += 1;
                                if let Some(w) = s.workers.get_mut(&connection_id) {
                                    w.last_seen_at_unix    = unix_now();
                                    w.accepted_shares     += 1;
                                    w.last_submit_result  = if is_block { "block" } else { "accepted" }.to_string();
                                    w.vardiff_window_shares += 1;

                                    let now     = unix_now();
                                    let elapsed = now.saturating_sub(w.vardiff_window_start).max(1);

                                    if elapsed >= VARDIFF_WINDOW_SECS {
                                        // shares/min over the window
                                        let spm = w.vardiff_window_shares
                                            .saturating_mul(60) / elapsed;
                                        let old = w.share_diff_bits;
                                        let new_bits = if spm > VARDIFF_TARGET_MAX {
                                            (old + 1).min(VARDIFF_MAX_BITS)
                                        } else if spm < VARDIFF_TARGET_MIN {
                                            old.saturating_sub(1).max(VARDIFF_MIN_BITS)
                                        } else {
                                            old
                                        };
                                        // Reset window regardless of whether diff changed.
                                        w.vardiff_window_shares = 0;
                                        w.vardiff_window_start  = now;
                                        w.share_diff_bits       = new_bits;

                                        if new_bits != old {
                                            Some((new_bits, old, spm))
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            };

                            // ── Push share to PPLNS window (non-block shares) ─────
                            // Block-finding shares are pushed inside the `if is_block`
                            // branch above to avoid double-counting.
                            if !is_block {
                                let (ledger_arc2, wname) = {
                                    let s = state.lock().unwrap();
                                    let wn = s.workers.get(&connection_id)
                                        .map(|w| w.worker_name.clone())
                                        .unwrap_or_else(|| "unknown".to_string());
                                    (s.ledger.clone(), wn)
                                };
                                let share = ShareRecord {
                                    wallet_address:  wallet_addr.clone(),
                                    worker_name:     wname,
                                    share_diff_bits: worker_diff_bits,
                                    submitted_at_unix: unix_now(),
                                };
                                ledger_arc2.lock().unwrap().push_share(share);
                                // Window persisted on next block found; no per-share save needed.
                            }

                            // Send new difficulty + re-notify outside the state lock.
                            if let Some((new_bits, old_bits, spm)) = vardiff_update {
                                let new_diff = bits_to_diff(new_bits);
                                let dir = if new_bits > old_bits { "↑" } else { "↓" };
                                eprintln!("[vardiff] {} {}{}->{} bits  ({}/min)  diff={}",
                                          wallet_addr, dir, old_bits, new_bits, spm, new_diff);

                                // 1. New difficulty
                                if !send_line(&mut writer, &json!({
                                    "id": null,
                                    "method": "mining.set_difficulty",
                                    "params": [new_diff],
                                })) {
                                    cleanup(&state, &connection_id, &peer_addr);
                                    return;
                                }

                                // 2. Re-send current job immediately so the GPU kernel
                                //    picks up the new share_bits right away.  Without
                                //    this the miner keeps submitting at the old threshold
                                //    until the next block, causing a burst of rejections.
                                let maybe_job = state.lock().unwrap().current_job.clone();
                                if let Some(ref job) = maybe_job {
                                    if !send_line(&mut writer, &notify_msg(job, new_diff)) {
                                        cleanup(&state, &connection_id, &peer_addr);
                                        return;
                                    }
                                    // Don't update last_job_id — same job_id, only diff changed.
                                }
                            }

                            "accepted"
                        }
                        Some((_, false, _)) | None => {
                            let mut s = state.lock().unwrap();
                            s.shares_rejected += 1;
                            if let Some(w) = s.workers.get_mut(&connection_id) {
                                w.last_seen_at_unix   = unix_now();
                                w.rejected_shares    += 1;
                                w.last_submit_result  = "rejected".to_string();
                            }
                            "rejected"
                        }
                    }
                } else {
                    "rejected"
                };

                if !send_line(&mut writer, &json!({"id":id,"result":result,"error":null})) { break; }
            }

            "mining.pong" => {
                if let Some(w) = state.lock().unwrap().workers.get_mut(&connection_id) {
                    w.last_seen_at_unix = unix_now();
                }
            }
            _ => {}
        }
    }

    cleanup(&state, &connection_id, &peer_addr);
}

// ── Job poller + broadcaster ───────────────────────────────────────────────────

fn run_job_poller(state: Arc<Mutex<StratumState>>) {
    let mut last_height = 0u64;
    loop {
        thread::sleep(Duration::from_secs(2));
        let (node_rpc, treasury) = {
            let s = state.lock().unwrap();
            (s.node_rpc.clone(), s.treasury.clone())
        };
        if let Some((job, raw)) = fetch_job(&node_rpc, &treasury) {
            if job.height != last_height {
                last_height = job.height;
                eprintln!("[stratum] new job height={} bits={}", job.height, job.difficulty_bits);
                let mut s = state.lock().unwrap();
                // Keep at most 2 templates: current + incoming.
                if s.job_template_cache.len() >= 2 {
                    let keep = s.current_job.as_ref().map(|j| j.job_id.clone());
                    s.job_template_cache.retain(|k, _| Some(k) == keep.as_ref());
                }
                s.job_template_cache.insert(job.job_id.clone(), raw);
                s.current_job = Some(job.clone());
                // Broadcast; prune dead senders in one pass.
                s.job_senders.retain(|_id, tx| tx.send(job.clone()).is_ok());
            }
        }
    }
}

// ── Stratum server entry ────────────────────────────────────────────────────────

pub fn run_stratum_server(state: Arc<Mutex<StratumState>>, bind: &str) {
    let listener = match TcpListener::bind(bind) {
        Ok(l)  => l,
        Err(e) => { eprintln!("[stratum] bind {}: {}", bind, e); return; }
    };
    eprintln!("[stratum] listening on {}", bind);

    // Spawn job poller thread.
    { let s = state.clone(); thread::spawn(move || run_job_poller(s)); }

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s)  => s,
            Err(e) => { eprintln!("[stratum] accept: {}", e); continue; }
        };

        let peer_addr = stream.peer_addr()
            .map(|a| a.to_string()).unwrap_or_else(|_| "unknown".to_string());
        let connection_id = format!("{}-{}", unix_now(), peer_addr);

        let state2   = state.clone();
        let conn_id2 = connection_id.clone();
        let (tx, rx) = std::sync::mpsc::channel::<StratumJob>();
        state.lock().unwrap().job_senders.insert(connection_id, tx);

        thread::spawn(move || handle_stratum_conn(stream, state2, rx, conn_id2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_template_response(with_epoch_seed: bool) -> String {
        let seed_field = if with_epoch_seed {
            format!("\"epoch_seed\": {:?},", vec![7u8; 32])
        } else {
            String::new()
        };
        format!(
            r#"{{
                "chain_id": "tensorium-testnet-0",
                {seed_field}
                "height": 7,
                "leading_zero_bits": 20,
                "previous_hash": {prev:?},
                "template": {{
                    "header": {{
                        "version": 1,
                        "chain_id": "tensorium-testnet-0",
                        "height": 7,
                        "previous_hash": {prev:?},
                        "merkle_root": {merkle:?},
                        "timestamp_seconds": 1780000000,
                        "leading_zero_bits": 20,
                        "nonce": 0
                    }},
                    "transactions": []
                }},
                "tx_count": 0
            }}"#,
            prev = vec![0x11u8; 32],
            merkle = vec![0x22u8; 32],
        )
    }

    #[test]
    fn parse_job_response_reads_epoch_seed() {
        let (job, _raw) = parse_job_response(&fixture_template_response(true))
            .expect("template with epoch_seed must parse");
        assert_eq!(job.epoch_seed, [7u8; 32]);
        assert_eq!(job.height, 7);
        assert_eq!(job.chain_id, "tensorium-testnet-0");
    }

    #[test]
    fn parse_job_response_rejects_template_without_epoch_seed() {
        // A node that does not send epoch_seed is too old for TensorHash —
        // the pool must refuse the job rather than guess a seed.
        assert!(parse_job_response(&fixture_template_response(false)).is_none());
    }

    fn fixture_job() -> StratumJob {
        StratumJob {
            job_id: "h7-test".into(),
            chain_id: "tensorium-testnet-0".into(),
            height: 7,
            previous_hash: [0x11; 32],
            merkle_root: [0x22; 32],
            epoch_seed: [0u8; 32],
            timestamp: 1_780_000_000,
            difficulty_bits: 20,
            version: 1,
        }
    }

    /// LE-hex encoding of a nonce, as miners submit it.
    fn nonce_to_le_hex(nonce: u64) -> String {
        nonce.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// CPU-mine the fixture job at `bits` leading zeros (zero epoch seed).
    /// At 12 bits this is ~4k pow_hash calls — fast even unoptimized.
    fn mine_fixture_nonce(bits: u8) -> u64 {
        let job = fixture_job();
        for nonce in 0u64.. {
            let hash = job_header(&job, nonce).pow_hash(Hash256(job.epoch_seed));
            if leading_zero_bits(&hash.0) >= bits {
                return nonce;
            }
        }
        unreachable!()
    }

    #[test]
    fn validate_share_accepts_tensorhash_share() {
        let job = fixture_job();
        let nonce = mine_fixture_nonce(12);
        let (zeros, is_share, _) =
            validate_share(&job, &nonce_to_le_hex(nonce), 12).expect("nonce parses");
        assert!(zeros >= 12);
        assert!(is_share);
    }

    #[test]
    fn validate_share_rejects_wrong_nonce_and_wrong_seed() {
        let job = fixture_job();
        let nonce = mine_fixture_nonce(12);

        // Neighbouring nonce: fails the 12-bit share check (deterministic
        // for these fixed inputs; a priori odds of passing were 2^-12).
        let (_, is_share, _) =
            validate_share(&job, &nonce_to_le_hex(nonce + 1), 12).unwrap();
        assert!(!is_share, "nonce+1 must not satisfy the share target");

        // Same nonce under a different epoch seed: different dataset,
        // different pow hash — must also fail.
        let mut other_seed_job = job.clone();
        other_seed_job.epoch_seed = [9u8; 32];
        let (_, is_share, _) =
            validate_share(&other_seed_job, &nonce_to_le_hex(nonce), 12).unwrap();
        assert!(!is_share, "share must be bound to the epoch seed");
    }

    #[test]
    fn job_header_id_hash_matches_pre_refactor_vector() {
        // Pins ledger block-hash stability: BlockHeader::hash() (double-
        // SHA256) must equal what sha256d(build_header(..)) produced before
        // this refactor. Vector computed against the pre-change code and
        // cross-checked with Python hashlib.
        let header = job_header(&fixture_job(), 424242);
        assert_eq!(
            bytes_to_hex(&header.hash().0),
            "dae08fc31e66282b654c853bf490a0677f0fd6f508ff4d1539ca4c3b88f84302",
        );
    }

    #[test]
    fn notify_msg_carries_epoch_seed_hex() {
        let job = StratumJob {
            job_id: "h7-test".into(),
            chain_id: "tensorium-testnet-0".into(),
            height: 7,
            previous_hash: [0x11; 32],
            merkle_root: [0x22; 32],
            epoch_seed: [7u8; 32],
            timestamp: 1_780_000_000,
            difficulty_bits: 20,
            version: 1,
        };
        let msg = notify_msg(&job, 1 << 28);
        assert_eq!(
            msg["params"]["epoch_seed"].as_str().unwrap(),
            "07".repeat(32),
        );
    }
}
