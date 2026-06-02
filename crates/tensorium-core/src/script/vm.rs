use crate::{hash::Hash256, script::*};

pub struct ScriptContext {
    pub sig_hash:     Hash256,
    pub block_height: u64,
}

/// Execute scriptSig then scriptPubKey against a shared stack.
/// Returns Ok(true) if the final stack top is truthy, Ok(false) otherwise.
pub fn execute(
    script_sig:    &[u8],
    script_pubkey: &[u8],
    ctx:           &ScriptContext,
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
    stack:       &mut Vec<Vec<u8>>,
    script:      &[u8],
    ctx:         &ScriptContext,
    allow_checksig: bool,
) -> Result<(), ScriptError> {
    use sha2::{Digest, Sha256};
    use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

    let mut i = 0;
    let mut if_stack: Vec<bool> = Vec::new();

    macro_rules! executing {
        () => { if_stack.is_empty() || *if_stack.last().unwrap() };
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
                if stack.len() >= MAX_STACK_DEPTH { return Err(ScriptError::StackOverflow); }
                stack.push(top);
            }
            OP_DROP => { stack.pop().ok_or(ScriptError::StackUnderflow)?; }
            OP_2DROP => {
                stack.pop().ok_or(ScriptError::StackUnderflow)?;
                stack.pop().ok_or(ScriptError::StackUnderflow)?;
            }
            OP_SWAP => {
                let len = stack.len();
                if len < 2 { return Err(ScriptError::StackUnderflow); }
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
                let sig_bytes    = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                let vk  = VerifyingKey::from_sec1_bytes(&pubkey_bytes)
                    .map_err(|_| ScriptError::InvalidKey)?;
                let sig = Signature::from_der(&sig_bytes)
                    .map_err(|_| ScriptError::InvalidSignature)?;
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
                if a != b { return Err(ScriptError::VerifyFailed); }
            }
            OP_VERIFY => {
                let top = stack.pop().ok_or(ScriptError::StackUnderflow)?;
                if !is_truthy(&top) { return Err(ScriptError::VerifyFailed); }
            }

            // ── Small integers OP_1..OP_16 (0x51..0x60) ───────────────────────────
            op @ 0x51..=0x60 => {
                let n = (op - 0x50) as u8; // OP_1(0x51) → 1, OP_16(0x60) → 16
                if stack.len() >= MAX_STACK_DEPTH {
                    return Err(ScriptError::StackOverflow);
                }
                stack.push(vec![n]);
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
    use crate::{hash::Hash256, script::{OP_DUP, OP_HASH160, OP_EQUALVERIFY, OP_CHECKSIG, OP_RETURN}};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;
    use sha2::{Digest, Sha256};

    fn fake_ctx() -> ScriptContext {
        ScriptContext { sig_hash: Hash256::ZERO, block_height: 0 }
    }

    fn real_ctx(sig_hash: Hash256) -> ScriptContext {
        ScriptContext { sig_hash, block_height: 0 }
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
        use crate::script::{OP_1, OP_2, OP_16};
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
}
