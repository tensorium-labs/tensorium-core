# Scripting Layer S1 — Design Spec

**Date:** 2026-06-02
**Status:** Approved
**Scope:** `crates/tensorium-core` + `crates/txmwallet`
**Sequence:** S1 of 3 (S2 = Multisig, S3 = Timelock + HTLC)

---

## Goal

Replace the plain `address: String` field in `TxOutput` with a locking script (`script_pubkey: Vec<u8>`), implement a minimal Bitcoin-inspired stack VM, and migrate all existing transaction paths to use P2PK scripts. This is the foundation for multisig (S2) and HTLC/atomic swap (S3).

The mainnet-candidate chain was still at effectively fresh state when this design was proposed, so it was the ideal moment for a clean break.

---

## Transaction Model Change

```rust
// BEFORE
pub struct TxOutput {
    pub value_atoms: u64,
    pub address: String,
}

// AFTER
pub struct TxOutput {
    pub value_atoms: u64,
    pub script_pubkey: Vec<u8>,
}
```

`TxInput.signature_script: Vec<u8>` type is unchanged. Content changes from JSON `{ pubkey, sig }` to raw concatenated bytes.

Transaction payload version marker: `payment:v2` for new-format transactions (P2PK scriptPubKey). `payment:v1` is retired with this change since the field type changes are breaking.

---

## Opcode Set (Phase S1 — 17 opcodes)

```
Data push (1 byte length prefix + data):
  0x01–0x4b  OP_PUSHDATA(n)   push next n bytes onto stack

Stack manipulation:
  0x76  OP_DUP           duplicate top item
  0x75  OP_DROP          discard top item
  0x6f  OP_2DROP         discard top two items
  0x7c  OP_SWAP          swap top two items

Crypto:
  0xa8  OP_SHA256        SHA256 of top item
  0xa9  OP_HASH160       RIPEMD160(SHA256(top))
  0xac  OP_CHECKSIG      pop pubkey, pop sig, verify ECDSA → push [0x01] or []

Comparison:
  0x87  OP_EQUAL         pop two, push [0x01] if equal else []
  0x88  OP_EQUALVERIFY   OP_EQUAL + OP_VERIFY
  0x69  OP_VERIFY        fail if top is falsy; pop it

Control:
  0x63  OP_IF            if top is truthy, execute if-branch
  0x67  OP_ELSE          negate branch
  0x68  OP_ENDIF         end if/else block
  0x6a  OP_RETURN        mark output unspendable (UTXO never created)
```

Phase S2 adds `OP_1–OP_16` (0x51–0x60) and `OP_CHECKMULTISIG` (0xae).
Phase S3 adds `OP_CHECKLOCKTIMEVERIFY` (0xb1), `OP_CHECKSEQUENCEVERIFY` (0xb2).

---

## Standard Script Templates

### P2PK (Pay-to-Public-Key)

Current wallet behaviour expressed as scripts:

```
scriptPubKey: <33-byte compressed pubkey> OP_CHECKSIG
scriptSig:    <DER signature bytes>
```

Execution: push sig → push pubkey → OP_CHECKSIG → [0x01] on stack → script succeeds.

### P2PKH (Pay-to-Public-Key-Hash)

Shorter output scripts using 20-byte hash:

```
scriptPubKey: OP_DUP OP_HASH160 <20-byte pubkey hash> OP_EQUALVERIFY OP_CHECKSIG
scriptSig:    <DER signature> <33-byte pubkey>
```

P2PKH is recognized and displayable but wallet defaults to P2PK for S1.

---

## Script VM

### Interface

```rust
// crates/tensorium-core/src/script/vm.rs

pub struct ScriptContext {
    pub sig_hash:     Hash256,   // tx signature hash for OP_CHECKSIG
    pub block_height: u64,       // tip height (for OP_CLTV in S3, unused in S1)
}

pub fn execute(
    script_sig:    &[u8],
    script_pubkey: &[u8],
    ctx:           &ScriptContext,
) -> Result<bool, ScriptError>
```

### Execution rules

1. Run `script_sig` first (OP_CHECKSIG not allowed in scriptSig — push-only rule)
2. Run `script_pubkey` against the resulting stack
3. Success = top stack item is truthy (non-empty, any non-zero byte)
4. Failure = script returned false, stack empty, or `ScriptError` propagated

### Limits (DoS prevention)

| Limit | Value |
|---|---|
| Max stack depth | 100 items |
| Max script size | 10,000 bytes |
| Max element size on stack | 520 bytes |

### OP_CHECKSIG detail

1. Pop pubkey bytes from stack
2. Pop signature bytes (DER-encoded) from stack
3. Verify ECDSA signature of `ctx.sig_hash` against pubkey using secp256k1
4. Push `[0x01]` on success, `[]` on failure (does not error — allows `OP_IF` branching)

### OP_RETURN

