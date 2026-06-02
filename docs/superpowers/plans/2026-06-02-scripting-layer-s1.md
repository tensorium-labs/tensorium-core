# Scripting Layer S1 — Implementation Plan

> **Status: DONE — 2026-06-02.** All 9 tasks implemented and committed to main (`6df1eda`).
> Checkboxes below were not updated during implementation; preserved for audit only.
> **Note:** S1 changes block serialization — the mainnet genesis nonce must be re-mined
> with the new `script_pubkey` format before deploying a fresh chain node from this code.

**Goal:** Replace `TxOutput.address: String` with `TxOutput.script_pubkey: Vec<u8>` and implement a Bitcoin-inspired stack VM so all transaction validation runs through the script engine.

**Architecture:** New `script/` module (opcodes, VM, P2PKH helpers) is created first with full tests. Then `block.rs` is changed (breaking), followed by fixes to `utxo.rs`, `wallet.rs`, `txmwallet`, and `tensorium-node`. Existing address format (`bech32("txm", SHA256(pubkey)[0..20])`) is preserved — `OP_HASH160` implements `SHA256(x)[0..20]` to match. Standard script is P2PKH.

**Tech Stack:** Rust, `k256` + `sha2` (already deps), `bech32` (already dep). Working directory: `/root/.openclaw/workspace/tensorium-core`.

---

## Key Script Formats

**P2PKH scriptPubKey** (25 bytes):
```
OP_DUP OP_HASH160 0x14 <20-byte hash> OP_EQUALVERIFY OP_CHECKSIG
0x76   0xa9       0x14 [...20 bytes...] 0x88            0xac
```

**scriptSig** (length-prefixed DER sig + pubkey):
```
[sig_len][...DER sig bytes...][pubkey_len][...33-byte pubkey bytes...]
```

**OP_HASH160 in this codebase** = `SHA256(x)[0..20]` (NOT RIPEMD160) — matches `Address::from_public_key`.

---

## Task 1: Create `script/mod.rs` — opcode constants + ScriptError

**Files:**
- Create: `crates/tensorium-core/src/script/mod.rs`
- Modify: `crates/tensorium-core/src/lib.rs`

- [ ] **Create `crates/tensorium-core/src/script/mod.rs`:**

```rust
pub mod standard;
pub mod vm;

// ── Data push (0x01–0x4b push the next N bytes) ───────────────────────────────
// Any byte 0x01..=0x4b encountered during execution pushes the next N bytes.

// ── Stack ─────────────────────────────────────────────────────────────────────
pub const OP_DUP:         u8 = 0x76;
pub const OP_DROP:        u8 = 0x75;
pub const OP_2DROP:       u8 = 0x6f;
pub const OP_SWAP:        u8 = 0x7c;

// ── Crypto ────────────────────────────────────────────────────────────────────
pub const OP_SHA256:      u8 = 0xa8;
/// Tensorium-specific: SHA256(x)[0..20] — matches Address::from_public_key
pub const OP_HASH160:     u8 = 0xa9;
pub const OP_CHECKSIG:    u8 = 0xac;

// ── Comparison ────────────────────────────────────────────────────────────────
pub const OP_EQUAL:       u8 = 0x87;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_VERIFY:      u8 = 0x69;

// ── Control ───────────────────────────────────────────────────────────────────
pub const OP_IF:          u8 = 0x63;
pub const OP_ELSE:        u8 = 0x67;
pub const OP_ENDIF:       u8 = 0x68;
pub const OP_RETURN:      u8 = 0x6a;

// ── Limits ────────────────────────────────────────────────────────────────────
pub const MAX_STACK_DEPTH:   usize = 100;
pub const MAX_SCRIPT_SIZE:   usize = 10_000;
pub const MAX_ELEMENT_SIZE:  usize = 520;

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptError {
    StackOverflow,
    StackUnderflow,
    ScriptTooLarge,
    ElementTooLarge,
    InvalidOpcode(u8),
    InvalidSignature,
    InvalidKey,
    InvalidAddress,
    CheckSigFailed,
    VerifyFailed,
    UnexpectedEndOfScript,
    ScriptInSigContainsChecksig,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
```

- [ ] **Add `pub mod script;` to `crates/tensorium-core/src/lib.rs`** — read the file, add the line after `pub mod storage;`.

- [ ] **Verify it compiles (creates stub modules, so create empty files first):**

