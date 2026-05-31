use std::{
    env, fs,
    path::{Path, PathBuf},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305,
    XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use tensorium_core::{chain::TESTNET, ChainState, UtxoSet, WalletKeypair};

const DEFAULT_WALLET_PATH: &str = "tensorium-wallet.json";
const DEFAULT_STATE_PATH: &str = "tensorium-testnet-state.json";
const COINBASE_MATURITY_BLOCKS: u64 = 100;
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

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
    kdf_memory_kib: u32,
    kdf_iterations: u32,
    kdf_parallelism: u32,
    cipher: String,
    salt_hex: String,
    nonce_hex: String,
    ciphertext_hex: String,
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
            let wallet = WalletFile::encrypt(keypair, &passphrase)?;
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
    fn encrypt(keypair: WalletKeypair, passphrase: &str) -> Result<Self, String> {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);

        let private_key = keypair.private_key_hex.as_bytes();
        let key = derive_key(
            passphrase,
            &salt,
            ARGON2_MEMORY_KIB,
            ARGON2_ITERATIONS,
            ARGON2_PARALLELISM,
        )?;
        let cipher = XChaCha20Poly1305::new_from_slice(&key)
            .map_err(|err| format!("wallet cipher init failed: {err}"))?;
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), private_key)
            .map_err(|err| format!("wallet encryption failed: {err}"))?;

        Ok(Self {
            version: 2,
            address: keypair.address.as_str().to_owned(),
            public_key_hex: keypair.public_key_hex,
            encrypted_private_key: EncryptedPrivateKey {
                kdf: "argon2id".to_owned(),
                kdf_memory_kib: ARGON2_MEMORY_KIB,
                kdf_iterations: ARGON2_ITERATIONS,
                kdf_parallelism: ARGON2_PARALLELISM,
                cipher: "xchacha20poly1305".to_owned(),
                salt_hex: hex::encode(salt),
                nonce_hex: hex::encode(nonce),
                ciphertext_hex: hex::encode(ciphertext),
            },
        })
    }

    fn decrypt(&self, passphrase: &str) -> Result<WalletKeypair, String> {
        if self.encrypted_private_key.kdf != "argon2id" {
            return Err(format!(
                "unsupported wallet KDF: {}",
                self.encrypted_private_key.kdf
            ));
        }
        if self.encrypted_private_key.cipher != "xchacha20poly1305" {
            return Err(format!(
                "unsupported wallet cipher: {}",
                self.encrypted_private_key.cipher
            ));
        }

        let salt = hex::decode(&self.encrypted_private_key.salt_hex)
            .map_err(|err| format!("invalid wallet salt: {err}"))?;
        let nonce = hex::decode(&self.encrypted_private_key.nonce_hex)
            .map_err(|err| format!("invalid wallet nonce: {err}"))?;
        let ciphertext = hex::decode(&self.encrypted_private_key.ciphertext_hex)
            .map_err(|err| format!("invalid wallet ciphertext: {err}"))?;
        let key = derive_key(
            passphrase,
            &salt,
            self.encrypted_private_key.kdf_memory_kib,
            self.encrypted_private_key.kdf_iterations,
            self.encrypted_private_key.kdf_parallelism,
        )?;
        let cipher = XChaCha20Poly1305::new_from_slice(&key)
            .map_err(|err| format!("wallet cipher init failed: {err}"))?;
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| "wallet passphrase is incorrect or wallet is corrupted".to_owned())?;

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

fn derive_key(
    passphrase: &str,
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<[u8; 32], String> {
    let params = Params::new(memory_kib, iterations, parallelism, Some(32))
        .map_err(|err| format!("invalid Argon2 params: {err}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|err| format!("Argon2 key derivation failed: {err}"))?;
    Ok(key)
}
