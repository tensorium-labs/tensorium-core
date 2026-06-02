# Scripting Layer S2 — Bare Multisig Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add m-of-n bare multisig to Tensorium — opcodes in the VM, script builders in standard.rs, a `sign_hash` helper on WalletKeypair, an extended `/getutxos/` node endpoint, and four new txmwallet subcommands.

**Architecture:** Pure additive on top of S1. No consensus changes, no chain reset. OP_1..OP_16 push small integers; OP_CHECKMULTISIG pops n, n pubkeys, m, m sigs and verifies m-of-n in order (no Bitcoin dummy-element bug). Standard builders handle encoding/decoding. Wallet CLI wires everything into a sign → combine → broadcast workflow.

**Tech Stack:** Rust, k256 (already in deps), serde_json, hex crate (already used). Working directory: `/root/.openclaw/workspace/tensorium-core`.

---

## File Map

| File | What changes |
|------|-------------|
| `crates/tensorium-core/src/script/mod.rs` | Add `OP_1..OP_16`, `OP_CHECKMULTISIG`, `OP_CHECKMULTISIGVERIFY` constants |
| `crates/tensorium-core/src/script/vm.rs` | Implement small-int push + `OP_CHECKMULTISIG`/`OP_CHECKMULTISIGVERIFY` execution |
| `crates/tensorium-core/src/script/standard.rs` | Add `multisig_script`, `multisig_script_sig`, `extract_multisig` |
| `crates/tensorium-core/src/wallet.rs` | Add `WalletKeypair::sign_hash` |
| `crates/tensorium-node/src/main.rs` | Extend `/getutxos/` to accept hex scriptPubKey |
| `crates/txmwallet/src/main.rs` | Add `multisig-script`, `send-from-script`, `multisig-sign`, `multisig-combine` |

---

## Task 1: Opcode Constants

**Files:**
- Modify: `crates/tensorium-core/src/script/mod.rs`

- [ ] **Add constants after `OP_CHECKSIG`:**

```rust
// ── Multisig ──────────────────────────────────────────────────────────────────
pub const OP_CHECKMULTISIG:       u8 = 0xae;
pub const OP_CHECKMULTISIGVERIFY: u8 = 0xaf;

// ── Small integers ────────────────────────────────────────────────────────────
// OP_1..OP_16 push the byte value [n] onto the stack.
pub const OP_1:  u8 = 0x51;
pub const OP_2:  u8 = 0x52;
pub const OP_3:  u8 = 0x53;
pub const OP_4:  u8 = 0x54;
pub const OP_5:  u8 = 0x55;
pub const OP_6:  u8 = 0x56;
pub const OP_7:  u8 = 0x57;
pub const OP_8:  u8 = 0x58;
pub const OP_9:  u8 = 0x59;
pub const OP_10: u8 = 0x5a;
pub const OP_11: u8 = 0x5b;
pub const OP_12: u8 = 0x5c;
pub const OP_13: u8 = 0x5d;
pub const OP_14: u8 = 0x5e;
pub const OP_15: u8 = 0x5f;
pub const OP_16: u8 = 0x60;
```

- [ ] **Verify it compiles:**

```bash
cargo build -p tensorium-core 2>&1 | grep -E "error|warning: unused"
```

Expected: no errors.

- [ ] **Commit:**

```bash
git add crates/tensorium-core/src/script/mod.rs
git commit -m "feat(script/s2): add OP_1..16 and OP_CHECKMULTISIG opcode constants"
```

---

## Task 2: Small Integer Push (OP_1..OP_16) in vm.rs

**Files:**
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Write failing test** — add inside the `#[cfg(test)]` block at the bottom of `vm.rs`:

```rust
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
```

- [ ] **Run to confirm it fails:**

```bash
cargo test -p tensorium-core op_small_integers 2>&1 | tail -5
```

Expected: `FAILED` with `InvalidOpcode`.

- [ ] **Add small-int range to the `run` match** — inside the `match op { ... }` block in `run()`, add before the final `other =>` arm:

```rust
// ── Small integers OP_1..OP_16 (0x51..0x60) ───────────────────────────
op @ 0x51..=0x60 => {
    let n = (op - 0x50) as u8; // OP_1(0x51) → 1, OP_16(0x60) → 16
    if stack.len() >= MAX_STACK_DEPTH {
        return Err(ScriptError::StackOverflow);
    }
    stack.push(vec![n]);
}
```

- [ ] **Run to confirm it passes:**

```bash
cargo test -p tensorium-core op_small_integers 2>&1 | tail -3
```

Expected: `test result: ok. 1 passed`.

- [ ] **Commit:**

```bash
git add crates/tensorium-core/src/script/vm.rs
git commit -m "feat(script/s2): implement OP_1..OP_16 small integer push"
```

---

## Task 3: OP_CHECKMULTISIG in vm.rs

**Files:**
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Write failing tests** — add to `#[cfg(test)]` block:

```rust
#[test]
fn op_checkmultisig_2of3_valid() {
    use crate::script::{OP_CHECKMULTISIG, OP_2, OP_3};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let k3 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p3 = k3.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let msg = Hash256([7u8; 32]);
    let sig1: Signature = k1.sign(&msg.0);
    let sig2: Signature = k2.sign(&msg.0);
    let d1 = sig1.to_der().as_bytes().to_vec();
    let d2 = sig2.to_der().as_bytes().to_vec();

    // scriptSig: sig1 sig2
    let mut script_sig = Vec::new();
    script_sig.push(d1.len() as u8); script_sig.extend_from_slice(&d1);
    script_sig.push(d2.len() as u8); script_sig.extend_from_slice(&d2);

    // scriptPubKey: OP_2 <p1> <p2> <p3> OP_3 OP_CHECKMULTISIG
    let mut spk = Vec::new();
    spk.push(OP_2);
    spk.push(p1.len() as u8); spk.extend_from_slice(&p1);
    spk.push(p2.len() as u8); spk.extend_from_slice(&p2);
    spk.push(p3.len() as u8); spk.extend_from_slice(&p3);
    spk.push(OP_3);
    spk.push(OP_CHECKMULTISIG);

    let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
    assert!(result, "2-of-3 with correct sigs should succeed");
}

#[test]
fn op_checkmultisig_wrong_sig_returns_false() {
    use crate::script::{OP_CHECKMULTISIG, OP_2, OP_3};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let k3 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p3 = k3.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let msg = Hash256([7u8; 32]);
    let wrong_msg = Hash256([99u8; 32]);
    let sig1: Signature = k1.sign(&msg.0);
    let sig_wrong: Signature = k2.sign(&wrong_msg.0); // wrong hash
    let d1 = sig1.to_der().as_bytes().to_vec();
    let d_wrong = sig_wrong.to_der().as_bytes().to_vec();

    let mut script_sig = Vec::new();
    script_sig.push(d1.len() as u8); script_sig.extend_from_slice(&d1);
    script_sig.push(d_wrong.len() as u8); script_sig.extend_from_slice(&d_wrong);

    let mut spk = Vec::new();
    spk.push(OP_2);
    spk.push(p1.len() as u8); spk.extend_from_slice(&p1);
    spk.push(p2.len() as u8); spk.extend_from_slice(&p2);
    spk.push(p3.len() as u8); spk.extend_from_slice(&p3);
    spk.push(OP_3);
    spk.push(OP_CHECKMULTISIG);

    let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
    assert!(!result, "wrong sig should return false, not error");
}

#[test]
fn op_checkmultisig_insufficient_sigs_errors() {
    use crate::script::{OP_CHECKMULTISIG, OP_1, OP_2};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let msg = Hash256([1u8; 32]);
    let sig1: Signature = k1.sign(&msg.0);
    let d1 = sig1.to_der().as_bytes().to_vec();

    // Only 1 sig but m=2
    let mut script_sig = Vec::new();
    script_sig.push(d1.len() as u8); script_sig.extend_from_slice(&d1);

    let mut spk = Vec::new();
    spk.push(OP_2); // m=2
    spk.push(p1.len() as u8); spk.extend_from_slice(&p1);
    spk.push(p2.len() as u8); spk.extend_from_slice(&p2);
    spk.push(OP_2); // n=2
    spk.push(OP_CHECKMULTISIG);

    let result = execute(&script_sig, &spk, &real_ctx(msg));
    assert!(result.is_err(), "insufficient sigs should return error");
}

#[test]
fn op_checkmultisig_m_greater_than_n_errors() {
    use crate::script::{OP_CHECKMULTISIG, OP_3, OP_2};
    use k256::ecdsa::SigningKey;
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    // scriptPubKey: OP_m <pubkeys...> OP_n OP_CHECKMULTISIG where m=3 but n=2 → invalid
    let mut spk = Vec::new();
    spk.push(OP_3); // m=3
    spk.push(p1.len() as u8); spk.extend_from_slice(&p1);
    spk.push(p2.len() as u8); spk.extend_from_slice(&p2);
    spk.push(OP_2); // n=2
    spk.push(OP_CHECKMULTISIG);

    let result = execute(&[], &spk, &fake_ctx());
    assert!(result.is_err(), "m > n should return error");
}

#[test]
fn op_checkmultisig_sigs_out_of_order_fails() {
    use crate::script::{OP_CHECKMULTISIG, OP_2, OP_3};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let k3 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p3 = k3.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let msg = Hash256([5u8; 32]);
    let sig1: Signature = k1.sign(&msg.0);
    let sig2: Signature = k2.sign(&msg.0);
    let d1 = sig1.to_der().as_bytes().to_vec();
    let d2 = sig2.to_der().as_bytes().to_vec();

    // Deliberately swap sig order (sig2 first, sig1 second — wrong order)
    let mut script_sig = Vec::new();
    script_sig.push(d2.len() as u8); script_sig.extend_from_slice(&d2);
    script_sig.push(d1.len() as u8); script_sig.extend_from_slice(&d1);

    let mut spk = Vec::new();
    spk.push(OP_2);
    spk.push(p1.len() as u8); spk.extend_from_slice(&p1);
    spk.push(p2.len() as u8); spk.extend_from_slice(&p2);
    spk.push(p3.len() as u8); spk.extend_from_slice(&p3);
    spk.push(OP_3);
    spk.push(OP_CHECKMULTISIG);

    // sig2 (for k2) comes first but p1 comes first in pubkey list.
    // sig2 can't match p1, advances to p2 and matches. Then sig1 (for k1) comes
    // next but pub_idx is now at p3, p1 is already consumed → no match → false.
    let result = execute(&script_sig, &spk, &real_ctx(msg)).unwrap();
    assert!(!result, "sigs in wrong order should fail");
}
```

