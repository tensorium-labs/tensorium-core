# Scripting Layer S4 — P2SH-Multisig Design

**Date:** 2026-06-05  
**Status:** Approved  
**Scope:** P2SH-multisig only (wrapping bare multisig scripts into Pay-to-Script-Hash)

---

## Background

S1 (P2PKH), S2 (bare multisig), and S3 (CLTV + HTLC) are deployed on mainnet (commit `840c998`).
S4 adds Pay-to-Script-Hash (P2SH) wrapping for multisig scripts.

**Why P2SH-multisig:** Bare multisig scriptPubKeys embed all public keys on-chain at locking time,
making the address long and revealing the participants before any spend. P2SH hides the redeem
script behind a 20-byte hash — the on-chain footprint at lock time is identical to P2PKH (23 bytes).
The redeem script is only revealed at spend time.

---

## Address Format

| Type | HRP | Example |
|------|-----|---------|
| P2PKH | `txm` | `txm1qy3kw8n...` |
| P2SH | `txms` | `txms1qxy7p2r...` |

Both use bech32 encoding of a 20-byte hash. The distinct HRP lets wallets and explorers identify
script type from the address string without parsing on-chain data.

---

## Architecture

### Approach

P2SH detection is inlined into `vm::execute()`. No changes to `utxo.rs` — it continues calling
`execute(script_sig, script_pubkey, ctx)` unchanged.

### Execution Flow

```
execute(script_sig, script_pubkey, ctx):
  1. run(stack, script_sig, allow_checksig=false)
  2. if is_p2sh(script_pubkey):                        ← new branch
       a. redeem_script = stack.pop()                  ← top item must be the redeem script
       b. hash20 = sha256(redeem_script)[0..20]
       c. if hash20 != script_pubkey[2..22]:
            return Err(ScriptError::P2shHashMismatch)
       d. run(stack, redeem_script, allow_checksig=true)
  3. else:                                             ← existing path, unchanged
       run(stack, script_pubkey, allow_checksig=true)
  4. return stack.last().is_truthy()
```

`is_p2sh(spk)` is a pure predicate:

```
spk.len() == 23
  && spk[0] == OP_HASH160   // 0xa9
  && spk[1] == 0x14         // push 20 bytes
  && spk[22] == OP_EQUAL    // 0x87
```

No new opcodes are needed. All existing opcodes (`OP_HASH160`, `OP_EQUAL`, `OP_CHECKMULTISIG`, etc.)
are already implemented.

---

## Components

### 1. `script/mod.rs`

Add one new error variant:

```rust
ScriptError::P2shHashMismatch
```

### 2. `script/vm.rs`

Modify `execute()` to add the P2SH branch (step 2 above) between the two `run()` calls.
Add `is_p2sh(spk: &[u8]) -> bool` as a private helper.

### 3. `script/standard.rs`

New public functions:

```rust
// Locking scripts
pub fn p2sh_script(hash20: &[u8]) -> Vec<u8>
// → OP_HASH160 0x14 <hash20> OP_EQUAL  (always 23 bytes)

pub fn p2sh_script_from_redeem(redeem_script: &[u8]) -> Vec<u8>
// → sha256(redeem)[0..20] → p2sh_script(hash)

// Address encoding (bech32, hrp = "txms")
pub fn p2sh_address_from_hash(hash20: &[u8]) -> String
pub fn p2sh_address_from_redeem(redeem_script: &[u8]) -> String

// Address decoding — extract hash20 from a txms1... address
pub fn p2sh_hash_from_address(addr: &str) -> Result<[u8; 20], ScriptError>

// Parsing
pub fn extract_p2sh_hash(spk: &[u8]) -> Option<[u8; 20]>

// scriptSig builder for P2SH-multisig spend
// Format: [sig1_len][sig1_bytes] ... [sigN_len][sigN_bytes] [redeem_len][redeem_bytes]
pub fn p2sh_multisig_script_sig(sigs: &[&[u8]], redeem_script: &[u8]) -> Vec<u8>
```

`extract_address()` is extended to detect P2SH first and return `txms1...` if matched,
then fall through to P2PKH detection. Callers get the right address type automatically.

### 4. `crates/txmwallet/src/main.rs`

**New command: `p2sh-multisig-script`**

```
txmwallet p2sh-multisig-script <m> <pubkey_hex1> ... <pubkey_hexN>
```

