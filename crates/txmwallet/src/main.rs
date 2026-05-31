use std::{
    env, fs,
    path::{Path, PathBuf},
};

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tensorium_core::{chain::TESTNET, ChainState, UtxoSet, WalletKeypair};

const DEFAULT_WALLET_PATH: &str = "tensorium-wallet.json";
const DEFAULT_STATE_PATH: &str = "tensorium-testnet-state.json";
const COINBASE_MATURITY_BLOCKS: u64 = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WalletFile {
    version: u32,
    address: String,
    public_key_hex: String,
    encrypted_private_key: EncryptedPrivateKey,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EncryptedPrivateKey {
    kdf: String,
    cipher: String,
    salt_hex: String,
    nonce_hex: String,
    ciphertext_hex: String,
    checksum_hex: String,
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
            let passphrase = passphrase_from_env()?;
            let keypair = WalletKeypair::generate();
            let wallet = WalletFile::encrypt(keypair, &passphrase);
            save_wallet(&wallet_path, &wallet)?;
            print_wallet_summary(&wallet);
        }
        "getnewaddress" => {
            let wallet = load_wallet(&wallet_path)?;
            println!("{}", wallet.address);
        }
        "show" => {
            let wallet = load_wallet(&wallet_path)?;
            print_wallet_summary(&wallet);
        }
        "balance" => {
            let wallet = load_wallet(&wallet_path)?;
            let state = load_state(&state_path_from_env())?;
            print_balance(&wallet, &state)?;
        }
        "unlock-check" => {
            let wallet = load_wallet(&wallet_path)?;
            let passphrase = passphrase_from_env()?;
            let keypair = wallet.decrypt(&passphrase)?;
            println!("address={}", keypair.address.as_str());
            println!("unlocked=true");
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

fn state_path_from_env() -> PathBuf {
    env::var("TENSORIUM_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_STATE_PATH))
}

fn passphrase_from_env() -> Result<String, String> {
    let passphrase = env::var("TENSORIUM_WALLET_PASSPHRASE")
        .map_err(|_| "set TENSORIUM_WALLET_PASSPHRASE first".to_owned())?;
    if passphrase.len() < 8 {
        return Err("wallet passphrase must be at least 8 characters".to_owned());
    }
    Ok(passphrase)
}

fn load_wallet(path: &Path) -> Result<WalletFile, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn load_state(path: &Path) -> Result<ChainState, String> {
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
    println!("address={}", wallet.address);
    println!("public_key={}", wallet.public_key_hex);
    println!("wallet_version={}", wallet.version);
    println!("encrypted=true");
}

fn print_balance(wallet: &WalletFile, state: &ChainState) -> Result<(), String> {
    let mut utxos = UtxoSet::new();
    for block in &state.blocks {
        utxos
            .apply_block(&TESTNET, block)
            .map_err(|err| err.to_string())?;
    }

    let tip_height = state.height().unwrap_or(0);
    let mut mature_atoms = 0u64;
    let mut immature_atoms = 0u64;
    for entry in utxos.entries.values() {
        if entry.output.address != wallet.address {
            continue;
        }

        let is_immature_coinbase = entry.coinbase
            && tip_height < entry.created_height.saturating_add(COINBASE_MATURITY_BLOCKS);
        if is_immature_coinbase {
            immature_atoms = immature_atoms.saturating_add(entry.output.value_atoms);
        } else {
            mature_atoms = mature_atoms.saturating_add(entry.output.value_atoms);
        }
    }

    println!("address={}", wallet.address);
    println!("mature_atoms={mature_atoms}");
    println!("immature_atoms={immature_atoms}");
    println!("mature_txm={}", format_atoms(mature_atoms));
    println!("immature_txm={}", format_atoms(immature_atoms));
    Ok(())
}

fn format_atoms(atoms: u64) -> String {
    let whole = atoms / 100_000_000;
    let frac = atoms % 100_000_000;
    format!("{whole}.{frac:08}")
}

fn print_help() {
    println!("txmwallet <command>");
    println!();
    println!("commands:");
    println!("  create          create a local wallet file");
    println!("  getnewaddress   print wallet address");
    println!("  balance         scan local chain state for wallet balance");
    println!("  show            print wallet public summary");
    println!("  unlock-check    verify passphrase can decrypt wallet");
    println!();
    println!("env:");
    println!("  TENSORIUM_WALLET    wallet file path, default {DEFAULT_WALLET_PATH}");
    println!("  TENSORIUM_STATE     chain state path, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_WALLET_PASSPHRASE required for create and unlock-check");
}

impl WalletFile {
    fn encrypt(keypair: WalletKeypair, passphrase: &str) -> Self {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);

        let private_key = keypair.private_key_hex.as_bytes();
        let key = derive_key(passphrase, &salt);
        let ciphertext = xor_keystream(private_key, &key, &nonce);
        let checksum = checksum(&key, private_key);

        Self {
            version: 2,
            address: keypair.address.as_str().to_owned(),
            public_key_hex: keypair.public_key_hex,
            encrypted_private_key: EncryptedPrivateKey {
                kdf: "sha256-100k".to_owned(),
                cipher: "sha256-stream-xor-dev".to_owned(),
                salt_hex: hex::encode(salt),
                nonce_hex: hex::encode(nonce),
                ciphertext_hex: hex::encode(ciphertext),
                checksum_hex: hex::encode(checksum),
            },
        }
    }

    fn decrypt(&self, passphrase: &str) -> Result<WalletKeypair, String> {
        let salt = hex::decode(&self.encrypted_private_key.salt_hex)
            .map_err(|err| format!("invalid wallet salt: {err}"))?;
        let nonce = hex::decode(&self.encrypted_private_key.nonce_hex)
            .map_err(|err| format!("invalid wallet nonce: {err}"))?;
        let ciphertext = hex::decode(&self.encrypted_private_key.ciphertext_hex)
            .map_err(|err| format!("invalid wallet ciphertext: {err}"))?;
        let expected_checksum = hex::decode(&self.encrypted_private_key.checksum_hex)
            .map_err(|err| format!("invalid wallet checksum: {err}"))?;
        let key = derive_key(passphrase, &salt);
        let plaintext = xor_keystream(&ciphertext, &key, &nonce);
        if checksum(&key, &plaintext) != expected_checksum.as_slice() {
            return Err("wallet passphrase is incorrect".to_owned());
        }

        let private_key_hex = String::from_utf8(plaintext)
            .map_err(|err| format!("wallet plaintext is invalid UTF-8: {err}"))?;
        let keypair = WalletKeypair::from_private_key_hex(&private_key_hex)
            .map_err(|err| err.to_string())?;
        if keypair.address.as_str() != self.address {
            return Err("wallet address does not match decrypted key".to_owned());
        }
        Ok(keypair)
    }
}

fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(salt);
    digest.update(passphrase.as_bytes());
    let mut key: [u8; 32] = digest.finalize().into();

    for _ in 0..100_000 {
        let mut digest = Sha256::new();
        digest.update(key);
        digest.update(salt);
        digest.update(passphrase.as_bytes());
        key = digest.finalize().into();
    }

    key
}

fn xor_keystream(input: &[u8], key: &[u8; 32], nonce: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut counter = 0u64;
    for chunk in input.chunks(32) {
        let mut digest = Sha256::new();
        digest.update(key);
        digest.update(nonce);
        digest.update(counter.to_le_bytes());
        let stream: [u8; 32] = digest.finalize().into();
        for (index, byte) in chunk.iter().enumerate() {
            output.push(byte ^ stream[index]);
        }
        counter = counter.saturating_add(1);
    }
    output
}

fn checksum(key: &[u8; 32], plaintext: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(key);
    digest.update(plaintext);
    digest.finalize().into()
}
