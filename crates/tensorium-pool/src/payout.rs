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
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

// ── Config ────────────────────────────────────────────────────────────────────

pub struct PayoutConfig {
    /// Path to the pool treasury wallet JSON file.
    pub wallet_path:       PathBuf,
    /// Passphrase for the treasury wallet (injected via env, never logged).
    pub wallet_passphrase: String,
    /// Node RPC address used by txmwallet (`host:port`).
    pub node_rpc:          String,
    /// Minimum unpaid net balance in atoms before a payout is triggered.
    pub threshold_atoms:   u64,
    /// How often to run the payout check (seconds).
    pub interval_secs:     u64,
    /// Path to the ledger file (for persisting mark-paid updates).
    pub ledger_path:       PathBuf,
}

const ATOMS_PER_TXM: f64 = 1_000_000_00.0; // 1e8

fn atoms_to_txm(a: u64) -> f64 { a as f64 / ATOMS_PER_TXM }

// ── Scheduler entry point ─────────────────────────────────────────────────────

pub fn run_payout_scheduler(ledger: Arc<Mutex<PayoutLedger>>, cfg: PayoutConfig) {
    eprintln!(
        "[payout] scheduler started  threshold={:.2} TXM  interval={}s  wallet={}",
        atoms_to_txm(cfg.threshold_atoms),
        cfg.interval_secs,
        cfg.wallet_path.display()
    );

    loop {
        thread::sleep(Duration::from_secs(cfg.interval_secs));
        run_cycle(&ledger, &cfg);
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
                *pending.entry(entry.miner_address.clone()).or_default() +=
                    entry.net_payout_atoms;
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

    for (miner_addr, pending_atoms) in due {
        pay_miner(ledger, cfg, &miner_addr, pending_atoms);
    }
}

// ── Single miner payout ───────────────────────────────────────────────────────

fn pay_miner(
    ledger:       &Arc<Mutex<PayoutLedger>>,
    cfg:          &PayoutConfig,
    miner_addr:   &str,
    pending_atoms: u64,
) {
    eprintln!(
        "[payout] → {} ({:.8} TXM / {} atoms)",
        miner_addr,
        atoms_to_txm(pending_atoms),
        pending_atoms
    );

    // Build and submit the transaction via txmwallet.
    // Passphrase is passed through env — never appears in `ps` output.
    let result = Command::new("txmwallet")
        .args(["send", miner_addr, &pending_atoms.to_string()])
        .env("TENSORIUM_WALLET", &cfg.wallet_path)
        .env("TENSORIUM_WALLET_PASSPHRASE", &cfg.wallet_passphrase)
        .env("TENSORIUM_RPC", &cfg.node_rpc)
        // Suppress any interactive prompts — wallet must be unlockable from env alone.
        .stdin(std::process::Stdio::null())
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // txmwallet prints the txid on success.
            let txid_line = stdout
                .lines()
                .find(|l| l.contains("txid") || l.len() == 64)
                .unwrap_or(stdout.trim());
            eprintln!(
                "[payout] ✓ paid {} ({:.8} TXM)  {}",
                miner_addr,
                atoms_to_txm(pending_atoms),
                txid_line
            );

            // Mark all unpaid entries for this miner as paid and persist.
            let mut lk = ledger.lock().unwrap();
            lk.mark_paid(miner_addr);
            if let Err(e) = lk.save(&cfg.ledger_path) {
                eprintln!("[payout] ledger save error: {e}");
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if !stderr.trim().is_empty() { stderr.trim() } else { stdout.trim() };
            eprintln!(
                "[payout] ✗ {} failed (exit {:?}): {}",
                miner_addr,
                output.status.code(),
                msg
            );
            // Common reasons: immature coinbase UTXOs (retry next cycle),
            // insufficient balance, or wrong passphrase.
        }
        Err(e) => {
            eprintln!("[payout] ✗ could not run txmwallet: {e}");
        }
    }
}