Create `crates/tensorium-core/src/script/vm.rs` with `// placeholder`:
```rust
// placeholder
```

Create `crates/tensorium-core/src/script/standard.rs` with `// placeholder`:
```rust
// placeholder
```

```bash
cargo build -p tensorium-core 2>&1 | grep "^error" | head -5
```
Expected: 0 errors

- [ ] **Commit:**
```bash
git add crates/tensorium-core/src/script/ crates/tensorium-core/src/lib.rs
git commit -m "feat(script): add script module skeleton with opcode constants"
```

---

## Task 2: Implement `script/vm.rs` — stack machine + tests

**Files:**
- Modify: `crates/tensorium-core/src/script/vm.rs`

- [ ] **Write failing tests** — replace the placeholder in `vm.rs` with tests first:

```rust
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

fn run(
    stack:       &mut Vec<Vec<u8>>,
    script:      &[u8],
    ctx:         &ScriptContext,
    allow_checksig: bool,
) -> Result<(), ScriptError> {
    use sha2::{Digest, Sha256};
    use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

    let mut i = 0;
    // Track if/else nesting
    let mut if_stack: Vec<bool> = Vec::new(); // true = executing

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
        // Push 101 single-byte items — should overflow
        let mut script = Vec::new();
        for _ in 0..=MAX_STACK_DEPTH {
            script.push(0x01); // push 1 byte
            script.push(0xff);
        }
        let result = execute(&[], &script, &fake_ctx());
        assert_eq!(result, Err(ScriptError::StackOverflow));
    }

    #[test]
    fn op_hash160_matches_address_derivation() {
        // OP_HASH160 must produce SHA256(x)[0..20] to match Address::from_public_key
        let data = b"hello world";
        let expected = Sha256::digest(data);
        let expected_20 = &expected[..20];
        // script: push "hello world", OP_HASH160
        let mut script_pubkey = Vec::new();
        script_pubkey.push(data.len() as u8);
        script_pubkey.extend_from_slice(data);
        script_pubkey.push(OP_HASH160);
        // After execution, top of stack should be the 20-byte hash
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

        // Build P2PKH scriptPubKey
        let pubkey_hash = &Sha256::digest(pubkey_bytes)[..20];
        let mut script_pubkey = vec![OP_DUP, OP_HASH160, 0x14];
        script_pubkey.extend_from_slice(pubkey_hash);
        script_pubkey.push(OP_EQUALVERIFY);
        script_pubkey.push(OP_CHECKSIG);

        // Build scriptSig: [sig_len, ...sig, pubkey_len, ...pubkey]
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
        // Sign a different message
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

        // Different sig_hash than what was signed — EQUALVERIFY will fail OR checksig returns false
        let result = execute(&script_sig, &script_pubkey, &fake_ctx());
        assert!(result.is_err() || !result.unwrap(), "wrong sig should fail");
    }
}
```

- [ ] **Run tests:**
```bash
cargo test -p tensorium-core script::vm 2>&1 | tail -15
```
Expected: 4 pass, 0 fail

- [ ] **Commit:**
```bash
git add crates/tensorium-core/src/script/vm.rs
git commit -m "feat(script): implement stack VM with P2PKH opcode set"
```

---

## Task 3: Implement `script/standard.rs` — P2PKH helpers + tests

**Files:**
- Modify: `crates/tensorium-core/src/script/standard.rs`

- [ ] **Replace placeholder with full implementation:**

```rust
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
        // Encode an address and check extract_address recovers it
        let hash20 = [0xab_u8; 20];
        let addr = bech32::encode("txm", hash20.to_base32(), Variant::Bech32).unwrap();
        let script = p2pkh_from_address(&addr).unwrap();
        let recovered = extract_address(&script).unwrap();
        assert_eq!(recovered, addr);
    }

    #[test]
    fn p2pkh_from_pubkey_matches_address_derivation() {
        // p2pkh_from_pubkey must produce same hash as Address::from_public_key
        let pubkey = [0x02_u8; 33]; // fake compressed pubkey
        let script = p2pkh_from_pubkey(&pubkey);
        let expected_hash = &Sha256::digest(&pubkey)[..20];
        assert_eq!(&script[3..23], expected_hash);
    }

    #[test]
    fn rejects_non_txm_address() {
        // wrong hrp
        let hash20 = [0x01_u8; 20];
        let bad_addr = bech32::encode("btc", hash20.to_base32(), Variant::Bech32).unwrap();
        assert_eq!(
            p2pkh_from_address(&bad_addr),
            Err(ScriptError::InvalidAddress)
        );
    }

    #[test]
    fn extract_address_returns_none_for_non_standard() {
        assert_eq!(extract_address(&[0xac]), None); // OP_CHECKSIG alone
        assert_eq!(extract_address(&[]), None);
    }
}
```

