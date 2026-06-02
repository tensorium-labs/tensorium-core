use std::{
    env, fs,
    io::{Read, Write},
    net::TcpStream,
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
use tensorium_core::{
    block::{Transaction, TxInput, TxOutput},
    chain::MAINNET_CANDIDATE,
    script::standard::{multisig_script, multisig_script_sig, extract_multisig,
                       p2pkh_from_address, p2pkh_from_pubkey},
    ChainState, UtxoSet, WalletKeypair,
};

const DEFAULT_WALLET_PATH: &str = "tensorium-wallet.json";
const DEFAULT_STATE_PATH: &str = "tensorium-mainnet-state.json";
const DEFAULT_SIGNED_TX_PATH: &str = "tensorium-signed-tx.json";
const DEFAULT_RPC: &str = "127.0.0.1:33332";
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

#[derive(Debug, Serialize, Deserialize)]
struct MultisigSig {
    input_index: usize,
    der_sig_hex: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MultisigSigFile {
    unsigned_txid: String,
    sigs: Vec<MultisigSig>,
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
        "send" => {
            let to_address = args
                .get(2)
                .ok_or_else(|| "usage: txmwallet send <to_address> <amount_atoms> [tx_file]".to_owned())?;
            let amount_atoms = args
                .get(3)
                .ok_or_else(|| "usage: txmwallet send <to_address> <amount_atoms> [tx_file]".to_owned())?
                .parse::<u64>()
                .map_err(|err| format!("invalid amount_atoms: {err}"))?;
            let tx_path = args
                .get(4)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SIGNED_TX_PATH));
            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            // Prefer RPC-based UTXO lookup (avoids RocksDB LOCK conflict when
            // the node is running alongside txmwallet).
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let tx = build_signed_payment_via_rpc(&wallet, &keypair, &rpc, to_address, amount_atoms)
                .or_else(|_rpc_err| {
                    // Fall back to state.db if RPC is not available.
                    let state = load_state(&state_path_from_env())?;
                    build_signed_payment(&wallet, &keypair, &state, to_address, amount_atoms)
                })?;
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|err| format!("failed to serialize signed tx: {err}"))?;
            fs::write(&tx_path, raw)
                .map_err(|err| format!("failed to write {}: {err}", tx_path.display()))?;
            println!("txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("outputs={}", tx.outputs.len());
            println!("written={}", tx_path.display());
        }
        "unlock-check" => {
            let wallet = load_wallet(&wallet_path)?;
            let passphrase = passphrase_from_env()?;
            let keypair = wallet.decrypt(&passphrase)?;
            println!("address={}", keypair.address.as_str());
            println!("unlocked=true");
        }
        "broadcast" => {
            let tx_path = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SIGNED_TX_PATH));
            let rpc = args.get(3).map(String::as_str).unwrap_or(DEFAULT_RPC);
            let raw = fs::read_to_string(&tx_path)
                .map_err(|err| format!("failed to read {}: {err}", tx_path.display()))?;
            let response = rpc_post(rpc, "/sendrawtransaction", &raw)?;
            println!("{response}");
        }
        "multisig-script" => {
            let m: u8 = args
                .get(2)
                .ok_or("usage: txmwallet multisig-script <m> <pubkey_hex1> ... <pubkey_hexN>")?
                .parse::<u8>()
                .map_err(|_| "invalid m: must be a number 1-16")?;
            let pubkey_args: Vec<Vec<u8>> = args[3..]
                .iter()
                .map(|h| hex::decode(h).map_err(|_| format!("invalid pubkey hex: {h}")))
                .collect::<Result<Vec<_>, _>>()?;
            let pubkey_refs: Vec<&[u8]> = pubkey_args.iter().map(|v| v.as_slice()).collect();
            let script = multisig_script(m, &pubkey_refs)
                .map_err(|e| format!("invalid multisig params: {e:?}"))?;
            println!("scriptpubkey: {}", hex::encode(&script));
            println!("m={m}  n={}", pubkey_refs.len());
            println!("size={} bytes", script.len());
        }
        "send-from-script" => {
            let scriptpubkey_hex = args
                .get(2)
                .ok_or("usage: txmwallet send-from-script <scriptpubkey_hex> <dest_addr> <atoms> [tx_file] [rpc]")?;
            let dest_addr = args
                .get(3)
                .ok_or("usage: txmwallet send-from-script <scriptpubkey_hex> <dest_addr> <atoms> [tx_file] [rpc]")?;
            let amount_atoms = args
                .get(4)
                .ok_or("missing amount_atoms")?
                .parse::<u64>()
                .map_err(|_| "invalid amount_atoms")?;
            let tx_path = args
                .get(5)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("unsigned-tx.json"));
            let rpc = args.get(6).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let tx = build_unsigned_multisig_tx(rpc, scriptpubkey_hex, dest_addr, amount_atoms)?;
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize tx: {e}"))?;
            fs::write(&tx_path, &raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("unsigned_txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("outputs={}", tx.outputs.len());
            println!("written={}", tx_path.display());
            println!("next: txmwallet multisig-sign {}", tx_path.display());
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
    let db_path: PathBuf = if path.extension().map(|e| e == "json").unwrap_or(false) {
        path.with_extension("db")
    } else {
        path.to_path_buf()
    };

    if !db_path.exists() && path.exists() {
        eprintln!(
            "[storage] Migrating {} -> {} (one-time)",
            path.display(),
            db_path.display()
        );
        tensorium_core::storage::migration::migrate_json_to_rocksdb(path, &db_path)?;
        let backup = path.with_extension("json.migrated");
        let _ = fs::rename(path, &backup);
        eprintln!("[storage] Migration complete. Backup at {}", backup.display());
    }

    ChainState::open_db(&db_path)
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
    for block in state.canonical_blocks_iter() {
        utxos
            .apply_block(&MAINNET_CANDIDATE, &block)
            .map_err(|err| err.to_string())?;
    }

    let tip_height = state.height().unwrap_or(0);
    let mut mature_atoms = 0u64;
    let mut immature_atoms = 0u64;
    let expected_script = p2pkh_from_pubkey(
        &hex::decode(&wallet.public_key_hex).unwrap_or_default()
    );
    for entry in utxos.entries.values() {
        if entry.output.script_pubkey != expected_script {
            continue;
        }

        let is_immature_coinbase = entry.coinbase
            && tip_height
                < entry
                    .created_height
                    .saturating_add(MAINNET_CANDIDATE.coinbase_maturity_blocks);
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

fn build_signed_payment(
    wallet: &WalletFile,
    keypair: &WalletKeypair,
    state: &ChainState,
    to_address: &str,
    amount_atoms: u64,
) -> Result<Transaction, String> {
    if amount_atoms == 0 {
        return Err("amount_atoms must be greater than zero".to_owned());
    }

    let mut utxos = UtxoSet::new();
    for block in state.canonical_blocks_iter() {
        utxos
            .apply_block(&MAINNET_CANDIDATE, &block)
            .map_err(|err| err.to_string())?;
    }

    let tip_height = state.height().unwrap_or(0);
    let mut selected = Vec::new();
    let mut selected_atoms = 0u64;
    let expected_script = p2pkh_from_pubkey(
        &hex::decode(&wallet.public_key_hex).unwrap_or_default()
    );
    for (outpoint, entry) in &utxos.entries {
        if entry.output.script_pubkey != expected_script {
            continue;
        }
        let immature = entry.coinbase
            && tip_height
                < entry
                    .created_height
                    .saturating_add(MAINNET_CANDIDATE.coinbase_maturity_blocks);
        if immature {
            continue;
        }

        selected.push((*outpoint, entry.output.clone()));
        selected_atoms = selected_atoms.saturating_add(entry.output.value_atoms);
        if selected_atoms >= amount_atoms {
            break;
        }
    }

    if selected_atoms < amount_atoms {
        return Err(format!(
            "insufficient mature balance: have {selected_atoms}, need {amount_atoms}"
        ));
    }

    let inputs: Vec<TxInput> = selected
        .iter()
        .map(|(outpoint, _)| TxInput {
            previous_output: *outpoint,
            signature_script: Vec::new(),
        })
        .collect();
    let mut outputs = vec![TxOutput {
        value_atoms: amount_atoms,
        script_pubkey: p2pkh_from_address(to_address)
            .map_err(|_| format!("invalid recipient address: {to_address}"))?,
    }];
    let change = selected_atoms - amount_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(&wallet.address)
                .map_err(|_| "invalid wallet address".to_owned())?,
        });
    }

    let mut tx = Transaction::payment(inputs, outputs);
    keypair
        .sign_transaction(&mut tx)
        .map_err(|err| err.to_string())?;
    Ok(tx)
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
    println!("  create                            create a local wallet file");
    println!("  getnewaddress                     print wallet address");
    println!("  balance                           scan local chain state for wallet balance");
    println!("  send <to> <atoms> [tx_file]       build and sign a transaction file");
    println!("  broadcast [tx_file] [rpc]         submit signed tx file to node RPC");
    println!("  show                              print wallet public summary");
    println!("  unlock-check                      verify passphrase can decrypt wallet");
    println!("  multisig-script <m> <pubkey_hex>... print scriptPubKey for m-of-n multisig");
    println!();
    println!("env:");
    println!("  TENSORIUM_WALLET             wallet file, default {DEFAULT_WALLET_PATH}");
    println!("  TENSORIUM_STATE              chain state, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_WALLET_PASSPHRASE  required for create, send, unlock-check");
}

