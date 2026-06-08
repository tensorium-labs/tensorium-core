use std::{
    env, fs,
    path::{Path, PathBuf},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tensorium_core::{
    block::{Transaction, TxInput, TxOutput},
    chain::MAINNET_CANDIDATE,
    script::standard::{
        cltv_p2pkh_script, extract_multisig, extract_p2sh_hash, htlc_claim_script_sig,
        htlc_refund_script_sig, htlc_script, multisig_script, multisig_script_sig,
        p2pkh_from_address, p2pkh_from_pubkey, p2pkh_script_sig, p2sh_address_from_redeem,
        p2sh_multisig_script_sig, p2sh_script_from_redeem,
    },
    assets::{encode_op, op_return_script, AssetOp},
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
            let to_address = args.get(2).ok_or_else(|| {
                "usage: txmwallet send <to_address> <amount_atoms> [--fee <atoms>|--priority]"
                    .to_owned()
            })?;
            let amount_atoms = args
                .get(3)
                .ok_or_else(|| {
                    "usage: txmwallet send <to_address> <amount_atoms> [--fee <atoms>|--priority]"
                        .to_owned()
                })?
                .parse::<u64>()
                .map_err(|err| format!("invalid amount_atoms: {err}"))?;

            // Fee flags: --priority (100_000 atoms) or --fee <atoms> (custom).
            // Default: MIN_RELAY_FEE_ATOMS (10_000 atoms = 0.0001 TXM).
            let fee_atoms: u64 = if args.iter().any(|a| a == "--priority") {
                tensorium_core::mempool::PRIORITY_FEE_ATOMS
            } else if let Some(pos) = args.iter().position(|a| a == "--fee") {
                args.get(pos + 1)
                    .ok_or("--fee requires a value in atoms")?
                    .parse::<u64>()
                    .map_err(|_| "--fee value must be a positive integer (atoms)")?
            } else {
                tensorium_core::mempool::MIN_RELAY_FEE_ATOMS
            };

            let tx_path = PathBuf::from(DEFAULT_SIGNED_TX_PATH);
            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;
            let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
            let tx = build_signed_payment_via_rpc(
                &wallet,
                &keypair,
                &rpc,
                to_address,
                amount_atoms,
                fee_atoms,
            )
            .or_else(|rpc_err| {
                let state = load_state(&state_path_from_env()).map_err(|state_err| {
                    format!("RPC path failed: {rpc_err}; state load failed: {state_err}")
                })?;
                build_signed_payment(
                    &wallet,
                    &keypair,
                    &state,
                    to_address,
                    amount_atoms,
                    fee_atoms,
                )
                .map_err(|state_err| {
                    format!("RPC path failed: {rpc_err}; local-state fallback failed: {state_err}")
                })
            })?;
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|err| format!("failed to serialize signed tx: {err}"))?;
            fs::write(&tx_path, raw)
                .map_err(|err| format!("failed to write {}: {err}", tx_path.display()))?;
            println!("txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("outputs={}", tx.outputs.len());
            println!("fee_atoms={fee_atoms}");
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
        "p2sh-multisig-script" => {
            let m: u8 = args
                .get(2)
                .ok_or("usage: txmwallet p2sh-multisig-script <m> <pubkey_hex1> ... <pubkey_hexN>")?
                .parse::<u8>()
                .map_err(|_| "invalid m: must be a number 1–16")?;
            let pubkey_args: Vec<Vec<u8>> = args[3..]
                .iter()
                .map(|h| hex::decode(h).map_err(|_| format!("invalid pubkey hex: {h}")))
                .collect::<Result<Vec<_>, _>>()?;
            if pubkey_args.is_empty() {
                return Err("p2sh-multisig-script requires at least one pubkey".to_owned());
            }
            let pubkey_refs: Vec<&[u8]> = pubkey_args.iter().map(|v| v.as_slice()).collect();
            let redeem = multisig_script(m, &pubkey_refs)
                .map_err(|e| format!("invalid multisig params: {e:?}"))?;
            let p2sh_spk = p2sh_script_from_redeem(&redeem);
            let address = p2sh_address_from_redeem(&redeem);
            println!("redeem_script:    {}", hex::encode(&redeem));
            println!("p2sh_scriptpubkey: {}", hex::encode(&p2sh_spk));
            println!("address:          {address}");
            println!("m={m}  n={}", pubkey_refs.len());
            println!("note: save the redeem_script hex — required to spend");
        }
        "p2sh-multisig-spend" => {
            let usage = "usage: txmwallet p2sh-multisig-spend <p2sh_spk_hex> <dest_addr> <redeem_script_hex> <amount_atoms> [rpc]";
            let p2sh_spk_hex = args.get(2).ok_or(usage)?;
            let dest_addr    = args.get(3).ok_or(usage)?;
            let redeem_hex   = args.get(4).ok_or(usage)?;
            let amount_atoms = args.get(5).ok_or(usage)?
                .parse::<u64>().map_err(|_| "invalid amount_atoms: must be a number")?;
            let rpc = args.get(6).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let p2sh_spk = hex::decode(p2sh_spk_hex)
                .map_err(|_| "invalid p2sh_spk_hex: must be lowercase hex")?;
            if extract_p2sh_hash(&p2sh_spk).is_none() {
                return Err("p2sh_spk_hex is not a valid P2SH scriptPubKey (expected OP_HASH160 <20 bytes> OP_EQUAL)".to_owned());
            }
            let redeem = hex::decode(redeem_hex)
                .map_err(|_| "invalid redeem_script_hex: must be lowercase hex")?;
            let expected_spk = p2sh_script_from_redeem(&redeem);
            if expected_spk != p2sh_spk {
                return Err("redeem_script_hex does not hash to the given p2sh_spk_hex".to_owned());
            }

            let tx = build_unsigned_multisig_tx(rpc, p2sh_spk_hex, dest_addr, amount_atoms)?;
            let tx_path = PathBuf::from("p2sh-multisig-spend-tx.json");
            let raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize tx: {e}"))?;
            fs::write(&tx_path, &raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("unsigned_txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("outputs={}", tx.outputs.len());
            println!("written={}", tx_path.display());
            println!("next:");
            println!("  1. TENSORIUM_WALLET_PASSPHRASE=... txmwallet multisig-sign {}", tx_path.display());
            println!("     (run for each required signer, each produces a .sig... file)");
            println!("  2. txmwallet multisig-combine {} <sig1> <sig2> --redeem {}", tx_path.display(), redeem_hex);
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
            let raw =
                serde_json::to_string_pretty(&tx).map_err(|e| format!("serialize tx: {e}"))?;
            fs::write(&tx_path, &raw).map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("unsigned_txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("outputs={}", tx.outputs.len());
            println!("written={}", tx_path.display());
            println!("next: txmwallet multisig-sign {}", tx_path.display());
        }
        "multisig-sign" => {
            let tx_path = PathBuf::from(
                args.get(2)
                    .ok_or("usage: txmwallet multisig-sign <tx_file>")?,
            );
            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            let raw = fs::read_to_string(&tx_path)
                .map_err(|e| format!("read {}: {e}", tx_path.display()))?;
            let tx: Transaction =
                serde_json::from_str(&raw).map_err(|e| format!("parse tx: {e}"))?;

            let sig_hash = tx.signature_hash();
            let der_sig = keypair
                .sign_hash(&sig_hash)
                .map_err(|e| format!("sign: {e:?}"))?;

            let sigs: Vec<MultisigSig> = (0..tx.inputs.len())
                .map(|i| MultisigSig {
                    input_index: i,
                    der_sig_hex: hex::encode(&der_sig),
                })
                .collect();

            let sig_file = MultisigSigFile {
                unsigned_txid: hex::encode(&tx.id.0),
                sigs,
            };

            let addr_prefix = &wallet.address[4..].chars().take(6).collect::<String>();
            let sig_path = tx_path.with_extension(format!("sig{addr_prefix}"));
            let sig_raw = serde_json::to_string_pretty(&sig_file)
                .map_err(|e| format!("serialize sig: {e}"))?;
            fs::write(&sig_path, &sig_raw)
                .map_err(|e| format!("write {}: {e}", sig_path.display()))?;

            println!("signed_by={}", wallet.address);
            println!("unsigned_txid={}", sig_file.unsigned_txid);
            println!("written={}", sig_path.display());
        }
        "multisig-combine" => {
            let tx_path = PathBuf::from(args.get(2).ok_or(
                "usage: txmwallet multisig-combine <tx_file> <sig_file1> <sig_file2> [...]",
            )?);
            let mut redeem_hex: Option<String> = None;
            let mut sig_path_strs: Vec<&str> = Vec::new();
            let mut idx = 3usize;
            while idx < args.len() {
                if args[idx] == "--redeem" {
                    idx += 1;
                    redeem_hex = Some(
                        args.get(idx)
                            .ok_or("--redeem requires a hex value")?
                            .clone(),
                    );
                } else {
                    sig_path_strs.push(&args[idx]);
                }
                idx += 1;
            }
            let sig_paths: Vec<PathBuf> = sig_path_strs.iter().map(PathBuf::from).collect();
            if sig_paths.len() < 2 {
                return Err("multisig-combine requires at least 2 sig files".to_owned());
            }

            let raw = fs::read_to_string(&tx_path)
                .map_err(|e| format!("read {}: {e}", tx_path.display()))?;
            let mut tx: Transaction =
                serde_json::from_str(&raw).map_err(|e| format!("parse tx: {e}"))?;

            let expected_txid = hex::encode(&tx.id.0);

            let mut collected_sigs: Vec<Vec<u8>> = Vec::new();
            for sig_path in &sig_paths {
                let sig_raw = fs::read_to_string(sig_path)
                    .map_err(|e| format!("read {}: {e}", sig_path.display()))?;
                let sig_file: MultisigSigFile = serde_json::from_str(&sig_raw)
                    .map_err(|e| format!("parse {}: {e}", sig_path.display()))?;
                if sig_file.unsigned_txid != expected_txid {
                    return Err(format!(
                        "sig file {} txid mismatch: expected {}, got {}",
                        sig_path.display(),
                        expected_txid,
                        sig_file.unsigned_txid
                    ));
                }
                let sig = sig_file
                    .sigs
                    .iter()
                    .find(|s| s.input_index == 0)
                    .ok_or_else(|| format!("no sig for input 0 in {}", sig_path.display()))?;
                collected_sigs.push(
                    hex::decode(&sig.der_sig_hex)
                        .map_err(|_| format!("invalid sig hex in {}", sig_path.display()))?,
                );
            }

            let sig_refs: Vec<&[u8]> = collected_sigs.iter().map(|v| v.as_slice()).collect();
            let script_sig = if let Some(ref r_hex) = redeem_hex {
                let redeem = hex::decode(r_hex)
                    .map_err(|_| "invalid --redeem hex: must be lowercase hex".to_owned())?;
                p2sh_multisig_script_sig(&sig_refs, &redeem)
            } else {
                multisig_script_sig(&sig_refs)
            };

            for input in &mut tx.inputs {
                input.signature_script = script_sig.clone();
            }
            tx.refresh_id();

            let combined_raw = serde_json::to_string_pretty(&tx)
                .map_err(|e| format!("serialize combined tx: {e}"))?;
            fs::write(&tx_path, &combined_raw)
                .map_err(|e| format!("write {}: {e}", tx_path.display()))?;

            println!("combined_txid={}", tx.id);
            println!("inputs={}", tx.inputs.len());
            println!("sigs_applied={}", collected_sigs.len());
            println!("written={}", tx_path.display());
            println!(
                "ready to broadcast: txmwallet broadcast {}",
                tx_path.display()
            );
        }
        "htlc-secret" => {
            let mut preimage = [0u8; 32];
            OsRng.fill_bytes(&mut preimage);
            let hash = Sha256::digest(preimage);
            println!("preimage: {}", hex::encode(preimage));
            println!("sha256:   {}", hex::encode(hash));
            println!("keep the preimage secret; share only the sha256 hash");
        }
        "htlc-script" => {
            let usage =
                "usage: txmwallet htlc-script <hash_hex> <recipient_addr> <refund_addr> <locktime_height>";
            let hash_hex = args.get(2).ok_or(usage)?;
            let recipient_addr = args.get(3).ok_or(usage)?;
            let refund_addr = args.get(4).ok_or(usage)?;
            let locktime: u64 = args
                .get(5)
                .ok_or(usage)?
                .parse()
                .map_err(|_| "invalid locktime height".to_owned())?;

            let hash_vec = hex::decode(hash_hex).map_err(|_| "invalid hash hex".to_owned())?;
            if hash_vec.len() != 32 {
                return Err("hash must be 32 bytes (SHA256)".to_owned());
            }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_vec);

            let recipient_hash = address_to_hash20(recipient_addr)?;
            let refund_hash = address_to_hash20(refund_addr)?;

            let script = htlc_script(&hash, &recipient_hash, &refund_hash, locktime);
            println!("scriptpubkey: {}", hex::encode(&script));
            println!("locktime_height: {locktime}");
            println!("size={} bytes", script.len());
            println!(
                "fund it by sending TXM to this scriptpubkey (send-from-script or a script output)"
            );
        }
        "htlc-claim" => {
            let usage = "usage: txmwallet htlc-claim <spk_hex> <dest_addr> <preimage_hex> [rpc]";
            let spk_hex = args.get(2).ok_or(usage)?;
            let dest_addr = args.get(3).ok_or(usage)?;
            let preimage_hex = args.get(4).ok_or(usage)?;
            let rpc = args.get(5).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let preimage =
                hex::decode(preimage_hex).map_err(|_| "invalid preimage hex".to_owned())?;
            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            let mut tx = build_unsigned_htlc_spend(rpc, spk_hex, dest_addr)?;
            let sig_hash = tx.signature_hash();
            let der_sig = keypair
                .sign_hash(&sig_hash)
                .map_err(|e| format!("sign: {e:?}"))?;
            let pubkey = hex::decode(&wallet.public_key_hex)
                .map_err(|_| "invalid wallet pubkey hex".to_owned())?;
            let script_sig = htlc_claim_script_sig(&der_sig, &pubkey, &preimage);
            for input in &mut tx.inputs {
                input.signature_script = script_sig.clone();
            }
            tx.refresh_id();

            let tx_path = PathBuf::from("htlc-claim-tx.json");
            let raw = serde_json::to_string_pretty(&tx).map_err(|e| format!("serialize: {e}"))?;
            fs::write(&tx_path, raw).map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("claim_txid={}", tx.id);
            println!("written={}", tx_path.display());
            println!("broadcast: txmwallet broadcast {} {rpc}", tx_path.display());
        }
        "htlc-refund" => {
            let usage = "usage: txmwallet htlc-refund <spk_hex> <dest_addr> [rpc]";
            let spk_hex = args.get(2).ok_or(usage)?;
            let dest_addr = args.get(3).ok_or(usage)?;
            let rpc = args.get(4).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            let mut tx = build_unsigned_htlc_spend(rpc, spk_hex, dest_addr)?;
            let sig_hash = tx.signature_hash();
            let der_sig = keypair
                .sign_hash(&sig_hash)
                .map_err(|e| format!("sign: {e:?}"))?;
            let pubkey = hex::decode(&wallet.public_key_hex)
                .map_err(|_| "invalid wallet pubkey hex".to_owned())?;
            let script_sig = htlc_refund_script_sig(&der_sig, &pubkey);
            for input in &mut tx.inputs {
                input.signature_script = script_sig.clone();
            }
            tx.refresh_id();

            let tx_path = PathBuf::from("htlc-refund-tx.json");
            let raw = serde_json::to_string_pretty(&tx).map_err(|e| format!("serialize: {e}"))?;
            fs::write(&tx_path, raw).map_err(|e| format!("write {}: {e}", tx_path.display()))?;
            println!("refund_txid={}", tx.id);
            println!("written={}", tx_path.display());
            println!("note: the node only accepts this once chain height >= the HTLC locktime");
            println!("broadcast: txmwallet broadcast {} {rpc}", tx_path.display());
        }
        "vesting-lock" => {
            let usage = "usage: TENSORIUM_WALLET_PASSPHRASE=... txmwallet vesting-lock <recipient_addr> <total_atoms> [rpc] [tranches] [interval_blocks] [liquid_bps]";
            let recipient = args.get(2).ok_or(usage)?;
            let total_atoms: u64 = args.get(3).ok_or(usage)?.parse().map_err(|_| "bad total_atoms")?;
            let rpc = args.get(4).map(String::as_str).unwrap_or(DEFAULT_RPC);
            let tranches: u64 = args.get(5).map(|s| s.parse().unwrap_or(6)).unwrap_or(6);
            let interval: u64 = args.get(6).map(|s| s.parse().unwrap_or(43_200)).unwrap_or(43_200);
            let liquid_bps: u64 = args.get(7).map(|s| s.parse().unwrap_or(2_000)).unwrap_or(2_000);

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            // First tranche unlocks one interval after the current tip.
            #[derive(serde::Deserialize)]
            struct Tip { height: u64 }
            let tip: Tip = serde_json::from_str(&rpc_get(rpc, "/getblockcount")?)
                .map_err(|e| format!("getblockcount parse: {e}"))?;
            let start_height = tip.height + interval;

            let (tx, schedule) = build_signed_vesting_via_rpc(
                &wallet, &keypair, rpc, recipient, total_atoms,
                tensorium_core::mempool::MIN_RELAY_FEE_ATOMS,
                start_height, tranches, interval, liquid_bps,
            )?;
            let raw = serde_json::to_string(&tx).map_err(|e| format!("serialize: {e}"))?;
            let resp = rpc_post(rpc, "/sendrawtransaction", &raw)?;
            println!("vesting_lock_txid={}", tx.id);
            println!("recipient={recipient} total_atoms={total_atoms}");
            println!("--- vesting schedule (give these to the buyer; claim each with: txmwallet vesting-claim <spk_hex> <dest> {rpc}) ---");
            for (label, height, atoms, spk_hex) in &schedule {
                println!("{label}: unlock_height={height} atoms={atoms} spk={spk_hex}");
            }
            println!("node response: {resp}");
        }

        "vesting-claim" => {
            let usage = "usage: TENSORIUM_WALLET_PASSPHRASE=... txmwallet vesting-claim <spk_hex> <dest_addr> [rpc]";
            let spk_hex = args.get(2).ok_or(usage)?;
            let dest_addr = args.get(3).ok_or(usage)?;
            let rpc = args.get(4).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            let mut tx = build_unsigned_htlc_spend(rpc, spk_hex, dest_addr)?;
            let sig_hash = tx.signature_hash();
            let der_sig = keypair.sign_hash(&sig_hash).map_err(|e| format!("sign: {e:?}"))?;
            let pubkey = hex::decode(&wallet.public_key_hex)
                .map_err(|_| "invalid wallet pubkey hex".to_owned())?;
            let script_sig = p2pkh_script_sig(&der_sig, &pubkey);
            for input in &mut tx.inputs {
                input.signature_script = script_sig.clone();
            }
            tx.refresh_id();
            let raw = serde_json::to_string(&tx).map_err(|e| format!("serialize: {e}"))?;
            let resp = rpc_post(rpc, "/sendrawtransaction", &raw)?;
            println!("vesting_claim_txid={}", tx.id);
            println!("note: the node accepts this only once chain height >= the tranche's unlock height");
            println!("node response: {resp}");
        }

        _ => print_help(),
    }

    Ok(())
}

/// Build + sign a vesting-lock transaction from the wallet's own (P2PKH) UTXOs.
/// `liquid_bps` of `total_atoms` goes to a plain P2PKH output for `recipient`
/// (immediately spendable); the remainder is split into `tranches` CLTV-locked
/// P2PKH outputs at `start_height`, `start_height + interval`, … each spendable
/// only by `recipient` at/after its unlock height. Returns the signed tx and a
/// human-readable schedule of (label, unlock_height, atoms, scriptpubkey_hex).
#[allow(clippy::too_many_arguments)]
fn build_signed_vesting_via_rpc(
    wallet: &WalletFile,
    keypair: &WalletKeypair,
    rpc: &str,
    recipient: &str,
    total_atoms: u64,
    fee_atoms: u64,
    start_height: u64,
    tranches: u64,
    interval: u64,
    liquid_bps: u64,
) -> Result<(Transaction, Vec<(String, u64, u64, String)>), String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

    if tranches == 0 {
        return Err("tranches must be >= 1".to_owned());
    }
    if liquid_bps > 10_000 {
        return Err("liquid_bps must be <= 10000".to_owned());
    }

    // recipient hash160 = bytes [3..23] of its P2PKH script.
    let rcpt_spk = p2pkh_from_address(recipient)
        .map_err(|_| format!("invalid recipient address: {recipient}"))?;
    let mut rcpt_hash = [0u8; 20];
    rcpt_hash.copy_from_slice(&rcpt_spk[3..23]);

    #[derive(serde::Deserialize)]
    struct RpcUtxo { txid_bytes: Vec<u8>, output_index: u32, value_atoms: u64, mature: bool }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp { utxos: Vec<RpcUtxo> }

    let needed = total_atoms.saturating_add(fee_atoms);
    let body = rpc_get(rpc, &format!("/getutxos/{}", wallet.address))?;
    let resp: RpcUtxoResp = serde_json::from_str(&body).map_err(|e| format!("UTXO parse: {e}"))?;
    let mut inputs = Vec::new();
    let mut selected_atoms = 0u64;
    for u in resp.utxos {
        if !u.mature { continue; }
        let hash = Hash256(u.txid_bytes.as_slice().try_into().map_err(|_| "bad txid len".to_owned())?);
        inputs.push(TxInput { previous_output: OutPoint { txid: hash, output_index: u.output_index }, signature_script: Vec::new() });
        selected_atoms = selected_atoms.saturating_add(u.value_atoms);
        if selected_atoms >= needed { break; }
    }
    if selected_atoms < needed {
        return Err(format!("insufficient mature balance: have {selected_atoms}, need {needed}"));
    }

    let liquid = total_atoms.saturating_mul(liquid_bps) / 10_000;
    let vested_total = total_atoms - liquid;
    let per = vested_total / tranches;

    let mut outputs = Vec::new();
    let mut schedule = Vec::new();
    if liquid > 0 {
        outputs.push(TxOutput { value_atoms: liquid, script_pubkey: rcpt_spk.clone() });
        schedule.push(("liquid".to_owned(), 0u64, liquid, hex::encode(&rcpt_spk)));
    }
    for i in 0..tranches {
        let amount = if i == tranches - 1 { vested_total - per * (tranches - 1) } else { per };
        let unlock = start_height + i * interval;
        let spk = cltv_p2pkh_script(unlock, &rcpt_hash);
        schedule.push((format!("tranche {}", i + 1), unlock, amount, hex::encode(&spk)));
        outputs.push(TxOutput { value_atoms: amount, script_pubkey: spk });
    }

    let change = selected_atoms - total_atoms - fee_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(&wallet.address).map_err(|_| "bad wallet addr".to_owned())?,
        });
    }

    let mut tx = Transaction::payment(inputs, outputs);
    keypair.sign_transaction(&mut tx).map_err(|e| e.to_string())?;
    Ok((tx, schedule))
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
        eprintln!(
            "[storage] Migration complete. Backup at {}",
            backup.display()
        );
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
    let expected_script =
        p2pkh_from_pubkey(&hex::decode(&wallet.public_key_hex).unwrap_or_default());
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
    fee_atoms: u64,
) -> Result<Transaction, String> {
    if amount_atoms == 0 {
        return Err("amount_atoms must be greater than zero".to_owned());
    }
    let needed = amount_atoms.saturating_add(fee_atoms);

    let mut utxos = UtxoSet::new();
    for block in state.canonical_blocks_iter() {
        utxos
            .apply_block(&MAINNET_CANDIDATE, &block)
            .map_err(|err| err.to_string())?;
    }

    let tip_height = state.height().unwrap_or(0);
    let mut selected = Vec::new();
    let mut selected_atoms = 0u64;
    let expected_script =
        p2pkh_from_pubkey(&hex::decode(&wallet.public_key_hex).unwrap_or_default());
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
        if selected_atoms >= needed {
            break;
        }
    }

    if selected_atoms < needed {
        return Err(format!(
            "insufficient mature balance: have {selected_atoms}, need {needed} (amount {amount_atoms} + fee {fee_atoms})"
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
    let change = selected_atoms - amount_atoms - fee_atoms;
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
    println!("  create                                        create a local wallet file");
    println!("  getnewaddress                                 print wallet address");
    println!(
        "  balance                                       scan local chain state for wallet balance"
    );
    println!("  send <to> <atoms> [--fee <atoms>|--priority]  build and sign a transaction file");
    println!("  broadcast [tx_file] [rpc]                     submit signed tx file to node RPC");
    println!("  show                                          print wallet public summary");
    println!(
        "  unlock-check                                  verify passphrase can decrypt wallet"
    );
    println!(
        "  multisig-script <m> <pubkey_hex>...           print scriptPubKey for m-of-n multisig"
    );
    println!("  send-from-script <spk_hex> <to> <atoms>       build unsigned multisig spend tx");
    println!("  multisig-sign <tx_file>                       sign a multisig tx with this wallet");
    println!(
        "  multisig-combine <tx_file> <sig1> <sig2>... [--redeem <hex>]  combine sigs (add --redeem for P2SH)"
    );
    println!("  p2sh-multisig-script <m> <pk1_hex>...        build P2SH-multisig address (txms1...)");
    println!("  p2sh-multisig-spend <spk_hex> <to> <redeem_hex> <atoms> [rpc]  build unsigned P2SH spend tx");
    println!("  htlc-secret                                            generate a 32-byte preimage + its sha256 hash");
    println!("  htlc-script <hash_hex> <recipient_addr> <refund_addr> <locktime_height>");
    println!("  htlc-claim <spk_hex> <dest_addr> <preimage_hex> [rpc]  spend HTLC via preimage (claim branch)");
    println!("  htlc-refund <spk_hex> <dest_addr> [rpc]                spend HTLC after locktime (refund branch)");
    println!("  vesting-lock <recipient_addr> <total_atoms> [rpc] [tranches] [interval] [liquid_bps]");
    println!("                                                         lock tokens for a buyer: liquid %% now + CLTV tranches (default 20%% + 6×monthly)");
    println!("  vesting-claim <spk_hex> <dest_addr> [rpc]              buyer spends a matured CLTV vesting tranche");
    println!();
    println!("env:");
    println!("  TENSORIUM_WALLET             wallet file, default {DEFAULT_WALLET_PATH}");
    println!("  TENSORIUM_STATE              chain state, default {DEFAULT_STATE_PATH}");
    println!(
        "  TENSORIUM_WALLET_PASSPHRASE  required for create, send, unlock-check, multisig-sign"
    );
}

// ---------------------------------------------------------------------------
// Minimal HTTP client for sendrawtransaction
// ---------------------------------------------------------------------------

/// Normalize an RPC endpoint into a full URL.
/// Accepts a full `http://`/`https://` URL (used verbatim, TLS handled for https),
/// or a bare `host:port` (legacy form, assumed plain HTTP). Trailing slashes are
/// trimmed so callers can append a path beginning with `/`.
fn normalize_rpc_url(rpc: &str) -> String {
    let t = rpc.trim().trim_end_matches('/');
    if t.starts_with("http://") || t.starts_with("https://") {
        t.to_owned()
    } else {
        format!("http://{t}")
    }
}

fn rpc_post(rpc: &str, path: &str, body: &str) -> Result<String, String> {
    let url = format!("{}{}", normalize_rpc_url(rpc), path);
    match ureq::post(&url)
        .set("content-type", "application/json")
        .send_string(body)
    {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("failed to read response: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let detail = resp.into_string().unwrap_or_default();
            Err(format!("RPC error {code}: {detail}"))
        }
        Err(e) => Err(format!("failed to connect to {url}: {e}")),
    }
}

fn rpc_get(rpc: &str, path: &str) -> Result<String, String> {
    let url = format!("{}{}", normalize_rpc_url(rpc), path);
    match ureq::get(&url).call() {
        Ok(resp) => resp.into_string().map_err(|e| format!("RPC read: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let detail = resp.into_string().unwrap_or_default();
            Err(format!("RPC error {code}: {detail}"))
        }
        Err(e) => Err(format!("RPC connect {url}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_rpc_url;

    #[test]
    fn normalize_bare_host_port_gets_http_scheme() {
        assert_eq!(normalize_rpc_url("127.0.0.1:33332"), "http://127.0.0.1:33332");
        assert_eq!(
            normalize_rpc_url("mc-rpc.tensoriumlabs.com:80"),
            "http://mc-rpc.tensoriumlabs.com:80"
        );
    }

    #[test]
    fn normalize_keeps_explicit_scheme_and_trims_trailing_slash() {
        assert_eq!(
            normalize_rpc_url("https://mc-rpc.tensoriumlabs.com"),
            "https://mc-rpc.tensoriumlabs.com"
        );
        assert_eq!(
            normalize_rpc_url("https://mc-rpc.tensoriumlabs.com/"),
            "https://mc-rpc.tensoriumlabs.com"
        );
        assert_eq!(normalize_rpc_url("http://host:8080"), "http://host:8080");
    }
}

/// Build the outputs for an asset tx: `[<dest P2PKH (transfer only)>, <TXMA
/// OP_RETURN>, <change to owner>]`. For a transfer, `dest` is `(recipient,
/// carrier_atoms)` and the op's `dest_output_index` must be 0 (this places the
/// recipient at output 0). Pure — no I/O.
fn build_asset_outputs(
    op: &AssetOp,
    dest: Option<(&str, u64)>,
    change_addr: &str,
    total_in: u64,
    fee_atoms: u64,
) -> Result<Vec<TxOutput>, String> {
    let dest_atoms = dest.map(|(_, a)| a).unwrap_or(0);
    let spent = dest_atoms.saturating_add(fee_atoms);
    if total_in < spent {
        return Err(format!(
            "insufficient mature balance: have {total_in}, need {spent} (carrier {dest_atoms} + fee {fee_atoms})"
        ));
    }

    let mut outputs = Vec::new();
    if let Some((addr, atoms)) = dest {
        outputs.push(TxOutput {
            value_atoms: atoms,
            script_pubkey: p2pkh_from_address(addr)
                .map_err(|_| format!("invalid recipient address: {addr}"))?,
        });
    }
    outputs.push(TxOutput {
        value_atoms: 0,
        script_pubkey: op_return_script(&encode_op(op)),
    });
    let change = total_in - dest_atoms - fee_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: p2pkh_from_address(change_addr)
                .map_err(|_| "invalid wallet address".to_owned())?,
        });
    }
    Ok(outputs)
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
    fee_atoms: u64,
) -> Result<Transaction, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;

    let needed = amount_atoms.saturating_add(fee_atoms);

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        coinbase: bool,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp {
        utxos: Vec<RpcUtxo>,
    }

    let body = rpc_get(rpc, &format!("/getutxos/{}", wallet.address))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;

    let mut selected: Vec<(OutPoint, u64)> = Vec::new();
    let mut selected_atoms = 0u64;
    for u in resp.utxos {
        if !u.mature {
            continue;
        }
        let hash = Hash256(
            u.txid_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid txid length from RPC".to_owned())?,
        );
        let outpoint = OutPoint {
            txid: hash,
            output_index: u.output_index,
        };
        selected.push((outpoint, u.value_atoms));
        selected_atoms = selected_atoms.saturating_add(u.value_atoms);
        if selected_atoms >= needed {
            break;
        }
    }

    if selected_atoms < needed {
        return Err(format!(
            "insufficient mature balance via RPC: have {selected_atoms}, need {needed} (amount {amount_atoms} + fee {fee_atoms})"
        ));
    }

    let inputs: Vec<TxInput> = selected
        .iter()
        .map(|(op, _)| TxInput {
            previous_output: *op,
            signature_script: Vec::new(),
        })
        .collect();
    let mut outputs = vec![TxOutput {
        value_atoms: amount_atoms,
        script_pubkey: p2pkh_from_address(to_address)
            .map_err(|_| format!("invalid recipient address: {to_address}"))?,
    }];
    let change = selected_atoms - amount_atoms - fee_atoms;
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
        .map_err(|e| e.to_string())?;
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
    use tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;

    if amount_atoms == 0 {
        return Err("amount_atoms must be greater than zero".to_owned());
    }
    let needed = amount_atoms.saturating_add(MIN_RELAY_FEE_ATOMS);

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp {
        utxos: Vec<RpcUtxo>,
    }

    let body = rpc_get(rpc, &format!("/getutxos/{scriptpubkey_hex}"))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;

    let mut selected: Vec<(OutPoint, u64)> = Vec::new();
    let mut selected_atoms = 0u64;
    for u in resp.utxos {
        if !u.mature {
            continue;
        }
        let hash = Hash256(
            u.txid_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid txid from RPC".to_owned())?,
        );
        selected.push((
            OutPoint {
                txid: hash,
                output_index: u.output_index,
            },
            u.value_atoms,
        ));
        selected_atoms = selected_atoms.saturating_add(u.value_atoms);
        if selected_atoms >= needed {
            break;
        }
    }

    if selected_atoms < needed {
        return Err(format!(
            "insufficient balance: have {selected_atoms}, need {needed} (amount {amount_atoms} + fee {MIN_RELAY_FEE_ATOMS})"
        ));
    }

    let inputs: Vec<TxInput> = selected
        .iter()
        .map(|(op, _)| TxInput {
            previous_output: *op,
            signature_script: Vec::new(),
        })
        .collect();

    let dest_script = p2pkh_from_address(dest_addr)
        .map_err(|_| format!("invalid destination address: {dest_addr}"))?;
    let source_script =
        hex::decode(scriptpubkey_hex).map_err(|_| "invalid scriptpubkey hex".to_owned())?;

    let mut outputs = vec![TxOutput {
        value_atoms: amount_atoms,
        script_pubkey: dest_script,
    }];
    let change = selected_atoms - amount_atoms - MIN_RELAY_FEE_ATOMS;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            script_pubkey: source_script,
        });
    }

    Ok(Transaction::payment(inputs, outputs))
}