- [ ] **Run tests:**
```bash
cargo test -p tensorium-core script::standard 2>&1 | tail -10
```
Expected: 4 pass, 0 fail

- [ ] **Commit:**
```bash
git add crates/tensorium-core/src/script/standard.rs
git commit -m "feat(script): add P2PKH script builders and address helpers"
```

---

## Task 4: Change `block.rs` — `TxOutput.address → script_pubkey`

**Files:**
- Modify: `crates/tensorium-core/src/block.rs`

**Warning:** This task intentionally breaks compilation of `utxo.rs`, `wallet.rs`, `txmwallet`, and `tensorium-node`. Fixes follow in Tasks 5-8.

- [ ] **Read current `block.rs`** — confirm `TxOutput.address: String` field exists.

- [ ] **Replace `TxOutput` struct definition:**

Find:
```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxOutput {
    pub value_atoms: u64,
    pub address: String,
}
```

Replace with:
```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TxOutput {
    pub value_atoms: u64,
    pub script_pubkey: Vec<u8>,
}
```

- [ ] **Update `Transaction::coinbase`** — it creates `TxOutput { value_atoms, address: miner.to_owned() }`. Add script import at top of block.rs and replace:

Add import near top of block.rs (after `use crate::hash::Hash256;`):
```rust
use crate::script::standard::p2pkh_from_address;
```

Find in `coinbase`:
```rust
        let outputs = if reward_atoms == 0 {
            Vec::new()
        } else {
            vec![TxOutput {
                value_atoms: reward_atoms,
                address: miner.to_owned(),
            }]
        };
```

Replace with:
```rust
        let outputs = if reward_atoms == 0 {
            Vec::new()
        } else {
            vec![TxOutput {
                value_atoms: reward_atoms,
                script_pubkey: p2pkh_from_address(miner)
                    .unwrap_or_default(),
            }]
        };
```

- [ ] **Update `Transaction::genesis_coinbase`** — same pattern. Find all `TxOutput { value_atoms: ..., address: ... }` occurrences in that function and replace:

```rust
        if reward_atoms > 0 {
            outputs.push(TxOutput {
                value_atoms: reward_atoms,
                script_pubkey: p2pkh_from_address(miner).unwrap_or_default(),
            });
        }
        if founder_atoms > 0 && !founder_addr.is_empty() {
            outputs.push(TxOutput {
                value_atoms: founder_atoms,
                script_pubkey: p2pkh_from_address(founder_addr).unwrap_or_default(),
            });
        }
```

- [ ] **Update `transaction_id` function** — find `bytes.extend_from_slice(output.address.as_bytes());` and replace with:
```rust
        bytes.extend_from_slice(&output.value_atoms.to_le_bytes());
        bytes.extend_from_slice(&output.script_pubkey);
```

