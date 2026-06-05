//! Tensorium Stratum Protocol v1 — TCP mining pool server.
//! Port 3333 (alongside existing HTTP pool on port 23336).

use crate::accounting::{PayoutEntry, PayoutLedger};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct StratumJob {
    pub job_id:          String,
    pub chain_id:        String,
    pub height:          u64,
    pub previous_hash:   [u8; 32],
    pub merkle_root:     [u8; 32],
    pub timestamp:       u64,
    pub difficulty_bits: u8,
    pub version:         u32,
}

#[derive(Clone, Debug)]
pub struct WorkerSession {
    pub connection_id: String,
    pub worker_name: String,
    pub wallet_address: String,
    pub peer_addr: String,
    pub authorized_at_unix: u64,
    pub last_seen_at_unix: u64,
    pub accepted_shares: u64,
    pub rejected_shares: u64,
    pub last_submit_result: String,
}

pub struct StratumState {
    pub current_job:      Option<StratumJob>,
    pub share_diff:       u64,
    pub node_rpc:         String,
    pub treasury:         String,
    /// connection_id → worker session
    pub workers:          HashMap<String, WorkerSession>,
    pub shares_accepted:  u64,
    pub shares_rejected:  u64,
    pub blocks_found:     u64,
    /// connection_id → job sender (cleaned up on disconnect)
    pub job_senders:      HashMap<String, std::sync::mpsc::Sender<StratumJob>>,
    /// job_id → raw template JSON (last 2 jobs, for stale share lookup)
    pub job_template_cache: HashMap<String, String>,
    /// Shared payout ledger — same instance as the HTTP pool
    pub ledger:           Arc<Mutex<PayoutLedger>>,
    pub ledger_path:      PathBuf,
}

