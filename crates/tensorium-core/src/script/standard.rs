use bech32::{self, FromBase32, ToBase32, Variant};
use sha2::{Digest, Sha256};

use crate::script::{ScriptError, OP_CHECKSIG, OP_DUP, OP_EQUALVERIFY, OP_HASH160};

const ADDRESS_HRP: &str = "txm";

/// Build a P2PKH locking script from a 20-byte address hash.
/// Script: OP_DUP OP_HASH160 0x14 <hash20> OP_EQUALVERIFY OP_CHECKSIG
pub fn p2pkh_script(hash20: &[u8]) -> Vec<u8> {
    assert_eq!(hash20.len(), 20, "P2PKH hash must be 20 bytes");
    let mut s = Vec::with_capacity(25);
    s.push(OP_DUP);
    s.push(OP_HASH160);
    s.push(0x14); // push 20 bytes
    s.extend_from_slice(hash20);
    s.push(OP_EQUALVERIFY);
    s.push(OP_CHECKSIG);
    s
}

/// Build a P2PKH locking script from a bech32 address ("txm1...").
/// Decodes the address to its 20-byte hash and calls p2pkh_script.
pub fn p2pkh_from_address(addr: &str) -> Result<Vec<u8>, ScriptError> {
    let (hrp, data, _) = bech32::decode(addr).map_err(|_| ScriptError::InvalidAddress)?;
    if hrp != ADDRESS_HRP {
        return Err(ScriptError::InvalidAddress);
    }
    let hash20 = Vec::<u8>::from_base32(&data).map_err(|_| ScriptError::InvalidAddress)?;
    if hash20.len() != 20 {
        return Err(ScriptError::InvalidAddress);
    }
    Ok(p2pkh_script(&hash20))
}

/// Build a P2PKH locking script from a 33-byte compressed public key.
/// Derives the 20-byte hash using SHA256(pubkey)[0..20] then calls p2pkh_script.
pub fn p2pkh_from_pubkey(pubkey_bytes: &[u8]) -> Vec<u8> {
    let hash = Sha256::digest(pubkey_bytes);
    p2pkh_script(&hash[..20])
}

/// Build a P2PKH scriptSig from DER signature bytes and compressed pubkey bytes.
/// Format: [sig_len][...DER sig...][pubkey_len][...pubkey...]
pub fn p2pkh_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(1 + der_sig.len() + 1 + pubkey.len());
    s.push(der_sig.len() as u8);
    s.extend_from_slice(der_sig);
    s.push(pubkey.len() as u8);
    s.extend_from_slice(pubkey);
    s
}

/// Try to extract a bech32 address from a P2PKH scriptPubKey.
/// Returns None if the script does not match the P2PKH pattern.
pub fn extract_address(script_pubkey: &[u8]) -> Option<String> {
    // P2PKH: OP_DUP OP_HASH160 0x14 [20 bytes] OP_EQUALVERIFY OP_CHECKSIG
    if script_pubkey.len() == 25
        && script_pubkey[0] == OP_DUP
        && script_pubkey[1] == OP_HASH160
        && script_pubkey[2] == 0x14
        && script_pubkey[23] == OP_EQUALVERIFY
        && script_pubkey[24] == OP_CHECKSIG
    {
        let hash20 = &script_pubkey[3..23];
        bech32::encode(ADDRESS_HRP, hash20.to_base32(), Variant::Bech32).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p2pkh_roundtrip_address() {
        let hash20 = [0xab_u8; 20];
        let addr = bech32::encode("txm", hash20.to_base32(), Variant::Bech32).unwrap();
        let script = p2pkh_from_address(&addr).unwrap();
        let recovered = extract_address(&script).unwrap();
        assert_eq!(recovered, addr);
    }

    #[test]
    fn p2pkh_from_pubkey_matches_address_derivation() {
        let pubkey = [0x02_u8; 33];
        let script = p2pkh_from_pubkey(&pubkey);
        let expected_hash = &Sha256::digest(&pubkey)[..20];
        assert_eq!(&script[3..23], expected_hash);
    }

    #[test]
    fn rejects_non_txm_address() {
        let hash20 = [0x01_u8; 20];
        let bad_addr = bech32::encode("btc", hash20.to_base32(), Variant::Bech32).unwrap();
        assert_eq!(
            p2pkh_from_address(&bad_addr),
            Err(ScriptError::InvalidAddress)
        );
    }

    #[test]
    fn extract_address_returns_none_for_non_standard() {
        assert_eq!(extract_address(&[0xac]), None);
        assert_eq!(extract_address(&[]), None);
    }
}
