# Scripting Layer S2 — Bare Multisig Design

Status: spec approved, pending implementation
Date: 2026-06-02

## Goal

Add m-of-n bare multisig to the Tensorium script VM. Primary use case: 2-of-3 treasury and pool hot-wallet protection. No chain reset required — purely additive on top of S1 (P2PKH).

## Scope

- VM: `OP_1..OP_16`, `OP_CHECKMULTISIG`, `OP_CHECKMULTISIGVERIFY`
- Standard builders: `multisig_script`, `multisig_script_sig`, `extract_multisig`
- Wallet CLI: `multisig-script`, `multisig-sign`, `multisig-combine`, `send-from-script`
- No P2SH (deferred to S3)
- No chain reset

## Script Formats

**scriptPubKey (2-of-3 example, 105 bytes):**
```
OP_2  0x21 <pubkey1:33>  0x21 <pubkey2:33>  0x21 <pubkey3:33>  OP_3  OP_CHECKMULTISIG
0x52  0x21 [33 bytes]    0x21 [33 bytes]    0x21 [33 bytes]    0x53  0xae
```

**scriptSig (2 signatures):**
```
[sig1_len][DER sig1 bytes]  [sig2_len][DER sig2 bytes]
```
Signatures must appear in the same order as their corresponding pubkeys in scriptPubKey.

## Opcodes

Added to `crates/tensorium-core/src/script/mod.rs`:

| Opcode | Hex | Behaviour |
|--------|-----|-----------|
| `OP_1` .. `OP_16` | `0x51` .. `0x60` | Push `[n]` (single byte) onto stack |
| `OP_CHECKMULTISIG` | `0xae` | Pop n, n pubkeys, m, m sigs. Verify m-of-n in order. Push `[0x01]` on success, `[]` on failure. |
| `OP_CHECKMULTISIGVERIFY` | `0xaf` | Same as `OP_CHECKMULTISIG` but return `Err(VerifyFailed)` instead of pushing false. |

**No dummy element.** Tensorium's `OP_CHECKMULTISIG` is a clean implementation — unlike Bitcoin's historical off-by-one, no extra stack element is consumed.

### OP_CHECKMULTISIG Execution (vm.rs)

```
1. pop top → n_bytes; n = n_bytes[0] as usize
2. validate: n >= 1, n <= 16
3. pop n pubkeys (each expected to be 33 bytes)
4. pop top → m_bytes; m = m_bytes[0] as usize
5. validate: m >= 1, m <= n
6. pop m signatures
7. match: iterate sigs in order; for each sig find a matching pubkey
   advancing pubkey pointer forward (pubkeys can only be consumed once, in order)
8. if all m sigs matched: push [0x01]
   else: push []
```

Errors returned (not just false) on structural failures: stack underflow, n > 16, m > n, invalid pubkey bytes.

## Standard Script Builders

Added to `crates/tensorium-core/src/script/standard.rs`:

### `multisig_script(m: u8, pubkeys: &[&[u8]]) -> Result<Vec<u8>, ScriptError>`

Builds a bare m-of-n scriptPubKey. Validates:
- `m >= 1`, `n = pubkeys.len()`, `m <= n`, `n <= 16`
- each pubkey is exactly 33 bytes (compressed secp256k1)

Returns `Err(ScriptError::InvalidKey)` on violation.

### `multisig_script_sig(sigs: &[&[u8]]) -> Vec<u8>`

Builds a scriptSig from a slice of DER-encoded signatures.
Format: `[len][sig_bytes]` repeated for each sig, in signing order.

### `extract_multisig(script_pubkey: &[u8]) -> Option<(u8, Vec<Vec<u8>>)>`

Parses a bare multisig scriptPubKey. Returns `Some((m, pubkeys))` if pattern matches, `None` otherwise. Used by explorer and RPC for output labelling.

## Wallet CLI

Added to `crates/txmwallet/src/main.rs`:

### `txmwallet multisig-script <m> <pubkey_hex1> ... <pubkey_hexN>`

