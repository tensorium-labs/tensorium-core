use crate::{hash::Hash256, script::*};

pub struct ScriptContext {
    pub sig_hash: Hash256,
    pub block_height: u64,
}

/// Execute scriptSig then scriptPubKey against a shared stack.
/// Returns Ok(true) if the final stack top is truthy, Ok(false) otherwise.
pub fn execute(
    script_sig: &[u8],
    script_pubkey: &[u8],
    ctx: &ScriptContext,
) -> Result<bool, ScriptError> {
    if script_sig.len() + script_pubkey.len() > MAX_SCRIPT_SIZE {
        return Err(ScriptError::ScriptTooLarge);
    }
    let mut stack: Vec<Vec<u8>> = Vec::new();
    run(&mut stack, script_sig, ctx, false)?;
    run(&mut stack, script_pubkey, ctx, true)?;
    Ok(stack.last().map(is_truthy).unwrap_or(false))
}

fn is_truthy(item: &Vec<u8>) -> bool {
    !item.is_empty() && item.iter().any(|&b| b != 0)
}

pub(crate) fn run(
    stack: &mut Vec<Vec<u8>>,
    script: &[u8],
    ctx: &ScriptContext,
    allow_checksig: bool,
) -> Result<(), ScriptError> {
    use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use sha2::{Digest, Sha256};

    let mut i = 0;
    let mut if_stack: Vec<bool> = Vec::new();

    macro_rules! executing {
        () => {
            if_stack.is_empty() || *if_stack.last().unwrap()
        };
    }

    while i < script.len() {
        let op = script[i];
        i += 1;

        // ── Data push 0x01..=0x4b ─────────────────────────────────────────
        if op >= 0x01 && op <= 0x4b {
            let n = op as usize;
            if i + n > script.len() {
                return Err(ScriptError::UnexpectedEndOfScript);
            }
            if executing!() {
                let data = script[i..i + n].to_vec();
                if data.len() > MAX_ELEMENT_SIZE {
                    return Err(ScriptError::ElementTooLarge);
                }
                if stack.len() >= MAX_STACK_DEPTH {
                    return Err(ScriptError::StackOverflow);
                }
                stack.push(data);
            }
            i += n;
            continue;
        }

        // ── Control ops handled regardless of executing state ─────────────
        match op {
            OP_IF => {
                let cond = if executing!() {
                    let top = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                    is_truthy(&top)
                } else {
                    false
                };
                if_stack.push(cond);
                continue;
            }
            OP_ELSE => {
                if let Some(last) = if_stack.last_mut() {
                    *last = !*last;
                }
                continue;
            }
            OP_ENDIF => {
                if_stack.pop();
                continue;
            }
            _ => {}
        }

        if !executing!() {
            continue;
        }

        match op {
            OP_RETURN => return Err(ScriptError::VerifyFailed),

            OP_DUP => {
                let top = stack.last().ok_or(ScriptError::StackUnderflow)?.clone();
                if stack.len() >= MAX_STACK_DEPTH {
                    return Err(ScriptError::StackOverflow);
                }
                stack.push(top);
            }
            OP_DROP => {
                stack.pop().ok_or(ScriptError::StackUnderflow)?;
            }
            OP_2DROP => {
                stack.pop().ok_or(ScriptError::StackUnderflow)?;
                stack.pop().ok_or(ScriptError::StackUnderflow)?;
            }
            OP_SWAP => {
                let len = stack.len();
                if len < 2 {
                    return Err(ScriptError::StackUnderflow);
                }
                stack.swap(len - 1, len - 2);
            }

            OP_SHA256 => {
                let top = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let hash = Sha256::digest(&top);
                stack.push(hash.to_vec());
            }
            OP_HASH160 => {
                // Tensorium-specific: SHA256(x)[0..20] — matches Address::from_public_key
                let top = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let hash = Sha256::digest(&top);
                stack.push(hash[..20].to_vec());
            }

            OP_CHECKSIG => {
                if !allow_checksig {
                    return Err(ScriptError::ScriptInSigContainsChecksig);
                }
                let pubkey_bytes = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let sig_bytes = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let vk = VerifyingKey::from_sec1_bytes(&pubkey_bytes)
                    .map_err(|_| ScriptError::InvalidKey)?;
                let sig =
                    Signature::from_der(&sig_bytes).map_err(|_| ScriptError::InvalidSignature)?;
                let ok = vk.verify(&ctx.sig_hash.0, &sig).is_ok();
                stack.push(if ok { vec![0x01] } else { vec![] });
            }

            OP_EQUAL => {
                let b = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let a = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                stack.push(if a == b { vec![0x01] } else { vec![] });
            }
            OP_EQUALVERIFY => {
                let b = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let a = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                if a != b {
                    return Err(ScriptError::VerifyFailed);
                }
            }
            OP_VERIFY => {
                let top = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                if !is_truthy(&top) {
                    return Err(ScriptError::VerifyFailed);
                }
            }

            // ── Small integers OP_1..OP_16 (0x51..0x60) ───────────────────────────
            op @ 0x51..=0x60 => {
                let n = (op - 0x50) as u8; // OP_1(0x51) → 1, OP_16(0x60) → 16
                if stack.len() >= MAX_STACK_DEPTH {
                    return Err(ScriptError::StackOverflow);
                }
                stack.push(vec![n]);
            }

            OP_CHECKMULTISIG | OP_CHECKMULTISIGVERIFY => {
                if !allow_checksig {
                    return Err(ScriptError::ScriptInSigContainsChecksig);
                }

                // Pop n (number of pubkeys)
                let n_item = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                if n_item.is_empty() {
                    return Err(ScriptError::InvalidOpcode(op));
                }
                let n = n_item[0] as usize;
                if n > 16 {
                    return Err(ScriptError::InvalidOpcode(op));
                }

                // Pop n pubkeys (stack order: last pubkey on top → reverse to get script order)
                if stack.len() < n {
                    return Err(ScriptError::StackUnderflow);
                }
                let mut pubkeys: Vec<Vec<u8>> = (0..n).map(|_| stack.pop().unwrap()).collect();
                pubkeys.reverse(); // pubkeys[0] = first pubkey in scriptPubKey

                // Pop m (signature threshold)
                let m_item = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                if m_item.is_empty() {
                    return Err(ScriptError::InvalidOpcode(op));
                }
                let m = m_item[0] as usize;
                if m > n {
                    return Err(ScriptError::InvalidOpcode(op));
                }

                // Pop m signatures (stack order: last sig on top → reverse to get script order)
                if stack.len() < m {
                    return Err(ScriptError::StackUnderflow);
                }
                let mut sigs: Vec<Vec<u8>> = (0..m).map(|_| stack.pop().unwrap()).collect();
                sigs.reverse(); // sigs[0] = first sig in scriptSig

                // Verify: each sig must match a pubkey, advancing forward through pubkeys
                let mut pub_idx = 0;
                let mut all_matched = true;
                'sigs: for sig_bytes in &sigs {
                    let sig = match Signature::from_der(sig_bytes) {
                        Ok(s) => s,
                        Err(_) => {
                            all_matched = false;
                            break;
                        }
                    };
                    let mut found = false;
                    while pub_idx < pubkeys.len() {
                        let pk_bytes = &pubkeys[pub_idx];
                        pub_idx += 1;
                        if let Ok(vk) = VerifyingKey::from_sec1_bytes(pk_bytes) {
                            if vk.verify(&ctx.sig_hash.0, &sig).is_ok() {
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        all_matched = false;
                        break 'sigs;
                    }
                }

                if op == OP_CHECKMULTISIGVERIFY {
                    if !all_matched {
                        return Err(ScriptError::VerifyFailed);
                    }
                    // CHECKMULTISIGVERIFY: leave nothing on stack, continue execution
                } else {
                    if stack.len() >= MAX_STACK_DEPTH {
                        return Err(ScriptError::StackOverflow);
                    }
                    stack.push(if all_matched { vec![0x01u8] } else { vec![] });
                }
            }

            OP_0 => {
                if stack.len() >= MAX_STACK_DEPTH {
                    return Err(ScriptError::StackOverflow);
                }
                stack.push(Vec::new());
            }

            OP_CHECKLOCKTIMEVERIFY => {
                // Peek (do NOT pop) the top item as a little-endian u64 locktime.
                let top = stack.last().ok_or(ScriptError::StackUnderflow)?;
                if top.len() > 8 {
                    return Err(ScriptError::LockTimeNotMet); // malformed → fail closed
                }
                let mut buf = [0u8; 8];
                buf[..top.len()].copy_from_slice(top);
                let locktime = u64::from_le_bytes(buf);
                if ctx.block_height < locktime {
                    return Err(ScriptError::LockTimeNotMet);
                }
                // value stays on the stack; the script removes it with OP_DROP next.
            }

            other => return Err(ScriptError::InvalidOpcode(other)),
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash::Hash256,
        script::{OP_CHECKSIG, OP_DUP, OP_EQUALVERIFY, OP_HASH160, OP_RETURN},
    };
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;
    use sha2::{Digest, Sha256};

    fn fake_ctx() -> ScriptContext {
        ScriptContext {
            sig_hash: Hash256::ZERO,
            block_height: 0,
        }
    }

    fn real_ctx(sig_hash: Hash256) -> ScriptContext {
        ScriptContext {
            sig_hash,
            block_height: 0,
        }
    }

    #[test]
    fn op_return_is_unspendable() {
        let result = execute(&[], &[OP_RETURN], &fake_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn stack_overflow_limit() {
        let mut script = Vec::new();
        for _ in 0..=MAX_STACK_DEPTH {
            script.push(0x01);
            script.push(0xff);
        }
        let result = execute(&[], &script, &fake_ctx());
        assert_eq!(result, Err(ScriptError::StackOverflow));
    }

    #[test]
    fn op_hash160_matches_address_derivation() {
        let data = b"hello world";
        let expected = Sha256::digest(data);
        let expected_20 = &expected[..20];
        let mut script_pubkey = Vec::new();
        script_pubkey.push(data.len() as u8);
        script_pubkey.extend_from_slice(data);
        script_pubkey.push(OP_HASH160);
        let mut stack: Vec<Vec<u8>> = Vec::new();
        super::run(&mut stack, &script_pubkey, &fake_ctx(), true).unwrap();
        assert_eq!(stack.last().unwrap().as_slice(), expected_20);
    }

    #[test]
    fn op_checksig_valid_p2pkh() {
        let signing_key = SigningKey::random(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_encoded_point(true);
        let pubkey_bytes = pubkey.as_bytes();
        let msg = Hash256([42u8; 32]);
        let sig: Signature = signing_key.sign(&msg.0);
        let der_bytes = sig.to_der().as_bytes().to_vec();

        let pubkey_hash = &Sha256::digest(pubkey_bytes)[..20];
        let mut script_pubkey = vec![OP_DUP, OP_HASH160, 0x14];
        script_pubkey.extend_from_slice(pubkey_hash);
        script_pubkey.push(OP_EQUALVERIFY);
        script_pubkey.push(OP_CHECKSIG);

        let mut script_sig = Vec::new();
        script_sig.push(der_bytes.len() as u8);
        script_sig.extend_from_slice(&der_bytes);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(pubkey_bytes);

        let result = execute(&script_sig, &script_pubkey, &real_ctx(msg)).unwrap();
        assert!(result, "valid P2PKH should execute to true");
    }

    #[test]
    fn op_checksig_wrong_sig_fails() {
        let signing_key = SigningKey::random(&mut OsRng);
        let pubkey = signing_key.verifying_key().to_encoded_point(true);
        let pubkey_bytes = pubkey.as_bytes();
        let wrong_msg = Hash256([99u8; 32]);
        let sig: Signature = signing_key.sign(&wrong_msg.0);
        let der_bytes = sig.to_der().as_bytes().to_vec();

        let pubkey_hash = &Sha256::digest(pubkey_bytes)[..20];
        let mut script_pubkey = vec![OP_DUP, OP_HASH160, 0x14];
        script_pubkey.extend_from_slice(pubkey_hash);
        script_pubkey.push(OP_EQUALVERIFY);
        script_pubkey.push(OP_CHECKSIG);

        let mut script_sig = Vec::new();
        script_sig.push(der_bytes.len() as u8);
        script_sig.extend_from_slice(&der_bytes);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(pubkey_bytes);

        let result = execute(&script_sig, &script_pubkey, &fake_ctx());
        assert!(result.is_err() || !result.unwrap(), "wrong sig should fail");
    }

    #[test]
    fn op_small_integers_push_correct_values() {
        use crate::script::{OP_1, OP_16, OP_2};
        let ctx = fake_ctx();

        // OP_1 pushes [0x01]
        let mut stack = Vec::new();
        run(&mut stack, &[OP_1], &ctx, false).unwrap();
        assert_eq!(stack, vec![vec![0x01]]);

        // OP_2 pushes [0x02]
        stack.clear();
        run(&mut stack, &[OP_2], &ctx, false).unwrap();
        assert_eq!(stack, vec![vec![0x02]]);

        // OP_16 pushes [0x10]
        stack.clear();
        run(&mut stack, &[OP_16], &ctx, false).unwrap();
        assert_eq!(stack, vec![vec![0x10]]);
    }

    #[test]
    fn op_checkmultisig_2of3_valid() {
        use crate::script::{OP_2, OP_3, OP_CHECKMULTISIG};
        use k256::ecdsa::{signature::Signer, Signature, SigningKey};
        use rand_core::OsRng;

        let k1 = SigningKey::random(&mut OsRng);
        let k2 = SigningKey::random(&mut OsRng);
        let k3 = SigningKey::random(&mut OsRng);
        let p1 = k1
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p2 = k2
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p3 = k3
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        let msg = Hash256([7u8; 32]);
        let sig1: Signature = k1.sign(&msg.0);
        let sig2: Signature = k2.sign(&msg.0);
        let d1 = sig1.to_der().as_bytes().to_vec();
        let d2 = sig2.to_der().as_bytes().to_vec();

        let mut script_sig = Vec::new();
        script_sig.push(d1.len() as u8);
        script_sig.extend_from_slice(&d1);
        script_sig.push(d2.len() as u8);
        script_sig.extend_from_slice(&d2);

        let mut spk = Vec::new();
        spk.push(OP_2);
        spk.push(p1.len() as u8);
        spk.extend_from_slice(&p1);
        spk.push(p2.len() as u8);
        spk.extend_from_slice(&p2);
        spk.push(p3.len() as u8);
        spk.extend_from_slice(&p3);
        spk.push(OP_3);
        spk.push(OP_CHECKMULTISIG);

        let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
        assert!(result, "2-of-3 with correct sigs should succeed");
    }

    #[test]
    fn op_checkmultisig_wrong_sig_returns_false() {
        use crate::script::{OP_2, OP_3, OP_CHECKMULTISIG};
        use k256::ecdsa::{signature::Signer, Signature, SigningKey};
        use rand_core::OsRng;

        let k1 = SigningKey::random(&mut OsRng);
        let k2 = SigningKey::random(&mut OsRng);
        let k3 = SigningKey::random(&mut OsRng);
        let p1 = k1
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p2 = k2
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p3 = k3
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        let msg = Hash256([7u8; 32]);
        let wrong_msg = Hash256([99u8; 32]);
        let sig1: Signature = k1.sign(&msg.0);
        let sig_wrong: Signature = k2.sign(&wrong_msg.0);
        let d1 = sig1.to_der().as_bytes().to_vec();
        let d_wrong = sig_wrong.to_der().as_bytes().to_vec();

        let mut script_sig = Vec::new();
        script_sig.push(d1.len() as u8);
        script_sig.extend_from_slice(&d1);
        script_sig.push(d_wrong.len() as u8);
        script_sig.extend_from_slice(&d_wrong);

        let mut spk = Vec::new();
        spk.push(OP_2);
        spk.push(p1.len() as u8);
        spk.extend_from_slice(&p1);
        spk.push(p2.len() as u8);
        spk.extend_from_slice(&p2);
        spk.push(p3.len() as u8);
        spk.extend_from_slice(&p3);
        spk.push(OP_3);
        spk.push(OP_CHECKMULTISIG);

        let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
        assert!(!result, "wrong sig should return false, not error");
    }

    #[test]
    fn op_checkmultisig_insufficient_sigs_errors() {
        use crate::script::{OP_1, OP_2, OP_CHECKMULTISIG};
        use k256::ecdsa::{signature::Signer, Signature, SigningKey};
        use rand_core::OsRng;

        let k1 = SigningKey::random(&mut OsRng);
        let k2 = SigningKey::random(&mut OsRng);
        let p1 = k1
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p2 = k2
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        let msg = Hash256([1u8; 32]);
        let sig1: Signature = k1.sign(&msg.0);
        let d1 = sig1.to_der().as_bytes().to_vec();

        // Only 1 sig but m=2
        let mut script_sig = Vec::new();
        script_sig.push(d1.len() as u8);
        script_sig.extend_from_slice(&d1);

        let mut spk = Vec::new();
        spk.push(OP_2); // m=2
        spk.push(p1.len() as u8);
        spk.extend_from_slice(&p1);
        spk.push(p2.len() as u8);
        spk.extend_from_slice(&p2);
        spk.push(OP_2); // n=2
        spk.push(OP_CHECKMULTISIG);

        let result = execute(&script_sig, &spk, &real_ctx(msg));
        assert!(result.is_err(), "insufficient sigs should return error");
    }

    #[test]
    fn op_checkmultisig_m_greater_than_n_errors() {
        use crate::script::{OP_2, OP_3, OP_CHECKMULTISIG};
        use k256::ecdsa::SigningKey;
        use rand_core::OsRng;

        let k1 = SigningKey::random(&mut OsRng);
        let k2 = SigningKey::random(&mut OsRng);
        let p1 = k1
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p2 = k2
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        // scriptPubKey: OP_m <pubkeys...> OP_n OP_CHECKMULTISIG where m=3 but n=2 → invalid
        let mut spk = Vec::new();
        spk.push(OP_3); // m=3
        spk.push(p1.len() as u8);
        spk.extend_from_slice(&p1);
        spk.push(p2.len() as u8);
        spk.extend_from_slice(&p2);
        spk.push(OP_2); // n=2
        spk.push(OP_CHECKMULTISIG);

        let result = execute(&[], &spk, &fake_ctx());
        assert!(result.is_err(), "m > n should return error");
    }

    #[test]
    fn op_0_pushes_empty() {
        use crate::script::OP_0;
        let mut stack: Vec<Vec<u8>> = Vec::new();
        run(&mut stack, &[OP_0], &fake_ctx(), false).unwrap();
        assert_eq!(stack, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn cltv_passes_when_height_ge_locktime() {
        use crate::script::{OP_1, OP_CHECKLOCKTIMEVERIFY, OP_DROP};
        // push 100 (0x64), CLTV, DROP, OP_1(true)
        let script = [0x01, 0x64, OP_CHECKLOCKTIMEVERIFY, OP_DROP, OP_1];
        let ctx = ScriptContext {
            sig_hash: Hash256::ZERO,
            block_height: 100,
        };
        let ok = execute(&[], &script, &ctx).unwrap();
        assert!(ok, "height == locktime should pass");
    }

    #[test]
    fn cltv_fails_below_locktime() {
        use crate::script::OP_CHECKLOCKTIMEVERIFY;
        let script = [0x01, 0x64, OP_CHECKLOCKTIMEVERIFY]; // locktime 100
        let ctx = ScriptContext {
            sig_hash: Hash256::ZERO,
            block_height: 99,
        };
        assert_eq!(
            execute(&[], &script, &ctx),
            Err(ScriptError::LockTimeNotMet)
        );
    }

    #[test]
    fn cltv_leaves_value_on_stack() {
        use crate::script::OP_CHECKLOCKTIMEVERIFY;
        let mut stack: Vec<Vec<u8>> = vec![vec![0x64u8]]; // locktime 100 already on stack
        let ctx = ScriptContext {
            sig_hash: Hash256::ZERO,
            block_height: 200,
        };
        run(&mut stack, &[OP_CHECKLOCKTIMEVERIFY], &ctx, true).unwrap();
        assert_eq!(stack, vec![vec![0x64u8]], "CLTV must not pop its operand");
    }

    #[test]
    fn op_checkmultisig_sigs_out_of_order_fails() {
        use crate::script::{OP_2, OP_3, OP_CHECKMULTISIG};
        use k256::ecdsa::{signature::Signer, Signature, SigningKey};
        use rand_core::OsRng;

        let k1 = SigningKey::random(&mut OsRng);
        let k2 = SigningKey::random(&mut OsRng);
        let k3 = SigningKey::random(&mut OsRng);
        let p1 = k1
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p2 = k2
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let p3 = k3
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();

        let msg = Hash256([5u8; 32]);
        let sig1: Signature = k1.sign(&msg.0);
        let sig2: Signature = k2.sign(&msg.0);
        let d1 = sig1.to_der().as_bytes().to_vec();
        let d2 = sig2.to_der().as_bytes().to_vec();

        // Deliberately swap sig order (sig2 first, sig1 second — wrong order)
        let mut script_sig = Vec::new();
        script_sig.push(d2.len() as u8);
        script_sig.extend_from_slice(&d2);
        script_sig.push(d1.len() as u8);
        script_sig.extend_from_slice(&d1);

        let mut spk = Vec::new();
        spk.push(OP_2);
        spk.push(p1.len() as u8);
        spk.extend_from_slice(&p1);
        spk.push(p2.len() as u8);
        spk.extend_from_slice(&p2);
        spk.push(p3.len() as u8);
        spk.extend_from_slice(&p3);
        spk.push(OP_3);
        spk.push(OP_CHECKMULTISIG);

        // sig2 (for k2) comes first but p1 is first → sig2 can't match p1, advances to p2 and matches.
        // Then sig1 (for k1) comes next but pub_idx is at p3, p1 already consumed → no match → false.
        let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
        assert!(!result, "sigs in wrong order should fail");
    }
}
