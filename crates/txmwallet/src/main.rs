use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tensorium_core::WalletKeypair;

const DEFAULT_WALLET_PATH: &str = "tensorium-wallet.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WalletFile {
    version: u32,
    keypair: WalletKeypair,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    let wallet_path = wallet_path_from_env();

    match command {
        "create" => {
            if wallet_path.exists() {
                return Err(format!("wallet already exists: {}", wallet_path.display()));
            }
            let wallet = WalletFile {
                version: 1,
                keypair: WalletKeypair::generate(),
            };
            save_wallet(&wallet_path, &wallet)?;
            print_wallet_summary(&wallet);
        }
        "getnewaddress" => {
            let wallet = load_wallet(&wallet_path)?;
            println!("{}", wallet.keypair.address.as_str());
        }
        "show" => {
            let wallet = load_wallet(&wallet_path)?;
            print_wallet_summary(&wallet);
        }
        _ => print_help(),
    }

    Ok(())
}

fn wallet_path_from_env() -> PathBuf {
    env::var("TENSORIUM_WALLET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WALLET_PATH))
}

fn load_wallet(path: &Path) -> Result<WalletFile, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn save_wallet(path: &Path, wallet: &WalletFile) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(wallet)
        .map_err(|err| format!("failed to serialize wallet: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn print_wallet_summary(wallet: &WalletFile) {
    println!("address={}", wallet.keypair.address.as_str());
    println!("public_key={}", wallet.keypair.public_key_hex);
    println!("wallet_version={}", wallet.version);
}

fn print_help() {
    println!("txmwallet <command>");
    println!();
    println!("commands:");
    println!("  create          create a local wallet file");
    println!("  getnewaddress   print wallet address");
    println!("  show            print wallet public summary");
    println!();
    println!("env:");
    println!("  TENSORIUM_WALLET    wallet file path, default {DEFAULT_WALLET_PATH}");
    println!();
    println!("warning:");
    println!("  wallet file is not encrypted yet; keep it private during this phase");
}
