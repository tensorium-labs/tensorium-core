use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    time::Instant,
};

use serde::Deserialize;
use tensorium_core::{pow::mine_header, Block};

const DEFAULT_RPC: &str = "127.0.0.1:23332";
const DEFAULT_MINER: &str = "local-dev-miner";
const DEFAULT_MAX_NONCE: u64 = 10_000_000;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let rpc = args.get(1).map(String::as_str).unwrap_or(DEFAULT_RPC);
    let miner = args.get(2).map(String::as_str).unwrap_or(DEFAULT_MINER);
    let max_nonce = args
        .get(3)
        .map(|raw| raw.parse::<u64>())
        .transpose()
        .map_err(|err| format!("invalid max nonce: {err}"))?
        .unwrap_or(DEFAULT_MAX_NONCE);

    let template = get_block_template(rpc, miner)?;
    println!(
        "mining height={} difficulty_bits={} max_nonce={}",
        template.template.header.height, template.template.header.leading_zero_bits, max_nonce
    );

    let started = Instant::now();
    let Some(mined_header) = mine_header(template.template.header.clone(), max_nonce) else {
        return Err(format!("no valid nonce found before {max_nonce}"));
    };
    let elapsed = started.elapsed();
    let mut mined_block = template.template;
    mined_block.header = mined_header;

    let response = submit_block(rpc, &mined_block)?;
    println!(
        "accepted={} height={} nonce={} hash={} elapsed_ms={}",
        response.accepted,
        response.height,
        mined_block.header.nonce,
        response.hash,
        elapsed.as_millis()
    );

    Ok(())
}

fn get_block_template(rpc: &str, miner: &str) -> Result<BlockTemplateResponse, String> {
    let response = http_get(rpc, &format!("/getblocktemplate/{miner}"))?;
    serde_json::from_str(&response).map_err(|err| format!("invalid template response: {err}"))
}

fn submit_block(rpc: &str, block: &Block) -> Result<SubmitBlockResponse, String> {
    let body = serde_json::to_string(block).map_err(|err| format!("block encode failed: {err}"))?;
    let response = http_post(rpc, "/submitblock", &body)?;
    serde_json::from_str(&response).map_err(|err| format!("invalid submit response: {err}"))
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
        TcpStream::connect(rpc).map_err(|err| format!("failed to connect to {rpc}: {err}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send request: {err}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| format!("failed to read response: {err}"))?;

    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_owned())?;
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