Internally: calls `multisig_script(m, pubkeys)` → wraps with `p2sh_script_from_redeem` →
prints redeem script hex, P2SH scriptPubKey hex, and `txms1...` address.

**New command: `p2sh-multisig-spend`**

```
txmwallet p2sh-multisig-spend <p2sh_spk_hex> <dest_addr> <redeem_script_hex> <amount_atoms> [rpc]
```

Looks up UTXOs via `/getutxos/<p2sh_spk_hex>` (the node already supports arbitrary scriptPubKey
hex at this endpoint — no node changes needed). Builds an unsigned `Transaction` with the correct
outputs, saves to `p2sh-multisig-spend-tx.json`. User then runs the existing `multisig-sign` flow
(signing logic is identical — signers sign `tx.signature_hash()`).

**Modified command: `multisig-combine` — add `--redeem <hex>` flag**

```
txmwallet multisig-combine <tx_file> <sig1> <sig2> [--redeem <redeem_script_hex>]
```

If `--redeem` is present: builds scriptSig via `p2sh_multisig_script_sig(sigs, redeem)` and
broadcasts as P2SH-multisig spend.  
Without `--redeem`: existing bare-multisig behavior, fully backward compatible.

---

## Full Spending Flow

```
# Step 1 — Create P2SH address for receiving
txmwallet p2sh-multisig-script 2 <pk1_hex> <pk2_hex> <pk3_hex>
# → prints: redeem_hex, p2sh_spk_hex, txms1... address

# Step 2 — Fund: send TXM to txms1... address from any wallet

# Step 3 — Build unsigned spend tx
txmwallet p2sh-multisig-spend <p2sh_spk_hex> <dest_txm_addr> <redeem_hex> <atoms> [rpc]
# → p2sh-multisig-spend-tx.json

# Step 4 — Each required signer signs (existing command, unchanged)
TENSORIUM_WALLET_PASSPHRASE=... txmwallet multisig-sign p2sh-multisig-spend-tx.json
# → sig-<address>.json

# Step 5 — Combine sigs + redeem script, broadcast
txmwallet multisig-combine p2sh-multisig-spend-tx.json sig1.json sig2.json --redeem <redeem_hex>
# → broadcasts tx
```

---

## Error Handling

| Scenario | Error |
|----------|-------|
| scriptSig does not push a redeem script (empty stack) | `ScriptError::StackUnderflow` |
| Pushed redeem script hash does not match P2SH scriptPubKey | `ScriptError::P2shHashMismatch` |
| Redeem script itself fails (wrong sigs, wrong m/n) | existing `ScriptError` variants |
| `p2sh-multisig-spend`: no UTXO found for given scriptPubKey | CLI error with human message |
| `multisig-combine --redeem`: redeem script hex malformed | CLI error with human message |

---

## Tests

**`script/vm.rs` — 4 new tests:**

| Test | Asserts |
|------|---------|
| `p2sh_multisig_2of3_valid` | Full roundtrip: lock with P2SH, spend with 2 valid sigs + redeem → `true` |
| `p2sh_hash_mismatch_fails` | Push wrong redeem script → `Err(P2shHashMismatch)` |
| `p2sh_empty_stack_fails` | Empty scriptSig → `Err(StackUnderflow)` |
| `p2sh_wrong_sig_fails` | Correct redeem, wrong signatures → `false` or `Err(InvalidSignature)` |

**`script/standard.rs` — 4 new tests:**

| Test | Asserts |
|------|---------|
| `p2sh_script_roundtrip` | `p2sh_script_from_redeem` → `extract_p2sh_hash` → hash matches |
| `p2sh_address_roundtrip` | `p2sh_address_from_hash` → `p2sh_hash_from_address` → same hash20 |
| `p2sh_address_rejects_txm_prefix` | `p2sh_hash_from_address("txm1...")` → `Err(InvalidAddress)` |
| `p2sh_multisig_script_sig_layout` | Verify byte layout: sigs then redeem script, correct length prefixes |

**Target:** 8 new tests, workspace total 95 → ~103 tests, 0 failures, 0 warnings.

---

## Out of Scope (future scripting milestones)

- P2SH-HTLC
- P2SH-P2PKH (redundant, no practical use)
- P2WSH (witness version)
- OP_CSV (relative timelock)
- Custom/arbitrary redeem scripts via CLI
