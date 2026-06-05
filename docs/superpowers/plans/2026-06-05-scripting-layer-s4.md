# Scripting Layer S4 — P2SH-Multisig Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Pay-to-Script-Hash (P2SH) wrapping for multisig scripts, giving Tensorium compact `txms1...` addresses for m-of-n multisig with a full wallet CLI flow.

**Architecture:** P2SH detection is inlined into `vm::execute()` — after `script_sig` runs, if `script_pubkey` matches the 23-byte P2SH pattern, the VM pops the redeem script, verifies its hash, then runs it against the remaining stack. `utxo.rs` is untouched. `standard.rs` gets P2SH builder functions. Two new `txmwallet` commands plus a `--redeem` flag on the existing `multisig-combine` command.

**Tech Stack:** Rust, `sha2` crate (already in deps), `bech32` crate (already in deps), `k256` (already in deps).

---

## Files Changed

| File | Change |
|------|--------|
| `crates/tensorium-core/src/script/mod.rs` | Add `ScriptError::P2shHashMismatch` |
| `crates/tensorium-core/src/script/vm.rs` | Add `is_p2sh()`, OP_PUSHDATA1 support, P2SH branch in `execute()` |
| `crates/tensorium-core/src/script/standard.rs` | Add 6 P2SH builder/parser functions, extend `extract_address()`, add `OP_EQUAL` to imports |
| `crates/txmwallet/src/main.rs` | New commands `p2sh-multisig-script`, `p2sh-multisig-spend`; modify `multisig-combine` with `--redeem`; update imports + help |

---

## Task 1 — ScriptError::P2shHashMismatch + is_p2sh predicate

**Files:**
- Modify: `crates/tensorium-core/src/script/mod.rs`
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block at the bottom of `crates/tensorium-core/src/script/vm.rs`:

```rust
#[test]
fn is_p2sh_recognizes_pattern() {
    let mut spk = vec![OP_HASH160, 0x14];
    spk.extend_from_slice(&[0xab_u8; 20]);
    spk.push(OP_EQUAL);
    assert!(is_p2sh(&spk));
}

#[test]
fn is_p2sh_rejects_non_p2sh() {
    // too short
    assert!(!is_p2sh(&[OP_HASH160, 0x14]));
    // wrong first byte (P2PKH starts with OP_DUP)
    let mut p2pkh = vec![OP_DUP, 0x14];
    p2pkh.extend_from_slice(&[0xab_u8; 20]);
    p2pkh.push(OP_EQUAL);
    assert!(!is_p2sh(&p2pkh));
    // wrong last byte
    let mut spk = vec![OP_HASH160, 0x14];
    spk.extend_from_slice(&[0xab_u8; 20]);
    spk.push(OP_EQUALVERIFY); // not OP_EQUAL
    assert!(!is_p2sh(&spk));
}
```

- [ ] **Step 2: Run tests — expect compile error (is_p2sh not defined yet)**

```bash
cd /root/.openclaw/workspace/tensorium-core
cargo test -p tensorium-core --lib -- script::vm::tests::is_p2sh 2>&1 | head -20
```

Expected: compile error `cannot find function \`is_p2sh\``

- [ ] **Step 3: Add P2shHashMismatch to ScriptError in mod.rs**

In `crates/tensorium-core/src/script/mod.rs`, add the new variant to the `ScriptError` enum after `LockTimeNotMet`:

```rust
    LockTimeNotMet,
    P2shHashMismatch,
```

- [ ] **Step 4: Add is_p2sh helper in vm.rs**

In `crates/tensorium-core/src/script/vm.rs`, add this private function just before `pub fn execute(`:

```rust
fn is_p2sh(spk: &[u8]) -> bool {
    spk.len() == 23 && spk[0] == OP_HASH160 && spk[1] == 0x14 && spk[22] == OP_EQUAL
}
```

- [ ] **Step 5: Run tests — expect PASS**

```bash
cargo test -p tensorium-core --lib -- script::vm::tests::is_p2sh
```

