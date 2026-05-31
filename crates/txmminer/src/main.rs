use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tensorium_core::{
    block::BlockHeader,
    pow::header_meets_work,
    Block,
};

const DEFAULT_RPC: &str = "127.0.0.1:23332";
const DEFAULT_MINER: &str = "local-dev-miner";

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let rpc  = args.get(1).map(String::as_str).unwrap_or(DEFAULT_RPC);
    let miner = args.get(2).map(String::as_str).unwrap_or(DEFAULT_MINER);
    let threads = args
        .get(3)
        .map(|s| s.parse::<usize>().map_err(|_| "invalid thread count"))
        .transpose()?
        .unwrap_or_else(|| thread::available_parallelism().map(|n| n.get()).unwrap_or(4));

    println!("txmminer  rpc={rpc}  miner={miner}  threads={threads}");
    println!("Press Ctrl+C to stop.\n");

    loop {
        let template = match get_block_template(rpc, miner) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("template error: {e} — retry in 3s");
                thread::sleep(Duration::from_secs(3));
                continue;
            }
        };

        let height = template.template.header.height;
        let diff   = template.template.header.leading_zero_bits;
        print!("mining  height={height}  bits={diff}  threads={threads}  … ");
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let started      = Instant::now();
        let total_hashes = Arc::new(AtomicU64::new(0));

        match mine_parallel(template.template.header.clone(), threads, total_hashes.clone()) {
            Some(mined_header) => {
                let elapsed  = started.elapsed();
                let hashes   = total_hashes.load(Ordering::Relaxed);
                let hashrate = hashes as f64 / elapsed.as_secs_f64().max(0.001);

                let mut block = template.template;
                block.header  = mined_header;

                match submit_block(rpc, &block) {
                    Ok(resp) if resp.accepted => println!(
                        "✓  height={}  nonce={}  {:.3}s  {}",
                        resp.height,
                        block.header.nonce,
                        elapsed.as_secs_f64(),
                        fmt_hashrate(hashrate),
                    ),
                    Ok(_)    => eprintln!("✗ block rejected (stale) — getting fresh template"),
                    Err(e)   => eprintln!("✗ submit error: {e}"),
                }
            }
            None => { /* u64 exhausted — practically impossible */ }
        }
    }
}

/// Parallel nonce search across `threads` threads.
/// Each thread covers a strided slice of the u64 nonce space.
/// Returns as soon as any thread finds a valid header.
fn mine_parallel(
    header: BlockHeader,
    threads: usize,
    total_hashes: Arc<AtomicU64>,
) -> Option<BlockHeader> {
    let done   = Arc::new(AtomicBool::new(false));
    let winner = Arc::new(Mutex::new(None::<BlockHeader>));
    let stride = threads as u64;

    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let mut h         = header.clone();
            let done          = done.clone();
            let winner        = winner.clone();
            let total_hashes  = total_hashes.clone();
            let start         = t as u64;

            thread::spawn(move || {
                let mut nonce       = start;
                let mut local_count = 0u64;
                const FLUSH: u64    = 500_000;

                loop {
                    if done.load(Ordering::Relaxed) { break; }

                    h.nonce = nonce;
                    local_count += 1;

                    if header_meets_work(&h) {
                        done.store(true, Ordering::SeqCst);
                        total_hashes.fetch_add(local_count, Ordering::Relaxed);
                        *winner.lock().unwrap() = Some(h);
                        return;
                    }

                    if local_count == FLUSH {
                        total_hashes.fetch_add(FLUSH, Ordering::Relaxed);
                        local_count = 0;
                    }

                    nonce = match nonce.checked_add(stride) {
                        Some(n) => n,
                        None    => {
                            total_hashes.fetch_add(local_count, Ordering::Relaxed);
                            break;
                        }
                    };
                }
            })
        })
        .collect();

    for h in handles { h.join().ok(); }
    let result = winner.lock().unwrap().clone();
    result
}

fn fmt_hashrate(hps: f64) -> String {
    match hps as u64 {
        h if h >= 1_000_000_000 => format!("{:.2} GH/s", hps / 1e9),
        h if h >= 1_000_000     => format!("{:.2} MH/s", hps / 1e6),
        h if h >= 1_000         => format!("{:.2} KH/s", hps / 1e3),
        _                       => format!("{:.0} H/s",  hps),
    }
}

// ── RPC helpers ──────────────────────────────────────────────────────────────

fn get_block_template(rpc: &str, miner: &str) -> Result<BlockTemplateResponse, String> {
    let raw = http_get(rpc, &format!("/getblocktemplate/{miner}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("template parse: {e}"))
}

fn submit_block(rpc: &str, block: &Block) -> Result<SubmitBlockResponse, String> {
    let body = serde_json::to_string(block).map_err(|e| format!("encode: {e}"))?;
    let raw  = http_post(rpc, "/submitblock", &body)?;
    serde_json::from_str(&raw).map_err(|e| format!("submit parse: {e}"))
}

fn http_get(rpc: &str, path: &str) -> Result<String, String> {
    send_http(rpc, &format!("GET {path} HTTP/1.1\r\nhost: {rpc}\r\nconnection: close\r\n\r\n"))
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
    let mut stream = TcpStream::connect(rpc)
        .map_err(|e| format!("connect {rpc}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.write_all(request.as_bytes())
        .map_err(|e| format!("send: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response)
        .map_err(|e| format!("read: {e}"))?;

    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or("invalid HTTP response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("RPC error: {body}"));
    }
    Ok(body.to_owned())
}

#[derive(Debug, Deserialize)]
struct BlockTemplateResponse {
    template: Block,
}

#[derive(Debug, Deserialize)]
struct SubmitBlockResponse {
    accepted: bool,
    height: u64,
    hash: tensorium_core::Hash256,
}