/// Build an unsigned transaction spending the first mature UTXO locked to an HTLC
/// scriptPubKey, sending its FULL value to `dest_addr` (no change — HTLC outputs
/// are single-value). UTXOs are discovered via the node's /getutxos/<hex> endpoint.
fn build_unsigned_htlc_spend(
    rpc: &str,
    scriptpubkey_hex: &str,
    dest_addr: &str,
) -> Result<Transaction, String> {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;
    use tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;

    #[derive(serde::Deserialize)]
    struct RpcUtxo {
        txid_bytes: Vec<u8>,
        output_index: u32,
        value_atoms: u64,
        mature: bool,
    }
    #[derive(serde::Deserialize)]
    struct RpcUtxoResp {
        utxos: Vec<RpcUtxo>,
    }

    let body = rpc_get(rpc, &format!("/getutxos/{scriptpubkey_hex}"))?;
    let resp: RpcUtxoResp =
        serde_json::from_str(&body).map_err(|e| format!("UTXO parse error: {e}"))?;

    let u = resp
        .utxos
        .into_iter()
        .find(|u| u.mature)
        .ok_or("no mature UTXO found for this HTLC script")?;

    if u.value_atoms <= MIN_RELAY_FEE_ATOMS {
        return Err(format!(
            "HTLC value {0} atoms is too small to cover minimum relay fee ({MIN_RELAY_FEE_ATOMS} atoms)",
            u.value_atoms
        ));
    }

    let hash = Hash256(
        u.txid_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "invalid txid from RPC".to_owned())?,
    );
    let input = TxInput {
        previous_output: OutPoint {
            txid: hash,
            output_index: u.output_index,
        },
        signature_script: Vec::new(),
    };
    let dest_script = p2pkh_from_address(dest_addr)
        .map_err(|_| format!("invalid destination address: {dest_addr}"))?;
    // HTLC has a single UTXO — fee comes out of the output value (no change output).
    let outputs = vec![TxOutput {
        value_atoms: u.value_atoms - MIN_RELAY_FEE_ATOMS,
        script_pubkey: dest_script,
    }];
    Ok(Transaction::payment(vec![input], outputs))
}