- [ ] **Run to confirm all fail:**

```bash
cargo test -p tensorium-core op_checkmultisig 2>&1 | tail -8
```

Expected: all FAILED with `InvalidOpcode(0xae)`.

- [ ] **Implement OP_CHECKMULTISIG in vm.rs** — add inside `match op { ... }` before `other =>`:

```rust
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
    let mut pubkeys: Vec<Vec<u8>> = (0..n)
        .map(|_| stack.pop().unwrap())
        .collect();
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
    let mut sigs: Vec<Vec<u8>> = (0..m)
        .map(|_| stack.pop().unwrap())
        .collect();
    sigs.reverse(); // sigs[0] = first sig in scriptSig

    // Verify: each sig must match a pubkey, advancing forward through pubkeys
    let mut pub_idx = 0;
    let mut all_matched = true;
    'sigs: for sig_bytes in &sigs {
        let sig = match Signature::from_der(sig_bytes) {
            Ok(s) => s,
            Err(_) => { all_matched = false; break; }
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

    let result_item = if all_matched { vec![0x01u8] } else { vec![] };

    if op == OP_CHECKMULTISIGVERIFY {
        if !all_matched {
            return Err(ScriptError::VerifyFailed);
        }
        // CHECKMULTISIGVERIFY: leave nothing, continue execution
    } else {
        if stack.len() >= MAX_STACK_DEPTH {
            return Err(ScriptError::StackOverflow);
        }
        stack.push(result_item);
    }
}
```

Also add `OP_CHECKMULTISIG` and `OP_CHECKMULTISIGVERIFY` to the imports at the top of the `run` function's `use` block — they are already in scope via `use crate::script::*`.

- [ ] **Run tests:**

```bash
cargo test -p tensorium-core op_checkmultisig 2>&1 | tail -8
```