// ---------------------------------------------------------------------------
// Minimal HTTP client for sendrawtransaction
// ---------------------------------------------------------------------------

fn rpc_post(rpc: &str, path: &str, body: &str) -> Result<String, String> {
    let request = format!(
        "POST {path} HTTP/1.1\r\nhost: {rpc}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream =
        TcpStream::connect(rpc).map_err(|err| format!("failed to connect to {rpc}: {err}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to send request: {err}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| format!("failed to read response: {err}"))?;

    let (head, response_body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_owned())?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("RPC error: {response_body}"));
    }
    Ok(response_body.to_owned())
}

fn rpc_get(rpc: &str, path: &str) -> Result<String, String> {
    let request = format!(
        "GET {path} HTTP/1.1\r\nhost: {rpc}\r\nconnection: close\r\n\r\n"
    );
    let mut stream =
        TcpStream::connect(rpc).map_err(|err| format!("RPC connect {rpc}: {err}"))?;
    stream.write_all(request.as_bytes()).map_err(|e| format!("RPC write: {e}"))?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| format!("RPC read: {e}"))?;
    let (head, body) = response.split_once("\r\n\r\n").ok_or("invalid HTTP response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("RPC error: {body}"));
    }
    Ok(body.to_owned())
}

/// Build a signed payment transaction using UTXOs fetched from the node RPC.
/// This avoids opening the RocksDB state file directly, which would conflict
/// with the node's exclusive lock on the database.
fn build_signed_payment_via_rpc(
    wallet: &WalletFile,
    keypair: &WalletKeypair,
    rpc: &str,
    to_address: &str,
    amount_atoms: u64,
) -> Result<Transaction, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        coinbase: bool,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp { utxos: Vec<RpcUtxo> }

    let body = rpc_get(rpc, &format!("/getutxos/{}", wallet.address))?;
    let resp: RpcUtxoResp = serde_json::from_str(&body)
        .map_err(|e| format!("UTXO parse error: {e}"))?;

    let mut selected: Vec<(OutPoint, u64)> = Vec::new();
    let mut selected_atoms = 0u64;
    for u in resp.utxos {
        if !u.mature { continue; }
        let hash = Hash256(
            u.txid_bytes.as_slice().try_into()
                .map_err(|_| "invalid txid length from RPC".to_owned())?
        );
        let outpoint = OutPoint { txid: hash, output_index: u.output_index };
        selected.push((outpoint, u.value_atoms));
        selected_atoms = selected_atoms.saturating_add(u.value_atoms);
        if selected_atoms >= amount_atoms { break; }
    }

    if selected_atoms < amount_atoms {
        return Err(format!(
            "insufficient mature balance via RPC: have {selected_atoms}, need {amount_atoms}"
        ));
    }

    let inputs: Vec<TxInput> = selected.iter()
        .map(|(op, _)| TxInput { previous_output: *op, signature_script: Vec::new() })
        .collect();
    let mut outputs = vec![TxOutput {
        value_atoms: amount_atoms,
        script_pubkey: p2pkh_from_address(to_address)
            .map_err(|_| format!("invalid recipient address: {to_address}"))?,
    }];
    let change = selected_atoms - amount_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(&wallet.address)
                .map_err(|_| "invalid wallet address".to_owned())?,
        });
    }

    let mut tx = Transaction::payment(inputs, outputs);
    keypair.sign_transaction(&mut tx).map_err(|e| e.to_string())?;
    Ok(tx)
}

