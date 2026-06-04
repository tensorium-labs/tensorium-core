# Scripting Layer S3 — CLTV + HTLC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add absolute block-height timelocks (`OP_CHECKLOCKTIMEVERIFY`) and HTLC scripts to the Tensorium VM, plus wallet tooling for hash-locked claim/refund spends, enabling trustless cross-chain atomic swaps.

**Architecture:** Purely additive on the existing script VM (S1 P2PKH + S2 multisig). `OP_CLTV` reads `ctx.block_height` directly — no new `Transaction` fields, no consensus change, no chain reset. HTLC is composed from existing opcodes (`OP_IF/ELSE`, `OP_SHA256`, `OP_HASH160`, `OP_CHECKSIG`) plus the two new opcodes. Wallet commands reuse the S2 `/getutxos/<hex>` endpoint.

**Tech Stack:** Rust (workspace crates `tensorium-core`, `txmwallet`), `k256` ECDSA, `sha2`, `bech32`, `hex`.

**Reference spec:** `docs/superpowers/specs/2026-06-04-scripting-layer-s3-design.md`

**Branch:** `scripting-s3` (already created)

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `crates/tensorium-core/src/script/mod.rs` | Opcode constants + `ScriptError` | Add `OP_0`, `OP_CHECKLOCKTIMEVERIFY`, `LockTimeNotMet` |
| `crates/tensorium-core/src/script/vm.rs` | Stack-machine execution + tests | Execute the two opcodes; add 4 unit tests |
| `crates/tensorium-core/src/script/standard.rs` | Script builders/parsers + tests | Add HTLC builders + `extract_htlc`; add 6 tests |
| `crates/txmwallet/src/main.rs` | Wallet CLI | Add `htlc-secret`, `htlc-script`, `htlc-claim`, `htlc-refund` + helpers |
| `crates/txmwallet/Cargo.toml` | Wallet deps | Ensure `sha2` dependency present |
| `docs/integrations/ATOMIC_SWAP_HTLC.md` | Integration guide | New file |

---

## Task 1: VM opcodes — OP_0 + OP_CHECKLOCKTIMEVERIFY

**Files:**
- Modify: `crates/tensorium-core/src/script/mod.rs`
- Modify: `crates/tensorium-core/src/script/vm.rs`
- Test: `crates/tensorium-core/src/script/vm.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Add opcode constants and error variant**

In `crates/tensorium-core/src/script/mod.rs`, add the `OP_0` constant just above the `OP_DUP` stack section:

```rust
// ── Push false / zero ─────────────────────────────────────────────────────────
pub const OP_0:           u8 = 0x00;
```

Add the CLTV constant in the Control section (after `OP_RETURN`):

```rust
// ── Timelock ──────────────────────────────────────────────────────────────────
/// Absolute timelock: fails the script unless ctx.block_height >= top-of-stack value.
pub const OP_CHECKLOCKTIMEVERIFY: u8 = 0xb1;
```

Add the error variant to the `ScriptError` enum (after `ScriptInSigContainsChecksig`):

```rust
    LockTimeNotMet,
```

- [ ] **Step 2: Write the failing tests**

In the `#[cfg(test)] mod tests` of `crates/tensorium-core/src/script/vm.rs`, add these four tests at the end of the module (before the closing `}`):