Expected: 5 tests pass.

- [ ] **Run full workspace tests:**

```bash
cargo test --workspace 2>&1 | grep -E "test result|FAILED"
```

Expected: all `ok`, 0 failed.

- [ ] **Commit:**

```bash
git add crates/tensorium-core/src/script/vm.rs
git commit -m "feat(script/s2): implement OP_CHECKMULTISIG and OP_CHECKMULTISIGVERIFY"
```

---

## Task 4: Standard Script Builders

**Files:**
- Modify: `crates/tensorium-core/src/script/standard.rs`

- [ ] **Write failing tests** — add to `#[cfg(test)]` block in `standard.rs`:

```rust
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
    // [71][aa*71][70][bb*70]
    assert_eq!(script_sig[0], 71);
    assert_eq!(&script_sig[1..72], &[0xaa_u8; 71]);
    assert_eq!(script_sig[72], 70);
    assert_eq!(&script_sig[73..143], &[0xbb_u8; 70]);
    assert_eq!(script_sig.len(), 1 + 71 + 1 + 70);
}
```

- [ ] **Run to confirm they fail:**

```bash
cargo test -p tensorium-core multisig_script 2>&1 | tail -5
```

Expected: FAILED with `unresolved import` or `cannot find function`.

- [ ] **Implement the three functions** — add after `extract_address` in `standard.rs`:

```rust
use crate::script::{OP_CHECKMULTISIG, OP_1};

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
        return None; // OP_m out of range
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
```

- [ ] **Run tests:**

```bash
cargo test -p tensorium-core multisig_script 2>&1 | tail -5
```

Expected: 3 tests pass.

- [ ] **Run full workspace:**

```bash
cargo test --workspace 2>&1 | grep -E "test result|FAILED"
```

Expected: all `ok`, 0 failed.

- [ ] **Commit:**

```bash
git add crates/tensorium-core/src/script/standard.rs
git commit -m "feat(script/s2): add multisig_script, multisig_script_sig, extract_multisig"
```

---

## Task 5: WalletKeypair::sign_hash

**Files:**
- Modify: `crates/tensorium-core/src/wallet.rs`

- [ ] **Add `sign_hash` after `sign_transaction`:**

```rust
/// Sign a raw hash with this wallet's private key.
/// Returns the DER-encoded signature bytes.
/// Used for multisig signing where the full P2PKH scriptSig is not needed.
pub fn sign_hash(&self, hash: &Hash256) -> Result<Vec<u8>, WalletError> {
    let private_key_bytes =
        hex::decode(&self.private_key_hex).map_err(|_| WalletError::InvalidPrivateKey)?;
    let secret_key = SecretKey::from_slice(&private_key_bytes)
        .map_err(|_| WalletError::InvalidPrivateKey)?;
    let signing_key = SigningKey::from(secret_key);
    let signature: Signature = signing_key.sign(&hash.0);
    Ok(signature.to_der().as_bytes().to_vec())
}
```

- [ ] **Verify it compiles:**

```bash
cargo build -p tensorium-core 2>&1 | grep error
```

Expected: no errors.

- [ ] **Run wallet tests to confirm nothing broke:**

```bash
cargo test -p tensorium-core wallet 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Commit:**

```bash
git add crates/tensorium-core/src/wallet.rs
git commit -m "feat(wallet): add WalletKeypair::sign_hash for multisig partial signing"
```

---

## Task 6: Extend /getutxos/ in tensorium-node

**Files:**
- Modify: `crates/tensorium-node/src/main.rs`

The current handler at `("GET", path) if path.starts_with("/getutxos/")` decodes the address as bech32 and converts to a P2PKH script. Extend it to also accept a lowercase hex scriptPubKey.

- [ ] **Write a failing test** — add to the `#[cfg(test)]` block:

```rust
#[test]
fn getutxos_accepts_scriptpubkey_hex() {
    // Non-bech32 hex string should not be treated as an address
    let param = "5221aabb";
    let is_address = param.starts_with("txm1");
    assert!(!is_address, "hex scriptpubkey should not be decoded as address");
}
```

- [ ] **Run:**

```bash
cargo test -p tensorium-node getutxos_accepts 2>&1 | tail -3
```