fn build_unsigned_multisig_tx(
    rpc: &str,
    scriptpubkey_hex: &str,
    dest_addr: &str,
    amount_atoms: u64,
) -> Result<Transaction, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

    if amount_atoms == 0 {
        return Err("amount_atoms must be greater than zero".to_owned());
    }

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp { utxos: Vec<RpcUtxo> }

    let body = rpc_get(rpc, &format!("/getutxos/{scriptpubkey_hex}"))?;
    let resp: RpcUtxoResp = serde_json::from_str(&body)
        .map_err(|e| format!("UTXO parse error: {e}"))?;

    let mut selected: Vec<(OutPoint, u64)> = Vec::new();
    let mut selected_atoms = 0u64;
    for u in resp.utxos {
        if !u.mature { continue; }
        let hash = Hash256(
            u.txid_bytes.as_slice().try_into()
                .map_err(|_| "invalid txid from RPC".to_owned())?
        );
        selected.push((OutPoint { txid: hash, output_index: u.output_index }, u.value_atoms));
        selected_atoms = selected_atoms.saturating_add(u.value_atoms);
        if selected_atoms >= amount_atoms { break; }
    }

    if selected_atoms < amount_atoms {
        return Err(format!(
            "insufficient balance: have {selected_atoms}, need {amount_atoms}"
        ));
    }

    let inputs: Vec<TxInput> = selected.iter()
        .map(|(op, _)| TxInput { previous_output: *op, signature_script: Vec::new() })
        .collect();

    let dest_script = p2pkh_from_address(dest_addr)
        .map_err(|_| format!("invalid destination address: {dest_addr}"))?;
    let source_script = hex::decode(scriptpubkey_hex)
        .map_err(|_| "invalid scriptpubkey hex".to_owned())?;

    let mut outputs = vec![TxOutput { value_atoms: amount_atoms, script_pubkey: dest_script }];
    let change = selected_atoms - amount_atoms;
    if change > 0 {
        outputs.push(TxOutput { value_atoms: change, script_pubkey: source_script });
    }

    Ok(Transaction::payment(inputs, outputs))
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