Prints the scriptPubKey hex for the given m-of-n configuration. Used once at treasury setup time; the hex is stored by operators and used as the send target.

```
$ txmwallet multisig-script 2 <pubA_hex> <pubB_hex> <pubC_hex>
scriptpubkey: 5221<pubA>21<pubB>21<pubC>53ae
```

### `txmwallet send-from-script <scriptpubkey_hex> <dest_addr> <atoms> [rpc_addr]`

Build an unsigned transaction spending a UTXO locked to the given scriptPubKey, sending `atoms` to `dest_addr`. Does not sign. Writes `unsigned-tx.json`.

UTXOs are discovered via `/getutxos/<scriptpubkey_hex>` RPC. The node's existing `/getutxos/<address>` endpoint is extended: if the path parameter starts with `txm1` it is decoded as a bech32 P2PKH address (existing behaviour); otherwise it is treated as a lowercase hex-encoded scriptPubKey and matched directly against UTXO script bytes.

### `txmwallet multisig-sign <tx_file>`

Load `tx_file` (unsigned or partial). Sign each input using the loaded wallet's private key. Write the DER signature to `<tx_file>.sig<wallet_address_prefix>`. Does not modify `tx_file` — signing is non-destructive and offline-capable.

### `txmwallet multisig-combine <tx_file> <sig_file1> <sig_file2> [sig_file3]`

Read `tx_file` (unsigned). Read each sig file. Build the complete scriptSig by calling `multisig_script_sig`. Write the combined, broadcast-ready transaction back to `tx_file`. Validates that the combined script will execute correctly before writing.

## File Changes

| File | Change |
|------|--------|
| `crates/tensorium-core/src/script/mod.rs` | Add `OP_1..OP_16`, `OP_CHECKMULTISIG`, `OP_CHECKMULTISIGVERIFY` constants |
| `crates/tensorium-core/src/script/vm.rs` | Implement `OP_1..OP_16` push and `OP_CHECKMULTISIG` / `OP_CHECKMULTISIGVERIFY` execution |
| `crates/tensorium-core/src/script/standard.rs` | Add `multisig_script`, `multisig_script_sig`, `extract_multisig` |
| `crates/txmwallet/src/main.rs` | Add `multisig-script`, `multisig-sign`, `multisig-combine`, `send-from-script` subcommands |
| `crates/tensorium-node/src/main.rs` | Extend `/getutxos/` to accept hex scriptPubKey in addition to bech32 address |

No changes to `block.rs`, `utxo.rs`, `chain.rs`, `state.rs`, or consensus parameters.

## Tests

8 unit tests across `vm.rs` and `standard.rs`:

1. `op_checkmultisig_2of3_valid` — 2 correct sigs, 3 pubkeys → true
2. `op_checkmultisig_wrong_sig_fails` — one wrong sig → false (not error)
3. `op_checkmultisig_insufficient_sigs` — only 1 sig for m=2 → stack underflow error
4. `op_checkmultisig_m_greater_than_n` → structural error
5. `op_checkmultisig_sigs_out_of_order` — sigs in wrong pubkey order → false
6. `multisig_script_roundtrip` — build then `extract_multisig` → same (m, pubkeys)
7. `multisig_script_rejects_m_greater_than_n` → `Err(InvalidKey)`
8. `multisig_script_sig_length` — correct byte layout

## Constraints

- Max n = 16 (matches `OP_1..OP_16` range)
- Pubkeys must be 33-byte compressed secp256k1
- Sig order in scriptSig must match pubkey order in scriptPubKey
- `OP_CHECKMULTISIG` is only valid in scriptPubKey execution context (`allow_checksig = true`)
- No dummy element — scripts written for Bitcoin's OP_CHECKMULTISIG are not compatible

## What S2 Does Not Include

- P2SH (deferred to S3)
- OP_CHECKLOCKTIMEVERIFY / timelocks (S3)
- Multi-input multisig transactions (wallet handles one input at a time for now)
- Hardware wallet signing integration
