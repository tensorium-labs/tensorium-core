use bech32::{self, FromBase32, ToBase32, Variant};
use sha2::{Digest, Sha256};

use crate::script::{ScriptError, OP_CHECKSIG, OP_CHECKMULTISIG, OP_DUP, OP_EQUALVERIFY, OP_HASH160, OP_1};

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

/// Build a bare m-of-n multisig scriptPubKey.
/// Format: OP_m [0x21 <pubkey33>]×n OP_n OP_CHECKMULTISIG
pub fn multisig_script(m: u8, pubkeys: &[&[u8]]) -> Result<Vec<u8>, ScriptError> {
    let n = pubkeys.len();
    if m == 0 || n == 0 || (m as usize) > n || n > 16 {
        return Err(ScriptError::InvalidKey);
    }
    for pk in pubkeys {
        if pk.len() != 33 {
            return Err(ScriptError::InvalidKey);
        }
    }
    let mut s = Vec::with_capacity(1 + n * 34 + 2);
    s.push(OP_1 - 1 + m);          // OP_m: OP_1=0x51, so OP_m = 0x50 + m
    for pk in pubkeys {
        s.push(0x21);               // push 33 bytes
        s.extend_from_slice(pk);
    }
    s.push(OP_1 - 1 + n as u8);    // OP_n
    s.push(OP_CHECKMULTISIG);
    Ok(s)
}

/// Build a multisig scriptSig from DER-encoded signatures.
/// Format: [sig_len][sig_bytes] repeated for each sig.
pub fn multisig_script_sig(sigs: &[&[u8]]) -> Vec<u8> {
    let mut s = Vec::new();
    for sig in sigs {
        s.push(sig.len() as u8);
        s.extend_from_slice(sig);
    }
    s
}

/// Parse a bare multisig scriptPubKey.
/// Returns Some((m, pubkeys)) if the pattern matches, None otherwise.
pub fn extract_multisig(script_pubkey: &[u8]) -> Option<(u8, Vec<Vec<u8>>)> {
    // Minimum: OP_m + 1*(0x21 + 33 bytes) + OP_n + OP_CHECKMULTISIG = 37 bytes
    if script_pubkey.len() < 37 {
        return None;
    }
    let first = *script_pubkey.first()?;
    let last = *script_pubkey.last()?;
    if last != OP_CHECKMULTISIG {
        return None;
    }
    if first < 0x51 || first > 0x60 {
        return None;
    }
    let m = first - 0x50;

    let mut pubkeys = Vec::new();
    let mut i = 1;
    while i < script_pubkey.len().saturating_sub(2) {
        if script_pubkey[i] != 0x21 {
            return None;
        }
        let end = i + 1 + 33;
        if end > script_pubkey.len() {
            return None;
        }
        pubkeys.push(script_pubkey[i + 1..end].to_vec());
        i = end;
    }

    let n_byte = script_pubkey.get(i)?;
    if *n_byte < 0x51 || *n_byte > 0x60 {
        return None;
    }
    let n = n_byte - 0x50;
    if pubkeys.len() != n as usize || m > n {
        return None;
    }
    Some((m, pubkeys))
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

    #[test]
    fn multisig_script_roundtrip() {
        let pk1 = [0x02u8; 33];
        let pk2 = [0x03u8; 33];
        let pk3 = [0x04u8; 33];
        let script = multisig_script(2, &[&pk1, &pk2, &pk3]).unwrap();
        let (m, pubkeys) = extract_multisig(&script).unwrap();
        assert_eq!(m, 2);
        assert_eq!(pubkeys.len(), 3);
        assert_eq!(pubkeys[0], pk1);
        assert_eq!(pubkeys[1], pk2);
        assert_eq!(pubkeys[2], pk3);
    }

    #[test]
    fn multisig_script_rejects_m_greater_than_n() {
        let pk = [0x02u8; 33];
        let result = multisig_script(3, &[&pk, &pk]);
        assert_eq!(result, Err(ScriptError::InvalidKey));
    }

    #[test]
    fn multisig_script_sig_correct_layout() {
        let sig_a = vec![0xaa_u8; 71];
        let sig_b = vec![0xbb_u8; 70];
        let script_sig = multisig_script_sig(&[&sig_a, &sig_b]);
        assert_eq!(script_sig[0], 71);
        assert_eq!(&script_sig[1..72], &[0xaa_u8; 71]);
        assert_eq!(script_sig[72], 70);
        assert_eq!(&script_sig[73..143], &[0xbb_u8; 70]);
        assert_eq!(script_sig.len(), 1 + 71 + 1 + 70);
    }
}
