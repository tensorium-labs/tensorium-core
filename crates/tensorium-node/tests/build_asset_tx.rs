//! Integration test: /buildAssetTx error path on a local devnet (TESTNET params)
//! node bound to a fixed port with a fresh state dir.
use std::process::Command;
use std::time::Duration;
use tensorium_core::wallet::WalletKeypair;

fn wait_for_health(base: &str) {
    for _ in 0..50 {
        if http_get(base, "/health").is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("node did not become healthy in time");
}

// Tiny GET helper using std TcpStream.
fn http_get(base: &str, path: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(base).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {base}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|e| e.to_string())?;
    if !status.contains("200") {
        return Err(status);
    }
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line == "\r\n" {
            break;
        }
    }
    let mut body = String::new();
    reader.read_to_string(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn http_post(base: &str, path: &str, body: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(base).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {base}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|e| e.to_string())?;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line == "\r\n" {
            break;
        }
    }
    let mut resp_body = String::new();
    reader.read_to_string(&mut resp_body).map_err(|e| e.to_string())?;
    if !status.contains("200") {
        return Err(format!("{}: {}", status.trim(), resp_body));
    }
    Ok(resp_body)
}

#[test]
fn build_asset_tx_insufficient_balance() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("devnet_state");
    let mempool = dir.path().join("devnet_mempool.json");
    let bin = env!("CARGO_BIN_EXE_tensorium-node");

    // Initialize a fresh devnet (TESTNET params: low difficulty, fast).
    let status = Command::new(bin)
        .arg("devnet")
        .arg("init")
        .env("TENSORIUM_DEVNET_STATE", &state)
        .env("TENSORIUM_DEVNET_MEMPOOL", &mempool)
        .status()
        .unwrap();
    assert!(status.success());

    let bind = "127.0.0.1:39001";
    let mut child = Command::new(bin)
        .arg("devnet")
        .arg("rpc")
        .arg(bind)
        .env("TENSORIUM_DEVNET_STATE", &state)
        .env("TENSORIUM_DEVNET_MEMPOOL", &mempool)
        .spawn()
        .unwrap();
    wait_for_health(bind);

    let keypair = WalletKeypair::generate();
    let body = format!(
        r#"{{"op":"issue","from":"{}","ticker":"GOLD","decimals":0,"supply":1000000,"name":"Gold Token"}}"#,
        keypair.address.0
    );
    let err = http_post(bind, "/buildAssetTx", &body).unwrap_err();
    assert!(err.contains("insufficient mature balance"), "unexpected error: {err}");

    let _ = child.kill();
}