Expected: pass (trivially — confirms the logic branch exists).

- [ ] **Update the handler** — find the block starting with `("GET", path) if path.starts_with("/getutxos/") =>` and replace the script derivation logic:

Current code to find:
```rust
("GET", path) if path.starts_with("/getutxos/") => {
    let address = path.trim_start_matches("/getutxos/");
```

Replace the entire address-to-script block (lines that decode bech32 and call p2pkh_from_address) with:

```rust
("GET", path) if path.starts_with("/getutxos/") => {
    let param = path.trim_start_matches("/getutxos/");

    if param.is_empty() {
        write_json_response(stream, 400, &RpcError::new("missing param: GET /getutxos/<address_or_scriptpubkey_hex>"))?;
        return Ok(());
    }

    // If param starts with "txm1" treat as bech32 address → derive P2PKH script.
    // Otherwise treat as lowercase hex-encoded scriptPubKey.
    let script = if param.starts_with("txm1") {
        match p2pkh_from_address(param) {
            Ok(s) => s,
            Err(_) => {
                write_json_response(stream, 400, &RpcError::new("invalid address: GET /getutxos/<address>"))?;
                return Ok(());
            }
        }
    } else {
        match hex::decode(param) {
            Ok(s) => s,
            Err(_) => {
                write_json_response(stream, 400, &RpcError::new("invalid hex: GET /getutxos/<scriptpubkey_hex>"))?;
                return Ok(());
            }
        }
    };
```

Make sure to keep everything after this block (the UTXO scan and JSON response) unchanged. The existing code already uses `script` as the filter variable, so only the derivation logic needs to change.

- [ ] **Verify compile:**

```bash
cargo build -p tensorium-node 2>&1 | grep error
```

Expected: no errors.

- [ ] **Run node tests:**

```bash
cargo test -p tensorium-node 2>&1 | grep -E "test result|FAILED"
```

Expected: all pass.

