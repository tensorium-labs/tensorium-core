//! Auto-payout scheduler for the Tensorium reference pool.
//!
//! Runs as a background thread inside `tensorium-pool serve`.
//! Every `interval_secs`, it scans the shared PayoutLedger for miners
//! whose total unpaid net balance exceeds `threshold_atoms`.  For each
//! qualifying miner it calls `txmwallet send` (treasury wallet → miner)
//! and, on success, marks those entries as paid and persists the ledger.
//!
//! Activation: set both TENSORIUM_POOL_TREASURY_WALLET and
//! TENSORIUM_POOL_TREASURY_PASSPHRASE environment variables.

use crate::accounting::PayoutLedger;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;

// ── Config ────────────────────────────────────────────────────────────────────

pub struct PayoutConfig {
    /// Path to the pool treasury wallet JSON file.
    pub wallet_path: PathBuf,
    /// Passphrase for the treasury wallet (injected via env, never logged).
    pub wallet_passphrase: String,
    /// Node RPC address used by txmwallet (`host:port`).
    pub node_rpc: String,
    /// Minimum unpaid net balance in atoms before a payout is triggered.
    pub threshold_atoms: u64,
    /// How often to run the payout check (seconds).
    pub interval_secs: u64,
    /// Path to the ledger file (for persisting mark-paid updates).
    pub ledger_path: PathBuf,
}

const ATOMS_PER_TXM: f64 = 1_000_000_00.0; // 1e8

fn atoms_to_txm(a: u64) -> f64 {
    a as f64 / ATOMS_PER_TXM
}

// ── Scheduler entry point ─────────────────────────────────────────────────────

pub fn run_payout_scheduler(ledger: Arc<Mutex<PayoutLedger>>, cfg: PayoutConfig) {
    eprintln!(
        "[payout] scheduler started  threshold={:.2} TXM  interval={}s  wallet={}",
        atoms_to_txm(cfg.threshold_atoms),
        cfg.interval_secs,
        cfg.wallet_path.display()
    );

    loop {
        run_cycle(&ledger, &cfg);
        thread::sleep(Duration::from_secs(cfg.interval_secs));
    }
}

// ── One payout cycle ──────────────────────────────────────────────────────────

fn run_cycle(ledger: &Arc<Mutex<PayoutLedger>>, cfg: &PayoutConfig) {
    // Collect all miners whose unpaid net balance >= threshold.
    let due: Vec<(String, u64)> = {
        let lk = ledger.lock().unwrap();
        let mut pending: HashMap<String, u64> = HashMap::new();
        for entry in &lk.entries {
            if !entry.paid_out {
                *pending.entry(entry.miner_address.clone()).or_default() += entry.net_payout_atoms;
            }
        }
        pending
            .into_iter()
            .filter(|(_, atoms)| *atoms >= cfg.threshold_atoms)
            .collect()
    };

    if due.is_empty() {
        return;
    }

    eprintln!(
        "[payout] {} miner(s) due for payout (threshold={:.2} TXM)",
        due.len(),
        atoms_to_txm(cfg.threshold_atoms)
    );

    let wallet_addr = match wallet_address(cfg) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[payout] could not resolve wallet address: {e}");
            return;
        }
    };

    let mut spendable_atoms = match mature_wallet_atoms(cfg, &wallet_addr) {
        Ok(atoms) => atoms,
        Err(e) => {
            eprintln!("[payout] could not query mature wallet balance: {e}");
            return;
        }
    };

    for (miner_addr, pending_atoms) in due {
        if spendable_atoms <= MIN_RELAY_FEE_ATOMS {
            eprintln!(
                "[payout] wallet mature balance exhausted for this cycle ({:.8} TXM)",
                atoms_to_txm(spendable_atoms)
            );
            break;
        }

        let payable_atoms = {
            let lk = ledger.lock().unwrap();
            lk.payable_atoms_up_to(
                &miner_addr,
                spendable_atoms.saturating_sub(MIN_RELAY_FEE_ATOMS),
            )
        };

        if payable_atoms == 0 {
            eprintln!(
                "[payout] skip {} pending={:.8} TXM, but only {:.8} TXM is mature right now",
                miner_addr,
                atoms_to_txm(pending_atoms),
                atoms_to_txm(spendable_atoms)
            );
            continue;
        }

        if let Some(total_spent_atoms) =
            pay_miner(ledger, cfg, &miner_addr, pending_atoms, payable_atoms)
        {
            spendable_atoms = spendable_atoms.saturating_sub(total_spent_atoms);
        }
    }
}

// ── Single miner payout ───────────────────────────────────────────────────────