```rust
    #[test]
    fn op_0_pushes_empty() {
        use crate::script::OP_0;
        let mut stack: Vec<Vec<u8>> = Vec::new();
        run(&mut stack, &[OP_0], &fake_ctx(), false).unwrap();
        assert_eq!(stack, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn cltv_passes_when_height_ge_locktime() {
        use crate::script::{OP_CHECKLOCKTIMEVERIFY, OP_DROP, OP_1};
        // push 100 (0x64), CLTV, DROP, OP_1(true)
        let script = [0x01, 0x64, OP_CHECKLOCKTIMEVERIFY, OP_DROP, OP_1];
        let ctx = ScriptContext { sig_hash: Hash256::ZERO, block_height: 100 };
        let ok = execute(&[], &script, &ctx).unwrap();
        assert!(ok, "height == locktime should pass");
    }

    #[test]
    fn cltv_fails_below_locktime() {
        use crate::script::OP_CHECKLOCKTIMEVERIFY;
        let script = [0x01, 0x64, OP_CHECKLOCKTIMEVERIFY]; // locktime 100
        let ctx = ScriptContext { sig_hash: Hash256::ZERO, block_height: 99 };
        assert_eq!(execute(&[], &script, &ctx), Err(ScriptError::LockTimeNotMet));
    }

    #[test]
    fn cltv_leaves_value_on_stack() {
        use crate::script::OP_CHECKLOCKTIMEVERIFY;
        let mut stack: Vec<Vec<u8>> = vec![vec![0x64u8]]; // locktime 100 already on stack
        let ctx = ScriptContext { sig_hash: Hash256::ZERO, block_height: 200 };
        run(&mut stack, &[OP_CHECKLOCKTIMEVERIFY], &ctx, true).unwrap();
        assert_eq!(stack, vec![vec![0x64u8]], "CLTV must not pop its operand");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tensorium-core script::vm::tests::cltv 2>&1 | tail -20`
Expected: compile error (`OP_CHECKLOCKTIMEVERIFY` execution not handled → `InvalidOpcode`) or test failures.

- [ ] **Step 4: Implement the opcodes in vm.rs**

In `crates/tensorium-core/src/script/vm.rs`, inside the main `match op { ... }` (the one starting `OP_RETURN => ...`), add these two arms just before the final `other => return Err(ScriptError::InvalidOpcode(other)),`:

```rust
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
```

Note: `OP_0` is `0x00`, which is outside the `0x01..=0x4b` data-push range and not a
control op, so it correctly falls through to this match and only pushes when executing.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tensorium-core script::vm::tests 2>&1 | tail -20`
Expected: all `vm::tests` pass, including the 4 new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-core/src/script/mod.rs crates/tensorium-core/src/script/vm.rs
git commit -m "feat(s3): OP_0 and OP_CHECKLOCKTIMEVERIFY opcodes

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: HTLC builders + extract_htlc

**Files:**
- Modify: `crates/tensorium-core/src/script/standard.rs`
- Test: `crates/tensorium-core/src/script/standard.rs` (inline test module)

- [ ] **Step 1: Add imports**

At the top of `crates/tensorium-core/src/script/standard.rs`, extend the `use crate::script::{...}` line to include the new opcodes:

```rust
use crate::script::{
    ScriptError, OP_CHECKSIG, OP_CHECKMULTISIG, OP_DUP, OP_EQUALVERIFY, OP_HASH160, OP_1,
    OP_0, OP_IF, OP_ELSE, OP_ENDIF, OP_SHA256, OP_DROP, OP_CHECKLOCKTIMEVERIFY,
};
```

- [ ] **Step 2: Write the failing tests**

In the `#[cfg(test)] mod tests` of `standard.rs`, add these six tests at the end (before the closing `}`):

```rust
    fn htlc_test_keypair() -> (k256::ecdsa::SigningKey, Vec<u8>, [u8; 20]) {
        use k256::ecdsa::SigningKey;
        use rand_core::OsRng;
        let sk = SigningKey::random(&mut OsRng);
        let pubkey = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
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

        let ctx = ScriptContext { sig_hash: msg, block_height: 0 };
        assert!(execute(&script_sig, &spk, &ctx).unwrap(), "valid claim must succeed");
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

        let ctx = ScriptContext { sig_hash: msg, block_height: 0 };
        let result = execute(&script_sig, &spk, &ctx);
        assert!(result.is_err() || !result.unwrap(), "wrong preimage must fail");
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

        let ctx = ScriptContext { sig_hash: verify_msg, block_height: 0 };
        let result = execute(&script_sig, &spk, &ctx);
        assert!(result.is_err() || !result.unwrap(), "wrong signature must fail");
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

        let ctx = ScriptContext { sig_hash: msg, block_height: 150 };
        assert!(execute(&script_sig, &spk, &ctx).unwrap(), "refund after locktime must succeed");
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

        let ctx = ScriptContext { sig_hash: msg, block_height: 99 };
        assert_eq!(execute(&script_sig, &spk, &ctx), Err(ScriptError::LockTimeNotMet));
    }
```