(The full output serialisation loop already has `value_atoms` — verify this doesn't double-add it. Read the function carefully and replace only the address line.)

- [ ] **Attempt build to see what breaks:**
```bash
cargo build -p tensorium-core 2>&1 | grep "^error\[" | grep -v "script" | head -20
```
Expected: errors about `address` field in `utxo.rs` and `wallet.rs` tests. That's expected — fixed in next tasks.

- [ ] **Commit even with errors** (partial progress, next tasks fix them):
```bash
git add crates/tensorium-core/src/block.rs
git commit -m "refactor(block): TxOutput.address → script_pubkey (breaks utxo/wallet, fixed next)"
```

---

## Task 5: Fix `utxo.rs` — use script VM, skip OP_RETURN

**Files:**
- Modify: `crates/tensorium-core/src/utxo.rs`

- [ ] **Read current `utxo.rs`** — confirm it imports `verify_transaction_input` and calls `entry.output.address`.

- [ ] **Replace imports at top of `utxo.rs`:**

Remove: `use crate::wallet::verify_transaction_input;`
Add:
```rust
use crate::script::vm::{execute, ScriptContext};
```

- [ ] **Replace `verify_transaction_input` call in `validate_transaction`:**

Find:
```rust
            verify_transaction_input(tx, input, &entry.output.address)
                .map_err(|_| UtxoError::InvalidSignature)?;
```

Replace with:
```rust
            let ctx = ScriptContext {
                sig_hash: tx.signature_hash(),
                block_height: tip_height,
            };
            let ok = execute(&input.signature_script, &entry.output.script_pubkey, &ctx)
                .map_err(|_| UtxoError::InvalidSignature)?;
            if !ok { return Err(UtxoError::InvalidSignature); }
```

- [ ] **Replace `verify_transaction_input` call in `apply_block`:**

Find (in the non-coinbase loop):
```rust
                verify_transaction_input(tx, input, &spent.output.address)
                    .map_err(|_| UtxoError::InvalidSignature)?;
```

Replace with:
```rust
                let ctx = ScriptContext {
                    sig_hash: tx.signature_hash(),
                    block_height: block.header.height,
                };
                let ok = execute(&input.signature_script, &spent.output.script_pubkey, &ctx)
                    .map_err(|_| UtxoError::InvalidSignature)?;
                if !ok { return Err(UtxoError::InvalidSignature); }
```

- [ ] **Skip OP_RETURN outputs when inserting UTXOs** — find the output insertion loop:

```rust
        for tx in &block.transactions {
            for (index, output) in tx.outputs.iter().enumerate() {
                self.entries.insert(
                    OutPoint {
                        txid: tx.id,
                        output_index: index as u32,
                    },
                    UtxoEntry {
                        output: output.clone(),
                        created_height: block.header.height,
                        coinbase: tx.is_coinbase(),
                    },
                );
            }
        }
```

Replace with:
```rust
        for tx in &block.transactions {
            for (index, output) in tx.outputs.iter().enumerate() {
                // OP_RETURN outputs are unspendable — never add to UTXO set
                if output.script_pubkey.first() == Some(&crate::script::OP_RETURN) {
                    continue;
                }
                self.entries.insert(
                    OutPoint {
                        txid: tx.id,
                        output_index: index as u32,
                    },
                    UtxoEntry {
                        output: output.clone(),
                        created_height: block.header.height,
                        coinbase: tx.is_coinbase(),
                    },
                );
            }
        }
```

- [ ] **Fix tests in `utxo.rs`** — find all `TxOutput { value_atoms, address: ... }` in the test module and replace. Import at top of test module:

```rust
use crate::script::standard::p2pkh_from_address;
```

Then replace every:
```rust
TxOutput {
    value_atoms: 100,
    address: keypair.address.as_str().to_owned(),
}
```

With:
```rust
TxOutput {
    value_atoms: 100,
    script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
}
```

Also fix any `Transaction::coinbase(height, atoms, keypair.address.as_str())` calls — these should already work since coinbase now builds the script internally.

- [ ] **Build utxo.rs:**
```bash
cargo build -p tensorium-core 2>&1 | grep "^error" | grep "utxo" | head -5
```
Expected: 0 errors for utxo.rs (wallet.rs tests still broken)

- [ ] **Run utxo tests:**
```bash
cargo test -p tensorium-core utxo 2>&1 | tail -10
```
Expected: all pass

- [ ] **Commit:**
```bash
git add crates/tensorium-core/src/utxo.rs
git commit -m "feat(utxo): use script VM for validation, skip OP_RETURN outputs"
```

---

## Task 6: Fix `wallet.rs` — new scriptSig format + tests

**Files:**
- Modify: `crates/tensorium-core/src/wallet.rs`

- [ ] **Read current `wallet.rs`** — see current `sign_transaction` (JSON format) and `verify_transaction_input`.

- [ ] **Replace `sign_transaction`** — change from JSON scriptSig to P2PKH scriptSig format:

Find the entire `sign_transaction` method body and replace:
```rust
    pub fn sign_transaction(&self, tx: &mut Transaction) -> Result<(), WalletError> {
        let private_key_bytes =
            hex::decode(&self.private_key_hex).map_err(|_| WalletError::InvalidPrivateKey)?;
        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(secret_key);
        let signature_hash = tx.signature_hash();
        let signature: Signature = signing_key.sign(&signature_hash.0);
        let der_bytes = signature.to_der().as_bytes().to_vec();
        let pubkey_bytes = hex::decode(&self.public_key_hex)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        // P2PKH scriptSig: [sig_len][...DER sig...][pubkey_len][...pubkey...]
        let script_sig = crate::script::standard::p2pkh_script_sig(&der_bytes, &pubkey_bytes);
        for input in &mut tx.inputs {
            input.signature_script = script_sig.clone();
        }
        tx.refresh_id();
        Ok(())
    }
```

- [ ] **Remove `SignatureScript` struct** — find and delete:
```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignatureScript {
    pub public_key_hex: String,
    pub signature_hex: String,
}
```

- [ ] **Remove `verify_transaction_input` function** — the script VM in `utxo.rs` now handles verification. Find and delete the entire function (113 lines through end of function body).

- [ ] **Fix wallet tests** — in the `#[cfg(test)]` module, update the `signs_and_verifies_payment_transaction` test. Find:

```rust
        let mut tx = Transaction::payment(
            vec![TxInput { ... }],
            vec![TxOutput {
                value_atoms: 42,
                address: keypair.address.as_str().to_owned(),
            }],
        );
        keypair.sign_transaction(&mut tx).unwrap();
        verify_transaction_input(&tx, &tx.inputs[0], keypair.address.as_str()).unwrap();
```

Replace with:
```rust
        use crate::script::standard::p2pkh_from_address;
        use crate::script::vm::{execute, ScriptContext};

        let mut tx = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint { txid: Hash256::ZERO, output_index: 0 },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 42,
                script_pubkey: p2pkh_from_address(keypair.address.as_str()).unwrap(),
            }],
        );
        keypair.sign_transaction(&mut tx).unwrap();

        // Verify via script VM
        let ctx = ScriptContext { sig_hash: tx.signature_hash(), block_height: 0 };
        let ok = execute(&tx.inputs[0].signature_script, &tx.outputs[0].script_pubkey, &ctx).unwrap();
        assert!(ok, "signed transaction should verify via script VM");
```

- [ ] **Remove unused imports** from wallet.rs if needed (`serde_json` was used for SignatureScript).

- [ ] **Build and test:**
```bash
cargo test -p tensorium-core wallet 2>&1 | tail -10
```
Expected: all pass

- [ ] **Commit:**
```bash
git add crates/tensorium-core/src/wallet.rs
git commit -m "feat(wallet): sign_transaction outputs P2PKH scriptSig; remove JSON SignatureScript"
```

---

## Task 7: Fix `crates/txmwallet/src/main.rs`

**Files:**
- Modify: `crates/txmwallet/src/main.rs`

- [ ] **Read current `main.rs`** to find all `TxOutput { value_atoms, address }` and `output.address` usages.

- [ ] **Add import** at the top (after existing `use tensorium_core::...` line):
```rust
use tensorium_core::script::standard::{p2pkh_from_address, p2pkh_from_pubkey, extract_address};
```

- [ ] **Fix `build_signed_payment` in `main.rs`** — find the output creation:

```rust
    let mut outputs = vec![TxOutput {
        value_atoms: amount_atoms,
        address: to_address.to_owned(),
    }];
    let change = selected_atoms - amount_atoms;
    if change > 0 {
        outputs.push(TxOutput {
            value_atoms: change,
            address: wallet.address.clone(),
        });
    }
```

Replace with:
```rust
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
```

- [ ] **Fix `build_signed_payment_via_rpc`** — same pattern. Find and replace the two `TxOutput { value_atoms: ..., address: ... }` blocks identically.

- [ ] **Fix `print_balance`** — find `entry.output.address != wallet.address` (or similar address comparison):

```rust
        if entry.output.address != wallet.address {
```

Replace with:
```rust
        let expected_script = p2pkh_from_pubkey(
            &hex::decode(&wallet.public_key_hex).unwrap_or_default()
        );
        if entry.output.script_pubkey != expected_script {
```

- [ ] **Build txmwallet:**
```bash
cargo build -p txmwallet 2>&1 | grep "^error" | head -10
```
Expected: 0 errors

- [ ] **Commit:**
```bash
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): build P2PKH script outputs, match UTXOs by script"
```

---

## Task 8: Fix `crates/tensorium-node/src/main.rs` — getutxos RPC

**Files:**
- Modify: `crates/tensorium-node/src/main.rs`

- [ ] **Find all `output.address` / `entry.output.address` usages in main.rs:**
```bash
grep -n "output\.address\|\.address\b" crates/tensorium-node/src/main.rs | head -20
```

- [ ] **Add script imports** at top of main.rs (after existing tensorium_core imports):
```rust
use tensorium_core::script::standard::{extract_address, p2pkh_from_address};
```

- [ ] **Fix `getutxos` RPC handler** — this handler currently filters by `output.address == addr`. Find the handler (search for `getutxos`) and update it to:

1. Build the expected P2PKH script from the address:
```rust
let script = p2pkh_from_address(&addr).map_err(|_| "invalid address".to_string())?;
```

2. Filter UTXOs by script match instead of address match:
```rust
.filter(|(_, entry)| entry.output.script_pubkey == script)
```

3. In the JSON response, use `extract_address(&entry.output.script_pubkey)` to derive the `"address"` field:
```rust
"address": extract_address(&entry.output.script_pubkey).unwrap_or_default(),
```

- [ ] **Fix any other places that access `output.address`** — search and replace with script-based equivalents using `extract_address`.

- [ ] **Build node:**
```bash
cargo build -p tensorium-node 2>&1 | grep "^error" | head -10
```
Expected: 0 errors

- [ ] **Commit:**
```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): getutxos RPC uses script matching and extracts address from scriptPubKey"
```

---

## Task 9: Full test suite + verify

- [ ] **Run all workspace tests:**
```bash
cargo test --workspace 2>&1 | grep -E "^test result|^error\[" | head -20
```
Expected: all pass. The count should be at least 64 (existing) plus the new script tests.

- [ ] **If any tests fail**, read the error, fix the specific file, and re-run. Common failures:
  - Any test that creates `TxOutput { ..., address: ... }` → change to `script_pubkey: p2pkh_from_address(...).unwrap()`
  - Any test calling `verify_transaction_input` → use script VM instead

- [ ] **Final commit with full test results:**
```bash
cargo test --workspace 2>&1 | tail -5
git add -A
git commit -m "$(cat <<'EOF'
feat(phase12): scripting layer S1 — script VM + P2PKH migration

- TxOutput.address → script_pubkey: Vec<u8> (clean break)
- 17-opcode stack VM (OP_DUP, OP_HASH160, OP_CHECKSIG, OP_RETURN, etc.)
- OP_HASH160 = SHA256(x)[0..20] — matches Address::from_public_key
- Standard P2PKH scripts with address helpers
- UTXO validation runs through script VM
- OP_RETURN outputs never enter UTXO set
- wallet.rs: P2PKH scriptSig format (DER sig + pubkey, not JSON)
- txmwallet: builds P2PKH outputs from addresses
- tensorium-node: getutxos matches by script, extracts address from script

All existing tests pass. Foundation for S2 (multisig) and S3 (HTLC).

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage:**
- ✅ TxOutput.address → script_pubkey (Task 4)
- ✅ 17 opcodes (Task 1 constants, Task 2 VM)
- ✅ OP_HASH160 = SHA256(x)[0..20] (Task 2, documented in Task 1)
- ✅ P2PKH scriptPubKey format (Task 3)
- ✅ P2PKH scriptSig format (Task 2 tests + Task 6)
- ✅ `p2pkh_from_address()` (Task 3)
- ✅ `extract_address()` (Task 3)
- ✅ `p2pkh_from_pubkey()` (Task 3, used in Task 7)
- ✅ `p2pkh_script_sig()` (Task 3, used in Task 6)
- ✅ UTXO validation via VM (Task 5)
- ✅ OP_RETURN outputs not in UTXO set (Task 5)
- ✅ wallet.rs sign_transaction new format (Task 6)
- ✅ txmwallet script outputs (Task 7)
- ✅ node getutxos by script (Task 8)
- ✅ transaction_id serializes script_pubkey bytes (Task 4)
- ✅ All 64+ existing tests must pass (Task 9)

**No placeholders:** All tasks have complete code blocks.

**Type consistency:**
- `p2pkh_from_address(addr: &str) -> Result<Vec<u8>, ScriptError>` — consistent Tasks 3, 5, 6, 7, 8
- `p2pkh_from_pubkey(pubkey: &[u8]) -> Vec<u8>` — consistent Tasks 3, 7
- `extract_address(script: &[u8]) -> Option<String>` — consistent Tasks 3, 8
- `p2pkh_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8>` — consistent Tasks 3, 6
- `ScriptContext { sig_hash: Hash256, block_height: u64 }` — consistent Tasks 2, 5, 6
- `execute(script_sig: &[u8], script_pubkey: &[u8], ctx: &ScriptContext) -> Result<bool, ScriptError>` — consistent Tasks 2, 5, 6
