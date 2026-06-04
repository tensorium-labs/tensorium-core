use bech32::{self, FromBase32, ToBase32, Variant};
use sha2::{Digest, Sha256};

use crate::script::{
    ScriptError, OP_0, OP_1, OP_CHECKLOCKTIMEVERIFY, OP_CHECKMULTISIG, OP_CHECKSIG, OP_DROP,
    OP_DUP, OP_ELSE, OP_ENDIF, OP_EQUALVERIFY, OP_HASH160, OP_IF, OP_SHA256,
};

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
    s.push(OP_1 - 1 + m); // OP_m: OP_1=0x51, so OP_m = 0x50 + m
    for pk in pubkeys {
        s.push(0x21); // push 33 bytes
        s.extend_from_slice(pk);
    }
    s.push(OP_1 - 1 + n as u8); // OP_n
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

/// Encode a u64 locktime as minimal little-endian bytes (at least 1 byte).
fn encode_locktime(locktime: u64) -> Vec<u8> {
    let bytes = locktime.to_le_bytes();
    let mut len = 8usize;
    while len > 1 && bytes[len - 1] == 0 {
        len -= 1;
    }
    bytes[..len].to_vec()
}

/// Build an HTLC (Hash Time Locked Contract) locking script.
///
/// Claim branch (IF): reveal a preimage whose SHA256 equals `hash`, signed by the
/// recipient key (hash160 == `recipient_hash`).
/// Refund branch (ELSE): only valid once `block_height >= locktime`, signed by the
/// refund key (hash160 == `refund_hash`).
pub fn htlc_script(
    hash: &[u8; 32],
    recipient_hash: &[u8; 20],
    refund_hash: &[u8; 20],
    locktime: u64,
) -> Vec<u8> {
    let lt = encode_locktime(locktime);
    let mut s = Vec::with_capacity(90 + lt.len());
    s.push(OP_IF);
    s.push(OP_SHA256);
    s.push(0x20);
    s.extend_from_slice(hash);
    s.push(OP_EQUALVERIFY);
    s.push(OP_DUP);
    s.push(OP_HASH160);
    s.push(0x14);
    s.extend_from_slice(recipient_hash);
    s.push(OP_EQUALVERIFY);
    s.push(OP_CHECKSIG);
    s.push(OP_ELSE);
    s.push(lt.len() as u8);
    s.extend_from_slice(&lt);
    s.push(OP_CHECKLOCKTIMEVERIFY);
    s.push(OP_DROP);
    s.push(OP_DUP);
    s.push(OP_HASH160);
    s.push(0x14);
    s.extend_from_slice(refund_hash);
    s.push(OP_EQUALVERIFY);
    s.push(OP_CHECKSIG);
    s.push(OP_ENDIF);
    s
}

/// Build an HTLC claim scriptSig: [sig][pubkey][preimage] OP_1.
pub fn htlc_claim_script_sig(der_sig: &[u8], pubkey: &[u8], preimage: &[u8]) -> Vec<u8> {
    assert!(
        preimage.len() <= 75,
        "HTLC preimage must be at most 75 bytes (single data push); got {}",
        preimage.len()
    );
    let mut s = Vec::with_capacity(3 + der_sig.len() + pubkey.len() + preimage.len());
    s.push(der_sig.len() as u8);
    s.extend_from_slice(der_sig);
    s.push(pubkey.len() as u8);
    s.extend_from_slice(pubkey);
    s.push(preimage.len() as u8);
    s.extend_from_slice(preimage);
    s.push(OP_1);
    s
}

/// Build an HTLC refund scriptSig: [sig][pubkey] OP_0.
pub fn htlc_refund_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(2 + der_sig.len() + pubkey.len() + 1);
    s.push(der_sig.len() as u8);
    s.extend_from_slice(der_sig);
    s.push(pubkey.len() as u8);
    s.extend_from_slice(pubkey);
    s.push(OP_0);
    s
}

