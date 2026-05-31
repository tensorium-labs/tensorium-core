use std::{
    env,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::json;
use tensorium_core::{chain::TESTNET, ChainState};

const DEFAULT_STATE_PATH: &str = "tensorium-testnet-state.json";
const DEFAULT_NONCE_LIMIT: u64 = 10_000_000;
const DEFAULT_RPC_BIND: &str = "127.0.0.1:23332";

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
            state
                .init_genesis(&TESTNET, now_seconds(), DEFAULT_NONCE_LIMIT)
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
        _ => print_help(),
    }

    Ok(())
}

fn state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_STATE_PATH))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs()
}

fn load_state(path: &Path) -> Result<ChainState, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn save_state(path: &Path, state: &ChainState) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(state)
        .map_err(|err| format!("failed to serialize chain state: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("failed to write {}: {err}", path.display()))
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
    println!();
    println!("rpc endpoints:");
    println!("  GET /getblockcount");
    println!("  GET /getdifficulty");
    println!("  GET /getblock/<height>");
    println!("  GET /health");
    println!();
    println!("env:");
    println!("  TENSORIUM_STATE      state file path, default {DEFAULT_STATE_PATH}");
}

fn serve_rpc(bind: &str, state_path: PathBuf) -> Result<(), String> {
    let listener = TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    println!("tensorium RPC listening on http://{bind}");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
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

fn handle_rpc_stream(stream: &mut TcpStream, state_path: &Path) -> Result<(), String> {
    let mut buffer = [0u8; 4096];
    let bytes_read = stream
        .read(&mut buffer)
        .map_err(|err| format!("failed to read request: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let path = parse_http_path(&request).ok_or_else(|| "invalid HTTP request".to_owned())?;

    match path.as_str() {
        "/health" => write_json_response(stream, 200, &json!({ "ok": true })),
        "/getblockcount" => {
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
        "/getdifficulty" => {
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
        path if path.starts_with("/getblock/") => {
            let height = path
                .trim_start_matches("/getblock/")
                .parse::<u64>()
                .map_err(|err| format!("invalid block height: {err}"))?;
            let state = load_state(state_path)?;
            let Some(block) = state.blocks.iter().find(|block| block.header.height == height) else {
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
        _ => write_json_response(stream, 404, &RpcError::new("unknown RPC endpoint")),
    }
}

fn parse_http_path(request: &str) -> Option<String> {
    let request_line = request.lines().next()?;
    let mut parts = request_line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(path)) => Some(path.to_owned()),
        _ => None,
    }
}

fn write_json_response<T: Serialize>(
    stream: &mut TcpStream,
    status_code: u16,
    body: &T,
) -> Result<(), String> {
    let status_text = match status_code {
        200 => "OK",
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
