use std::{
    env,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tensorium_core::{chain::TESTNET, Block, ChainState, Hash256};

const DEFAULT_STATE_PATH: &str = "tensorium-testnet-state.json";
const DEFAULT_NONCE_LIMIT: u64 = 10_000_000;
const DEFAULT_RPC_BIND: &str = "127.0.0.1:23332";
const DEFAULT_P2P_BIND: &str = "127.0.0.1:23333";
const P2P_PROTOCOL_VERSION: u32 = 1;

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
        "peers" => print_manual_peers(),
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
    println!("  p2p-listen [bind]    listen for peer connections and blocks");
    println!("  p2p-connect <peer>   connect to a peer for diagnostics");
    println!("  peers                print manual peers from TENSORIUM_PEERS");
    println!();
    println!("rpc endpoints:");
    println!("  GET /getblockcount");
    println!("  GET /getdifficulty");
    println!("  GET /getblock/<height>");
    println!("  GET /getblocktemplate/<miner>");
    println!("  POST /submitblock    (also broadcasts to TENSORIUM_PEERS)");
    println!("  GET /health");
    println!();
    println!("env:");
    println!("  TENSORIUM_STATE      state file path, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_PEERS      comma-separated peers to broadcast blocks to");
    println!("  TENSORIUM_NODE_ID    node identity string");
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
    /// Peer is pushing a newly mined block.
    NewBlock { block: Box<Block> },
    /// Block was accepted; echoes height and hash.
    Ack { height: u64, hash: Hash256 },
    /// Block was rejected; includes the reason.
    Reject { reason: String },
}

/// Read bytes from `stream` one at a time until a newline, returning the line
/// without the trailing `\n`.  Returns an error if the peer closes before
/// sending a newline.
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
        return Err(format!("wrong chain_id: {} (expected {})", hello.chain_id, TESTNET.chain_id));
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

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let path = state_path.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_p2p_connection(&mut stream, &path) {
                        eprintln!("p2p connection error: {err}");
                    }
                });
            }
            Err(err) => eprintln!("p2p accept error: {err}"),
        }
    }

    Ok(())
}

/// Full lifecycle of a single inbound P2P connection:
/// 1. Read remote hello and validate.
/// 2. Send local hello.
/// 3. Loop reading P2pMsg until the peer closes.
fn handle_p2p_connection(stream: &mut TcpStream, state_path: &Path) -> Result<(), String> {
    // 1. Read remote hello
    let line = read_p2p_line(stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello: {err}"))?;
    validate_hello(&remote)?;

    // 2. Send local hello
    let state = load_state(state_path)?;
    write_p2p_line(stream, &local_hello(&state))?;

    println!(
        "p2p accepted peer={} chain_id={} height={} tip={}",
        remote.node_id, remote.chain_id, remote.height, remote.tip_hash
    );

    // 3. Message loop
    loop {
        let line = match read_p2p_line(stream) {
            Ok(line) => line,
            Err(_) => break,
        };

        let msg: P2pMsg = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(err) => {
                eprintln!("p2p invalid message from {}: {err}", remote.node_id);
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
                    Err(reason) => {
                        eprintln!(
                            "p2p rejected block from {}: {reason}",
                            remote.node_id
                        );
                        let _ = write_p2p_line(stream, &P2pMsg::Reject { reason });
                    }
                }
            }
            P2pMsg::Ack { .. } | P2pMsg::Reject { .. } => {
                eprintln!("p2p unexpected message type from {}", remote.node_id);
            }
        }
    }

    println!("p2p disconnected peer={}", remote.node_id);
    Ok(())
}

/// Load state, validate and append the block, then persist.
fn accept_peer_block(state_path: &Path, block: Block) -> Result<(u64, Hash256), String> {
    let mut state = load_state(state_path)?;
    state
        .submit_block(&TESTNET, block, now_seconds())
        .map_err(|err| err.to_string())?;
    let tip = state.tip().expect("block was just pushed");
    let height = tip.header.height;
    let hash = tip.hash();
    save_state(state_path, &state)?;
    Ok((height, hash))
}