impl StratumState {
    pub fn new(
        node_rpc: String,
        treasury: String,
        share_diff: u64,
        ledger: Arc<Mutex<PayoutLedger>>,
        ledger_path: PathBuf,
    ) -> Self {
        Self {
            current_job: None,
            share_diff,
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

    /// Stats for HTTP /pool/stratum endpoint
    pub fn stats_json(&self) -> serde_json::Value {
        let active_workers: Vec<Value> = self
            .workers
            .values()
            .map(|worker| {
                json!({
                    "connection_id": worker.connection_id,
                    "worker_name": worker.worker_name,
                    "wallet_address": worker.wallet_address,
                    "peer_addr": worker.peer_addr,
                    "authorized_at_unix": worker.authorized_at_unix,
                    "last_seen_at_unix": worker.last_seen_at_unix,
                    "accepted_shares": worker.accepted_shares,
                    "rejected_shares": worker.rejected_shares,
                    "last_submit_result": worker.last_submit_result,
                })
            })
            .collect();

        json!({
            "stratum_workers": self.job_senders.len(),
            "authorized_workers": self.workers.len(),
            "stratum_port": 3333,
            "share_difficulty": self.share_diff,
            "shares_accepted": self.shares_accepted,
            "shares_rejected": self.shares_rejected,
            "blocks_found": self.blocks_found,
            "active_workers": active_workers,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── SHA256d ───────────────────────────────────────────────────────────────────

fn sha256d(data: &[u8]) -> [u8; 32] {
    let first  = Sha256::digest(data);
    let second = Sha256::digest(&first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

fn leading_zero_bits(hash: &[u8; 32]) -> u8 {
    let mut bits = 0u8;
    for &byte in hash.iter() {
        if byte == 0 {
            bits += 8;
        } else {
            bits += byte.leading_zeros() as u8;
            break;
        }
    }
    bits
}

// ── Header builder ─────────────────────────────────────────────────────────────

fn build_header(job: &StratumJob, nonce: u64) -> Vec<u8> {
    let cid = job.chain_id.as_bytes();
    let mut h = Vec::with_capacity(4 + cid.len() + 8 + 32 + 32 + 8 + 1 + 8);
    h.extend_from_slice(&job.version.to_le_bytes());
    h.extend_from_slice(cid);
    h.extend_from_slice(&job.height.to_le_bytes());
    h.extend_from_slice(&job.previous_hash);
    h.extend_from_slice(&job.merkle_root);
    h.extend_from_slice(&job.timestamp.to_le_bytes());
    h.push(job.difficulty_bits);
    h.extend_from_slice(&nonce.to_le_bytes());
    h
}

// ── Nonce encoding helpers ─────────────────────────────────────────────────────

/// Parse a nonce from the little-endian hex string sent by the miner.
/// The miner encodes the nonce as 8 LE bytes formatted as hex (LSB first),
/// e.g. nonce=1000 (0x3E8) → "e803000000000000".
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

fn validate_share(job: &StratumJob, nonce_hex: &str, share_diff: u64) -> Option<(u8, bool, bool)> {
    let nonce = le_hex_to_u64(nonce_hex)?;
    let header = build_header(job, nonce);
    let hash   = sha256d(&header);
    let zeros  = leading_zero_bits(&hash);

    let share_bits = {
        let mut d = share_diff;
        let mut b = 0u8;
        while d > 1 { d >>= 1; b += 1; }
        b
    };

    let is_share = zeros >= share_bits;
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

// ── Gross reward extraction from cached template ───────────────────────────────

fn gross_from_template(raw: &str) -> u64 {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v["template"]["transactions"][0]["outputs"][0]["value_atoms"].as_u64()
        })
        .unwrap_or(0)
}

// ── Job fetch from node ────────────────────────────────────────────────────────

/// Fetch a job from the node. Returns (job, raw_template_json) on success.
pub fn fetch_job(node_rpc: &str, treasury: &str) -> Option<(StratumJob, String)> {
    let url = format!("http://{}/getblocktemplate/{}", node_rpc, treasury);
    let resp = http_get_body(&url)?;
    let v: Value = serde_json::from_str(&resp).ok()?;

    let hdr = v["template"]["header"].as_object()?;

    let chain_id    = hdr["chain_id"].as_str()?.to_string();
    let height      = hdr["height"].as_u64()?;
    let diff_bits   = hdr["leading_zero_bits"].as_u64()? as u8;
    let timestamp   = hdr["timestamp_seconds"].as_u64()?;
    let version     = hdr["version"].as_u64().unwrap_or(1) as u32;

    let prev = parse_byte_array(hdr.get("previous_hash")?)?;
    let mroot= parse_byte_array(hdr.get("merkle_root")?)?;

    let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_millis();
    let job_id = format!("h{}-{}", height, ms);

    let job = StratumJob {
        job_id,
        chain_id,
        height,
        previous_hash: prev,
        merkle_root: mroot,
        timestamp,
        difficulty_bits: diff_bits,
        version,
    };
    Some((job, resp))
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
    write!(conn, "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n", path, host, port).ok()?;
    let mut resp = String::new();
    conn.read_to_string(&mut resp).ok()?;
    Some(resp.split("\r\n\r\n").nth(1)?.to_string())
}

// ── Submit block to node ────────────────────────────────────────────────────────

/// Submit a found block to the node. Uses the cached raw template JSON so the
/// full coinbase transaction is preserved (only the nonce is updated).
/// Returns true if the node accepted the block.
fn submit_block(node_rpc: &str, job: &StratumJob, nonce: u64, raw_template: Option<&str>) -> bool {
    use std::io::Read;

    let colon = match node_rpc.rfind(':') {
        Some(c) => c,
        None    => return false,
    };
    let host = &node_rpc[..colon];
    let port = &node_rpc[colon + 1..];

    eprintln!("[stratum] BLOCK! height={} nonce={} job={}",
              job.height, nonce, job.job_id);

    // Use cached template with correct coinbase; fall back to reconstructed header if unavailable.
    let body = if let Some(raw) = raw_template {
        if let Ok(mut v) = serde_json::from_str::<Value>(raw) {
            v["template"]["header"]["nonce"] = json!(nonce);
            v.to_string()
        } else {
            build_fallback_body(job, nonce)
        }
    } else {
        build_fallback_body(job, nonce)
    };

    let mut conn = match TcpStream::connect(format!("{}:{}", host, port)) {
        Ok(c) => c,
        Err(_)=> return false,
    };
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let req = format!(
        "POST /submitblock HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        host, port, body.len(), body
    );
    if conn.write_all(req.as_bytes()).is_err() { return false; }
    let mut resp = String::new();
    conn.read_to_string(&mut resp).ok();
    resp.contains("accepted") || resp.contains("true")
}

fn build_fallback_body(job: &StratumJob, nonce: u64) -> String {
    json!({
        "template": {
            "header": {
                "chain_id":          job.chain_id,
                "height":            job.height,
                "previous_hash":     job.previous_hash.to_vec(),
                "merkle_root":       job.merkle_root.to_vec(),
                "timestamp_seconds": job.timestamp,
                "leading_zero_bits": job.difficulty_bits,
                "version":           job.version,
                "nonce":             nonce
            },
            "transactions": []
        }
    }).to_string()
}

// ── Mining.notify message builder ─────────────────────────────────────────────

fn notify_msg(job: &StratumJob, share_diff: u64) -> Value {
    json!({
        "id": null,
        "method": "mining.notify",
        "params": {
            "job_id":           job.job_id,
            "chain_id":         job.chain_id,
            "height":           job.height,
            "previous_hash":    bytes_to_hex(&job.previous_hash),
            "merkle_root":      bytes_to_hex(&job.merkle_root),
            "timestamp":        job.timestamp,
            "difficulty_bits":  job.difficulty_bits,
            "share_difficulty": share_diff,
            "clean_jobs":       true
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
    stream: TcpStream,
    state:  Arc<Mutex<StratumState>>,
    job_rx: std::sync::mpsc::Receiver<StratumJob>,
    connection_id: String,
) {
    let peer_addr = stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Short read timeout so the pool can proactively send pings even when the
    // miner is silent (e.g. immediately after a new job before the first share).
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut writer = match stream.try_clone() {
        Ok(w)  => w,
        Err(_) => {
            state.lock().unwrap().job_senders.remove(&connection_id);
            return;
        }
    };
    let reader = BufReader::new(stream);

    let mut authorized   = false;
    let mut wallet_addr  = String::new();
    let mut last_job_id  = String::new();
    let mut last_ping    = Instant::now();

    for line_res in reader.lines() {
        /* Drain pending new jobs */
        while let Ok(job) = job_rx.try_recv() {
            if authorized {
                let diff = state.lock().unwrap().share_diff;
                if !send_line(&mut writer, &notify_msg(&job, diff)) {
                    state.lock().unwrap().job_senders.remove(&connection_id);
                    return;
                }
                last_job_id = job.job_id.clone();
            }
        }

        let line = match line_res {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                // Read timeout — send ping every 5s to break the miner's recv
                // block so it can submit any GPU shares it has queued up.
                if authorized && last_ping.elapsed().as_secs() >= 5 {
                    let ping = json!({"id":null,"method":"mining.ping","params":[]});
                    if !send_line(&mut writer, &ping) {
                        state.lock().unwrap().job_senders.remove(&connection_id);
                        return;
                    }
                    last_ping = Instant::now();
                }
                continue;
            }
            Err(_) => break,
        };
        if line.is_empty() { continue; }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v)  => v,
            Err(_) => continue,
        };

        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id     = msg["id"].clone();

        match method.as_str() {
            "mining.subscribe" => {
                let resp = json!({
                    "id": id,
                    "result": {
                        "session_id": format!("s{}", SystemTime::now()
                            .duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()),
                        "protocol":   "tensorium-stratum/1",
                        "nonce_bits": 64
                    },
                    "error": null
                });
                if !send_line(&mut writer, &resp) { break; }
            }

            "mining.authorize" => {
                let auth = msg["params"][0].as_str().unwrap_or("").to_string();
                let parts: Vec<&str> = auth.splitn(2, '.').collect();
                wallet_addr = parts[0].to_string();
                let wname   = parts.get(1).copied().unwrap_or("default").to_string();

                {
                    let mut s = state.lock().unwrap();
                    s.workers.insert(connection_id.clone(), WorkerSession {
                        connection_id: connection_id.clone(),
                        worker_name: wname,
                        wallet_address: wallet_addr.clone(),
                        peer_addr: peer_addr.clone(),
                        authorized_at_unix: unix_now(),
                        last_seen_at_unix: unix_now(),
                        accepted_shares: 0,
                        rejected_shares: 0,
                        last_submit_result: "authorized".to_string(),
                    });
                }

                authorized = true;

                /* 1. auth response */
                if !send_line(&mut writer, &json!({"id":id,"result":true,"error":null})) { break; }

                /* 2. set_difficulty */
                let diff = state.lock().unwrap().share_diff;
                if !send_line(&mut writer,
                    &json!({"id":null,"method":"mining.set_difficulty","params":[diff]})) { break; }

                /* 3. current job (if any) */
                let maybe_job = state.lock().unwrap().current_job.clone();
                if let Some(job) = maybe_job {
                    let diff2 = state.lock().unwrap().share_diff;
                    if !send_line(&mut writer, &notify_msg(&job, diff2)) { break; }
                    last_job_id = job.job_id.clone();
                }
            }

            "mining.submit" => {
                if !authorized { continue; }
                let params    = &msg["params"];
                let job_id    = params["job_id"].as_str().unwrap_or("").to_string();
                let nonce_hex = params["nonce"].as_str().unwrap_or("0").to_string();

                let (job_opt, share_diff) = {
                    let s = state.lock().unwrap();
                    (s.current_job.clone(), s.share_diff)
                };

                let result = if let Some(ref job) = job_opt {
                    /* Allow current job and the previous job_id (1 stale allowed) */
                    let stale = job.job_id != job_id && job_id != last_job_id;
                    if stale {
                        let mut s = state.lock().unwrap();
                        s.shares_rejected += 1;
                        send_line(&mut writer, &json!({"id":id,"result":"rejected","error":"stale"}));
                        continue;
                    }

                    match validate_share(job, &nonce_hex, share_diff) {
                        Some((_, true, is_block)) => {
                            if is_block {
                                let nonce = le_hex_to_u64(&nonce_hex).unwrap_or(0);

                                // Look up cached template for full coinbase submission.
                                let (node_rpc, raw_template, ledger_arc, ledger_path) = {
                                    let s = state.lock().unwrap();
                                    let raw = s.job_template_cache.get(&job.job_id)
                                        .or_else(|| s.job_template_cache.get(&job_id))
                                        .cloned();
                                    (s.node_rpc.clone(), raw, s.ledger.clone(), s.ledger_path.clone())
                                };

                                let accepted = submit_block(&node_rpc, job, nonce, raw_template.as_deref());

                                if accepted {
                                    // Compute block hash from header bytes for ledger entry.
                                    let header_bytes = build_header(job, nonce);
                                    let hash_bytes   = sha256d(&header_bytes);
                                    let block_hash   = bytes_to_hex(&hash_bytes);

                                    // Gross reward from cached template coinbase.
                                    let gross = raw_template.as_deref()
                                        .map(gross_from_template)
                                        .unwrap_or(0);
                                    if gross == 0 {
                                        eprintln!("[stratum] WARNING: could not extract gross reward from template for height={}", job.height);
                                    }

                                    let entry = PayoutEntry::new(
                                        job.height,
                                        block_hash,
                                        wallet_addr.clone(),
                                        gross,
                                    );

                                    // state mutex is NOT held here — safe to lock ledger
                                    let mut ledger = ledger_arc.lock().unwrap();
                                    ledger.push(entry);
                                    if let Err(e) = ledger.save(&ledger_path) {
                                        eprintln!("[stratum] ledger save error: {e}");
                                    } else {
                                        let fee = gross * crate::accounting::POOL_FEE_BPS / 10_000;
                                        let net = gross.saturating_sub(fee);
                                        eprintln!("[stratum] BLOCK ACCEPTED height={} miner={} gross={} fee={} net={}",
                                            job.height, wallet_addr, gross, fee, net);
                                    }
                                }

                                let mut s = state.lock().unwrap();
                                s.blocks_found += 1;
                                eprintln!("[stratum] BLOCK by {} nonce={}", wallet_addr, nonce_hex);
                            }

                            let mut s = state.lock().unwrap();
                            s.shares_accepted += 1;
                            if let Some(worker) = s.workers.get_mut(&connection_id) {
                                worker.last_seen_at_unix = unix_now();
                                worker.accepted_shares += 1;
                                worker.last_submit_result = if is_block {
                                    "block".to_string()
                                } else {
                                    "accepted".to_string()
                                };
                            }
                            "accepted"
                        }
                        Some((_, false, _)) | None => {
                            let mut s = state.lock().unwrap();
                            s.shares_rejected += 1;
                            if let Some(worker) = s.workers.get_mut(&connection_id) {
                                worker.last_seen_at_unix = unix_now();
                                worker.rejected_shares += 1;
                                worker.last_submit_result = "rejected".to_string();
                            }
                            "rejected"
                        }
                    }
                } else {
                    "rejected"
                };

                if !send_line(&mut writer, &json!({"id":id,"result":result,"error":null})) { break; }
            }

            "mining.pong" => { /* client responded to our ping — reset timer */ }
            _ => { /* ignore unknown methods */ }
        }
    }

    // Clean up on disconnect.
    let mut s = state.lock().unwrap();
    s.workers.remove(&connection_id);
    s.job_senders.remove(&connection_id);
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
                // Prune template cache: keep previous job + new job (≤2 entries).
                if s.job_template_cache.len() >= 2 {
                    // Remove entries that are neither the current nor the incoming job.
                    let keep_id = s.current_job.as_ref().map(|j| j.job_id.clone());
                    s.job_template_cache.retain(|k, _| Some(k) == keep_id.as_ref());
                }
                s.job_template_cache.insert(job.job_id.clone(), raw);
                s.current_job = Some(job.clone());

                // Broadcast to all connected workers; remove dead senders.
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

    /* Spawn job poller */
    {
        let s = state.clone();
        thread::spawn(move || run_job_poller(s));
    }

    /* Accept loop */
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s)  => s,
            Err(e) => { eprintln!("[stratum] accept: {}", e); continue; }
        };

        let peer_addr = stream.peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let connection_id = format!("{}-{}", unix_now(), peer_addr);

        let state2 = state.clone();
        let conn_id2 = connection_id.clone();
        let (tx, rx) = std::sync::mpsc::channel::<StratumJob>();
        state.lock().unwrap().job_senders.insert(connection_id, tx);

        thread::spawn(move || handle_stratum_conn(stream, state2, rx, conn_id2));
    }
}