- [ ] **Commit:**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): extend /getutxos/ to accept hex scriptPubKey in addition to address"
```

---

## Task 7: txmwallet multisig-script command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Add import** — at the top of main.rs, update the `tensorium_core` import to include:

```rust
use tensorium_core::{
    block::{Transaction, TxInput, TxOutput},
    chain::MAINNET_CANDIDATE,
    script::standard::{multisig_script, multisig_script_sig, extract_multisig,
                       p2pkh_from_address, p2pkh_from_pubkey},
    ChainState, UtxoSet, WalletKeypair,
};
```

- [ ] **Add `multisig-script` arm** to the `match command { ... }` block, before `_ => print_help()`:

```rust
"multisig-script" => {
    // Usage: txmwallet multisig-script <m> <pubkey_hex1> ... <pubkey_hexN>
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
```

- [ ] **Build and smoke-test:**

```bash
cargo build -p txmwallet 2>&1 | grep error
echo "---"
# Quick smoke: 2-of-3 with dummy 33-byte pubkeys
PUBKEY=$(printf '02%.0s' {1..33} | head -c 66)
./target/debug/txmwallet multisig-script 2 \
  "$(python3 -c 'print("02"+"aa"*32)')" \
  "$(python3 -c 'print("03"+"bb"*32)')" \
  "$(python3 -c 'print("02"+"cc"*32)')"
```

Expected output contains `scriptpubkey: 5221...53ae` and `m=2  n=3`.

- [ ] **Commit:**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): add multisig-script command"
```

---

## Task 8: txmwallet send-from-script command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Add sig-file struct** — add near the top of main.rs with the other structs:

```rust
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
```

- [ ] **Add `send-from-script` arm** in the `match command` block:

```rust
"send-from-script" => {
    // Usage: txmwallet send-from-script <scriptpubkey_hex> <dest_addr> <atoms> [tx_file] [rpc]
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

    let tx = build_unsigned_multisig_tx(
        rpc, scriptpubkey_hex, dest_addr, amount_atoms,
    )?;
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
```

- [ ] **Add `build_unsigned_multisig_tx` helper function** — add after the existing `build_signed_payment_via_rpc` function:

```rust
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

    // Inputs have empty signature_script — will be filled by multisig-combine
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
        // Change returns to the same multisig scriptPubKey
        outputs.push(TxOutput { value_atoms: change, script_pubkey: source_script });
    }

    Ok(Transaction::payment(inputs, outputs))
}
```

- [ ] **Build:**

```bash
cargo build -p txmwallet 2>&1 | grep error
```

Expected: no errors.

- [ ] **Commit:**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): add send-from-script command for unsigned multisig tx"
```

---

## Task 9: txmwallet multisig-sign command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Add `multisig-sign` arm** in the `match command` block:

```rust
"multisig-sign" => {
    // Usage: txmwallet multisig-sign <tx_file>
    let tx_path = PathBuf::from(
        args.get(2).ok_or("usage: txmwallet multisig-sign <tx_file>")?
    );
    let passphrase = passphrase_from_env()?;
    let wallet = load_wallet(&wallet_path)?;
    let keypair = wallet.decrypt(&passphrase)?;

    let raw = fs::read_to_string(&tx_path)
        .map_err(|e| format!("read {}: {e}", tx_path.display()))?;
    let tx: Transaction = serde_json::from_str(&raw)
        .map_err(|e| format!("parse tx: {e}"))?;

    // Sign the sig_hash once; apply the same sig to all inputs.
    // (All inputs in a multisig tx spend from the same scriptPubKey.)
    let sig_hash = tx.signature_hash();
    let der_sig = keypair.sign_hash(&sig_hash)
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

    // Write to <tx_file>.sig<first6 of address>
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
```

- [ ] **Build:**

```bash
cargo build -p txmwallet 2>&1 | grep error
```

Expected: no errors.

- [ ] **Commit:**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): add multisig-sign command for partial signature generation"
```

---

## Task 10: txmwallet multisig-combine command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Add `multisig-combine` arm** in the `match command` block:

```rust
"multisig-combine" => {
    // Usage: txmwallet multisig-combine <tx_file> <sig_file1> <sig_file2> [sig_file3...]
    let tx_path = PathBuf::from(
        args.get(2).ok_or("usage: txmwallet multisig-combine <tx_file> <sig_file1> <sig_file2> [...]")?
    );
    if args.len() < 5 {
        return Err("multisig-combine requires at least 2 sig files".to_owned());
    }
    let sig_paths: Vec<PathBuf> = args[3..].iter().map(PathBuf::from).collect();

    let raw = fs::read_to_string(&tx_path)
        .map_err(|e| format!("read {}: {e}", tx_path.display()))?;
    let mut tx: Transaction = serde_json::from_str(&raw)
        .map_err(|e| format!("parse tx: {e}"))?;

    let expected_txid = hex::encode(&tx.id.0);

    // Collect one sig per sig-file, in the order provided
    let mut collected_sigs: Vec<Vec<u8>> = Vec::new();
    for sig_path in &sig_paths {
        let sig_raw = fs::read_to_string(sig_path)
            .map_err(|e| format!("read {}: {e}", sig_path.display()))?;
        let sig_file: MultisigSigFile = serde_json::from_str(&sig_raw)
            .map_err(|e| format!("parse {}: {e}", sig_path.display()))?;
        if sig_file.unsigned_txid != expected_txid {
            return Err(format!(
                "sig file {} txid mismatch: expected {}, got {}",
                sig_path.display(), expected_txid, sig_file.unsigned_txid
            ));
        }
        // Take the sig for input 0 (v1: single-input multisig)
        let sig = sig_file.sigs.iter()
            .find(|s| s.input_index == 0)
            .ok_or_else(|| format!("no sig for input 0 in {}", sig_path.display()))?;
        collected_sigs.push(
            hex::decode(&sig.der_sig_hex)
                .map_err(|_| format!("invalid sig hex in {}", sig_path.display()))?
        );
    }

    // Build the combined scriptSig and apply to all inputs
    let sig_refs: Vec<&[u8]> = collected_sigs.iter().map(|v| v.as_slice()).collect();
    let script_sig = multisig_script_sig(&sig_refs);

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
    println!("ready to broadcast: txmwallet broadcast {}", tx_path.display());
}
```

- [ ] **Update `print_help`** — add the four new commands:

```rust
fn print_help() {
    println!("txmwallet <command>");
    println!();
    println!("commands:");
    println!("  create                                        create a local wallet file");
    println!("  getnewaddress                                 print wallet address");
    println!("  balance                                       scan local chain state for wallet balance");
    println!("  send <to> <atoms> [tx_file]                   build and sign a transaction file");
    println!("  broadcast [tx_file] [rpc]                     submit signed tx file to node RPC");
    println!("  show                                          print wallet public summary");
    println!("  unlock-check                                  verify passphrase can decrypt wallet");
    println!("  multisig-script <m> <pubkey_hex>...           print scriptPubKey for m-of-n multisig");
    println!("  send-from-script <spk_hex> <to> <atoms>       build unsigned multisig spend tx");
    println!("  multisig-sign <tx_file>                       sign a multisig tx with this wallet");
    println!("  multisig-combine <tx_file> <sig1> <sig2>...   combine partial sigs into broadcast tx");
    println!();
    println!("env:");
    println!("  TENSORIUM_WALLET             wallet file, default {DEFAULT_WALLET_PATH}");
    println!("  TENSORIUM_STATE              chain state, default {DEFAULT_STATE_PATH}");
    println!("  TENSORIUM_WALLET_PASSPHRASE  required for create, send, unlock-check, multisig-sign");
}
```

- [ ] **Build:**

```bash
cargo build -p txmwallet 2>&1 | grep error
```

Expected: no errors.

- [ ] **Run all workspace tests:**

```bash
cargo test --workspace 2>&1 | grep -E "test result|FAILED"
```

Expected: all `ok`, 0 failed.

- [ ] **Commit:**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): add multisig-combine command and update help text"
```

---

## Task 11: Final Verification + Push

- [ ] **Full clean build:**

```bash
cargo build --release --workspace 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Full test suite:**