// ---------------------------------------------------------------------------
// P2P client — push a block to a single peer
// ---------------------------------------------------------------------------

/// Connect to `peer`, handshake, push `block`, and wait for Ack/Reject.
/// Returns the accepted height on success.
fn push_block_to_peer(peer: &str, block: &Block, state: &ChainState) -> Result<u64, String> {
    let mut stream =
        TcpStream::connect(peer).map_err(|err| format!("connect {peer}: {err}"))?;

    // Send hello
    write_p2p_line(&mut stream, &local_hello(state))?;

    // Read remote hello
    let line = read_p2p_line(&mut stream)?;
    let remote: P2pHello =
        serde_json::from_str(&line).map_err(|err| format!("parse hello from {peer}: {err}"))?;
    validate_hello(&remote)?;

    // Send NewBlock
    write_p2p_line(
        &mut stream,
        &P2pMsg::NewBlock {
            block: Box::new(block.clone()),
        },
    )?;

    // Read response
    let line = read_p2p_line(&mut stream)?;
    let response: P2pMsg = serde_json::from_str(&line)
        .map_err(|err| format!("parse response from {peer}: {err}"))?;

    match response {
        P2pMsg::Ack { height, .. } => Ok(height),
        P2pMsg::Reject { reason } => Err(format!("block rejected by {peer}: {reason}")),
        P2pMsg::NewBlock { .. } => Err(format!("unexpected NewBlock from {peer}")),
    }
}

/// Broadcast `block` to every peer in `TENSORIUM_PEERS`.  Errors per peer are
/// logged but do not abort the broadcast to remaining peers.
fn broadcast_block_to_peers(block: &Block, state: &ChainState) {
    let peers = configured_peers();
    if peers.is_empty() {
        return;
    }
    for peer in &peers {
        match push_block_to_peer(peer, block, state) {
            Ok(height) => {
                println!("broadcast block to {peer} accepted height={height}");
            }
            Err(err) => {
                eprintln!("broadcast block to {peer} failed: {err}");
            }
        }
    }
}

fn configured_peers() -> Vec<String> {
    let raw = env::var("TENSORIUM_PEERS").unwrap_or_default();
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
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

// ---------------------------------------------------------------------------
// HTTP RPC server
// ---------------------------------------------------------------------------

fn serve_rpc(bind: &str, state_path: PathBuf) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
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
    let mut buffer = [0u8; 65_536];
    let bytes_read = stream
        .read(&mut buffer)
        .map_err(|err| format!("failed to read request: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let parsed = parse_http_request(&request).ok_or_else(|| "invalid HTTP request".to_owned())?;

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
            let Some(block) = state.blocks.iter().find(|block| block.header.height == height)
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
            let block = state
                .candidate_block(&TESTNET, now_seconds(), miner)
                .map_err(|err| err.to_string())?;
            write_json_response(
                stream,
                200,
                &json!({
                    "chain_id": TESTNET.chain_id,
                    "height": block.header.height,
                    "previous_hash": block.header.previous_hash,
                    "leading_zero_bits": block.header.leading_zero_bits,
                    "template": block,
                }),
            )
        }
        ("POST", "/submitblock") => {
            let block: Block = serde_json::from_str(parsed.body)
                .map_err(|err| format!("failed to parse submitted block: {err}"))?;
            let mut state = load_state(state_path)?;
            state
                .submit_block(&TESTNET, block, now_seconds())
                .map_err(|err| err.to_string())?;

            let tip = state.tip().expect("block was just pushed");
            let height = tip.header.height;
            let hash = tip.hash();
            let block_to_broadcast = tip.clone();

            save_state(state_path, &state)?;

            // Broadcast to configured peers; errors are logged, not fatal.
            broadcast_block_to_peers(&block_to_broadcast, &state);

            write_json_response(
                stream,
                200,
                &json!({
                    "accepted": true,
                    "height": height,
                    "hash": hash,
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