The test module already imports `Sha256`/`Digest` via `use super::*;`? Verify: `standard.rs`
top has `use sha2::{Digest, Sha256};`, and the test module has `use super::*;`, so `Sha256`
and `Digest` are in scope. `to_encoded_point` requires the `k256` elliptic-curve trait; it is
already used the same way in `vm.rs` tests, so no extra import beyond `k256::ecdsa`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tensorium-core script::standard::tests::htlc 2>&1 | tail -20`
Expected: compile error — `htlc_script`, `htlc_claim_script_sig`, `htlc_refund_script_sig`, `extract_htlc` not found.

- [ ] **Step 4: Implement the builders**

In `crates/tensorium-core/src/script/standard.rs`, add after `extract_multisig` (before the `#[cfg(test)]` module):

```rust
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
    let mut s = Vec::with_capacity(70 + lt.len());
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tensorium-core script::standard::tests 2>&1 | tail -20`
Expected: all `standard::tests` pass, including the 6 new HTLC tests.

- [ ] **Step 6: Commit**

```bash
git add crates/tensorium-core/src/script/standard.rs
git commit -m "feat(s3): HTLC script builders and extract_htlc

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Wallet CLI — htlc-secret + htlc-script

**Files:**
- Modify: `crates/txmwallet/Cargo.toml`
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Ensure sha2 dependency**

Check `crates/txmwallet/Cargo.toml`:

```bash
grep -n 'sha2' crates/txmwallet/Cargo.toml
```

If absent, add under `[dependencies]` (match the workspace version used by `tensorium-core`):

```toml
sha2 = "0.10"
```

- [ ] **Step 2: Add imports and helper**

In `crates/txmwallet/src/main.rs`, extend the `tensorium_core::script::standard` import to add the HTLC builders:

```rust
    script::standard::{multisig_script, multisig_script_sig, extract_multisig,
                       p2pkh_from_address, p2pkh_from_pubkey,
                       htlc_script, htlc_claim_script_sig, htlc_refund_script_sig},
```

Add `use sha2::{Digest, Sha256};` to the top `use` block.

Add this helper near `build_unsigned_multisig_tx` (e.g. just below it):

```rust
/// Decode a txm1 bech32 address to its 20-byte pubkey hash by reusing the P2PKH builder.
fn address_to_hash20(addr: &str) -> Result<[u8; 20], String> {
    let script = p2pkh_from_address(addr).map_err(|_| format!("invalid address: {addr}"))?;
    // P2PKH layout: OP_DUP OP_HASH160 0x14 <hash20> OP_EQUALVERIFY OP_CHECKSIG
    let mut h = [0u8; 20];
    h.copy_from_slice(&script[3..23]);
    Ok(h)
}
```

- [ ] **Step 3: Add the two subcommands**

In the `match command { ... }` block of `run()`, add these arms after the `"multisig-combine" => { ... }` arm:

```rust
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
            println!("fund it by sending TXM to this scriptpubkey (send-from-script or a script output)");
        }
```

- [ ] **Step 4: Build to verify it compiles**

Run: `cargo build -p txmwallet 2>&1 | tail -20`
Expected: builds with no errors.

- [ ] **Step 5: Smoke-test the commands**

Run:
```bash
cargo run -q -p txmwallet -- htlc-secret
```
Expected: prints `preimage:` and `sha256:` hex lines (64 hex chars each).

- [ ] **Step 6: Commit**

```bash
git add crates/txmwallet/Cargo.toml crates/txmwallet/src/main.rs
git commit -m "feat(s3): txmwallet htlc-secret and htlc-script commands

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Wallet CLI — htlc-claim + htlc-refund

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Step 1: Add the spend builder helper**

In `crates/txmwallet/src/main.rs`, add below `build_unsigned_multisig_tx`:

```rust
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
    let hash = Hash256(
        u.txid_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "invalid txid from RPC".to_owned())?,
    );
    let input = TxInput {
        previous_output: OutPoint { txid: hash, output_index: u.output_index },
        signature_script: Vec::new(),
    };
    let dest_script = p2pkh_from_address(dest_addr)
        .map_err(|_| format!("invalid destination address: {dest_addr}"))?;
    let outputs = vec![TxOutput { value_atoms: u.value_atoms, script_pubkey: dest_script }];
    Ok(Transaction::payment(vec![input], outputs))
}
```

- [ ] **Step 2: Add the two subcommands**

In the `match command { ... }`, add after the `"htlc-script" => { ... }` arm:

```rust
        "htlc-claim" => {
            let usage =
                "usage: txmwallet htlc-claim <spk_hex> <dest_addr> <preimage_hex> [rpc]";
            let spk_hex = args.get(2).ok_or(usage)?;
            let dest_addr = args.get(3).ok_or(usage)?;
            let preimage_hex = args.get(4).ok_or(usage)?;
            let rpc = args.get(5).map(String::as_str).unwrap_or(DEFAULT_RPC);

            let preimage = hex::decode(preimage_hex).map_err(|_| "invalid preimage hex".to_owned())?;
            let passphrase = passphrase_from_env()?;
            let wallet = load_wallet(&wallet_path)?;
            let keypair = wallet.decrypt(&passphrase)?;

            let mut tx = build_unsigned_htlc_spend(rpc, spk_hex, dest_addr)?;
            let sig_hash = tx.signature_hash();
            let der_sig = keypair.sign_hash(&sig_hash).map_err(|e| format!("sign: {e:?}"))?;
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
            let der_sig = keypair.sign_hash(&sig_hash).map_err(|e| format!("sign: {e:?}"))?;
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
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p txmwallet 2>&1 | tail -20`
Expected: builds with no errors (warnings tolerated).

- [ ] **Step 4: Commit**

```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(s3): txmwallet htlc-claim and htlc-refund commands

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Help text, atomic-swap guide, full verification

**Files:**
- Modify: `crates/txmwallet/src/main.rs` (`print_help`)
- Create: `docs/integrations/ATOMIC_SWAP_HTLC.md`

- [ ] **Step 1: Update print_help**

Find the `fn print_help()` in `crates/txmwallet/src/main.rs` and add these lines to the
printed command list (alongside the existing `multisig-*` / `send-from-script` lines), matching
the surrounding `println!` style:

```rust
    println!("  htlc-secret                                            generate a 32-byte preimage + its sha256 hash");
    println!("  htlc-script <hash_hex> <recipient_addr> <refund_addr> <locktime_height>");
    println!("  htlc-claim <spk_hex> <dest_addr> <preimage_hex> [rpc]  spend HTLC via preimage (claim branch)");
    println!("  htlc-refund <spk_hex> <dest_addr> [rpc]                spend HTLC after locktime (refund branch)");