fn pay_miner(
    ledger: &Arc<Mutex<PayoutLedger>>,
    cfg: &PayoutConfig,
    miner_addr: &str,
    pending_atoms: u64,
    payable_atoms: u64,
) -> Option<u64> {
    eprintln!(
        "[payout] → {} pending={:.8} TXM payable_now={:.8} TXM",
        miner_addr,
        atoms_to_txm(pending_atoms),
        atoms_to_txm(payable_atoms)
    );

    // Build and submit the transaction via txmwallet.
    // Passphrase is passed through env — never appears in `ps` output.
    let send_result = Command::new("txmwallet")
        .args(["send", miner_addr, &payable_atoms.to_string()])
        .env("TENSORIUM_WALLET", &cfg.wallet_path)
        .env("TENSORIUM_WALLET_PASSPHRASE", &cfg.wallet_passphrase)
        .env("TENSORIUM_RPC", &cfg.node_rpc)
        // Suppress any interactive prompts — wallet must be unlockable from env alone.
        .stdin(std::process::Stdio::null())
        .output();

    match send_result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let txid_line = stdout
                .lines()
                .find(|l| l.contains("txid") || l.len() == 64)
                .unwrap_or(stdout.trim())
                .to_owned();
            let tx_path = stdout
                .lines()
                .find_map(|line| line.strip_prefix("written="))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("tensorium-signed-tx.json"));
            let tx_path_arg = tx_path.to_string_lossy().into_owned();

            let broadcast_result = Command::new("txmwallet")
                .args(["broadcast", tx_path_arg.as_str(), &cfg.node_rpc])
                .env("TENSORIUM_WALLET", &cfg.wallet_path)
                .env("TENSORIUM_WALLET_PASSPHRASE", &cfg.wallet_passphrase)
                .env("TENSORIUM_RPC", &cfg.node_rpc)
                .stdin(std::process::Stdio::null())
                .output();

            match broadcast_result {
                Ok(broadcast) if broadcast.status.success() => {
                    let broadcast_stdout = String::from_utf8_lossy(&broadcast.stdout);
                    let node_line = broadcast_stdout
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or(broadcast_stdout.trim());
                    eprintln!(
                        "[payout] ✓ paid {} ({:.8} TXM)  {}  {}",
                        miner_addr,
                        atoms_to_txm(payable_atoms),
                        txid_line,
                        node_line
                    );

                    let mut lk = ledger.lock().unwrap();
                    let marked = lk.mark_paid_up_to(miner_addr, payable_atoms);
                    if marked != payable_atoms {
                        eprintln!(
                            "[payout] warning: marked {} atoms but expected {} for {}",
                            marked, payable_atoms, miner_addr
                        );
                    }
                    if let Err(e) = lk.save(&cfg.ledger_path) {
                        eprintln!("[payout] ledger save error: {e}");
                    }
                    return Some(payable_atoms.saturating_add(MIN_RELAY_FEE_ATOMS));
                }
                Ok(broadcast) => {
                    let stderr = String::from_utf8_lossy(&broadcast.stderr);
                    let stdout = String::from_utf8_lossy(&broadcast.stdout);
                    let msg = if !stderr.trim().is_empty() {
                        stderr.trim()
                    } else {
                        stdout.trim()
                    };
                    eprintln!(
                        "[payout] ✗ {} broadcast failed (exit {:?}): {}",
                        miner_addr,
                        broadcast.status.code(),
                        msg
                    );
                    return None;
                }
                Err(e) => {
                    eprintln!("[payout] ✗ could not run txmwallet broadcast: {e}");
                    return None;
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if !stderr.trim().is_empty() {
                stderr.trim()
            } else {
                stdout.trim()
            };
            eprintln!(
                "[payout] ✗ {} failed (exit {:?}): {}",
                miner_addr,
                output.status.code(),
                msg
            );
            // Common reasons: immature coinbase UTXOs (retry next cycle),
            // insufficient balance, or wrong passphrase.
            None
        }
        Err(e) => {
            eprintln!("[payout] ✗ could not run txmwallet: {e}");
            None
        }
    }
}

fn wallet_address(cfg: &PayoutConfig) -> Result<String, String> {
    let output = Command::new("txmwallet")
        .arg("getnewaddress")
        .env("TENSORIUM_WALLET", &cfg.wallet_path)
        .env("TENSORIUM_WALLET_PASSPHRASE", &cfg.wallet_passphrase)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("run txmwallet getnewaddress: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        return Err(format!(
            "txmwallet getnewaddress failed (exit {:?}): {}",
            output.status.code(),
            msg
        ));
    }
    let addr = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if addr.is_empty() {
        return Err("txmwallet getnewaddress returned empty address".to_owned());
    }
    Ok(addr)
}

fn rpc_get(rpc: &str, path: &str) -> Result<String, String> {
    let request = format!("GET {path} HTTP/1.1\r\nhost: {rpc}\r\nconnection: close\r\n\r\n");
    let mut stream = TcpStream::connect(rpc).map_err(|err| format!("RPC connect {rpc}: {err}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("RPC write: {e}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("RPC read: {e}"))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_owned())?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("RPC error: {body}"));
    }
    Ok(body.to_owned())
}

fn mature_wallet_atoms(cfg: &PayoutConfig, wallet_addr: &str) -> Result<u64, String> {
    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        value_atoms: u64,
        mature: bool,
    }

    #[derive(serde::Deserialize)]
    struct RpcUtxoResp {
        utxos: Vec<RpcUtxo>,
    }

    let body = rpc_get(&cfg.node_rpc, &format!("/getutxos/{wallet_addr}"))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;
    Ok(resp
        .utxos
        .into_iter()
        .filter(|utxo| utxo.mature)
        .fold(0u64, |sum, utxo| sum.saturating_add(utxo.value_atoms)))
}