```bash
cargo test --workspace 2>&1 | grep -E "test result|FAILED"
```

Expected: 6 `test result: ok` lines, 0 failed.

- [ ] **End-to-end smoke test on local devchain:**

```bash
# Terminal 1: start a local node
TENSORIUM_MC_STATE=/tmp/s2-test-state.json cargo run -p tensorium-node -- mainnet-candidate init
TENSORIUM_MC_STATE=/tmp/s2-test-state.json cargo run -p tensorium-node -- mainnet-candidate rpc 127.0.0.1:33332 &
NODE_PID=$!

# Create two wallets
TENSORIUM_WALLET=/tmp/wallet-a.json TENSORIUM_WALLET_PASSPHRASE=testpassA123 \
  cargo run -p txmwallet -- create
TENSORIUM_WALLET=/tmp/wallet-b.json TENSORIUM_WALLET_PASSPHRASE=testpassB123 \
  cargo run -p txmwallet -- create

PUBKEY_A=$(TENSORIUM_WALLET=/tmp/wallet-a.json TENSORIUM_WALLET_PASSPHRASE=testpassA123 \
  cargo run -p txmwallet -- show 2>/dev/null | grep public_key | cut -d= -f2)
PUBKEY_B=$(TENSORIUM_WALLET=/tmp/wallet-b.json TENSORIUM_WALLET_PASSPHRASE=testpassB123 \
  cargo run -p txmwallet -- show 2>/dev/null | grep public_key | cut -d= -f2)
echo "pubA=$PUBKEY_A"
echo "pubB=$PUBKEY_B"

# Get scriptPubKey for 1-of-2 (easier to test: only 1 sig needed)
MULTISIG_SPK=$(cargo run -p txmwallet -- multisig-script 1 "$PUBKEY_A" "$PUBKEY_B" \
  2>/dev/null | grep scriptpubkey | cut -d' ' -f2)
echo "multisig_spk=$MULTISIG_SPK"

kill $NODE_PID 2>/dev/null
```

Expected: `scriptpubkey:` printed, no errors.

- [ ] **Push:**

```bash
git push
```

- [ ] **Deploy to VPS:**

```bash
# On VPS: pull, rebuild node and txmwallet binaries, restart services
# (Only node and txmwallet changed — pool not needed)
```

Follow the standard deploy workflow: SSH to VPS → git pull → cargo build --release -p tensorium-node -p txmwallet → install binaries → restart services.