Expected: `test script::vm::tests::is_p2sh_recognizes_pattern ... ok` and `test script::vm::tests::is_p2sh_rejects_non_p2sh ... ok`

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-core/src/script/mod.rs crates/tensorium-core/src/script/vm.rs
git commit -m "feat(s4): add ScriptError::P2shHashMismatch and is_p2sh predicate"
```

---

## Task 2 — OP_PUSHDATA1 (0x4c) support in VM

A 2-of-3 multisig redeem script is 105 bytes — too large for the existing 0x01–0x4b single-byte push. OP_PUSHDATA1 must be handled in the script runner to allow pushing the redeem script in scriptSig.

**Files:**
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Step 1: Write the failing test**

Add to the test block in `vm.rs`:

```rust
#[test]
fn pushdata1_pushes_100_byte_item() {
    let mut script = vec![0x4c, 100u8]; // OP_PUSHDATA1, length = 100
    script.extend_from_slice(&[0xab_u8; 100]);
    let mut stack: Vec<Vec<u8>> = Vec::new();
    super::run(&mut stack, &script, &fake_ctx(), false).unwrap();
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0], vec![0xab_u8; 100]);
}
```

- [ ] **Step 2: Run test — expect FAIL with InvalidOpcode(76)**

```bash
cargo test -p tensorium-core --lib -- script::vm::tests::pushdata1
```

Expected: `FAILED` — `InvalidOpcode(76)` (0x4c = 76 decimal)

- [ ] **Step 3: Add OP_PUSHDATA1 handling in run()**

In `crates/tensorium-core/src/script/vm.rs`, inside `pub(crate) fn run()`, add this block immediately after the `if op >= 0x01 && op <= 0x4b { ... continue; }` block (around line 68):

```rust
        // ── OP_PUSHDATA1 (0x4c): next byte is data length ─────────────────
        if op == 0x4c {
            if i >= script.len() {
                return Err(ScriptError::UnexpectedEndOfScript);
            }
            let n = script[i] as usize;
            i += 1;
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
```

- [ ] **Step 4: Run test — expect PASS**

```bash
cargo test -p tensorium-core --lib -- script::vm::tests::pushdata1
```

Expected: `test script::vm::tests::pushdata1_pushes_100_byte_item ... ok`

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-core/src/script/vm.rs
git commit -m "feat(s4): add OP_PUSHDATA1 support in script VM"
```

---

## Task 3 — P2SH execution path in execute()

Two tests use only hardcoded byte sequences (no `standard.rs` deps) and can be written and run now.
The two integration tests (`p2sh_multisig_2of3_valid`, `p2sh_wrong_sig_fails`) that need
`p2sh_script_from_redeem` and `p2sh_multisig_script_sig` are added in **Task 4** after those
functions exist.

**Files:**
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Step 1: Write 2 failing tests that only use hardcoded bytes**

Add to the test block in `vm.rs`:

```rust
#[test]
fn p2sh_hash_mismatch_fails() {
    // Manually: OP_HASH160 0x14 [0xaa×20] OP_EQUAL  (no standard.rs import needed)
    let mut p2sh_spk = vec![0xa9u8, 0x14]; // OP_HASH160, push-20
    p2sh_spk.extend_from_slice(&[0xaa_u8; 20]);
    p2sh_spk.push(0x87); // OP_EQUAL
    // Push a 3-byte redeem script whose sha256 != [0xaa×20]
    let script_sig = vec![0x03u8, 0x01, 0x02, 0x03];
    let result = execute(&script_sig, &p2sh_spk, &fake_ctx());
    assert_eq!(result, Err(ScriptError::P2shHashMismatch));
}

#[test]
fn p2sh_empty_stack_fails() {
    let mut p2sh_spk = vec![0xa9u8, 0x14];
    p2sh_spk.extend_from_slice(&[0xab_u8; 20]);
    p2sh_spk.push(0x87); // OP_EQUAL
    // Empty scriptSig → nothing on stack → can't pop redeem script
    let result = execute(&[], &p2sh_spk, &fake_ctx());
    assert_eq!(result, Err(ScriptError::StackUnderflow));
}
```

- [ ] **Step 2: Run tests — expect FAIL (P2SH branch not added yet)**

```bash
cargo test -p tensorium-core --lib -- "script::vm::tests::p2sh_hash_mismatch\|script::vm::tests::p2sh_empty_stack" 2>&1 | head -20
```

Expected: both tests FAIL (the VM currently returns `Ok(false)` or `InvalidOpcode` for a P2SH script, not `P2shHashMismatch`/`StackUnderflow`)

- [ ] **Step 3: Add P2SH branch to execute() in vm.rs**

Replace the existing `execute()` function body with this version that adds the P2SH branch between the two `run()` calls:

```rust
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

    if is_p2sh(script_pubkey) {
        use sha2::{Digest, Sha256};
        let redeem_script = stack.pop().ok_or(ScriptError::StackUnderflow)?;
        let computed = &Sha256::digest(&redeem_script)[..20];
        let expected = &script_pubkey[2..22];
        if computed != expected {
            return Err(ScriptError::P2shHashMismatch);
        }
        run(&mut stack, &redeem_script, ctx, true)?;
    } else {
        run(&mut stack, script_pubkey, ctx, true)?;
    }

    Ok(stack.last().map(is_truthy).unwrap_or(false))
}
```

- [ ] **Step 4: Run tests — expect both PASS**

```bash
cargo test -p tensorium-core --lib -- "script::vm::tests::p2sh_hash_mismatch\|script::vm::tests::p2sh_empty_stack"
```

Expected:
```
test script::vm::tests::p2sh_empty_stack_fails ... ok
test script::vm::tests::p2sh_hash_mismatch_fails ... ok
```

- [ ] **Step 5: Commit vm.rs changes**

```bash
git add crates/tensorium-core/src/script/vm.rs
git commit -m "feat(s4): add P2SH execution path in vm::execute()"
```

---

## Task 4 — P2SH builders in standard.rs

**Files:**
- Modify: `crates/tensorium-core/src/script/standard.rs`

- [ ] **Step 1: Write the failing tests**

**In `crates/tensorium-core/src/script/vm.rs`** test block, add the 2 integration tests that need `standard.rs` functions (now that those functions will exist after this task):

```rust
#[test]
fn p2sh_multisig_2of3_valid() {
    use crate::script::standard::{multisig_script, p2sh_multisig_script_sig, p2sh_script_from_redeem};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let k3 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p3 = k3.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let redeem = multisig_script(2, &[p1.as_slice(), p2.as_slice(), p3.as_slice()]).unwrap();
    let p2sh_spk = p2sh_script_from_redeem(&redeem);

    let msg = Hash256([3u8; 32]);
    let sig1: Signature = k1.sign(&msg.0);
    let sig2: Signature = k2.sign(&msg.0);
    let d1 = sig1.to_der().as_bytes().to_vec();
    let d2 = sig2.to_der().as_bytes().to_vec();
    let script_sig = p2sh_multisig_script_sig(&[d1.as_slice(), d2.as_slice()], &redeem);

    let result = execute(&script_sig, &p2sh_spk, &real_ctx(msg)).unwrap();
    assert!(result, "valid P2SH 2-of-3 must succeed");
}

#[test]
fn p2sh_wrong_sig_fails() {
    use crate::script::standard::{multisig_script, p2sh_multisig_script_sig, p2sh_script_from_redeem};
    use k256::ecdsa::{signature::Signer, Signature, SigningKey};
    use rand_core::OsRng;

    let k1 = SigningKey::random(&mut OsRng);
    let k2 = SigningKey::random(&mut OsRng);
    let k3 = SigningKey::random(&mut OsRng);
    let p1 = k1.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p2 = k2.verifying_key().to_encoded_point(true).as_bytes().to_vec();
    let p3 = k3.verifying_key().to_encoded_point(true).as_bytes().to_vec();

    let redeem = multisig_script(2, &[p1.as_slice(), p2.as_slice(), p3.as_slice()]).unwrap();
    let p2sh_spk = p2sh_script_from_redeem(&redeem);

    let msg = Hash256([3u8; 32]);
    let wrong_msg = Hash256([99u8; 32]);
    let sig1: Signature = k1.sign(&wrong_msg.0);
    let sig2: Signature = k2.sign(&wrong_msg.0);
    let d1 = sig1.to_der().as_bytes().to_vec();
    let d2 = sig2.to_der().as_bytes().to_vec();
    let script_sig = p2sh_multisig_script_sig(&[d1.as_slice(), d2.as_slice()], &redeem);

    let result = execute(&script_sig, &p2sh_spk, &real_ctx(msg));
    assert!(result.is_err() || !result.unwrap(), "wrong sigs must fail");
}
```

**In `crates/tensorium-core/src/script/standard.rs`** test block, add 4 standard.rs tests:

Add to the `#[cfg(test)] mod tests` block at the bottom of `standard.rs`:

```rust
    #[test]
    fn p2sh_script_roundtrip() {
        let redeem = vec![0x52u8, 0x21, 0xab, 0x21, 0xcd, 0x52, 0xae];
        let spk = p2sh_script_from_redeem(&redeem);
        let hash = extract_p2sh_hash(&spk).unwrap();
        let expected = &Sha256::digest(&redeem)[..20];
        assert_eq!(&hash, expected);
    }

    #[test]
    fn p2sh_address_roundtrip() {
        let hash20 = [0x42_u8; 20];
        let addr = p2sh_address_from_hash(&hash20);
        assert!(addr.starts_with("txms"), "P2SH address must have txms prefix");
        let recovered = p2sh_hash_from_address(&addr).unwrap();
        assert_eq!(recovered, hash20);
    }

    #[test]
    fn p2sh_address_rejects_txm_prefix() {
        let hash20 = [0x11_u8; 20];
        let p2pkh_addr = bech32::encode("txm", hash20.to_base32(), Variant::Bech32).unwrap();
        assert_eq!(
            p2sh_hash_from_address(&p2pkh_addr),
            Err(ScriptError::InvalidAddress)
        );
    }

    #[test]
    fn p2sh_multisig_script_sig_layout_with_pushdata1() {
        // 2 sigs (71 + 70 bytes) + redeem script 100 bytes (> 0x4b, needs PUSHDATA1)
        let sig1 = vec![0xaa_u8; 71];
        let sig2 = vec![0xbb_u8; 70];
        let redeem = vec![0xcc_u8; 100];

        let script_sig = p2sh_multisig_script_sig(&[sig1.as_slice(), sig2.as_slice()], &redeem);

        // [71][aa×71][70][bb×70][0x4c][100][cc×100]
        assert_eq!(script_sig[0], 71);
        assert_eq!(&script_sig[1..72], &[0xaa_u8; 71]);
        assert_eq!(script_sig[72], 70);
        assert_eq!(&script_sig[73..143], &[0xbb_u8; 70]);
        assert_eq!(script_sig[143], 0x4c); // OP_PUSHDATA1
        assert_eq!(script_sig[144], 100);
        assert_eq!(&script_sig[145..245], &[0xcc_u8; 100]);
        assert_eq!(script_sig.len(), 245);
    }
```

- [ ] **Step 2: Run tests — expect compile errors (functions not defined yet)**

```bash
cargo test -p tensorium-core --lib 2>&1 | grep "^error" | head -10
```

Expected: compile errors for `p2sh_script_from_redeem`, `p2sh_multisig_script_sig`, etc. not found.

- [ ] **Step 3: Add OP_EQUAL to the imports in standard.rs**

Find the existing `use crate::script::{...};` line at the top of `standard.rs` and add `OP_EQUAL`:

```rust
use crate::script::{
    ScriptError, OP_0, OP_1, OP_CHECKLOCKTIMEVERIFY, OP_CHECKMULTISIG, OP_CHECKSIG, OP_DROP,
    OP_DUP, OP_ELSE, OP_ENDIF, OP_EQUAL, OP_EQUALVERIFY, OP_HASH160, OP_IF, OP_SHA256,
};
```

- [ ] **Step 4: Add the P2SH constant and six new functions**

Add after the existing `const ADDRESS_HRP: &str = "txm";` line:

```rust
const P2SH_HRP: &str = "txms";
```

Then add the following functions after `extract_address()` (around line 73), before `multisig_script`:

```rust
/// Build a P2SH locking script from a 20-byte script hash.
/// Script: OP_HASH160 0x14 <hash20> OP_EQUAL  (23 bytes)
pub fn p2sh_script(hash20: &[u8]) -> Vec<u8> {
    assert_eq!(hash20.len(), 20, "P2SH hash must be 20 bytes");
    let mut s = Vec::with_capacity(23);
    s.push(OP_HASH160);
    s.push(0x14);
    s.extend_from_slice(hash20);
    s.push(OP_EQUAL);
    s
}

/// Build a P2SH locking script by hashing the serialized redeem script.
pub fn p2sh_script_from_redeem(redeem_script: &[u8]) -> Vec<u8> {
    let hash = Sha256::digest(redeem_script);
    p2sh_script(&hash[..20])
}

/// Encode a 20-byte P2SH hash as a bech32 "txms1..." address.
pub fn p2sh_address_from_hash(hash20: &[u8]) -> String {
    bech32::encode(P2SH_HRP, hash20.to_base32(), Variant::Bech32)
        .expect("bech32 encoding should never fail for 20-byte input")
}

/// Derive the P2SH address directly from the redeem script.
pub fn p2sh_address_from_redeem(redeem_script: &[u8]) -> String {
    let hash = Sha256::digest(redeem_script);
    p2sh_address_from_hash(&hash[..20])
}

/// Decode a "txms1..." P2SH address into its 20-byte hash.
/// Returns Err(InvalidAddress) if the HRP is not "txms" or the payload is not 20 bytes.
pub fn p2sh_hash_from_address(addr: &str) -> Result<[u8; 20], ScriptError> {
    let (hrp, data, _) = bech32::decode(addr).map_err(|_| ScriptError::InvalidAddress)?;
    if hrp != P2SH_HRP {
        return Err(ScriptError::InvalidAddress);
    }
    let bytes = Vec::<u8>::from_base32(&data).map_err(|_| ScriptError::InvalidAddress)?;
    if bytes.len() != 20 {
        return Err(ScriptError::InvalidAddress);
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Extract the 20-byte hash from a P2SH scriptPubKey.
/// Returns None if the script does not match the P2SH pattern.
pub fn extract_p2sh_hash(spk: &[u8]) -> Option<[u8; 20]> {
    if spk.len() != 23 || spk[0] != OP_HASH160 || spk[1] != 0x14 || spk[22] != OP_EQUAL {
        return None;
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&spk[2..22]);
    Some(out)
}

/// Build a P2SH-multisig scriptSig: [sig_len][sig]... [redeem_push][redeem_script].
/// Sigs are pushed with single-byte length prefix (DER sigs are always ≤ 72 bytes).
/// The redeem script uses OP_PUSHDATA1 (0x4c) if it exceeds 75 bytes.
pub fn p2sh_multisig_script_sig(sigs: &[&[u8]], redeem_script: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    for sig in sigs {
        debug_assert!(sig.len() <= 0x4b, "DER sig unexpectedly large");
        s.push(sig.len() as u8);
        s.extend_from_slice(sig);
    }
    let rlen = redeem_script.len();
    if rlen <= 0x4b {
        s.push(rlen as u8);
    } else {
        assert!(rlen <= 0xff, "redeem script > 255 bytes is not supported");
        s.push(0x4c); // OP_PUSHDATA1
        s.push(rlen as u8);
    }
    s.extend_from_slice(redeem_script);
    s
}
```

- [ ] **Step 5: Extend extract_address() to handle P2SH**

Replace the existing `extract_address()` function body so it checks P2SH first:

```rust
pub fn extract_address(script_pubkey: &[u8]) -> Option<String> {
    // P2SH: OP_HASH160 0x14 [20 bytes] OP_EQUAL
    if let Some(hash20) = extract_p2sh_hash(script_pubkey) {
        return Some(p2sh_address_from_hash(&hash20));
    }
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
```

- [ ] **Step 6: Run all script tests — expect all 11 new tests PASS**

```bash
cargo test -p tensorium-core --lib -- script 2>&1
```

Expected output (all 11 new tests must appear as `ok`):
```
test script::standard::tests::p2sh_address_rejects_txm_prefix ... ok
test script::standard::tests::p2sh_address_roundtrip ... ok
test script::standard::tests::p2sh_multisig_script_sig_layout_with_pushdata1 ... ok
test script::standard::tests::p2sh_script_roundtrip ... ok
test script::vm::tests::is_p2sh_recognizes_pattern ... ok
test script::vm::tests::is_p2sh_rejects_non_p2sh ... ok
test script::vm::tests::p2sh_empty_stack_fails ... ok
test script::vm::tests::p2sh_hash_mismatch_fails ... ok
test script::vm::tests::p2sh_multisig_2of3_valid ... ok
test script::vm::tests::p2sh_wrong_sig_fails ... ok
test script::vm::tests::pushdata1_pushes_100_byte_item ... ok
```

- [ ] **Step 7: Run full workspace tests — expect no regressions**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: `test result: ok. N passed; 0 failed; 0 ignored`  (N ≈ 106)

- [ ] **Step 8: Commit**

```bash
git add crates/tensorium-core/src/script/standard.rs
git commit -m "feat(s4): add P2SH builder functions and extend extract_address()"
```

---

## Task 5 — txmwallet: p2sh-multisig-script command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add p2sh imports to main.rs**

Find the `use tensorium_core::{ ... script::standard::{ ... } ... };` block at the top of `main.rs` (around line 19) and add the four new P2SH functions:

```rust
    script::standard::{
        extract_multisig, extract_p2sh_hash, htlc_claim_script_sig, htlc_refund_script_sig,
        htlc_script, multisig_script, multisig_script_sig, p2pkh_from_address, p2pkh_from_pubkey,
        p2sh_address_from_redeem, p2sh_multisig_script_sig, p2sh_script_from_redeem,
    },
```

- [ ] **Step 2: Add the p2sh-multisig-script command arm**

In `main.rs`, inside `match command { ... }`, add the new arm after the `"multisig-script" => { ... }` block:

```rust
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
```

- [ ] **Step 3: Add the command to print_help()**

In `print_help()`, after the `multisig-combine` line, add:

```rust
    println!("  p2sh-multisig-script <m> <pk1_hex>...        build P2SH-multisig address (txms1...)");
    println!("  p2sh-multisig-spend <spk_hex> <to> <redeem_hex> <atoms> [rpc]  build unsigned P2SH spend tx");
```

- [ ] **Step 4: Build and smoke-test**

```bash
cargo build -p txmwallet --release 2>&1 | grep -E "error|warning" | head -20
```

Expected: builds with 0 errors.

Manual test with dummy pubkeys:
```bash
cd /root/.openclaw/workspace/tensorium-core
PUBKEY=020202020202020202020202020202020202020202020202020202020202020202
./target/release/txmwallet p2sh-multisig-script 2 $PUBKEY $PUBKEY $PUBKEY
```

Expected output:
```
redeem_script:    52210202...ae
p2sh_scriptpubkey: a914...87
address:          txms1q...
m=2  n=3
note: save the redeem_script hex — required to spend
```

The `address` field must start with `txms1`.

- [ ] **Step 5: Commit**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(s4): add p2sh-multisig-script wallet command"
```

---

## Task 6 — txmwallet: p2sh-multisig-spend command

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the p2sh-multisig-spend command arm**

Add after the `"p2sh-multisig-script" => { ... }` block:

```rust
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
```

- [ ] **Step 2: Build — expect 0 errors**

```bash
cargo build -p txmwallet --release 2>&1 | grep "^error" | head -10
```

Expected: no output (0 errors).

- [ ] **Step 3: Commit**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(s4): add p2sh-multisig-spend wallet command"
```

---

## Task 7 — txmwallet: multisig-combine --redeem flag

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Modify the multisig-combine arm**

Find the `"multisig-combine" => { ... }` block (around line 273). Replace the argument parsing section (the `if args.len() < 5` check and `let sig_paths` line) with a version that strips `--redeem <hex>` from the args before building the sig path list:

Replace this existing code:
```rust
            if args.len() < 5 {
                return Err("multisig-combine requires at least 2 sig files".to_owned());
            }
            let sig_paths: Vec<PathBuf> = args[3..].iter().map(PathBuf::from).collect();
```

With:
```rust
            // Parse optional --redeem <hex> flag, leaving only sig file paths
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
```

Then replace the `script_sig` assembly line (the one that calls `multisig_script_sig`):

Replace:
```rust
            let sig_refs: Vec<&[u8]> = collected_sigs.iter().map(|v| v.as_slice()).collect();
            let script_sig = multisig_script_sig(&sig_refs);
```

With:
```rust
            let sig_refs: Vec<&[u8]> = collected_sigs.iter().map(|v| v.as_slice()).collect();
            let script_sig = if let Some(ref r_hex) = redeem_hex {
                let redeem = hex::decode(r_hex)
                    .map_err(|_| "invalid --redeem hex: must be lowercase hex".to_owned())?;
                p2sh_multisig_script_sig(&sig_refs, &redeem)
            } else {
                multisig_script_sig(&sig_refs)
            };
```

- [ ] **Step 2: Update the multisig-combine line in print_help()**

Find:
```rust
    println!(
        "  multisig-combine <tx_file> <sig1> <sig2>...   combine partial sigs into broadcast tx"
    );
```

Replace with:
```rust
    println!(
        "  multisig-combine <tx_file> <sig1> <sig2>... [--redeem <hex>]  combine sigs (add --redeem for P2SH)"
    );
```

- [ ] **Step 3: Build — expect 0 errors**

```bash
cargo build -p txmwallet --release 2>&1 | grep "^error" | head -10
```

Expected: no output.

- [ ] **Step 4: Verify backward compatibility — bare multisig path untouched**

```bash
./target/release/txmwallet multisig-combine 2>&1 | head -5
```

Expected: error message `read : No such file or directory` or similar (not a crash on the arg parsing change).

- [ ] **Step 5: Commit**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(s4): add --redeem flag to multisig-combine for P2SH spending"
```

---

## Task 8 — Full workspace tests + push + VPS deploy

**Files:** none (verification + deployment)

- [ ] **Step 1: Run full workspace tests**

```bash
cd /root/.openclaw/workspace/tensorium-core
cargo test --workspace 2>&1 | tail -15
```

Expected:
```
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

If any test fails, stop and fix before continuing.

- [ ] **Step 2: Verify txmwallet binary smoke test**

```bash
PUBKEY=020202020202020202020202020202020202020202020202020202020202020202
./target/release/txmwallet p2sh-multisig-script 2 $PUBKEY $PUBKEY $PUBKEY
```

Expected: prints `address: txms1q...` (starts with `txms`).

- [ ] **Step 3: Push to GitHub**

```bash
git push origin main
```

Expected: `main -> main` with the new commits.

- [ ] **Step 4: Deploy to DO VPS (157.230.44.162)**

SSH into DO VPS and rebuild + redeploy:

```bash
ssh root@157.230.44.162 "
  cd /root/tensorium-core &&
  git pull origin main &&
  cargo build --release -p tensorium-node -p txmwallet 2>&1 | tail -5 &&
  cp target/release/tensorium-node /usr/local/bin/tensorium-node &&
  cp target/release/txmwallet /usr/local/bin/txmwallet &&
  systemctl restart tensorium-mc &&
  sleep 3 &&
  systemctl is-active tensorium-mc &&
  curl -s http://127.0.0.1:33332 -d '{\"method\":\"getblockcount\"}' | python3 -m json.tool
"
```

Expected: `tensorium-mc` shows `active`, `getblockcount` returns a JSON with `result` ≥ current chain height.

- [ ] **Step 5: Deploy to Vultr VPS (139.180.137.144)**

```bash
ssh root@139.180.137.144 "
  cd /root/tensorium-core &&
  git pull origin main &&
  cargo build --release -p tensorium-node -p txmwallet 2>&1 | tail -5 &&
  cp target/release/tensorium-node /usr/local/bin/tensorium-node &&
  cp target/release/txmwallet /usr/local/bin/txmwallet &&
  systemctl restart tensorium-mc &&
  sleep 3 &&
  systemctl is-active tensorium-mc
"
```

Expected: `active`.

- [ ] **Step 6: Verify P2SH end-to-end on VPS (smoke test)**

On DO VPS, test that the new binary can derive a P2SH address:

```bash
ssh root@157.230.44.162 "
  PUBKEY=020202020202020202020202020202020202020202020202020202020202020202
  txmwallet p2sh-multisig-script 2 \$PUBKEY \$PUBKEY \$PUBKEY
"
```

Expected: output includes `address: txms1q...`

- [ ] **Step 7: Create GitHub release v0.3.4-mainnet**

On VPS:
```bash
ssh root@157.230.44.162 "
  cd /root/tensorium-core &&
  sha256sum target/release/tensorium-node target/release/txmwallet > /tmp/CHECKSUMS-v0.3.4-mainnet.txt &&
  cat /tmp/CHECKSUMS-v0.3.4-mainnet.txt
"
```

Then from local or via gh CLI:
```bash
gh release create v0.3.4-mainnet \
  --title "v0.3.4-mainnet — Scripting S4: P2SH-Multisig" \
  --notes "$(cat <<'EOF'
## What's New

### Scripting Layer S4 — P2SH-Multisig

- **P2SH addresses** (`txms1...`) — wrap m-of-n multisig behind a compact 23-byte script hash
- **OP_PUSHDATA1** — VM now handles data pushes up to 255 bytes (required for 2-of-3 redeem scripts)
- **txmwallet commands**: `p2sh-multisig-script`, `p2sh-multisig-spend`, `multisig-combine --redeem`

### Full Spending Flow

```
# Create P2SH address
txmwallet p2sh-multisig-script 2 <pk1> <pk2> <pk3>

# Fund address, then build unsigned spend tx
txmwallet p2sh-multisig-spend <spk_hex> <dest> <redeem_hex> <atoms>

# Each signer signs
txmwallet multisig-sign p2sh-multisig-spend-tx.json

# Combine sigs + redeem script, broadcast
txmwallet multisig-combine p2sh-multisig-spend-tx.json sig1.json sig2.json --redeem <redeem_hex>
```

### Tests

106 workspace tests, 0 failures.
EOF
)"
```

- [ ] **Step 8: Update memory**

Update the project memory file at `/root/.claude/projects/-root/memory/project_tensorium.md`:
- Mark S4 P2SH as **DONE** in the next-session priorities section
- Add S4 commit hashes and deployment confirmation

---

## Summary of New Tests

| Test | File | Validates |
|------|------|-----------|
| `is_p2sh_recognizes_pattern` | vm.rs | P2SH 23-byte pattern identified |
| `is_p2sh_rejects_non_p2sh` | vm.rs | P2PKH and short scripts rejected |
| `pushdata1_pushes_100_byte_item` | vm.rs | OP_PUSHDATA1 pushes correct data |
| `p2sh_multisig_2of3_valid` | vm.rs | Full 2-of-3 P2SH roundtrip passes |
| `p2sh_hash_mismatch_fails` | vm.rs | Wrong redeem → P2shHashMismatch |
| `p2sh_empty_stack_fails` | vm.rs | Empty scriptSig → StackUnderflow |
| `p2sh_wrong_sig_fails` | vm.rs | Correct structure, wrong sigs → false |
| `p2sh_script_roundtrip` | standard.rs | Build + extract hash matches |
| `p2sh_address_roundtrip` | standard.rs | Encode → decode → same hash20 |
| `p2sh_address_rejects_txm_prefix` | standard.rs | txm1... rejected by p2sh_hash_from_address |
| `p2sh_multisig_script_sig_layout_with_pushdata1` | standard.rs | Byte layout with PUSHDATA1 correct |