Immediate halt. Output is unspendable — `UtxoSet::apply_block` never inserts an OP_RETURN output as a UTXO entry. Data after OP_RETURN in the scriptPubKey is ignored.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `crates/tensorium-core/src/script/mod.rs` | Create | Opcode constants, `ScriptError` enum, re-exports |
| `crates/tensorium-core/src/script/vm.rs` | Create | `execute()`, `ScriptContext`, stack machine loop |
| `crates/tensorium-core/src/script/standard.rs` | Create | `p2pk(pubkey)`, `p2pkh(pubkey_hash)`, `p2pk_from_address(addr)`, `extract_address(script) -> Option<String>` |
| `crates/tensorium-core/src/lib.rs` | Modify | Add `pub mod script;` |
| `crates/tensorium-core/src/block.rs` | Modify | `TxOutput.address → script_pubkey: Vec<u8>`; coinbase/payment constructors build P2PK scripts; `transaction_id` serializes `script_pubkey` bytes |
| `crates/tensorium-core/src/utxo.rs` | Modify | Replace JSON sig-verify with `script::vm::execute()`; skip UTXO creation for OP_RETURN outputs |
| `crates/tensorium-core/src/wallet.rs` | Modify | `sign_transaction` writes raw DER bytes to `signature_script`; add `p2pk_script_pubkey()` helper |
| `crates/txmwallet/src/main.rs` | Modify | Build `TxOutput { script_pubkey }` using `p2pk_from_address()`; `print_balance` reads `script_pubkey` for UTXO matching |
| `crates/tensorium-node/src/main.rs` | Modify | `getutxos` response derives `address` field from script via `extract_address()` for backward-compatible JSON |

---

## UTXO Validation Change

```rust
// BEFORE (utxo.rs)
fn verify_input(input, utxo, tip_height, params) {
    let script: SignatureScript = serde_json::from_slice(&input.signature_script)?;
    // verify single ECDSA sig against utxo.output.address pubkey
}

// AFTER
fn verify_input(input, utxo, sig_hash, tip_height) {
    let ctx = ScriptContext { sig_hash, block_height: tip_height };
    let ok = script::vm::execute(
        &input.signature_script,
        &utxo.output.script_pubkey,
        &ctx,
    )?;
    if !ok { return Err(UtxoError::InvalidSignature); }
}
```

Coinbase inputs have `inputs: []` so `verify_input` is never called for them.

---

## Wallet Sign Path

```rust
// sign_transaction: raw DER bytes per input (was JSON)
for input in &mut tx.inputs {
    input.signature_script = der_sig_bytes.clone();
}

// build_signed_payment_via_rpc: outputs use scripts
TxOutput {
    value_atoms: amount_atoms,
    script_pubkey: script::standard::p2pk_from_address(to_address)?,
}
```

### `p2pk_from_address(addr: &str) -> Result<Vec<u8>, String>`

1. Decode bech32 `txm1...` address → 33-byte compressed pubkey bytes
2. Return `[pubkey_len, ...pubkey_bytes, OP_CHECKSIG]`

### `extract_address(script: &[u8]) -> Option<String>`

Recognizes P2PK and P2PKH patterns, derives bech32 address for display in RPC/explorer.

---

## RPC Backward Compatibility

`/getutxos/<address>` currently returns:
```json
{ "txid": "...", "address": "txm1...", "value_atoms": N, ... }
```

After this change: `utxo.output.script_pubkey` is the canonical field. The `address` in the JSON response is derived by `extract_address(script_pubkey)`. If the script is non-standard, `address` is omitted and `script_pubkey_hex` is included instead.

---

## Tests

All tests in `crates/tensorium-core`. New file: `src/script/tests.rs`.

| Test | What |
|---|---|
| `op_checksig_valid_p2pk` | Valid DER sig + matching pubkey → `execute()` returns `Ok(true)` |
| `op_checksig_invalid_sig` | Wrong sig → `Ok(false)` (not error) |
| `op_checksig_wrong_pubkey` | Right sig, wrong pubkey → `Ok(false)` |
| `p2pkh_valid` | Full `OP_DUP OP_HASH160 ... OP_EQUALVERIFY OP_CHECKSIG` execution |
| `op_return_unspendable` | `execute([OP_RETURN ...], [])` returns `Ok(false)` |
| `stack_depth_limit` | Script that pushes 101 items → `Err(ScriptError::StackOverflow)` |
| `script_too_large` | 10001-byte script → `Err(ScriptError::ScriptTooLarge)` |
| `p2pk_roundtrip` | `p2pk(pubkey)` → execute with matching sig → `Ok(true)` |
| `p2pk_from_address_roundtrip` | encode address → decode → P2PK script → verify |
| `payment_v2_block_validates` | Mine a block with `payment:v2` transactions; `apply_block` succeeds |
| `existing_64_tests_unchanged` | `cargo test --workspace` — all must pass |

---

## What is NOT in S1

- Multisig (`OP_CHECKMULTISIG`) — S2
- Timelock (`OP_CHECKLOCKTIMEVERIFY`) — S3
- HTLC / atomic swap — S3
- Script address format change (bech32 stays as-is, addresses derived from P2PK scripts)
- Witness / SegWit-style separation
- Script descriptor language