/// Parse an HTLC scriptPubKey built by `htlc_script`.
/// Returns Some((hash32, recipient_hash20, refund_hash20, locktime)) on match.
pub fn extract_htlc(spk: &[u8]) -> Option<([u8; 32], [u8; 20], [u8; 20], u64)> {
    // Claim-branch prefix is fixed at 61 bytes, then OP_ELSE at index 61.
    if spk.len() < 62 {
        return None;
    }
    if spk[0] != OP_IF || spk[1] != OP_SHA256 || spk[2] != 0x20 {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&spk[3..35]);
    if spk[35] != OP_EQUALVERIFY || spk[36] != OP_DUP || spk[37] != OP_HASH160 || spk[38] != 0x14 {
        return None;
    }
    let mut recipient_hash = [0u8; 20];
    recipient_hash.copy_from_slice(&spk[39..59]);
    if spk[59] != OP_EQUALVERIFY || spk[60] != OP_CHECKSIG || spk[61] != OP_ELSE {
        return None;
    }
    let lt_len = spk[62] as usize;
    if lt_len == 0 || lt_len > 8 {
        return None;
    }
    let lt_start = 63;
    let lt_end = lt_start + lt_len;
    // remaining after locktime: CLTV DROP DUP HASH160 0x14 <20> EQUALVERIFY CHECKSIG ENDIF = 28 bytes
    if spk.len() != lt_end + 28 {
        return None;
    }
    let mut lt_buf = [0u8; 8];
    lt_buf[..lt_len].copy_from_slice(&spk[lt_start..lt_end]);
    let locktime = u64::from_le_bytes(lt_buf);

    let i = lt_end;
    if spk[i] != OP_CHECKLOCKTIMEVERIFY
        || spk[i + 1] != OP_DROP
        || spk[i + 2] != OP_DUP
        || spk[i + 3] != OP_HASH160
        || spk[i + 4] != 0x14
    {
        return None;
    }
    let r_start = i + 5;
    let r_end = r_start + 20;
    let mut refund_hash = [0u8; 20];
    refund_hash.copy_from_slice(&spk[r_start..r_end]);
    if spk[r_end] != OP_EQUALVERIFY || spk[r_end + 1] != OP_CHECKSIG || spk[r_end + 2] != OP_ENDIF {
        return None;
    }
    Some((hash, recipient_hash, refund_hash, locktime))
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

    fn htlc_test_keypair() -> (k256::ecdsa::SigningKey, Vec<u8>, [u8; 20]) {
        use k256::ecdsa::SigningKey;
        use rand_core::OsRng;
        let sk = SigningKey::random(&mut OsRng);
        let pubkey = sk
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let mut hash20 = [0u8; 20];
        hash20.copy_from_slice(&Sha256::digest(&pubkey)[..20]);
        (sk, pubkey, hash20)
    }

    #[test]
    fn htlc_script_roundtrip() {
        let hash = [0x11u8; 32];
        let recipient = [0x22u8; 20];
        let refund = [0x33u8; 20];
        let script = htlc_script(&hash, &recipient, &refund, 500);
        let (h, r, f, lt) = extract_htlc(&script).unwrap();
        assert_eq!(h, hash);
        assert_eq!(r, recipient);
        assert_eq!(f, refund);
        assert_eq!(lt, 500);
    }

    #[test]
    fn htlc_extract_rejects_non_htlc() {
        assert_eq!(extract_htlc(&[0xac]), None);
        assert_eq!(extract_htlc(&[]), None);
    }

    #[test]
    fn htlc_claim_valid() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (recipient_sk, recipient_pk, recipient_hash) = htlc_test_keypair();
        let (_refund_sk, _refund_pk, refund_hash) = htlc_test_keypair();
        let preimage = b"the secret preimage value!!!1234".to_vec();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&Sha256::digest(&preimage));

        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        let msg = Hash256([5u8; 32]);
        let sig: Signature = recipient_sk.sign(&msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let script_sig = htlc_claim_script_sig(&der, &recipient_pk, &preimage);

        let ctx = ScriptContext {
            sig_hash: msg,
            block_height: 0,
        };
        assert!(
            execute(&script_sig, &spk, &ctx).unwrap(),
            "valid claim must succeed"
        );
    }

    #[test]
    fn htlc_claim_wrong_preimage_fails() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (recipient_sk, recipient_pk, recipient_hash) = htlc_test_keypair();
        let (_r_sk, _r_pk, refund_hash) = htlc_test_keypair();
        let preimage = b"the real secret preimage value..".to_vec();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&Sha256::digest(&preimage));

        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        let msg = Hash256([5u8; 32]);
        let sig: Signature = recipient_sk.sign(&msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let wrong = b"a totally different fake preimage".to_vec();
        let script_sig = htlc_claim_script_sig(&der, &recipient_pk, &wrong);

        let ctx = ScriptContext {
            sig_hash: msg,
            block_height: 0,
        };
        let result = execute(&script_sig, &spk, &ctx);
        assert!(
            result.is_err() || !result.unwrap(),
            "wrong preimage must fail"
        );
    }

    #[test]
    fn htlc_claim_wrong_sig_fails() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (recipient_sk, recipient_pk, recipient_hash) = htlc_test_keypair();
        let (_r_sk, _r_pk, refund_hash) = htlc_test_keypair();
        let preimage = b"the real secret preimage value..".to_vec();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&Sha256::digest(&preimage));

        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        // Correct preimage + correct pubkey, but signature over a DIFFERENT message.
        let signed_msg = Hash256([9u8; 32]);
        let verify_msg = Hash256([5u8; 32]);
        let sig: Signature = recipient_sk.sign(&signed_msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let script_sig = htlc_claim_script_sig(&der, &recipient_pk, &preimage);

        let ctx = ScriptContext {
            sig_hash: verify_msg,
            block_height: 0,
        };
        let result = execute(&script_sig, &spk, &ctx);
        assert!(
            result.is_err() || !result.unwrap(),
            "wrong signature must fail"
        );
    }

    #[test]
    fn htlc_refund_valid_after_locktime() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (_recipient_sk, _recipient_pk, recipient_hash) = htlc_test_keypair();
        let (refund_sk, refund_pk, refund_hash) = htlc_test_keypair();
        let hash = [0x44u8; 32];
        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        let msg = Hash256([6u8; 32]);
        let sig: Signature = refund_sk.sign(&msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let script_sig = htlc_refund_script_sig(&der, &refund_pk);

        let ctx = ScriptContext {
            sig_hash: msg,
            block_height: 150,
        };
        assert!(
            execute(&script_sig, &spk, &ctx).unwrap(),
            "refund after locktime must succeed"
        );
    }

    #[test]
    fn htlc_refund_before_locktime_fails() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (_recipient_sk, _recipient_pk, recipient_hash) = htlc_test_keypair();
        let (refund_sk, refund_pk, refund_hash) = htlc_test_keypair();
        let hash = [0x44u8; 32];
        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        let msg = Hash256([6u8; 32]);
        let sig: Signature = refund_sk.sign(&msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let script_sig = htlc_refund_script_sig(&der, &refund_pk);

        let ctx = ScriptContext {
            sig_hash: msg,
            block_height: 99,
        };
        assert_eq!(
            execute(&script_sig, &spk, &ctx),
            Err(ScriptError::LockTimeNotMet)
        );
    }

    #[test]
    fn htlc_refund_wrong_key_fails() {
        use crate::hash::Hash256;
        use crate::script::vm::{execute, ScriptContext};
        use k256::ecdsa::{signature::Signer, Signature};

        let (recipient_sk, recipient_pk, recipient_hash) = htlc_test_keypair();
        let (_refund_sk, _refund_pk, refund_hash) = htlc_test_keypair();
        let hash = [0x44u8; 32];
        let spk = htlc_script(&hash, &recipient_hash, &refund_hash, 100);

        // Past the locktime, but signing the refund branch with the RECIPIENT key,
        // whose hash160 does not match refund_hash → OP_EQUALVERIFY fails.
        let msg = Hash256([6u8; 32]);
        let sig: Signature = recipient_sk.sign(&msg.0);
        let der = sig.to_der().as_bytes().to_vec();
        let script_sig = htlc_refund_script_sig(&der, &recipient_pk);

        let ctx = ScriptContext {
            sig_hash: msg,
            block_height: 150,
        };
        let result = execute(&script_sig, &spk, &ctx);
        assert!(
            result.is_err() || !result.unwrap(),
            "refund with wrong key must fail"
        );
    }
}