/// Decode a txm1 bech32 address to its 20-byte pubkey hash by reusing the P2PKH builder.
fn address_to_hash20(addr: &str) -> Result<[u8; 20], String> {
    let script = p2pkh_from_address(addr).map_err(|_| format!("invalid address: {addr}"))?;
    // P2PKH layout: OP_DUP OP_HASH160 0x14 <hash20> OP_EQUALVERIFY OP_CHECKSIG
    let mut h = [0u8; 20];
    h.copy_from_slice(&script[3..23]);
    Ok(h)
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
        let keypair =
            WalletKeypair::from_private_key_hex(&private_key_hex).map_err(|err| err.to_string())?;
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

#[cfg(test)]
mod asset_tests {
    use super::*;
    use tensorium_core::assets::{extract_asset_op, IssueData, TransferData};
    use tensorium_core::script::standard::extract_address;
    use tensorium_core::WalletKeypair;

    fn addr() -> String {
        WalletKeypair::generate().address.as_str().to_string()
    }

    #[test]
    fn issue_outputs_carry_op_return_and_change() {
        let owner = addr();
        let op = AssetOp::Issue(IssueData {
            ticker: "GOLD".into(), decimals: 8, supply: 1000, name: "Gold".into(), flags: 0,
        });
        // total_in 50_000, fee 10_000, no dest → [OP_RETURN, change 40_000].
        let outs = build_asset_outputs(&op, None, &owner, 50_000, 10_000).unwrap();
        assert_eq!(outs.len(), 2);
        // The carrier decodes back to the op.
        let tx = Transaction::payment(vec![], outs.clone());
        assert_eq!(extract_asset_op(&tx), Some(op));
        // Change goes back to the owner.
        assert_eq!(outs[1].value_atoms, 40_000);
        assert_eq!(extract_address(&outs[1].script_pubkey).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn transfer_outputs_put_dest_at_index_zero() {
        let owner = addr();
        let bob = addr();
        let op = AssetOp::Transfer(TransferData {
            asset_id: [4u8; 32], amount: 250, dest_output_index: 0,
        });
        // dest carrier 1_000 atoms, fee 10_000, total_in 30_000.
        let outs = build_asset_outputs(&op, Some((&bob, 1_000)), &owner, 30_000, 10_000).unwrap();
        assert_eq!(outs.len(), 3);
        // Output 0 = dest (matches dest_output_index 0).
        assert_eq!(extract_address(&outs[0].script_pubkey).as_deref(), Some(bob.as_str()));
        assert_eq!(outs[0].value_atoms, 1_000);
        // Output 1 = TXMA carrier.
        let tx = Transaction::payment(vec![], outs.clone());
        assert_eq!(extract_asset_op(&tx), Some(op));
        // Output 2 = change = 30_000 - 1_000 - 10_000.
        assert_eq!(outs[2].value_atoms, 19_000);
        assert_eq!(extract_address(&outs[2].script_pubkey).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn rejects_insufficient_input() {
        let owner = addr();
        let op = AssetOp::Issue(IssueData {
            ticker: "X".into(), decimals: 0, supply: 1, name: "X".into(), flags: 0,
        });
        assert!(build_asset_outputs(&op, None, &owner, 5_000, 10_000).is_err());
    }
}