```

- [ ] **Step 2: Write the atomic-swap guide**

Create `docs/integrations/ATOMIC_SWAP_HTLC.md`:

````markdown
# Atomic Swaps with Tensorium HTLC

Tensorium's script VM (S3) supports Hash Time Locked Contracts (HTLC), the
primitive behind trustless cross-chain atomic swaps. This guide walks through a
TXM ⇄ wTXM (Optimism) swap. No trusted third party is involved.

## The HTLC primitive

An HTLC output can be spent two ways:

- **Claim** — anyone who knows the secret `preimage` (where `SHA256(preimage)`
  equals the hashlock) AND holds the recipient key can spend it immediately.
- **Refund** — after a block-height deadline (`locktime`), the original sender
  can reclaim the funds with the refund key.

The hashlock uses **SHA256**, which also exists as an EVM precompile, so the same
secret unlocks both sides of a cross-chain swap.

## Roles

- **Alice** holds TXM, wants wTXM.
- **Bob** holds wTXM (Optimism), wants TXM.

## Steps

1. **Alice generates the secret.**
   ```
   txmwallet htlc-secret
   # preimage: <64 hex>   (Alice keeps this private)
   # sha256:   <64 hex>   (Alice shares this hash with Bob)
   ```

2. **Alice locks TXM on Tensorium.** Recipient = Bob, refund = Alice,
   `locktime = H1` (a Tensorium block height comfortably in the future).
   ```
   txmwallet htlc-script <sha256> <bob_txm_addr> <alice_txm_addr> H1
   # scriptpubkey: <hex>
   ```
   Alice funds the printed scriptpubkey with the swap amount of TXM.

3. **Bob locks wTXM on Optimism** in an EVM HTLC using the **same** `sha256`
   hashlock, recipient = Alice, refund = Bob, with an EVM timeout **earlier** than
   H1 in wall-clock terms (see the safety note).

4. **Alice claims the wTXM** on Optimism by revealing `preimage`. This publishes
   the preimage on the EVM chain.

5. **Bob reads the preimage** from Alice's Optimism claim and uses it to claim the
   TXM:
   ```
   txmwallet htlc-claim <tensorium_spk_hex> <bob_txm_addr> <preimage> <rpc>
   txmwallet broadcast htlc-claim-tx.json <rpc>
   ```

If the swap is abandoned, each party reclaims their own funds after their
respective timeout (Alice via `txmwallet htlc-refund` once Tensorium height ≥ H1).

## Safety: order the timeouts correctly

Alice's TXM refund deadline (H1) **must be later** than Bob's wTXM timeout. Bob
must be able to claim TXM (after learning the preimage) before Alice can refund it.
A common rule of thumb is H1 ≈ 2× Bob's timeout.

## Height ↔ time conversion

Tensorium targets ≈ **132 seconds per block**.

| Duration | ~Blocks |
|----------|---------|
| 1 hour   | ~27     |
| 6 hours  | ~164    |
| 24 hours | ~655    |
| 48 hours | ~1310   |

Pick H1 (TXM) and Bob's EVM timeout so the TXM refund window is strictly the
longer of the two.

## Limitations

- HTLC enforces the hashlock and timelock; it does not enforce that both legs of a
  swap actually exist. Each party must verify the counterparty's on-chain lock
  before revealing or committing further.
- Timelocks are **absolute block heights** (no relative `OP_CSV` in S3).
- Spend is single-UTXO, full-value (fund one HTLC output per swap leg).
````

- [ ] **Step 3: Run cargo fmt**

Run: `cargo fmt`
Expected: no errors. (Reformats any spacing in the new code.)

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -25`
Expected: all tests pass (existing suite + 4 new vm tests + 7 new standard tests = 11 new), 0 failures.

- [ ] **Step 5: Verify no warnings in touched crates**

Run: `cargo build --workspace 2>&1 | grep -E "warning|error" | head`
Expected: no new warnings from the S3 changes (pre-existing warnings elsewhere are out of scope).

- [ ] **Step 6: Commit**

```bash
git add crates/txmwallet/src/main.rs docs/integrations/ATOMIC_SWAP_HTLC.md
git commit -m "docs(s3): atomic swap HTLC guide + wallet help

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Done When

- `cargo test --workspace` passes with 11 new S3 tests, 0 failures
- `txmwallet htlc-secret`, `htlc-script`, `htlc-claim`, `htlc-refund` all build and run
- `docs/integrations/ATOMIC_SWAP_HTLC.md` exists
- Branch `scripting-s3` holds the commits; ready for VPS verification + merge per the deploy workflow

## Notes for the Implementer

- **TDD order matters:** write the test, watch it fail, then implement. Do not implement ahead of the test.
- **No consensus change:** do not add fields to `Transaction`/`TxInput`, do not touch `utxo.rs`, `state.rs`, `chain.rs`, or `tensorium-node`. CLTV reads the `block_height` already threaded into `ScriptContext` by `utxo.rs`.
- **Stack-order reasoning** for HTLC is in the spec; if a script test fails, trace the stack op-by-op rather than guessing.
- **VPS verification** (per project workflow): after local green, sync to VPS `157.230.44.162`, run `cargo fmt` + `cargo test --workspace`, then push to `tensorium-labs`. The MC chain need NOT be reset — S3 is additive.
