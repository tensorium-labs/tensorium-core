# Scripting Layer S3 — CLTV + HTLC Design

Status: spec approved, pending implementation
Date: 2026-06-04

## Goal

Add absolute timelocks (`OP_CHECKLOCKTIMEVERIFY`) and Hash Time Locked Contracts
(HTLC) to the Tensorium script VM. Primary use case: trustless cross-chain atomic
swaps (TXM ⇄ wTXM on Optimism) and time-locked escrow. Purely additive on top of
S1 (P2PKH) and S2 (multisig). **No chain reset, no consensus-field changes.**

## Scope

- VM: `OP_0` (0x00), `OP_CHECKLOCKTIMEVERIFY` (0xb1), `ScriptError::LockTimeNotMet`
- Standard builders: `htlc_script`, `htlc_claim_script_sig`, `htlc_refund_script_sig`, `extract_htlc`
- Wallet CLI: `htlc-secret`, `htlc-script`, `htlc-claim`, `htlc-refund`
- Atomic swap integration guide: `docs/integrations/ATOMIC_SWAP_HTLC.md`
- `wallet.rs` local-verify uses RPC tip height (was hardcoded `block_height: 0`)

### Out of scope (explicit)

- P2SH (pay-to-script-hash) — deferred to S4
- `OP_CHECKSEQUENCEVERIFY` / relative timelocks (needs `sequence` field = consensus change)
- Nested-IF correctness inside non-executing branches (HTLC uses flat IF/ELSE only)
- New `Transaction` fields (`nLockTime`) — not needed, see CLTV model below
- Chain reset

## Design Decisions

1. **CLTV model = direct block-height.** The script VM already receives
   `ctx.block_height` (passed by `utxo.rs` as `tip_height` during mempool/tx
   validation and `block.header.height` during block validation). `OP_CLTV`
   compares the on-stack locktime operand directly against `ctx.block_height`.
   This avoids adding a transaction-level `nLockTime` field (which would be a
   consensus change + chain reset) and an input-level `sequence`. Consistent with
   Tensorium's already-simplified VM (e.g. multisig has no Bitcoin dummy element).

2. **Hashlock = SHA256(secret), 32 bytes.** `OP_SHA256` already exists in the VM.
   Solidity exposes a `sha256` precompile, so the same hashlock works on the EVM
   wTXM side — a secret revealed on one chain unlocks the matching HTLC on the
   other. This is the standard, portable HTLC hash.

3. **Height-based timelocks only** (no wall-clock time). Block height is
   deterministic and already in context. `ScriptContext` carries no block
   timestamp, and adding median-time-past is unnecessary for HTLC. Tensorium
   block time ≈ 132 s/block; the atomic-swap guide documents height↔time conversion.

## VM Changes

Added to `crates/tensorium-core/src/script/mod.rs`:

| Opcode | Hex | Behaviour |
|--------|-----|-----------|
| `OP_0` / `OP_FALSE` | `0x00` | Push an empty element `[]` (falsy) onto the stack. |
| `OP_CHECKLOCKTIMEVERIFY` | `0xb1` | **Peek** top of stack as a little-endian unsigned integer (≤ 8 bytes) = `N`. If `ctx.block_height < N` → `Err(LockTimeNotMet)`. Value is **not popped** (Bitcoin-style; removed later by an explicit `OP_DROP`). |

New `ScriptError` variant: `LockTimeNotMet`.

### OP_0 Execution (vm.rs)

`0x00` is handled before the data-push range. When executing, push `Vec::new()`
(empty element). Respects `MAX_STACK_DEPTH`.

### OP_CHECKLOCKTIMEVERIFY Execution (vm.rs)

```
1. require stack non-empty → top = stack.last()  (peek, do NOT pop)
   → else Err(StackUnderflow)
2. if top.len() > 8 → Err(LockTimeNotMet)  (malformed locktime fails closed)
3. N = u64 from little-endian bytes of top (zero-extended for len < 8)
4. if ctx.block_height < N → Err(LockTimeNotMet)
5. otherwise continue (value stays on stack; script uses OP_DROP next)
```

The locktime operand is pushed by a normal data push (`0x01..0x4b <bytes>`). A
height like 500 encodes as `0x02 0xF4 0x01`. CLTV decodes little-endian, matching
the push encoding.

## Script Formats

### HTLC scriptPubKey (`htlc_script`)

```
OP_IF
    OP_SHA256 0x20 <hash:32> OP_EQUALVERIFY
    OP_DUP OP_HASH160 0x14 <recipient_hash:20> OP_EQUALVERIFY OP_CHECKSIG
OP_ELSE
    <push locktime LE> OP_CHECKLOCKTIMEVERIFY OP_DROP
    OP_DUP OP_HASH160 0x14 <refund_hash:20> OP_EQUALVERIFY OP_CHECKSIG
OP_ENDIF
```

- **Claim branch (IF):** spender reveals the preimage (SHA256 must match `hash`)
  and signs with the recipient key.
- **Refund branch (ELSE):** only valid once `block_height ≥ locktime`; signed with
  the refund (sender) key.

`recipient_hash` / `refund_hash` are `OP_HASH160` of the respective compressed
pubkeys (= `SHA256(pubkey)[0..20]`, matching `Address::from_public_key`).

### HTLC claim scriptSig (`htlc_claim_script_sig`)

```
<sig_len><DER sig>  <pk_len><pubkey:33>  <preimage_len><preimage>  OP_1
```

Trailing `OP_1` (0x51) is the truthy condition consumed by `OP_IF` → enters the
claim branch.

### HTLC refund scriptSig (`htlc_refund_script_sig`)

```
<sig_len><DER sig>  <pk_len><pubkey:33>  OP_0
```

Trailing `OP_0` (0x00) is the falsy condition consumed by `OP_IF` → enters the
refund branch.

Combined execution order: scriptSig runs first (pushes sig, pubkey, [preimage],
condition), then scriptPubKey.

## Standard Script Builders

Added to `crates/tensorium-core/src/script/standard.rs`:

### `htlc_script(hash: &[u8; 32], recipient_hash: &[u8; 20], refund_hash: &[u8; 20], locktime: u64) -> Vec<u8>`

Builds the HTLC scriptPubKey above. Encodes `locktime` as minimal little-endian
bytes (1–8 bytes) with a matching data-push prefix.

### `htlc_claim_script_sig(sig: &[u8], pubkey: &[u8], preimage: &[u8]) -> Vec<u8>`

Builds the claim scriptSig (`sig`, `pubkey`, `preimage`, `OP_1`).

### `htlc_refund_script_sig(sig: &[u8], pubkey: &[u8]) -> Vec<u8>`

Builds the refund scriptSig (`sig`, `pubkey`, `OP_0`).

### `extract_htlc(spk: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>, u64)>`

Parses an HTLC scriptPubKey → `Some((hash, recipient_hash, refund_hash, locktime))`
if the pattern matches, else `None`. Used by explorer/RPC for output labelling.

## Wallet CLI

Added to `crates/txmwallet/src/main.rs`:

### `txmwallet htlc-secret`

Generate a random 32-byte preimage. Print `preimage: <hex>` and
`sha256: <hex>`. The recipient keeps the preimage secret and shares only the hash.

### `txmwallet htlc-script <hash_hex> <recipient_addr> <refund_addr> <locktime_height>`

Decode the two bech32 addresses to their 20-byte pubkey hashes, build the HTLC
scriptPubKey, print as hex. The hex is the funding target for `send-from-script`
(from S2) or a plain payment.

### `txmwallet htlc-claim <spk_hex> <dest_addr> <atoms> <preimage_hex> [rpc_addr]`

Discover a UTXO locked to `spk_hex` via `/getutxos/<spk_hex>` (S2 extension),
build an unsigned spend to `dest_addr`, sign with the loaded wallet key, assemble
the claim scriptSig (revealing the preimage), and write a broadcast-ready
`htlc-claim-tx.json`.

### `txmwallet htlc-refund <spk_hex> <dest_addr> <atoms> [rpc_addr]`

Same discovery/build/sign flow but assembles the refund scriptSig. The resulting
transaction is only accepted once `block_height ≥ locktime`; the node enforces
this via `OP_CHECKLOCKTIMEVERIFY`. Writes `htlc-refund-tx.json`.

Both `htlc-claim`/`htlc-refund` sign over `tx.signature_hash()` (same scheme as
S2 multisig). Funding and UTXO discovery reuse existing endpoints — **no node
changes**.

## wallet.rs Local-Verify Update

`wallet.rs` currently constructs `ScriptContext { sig_hash, block_height: 0 }` for
local pre-broadcast verification. With `block_height: 0`, a refund script would
fail CLTV locally even when it would be valid on-chain. Update the local verifier
to fetch the current tip height via RPC (`/getblockcount`) when an RPC endpoint is
available, falling back to `0` (or skipping CLTV-strict verification) when offline.
Claim-path verification is unaffected (no CLTV in the IF branch).

## Atomic Swap Integration Guide

New file `docs/integrations/ATOMIC_SWAP_HTLC.md`. No new code — documents how to
compose the HTLC primitive into a trustless cross-chain swap:

- **Scenario:** Alice has TXM, Bob has wTXM (Optimism). They swap atomically.
- **Flow:**
  1. Alice runs `htlc-secret` → keeps `preimage`, shares `sha256` hash.
  2. Alice locks TXM in a TXM HTLC: recipient = Bob, refund = Alice, locktime = H1.
  3. Bob locks wTXM in an EVM HTLC with the **same** SHA256 hashlock, timeout < H1
     (in EVM seconds), recipient = Alice, refund = Bob.
  4. Alice claims the wTXM by revealing `preimage` on Optimism.
  5. Bob reads `preimage` from the Optimism claim and uses it to run `htlc-claim`
     on TXM.
- **Safety:** Alice's TXM refund timeout (H1) must be strictly later than Bob's
  wTXM timeout, so Alice cannot claim wTXM and also reclaim her TXM. Includes a
  height↔time conversion table (TXM ≈ 132 s/block).

## File Changes

| File | Change |
|------|--------|
| `crates/tensorium-core/src/script/mod.rs` | Add `OP_0`, `OP_CHECKLOCKTIMEVERIFY` constants; `LockTimeNotMet` error |
| `crates/tensorium-core/src/script/vm.rs` | Execute `OP_0` and `OP_CHECKLOCKTIMEVERIFY` |
| `crates/tensorium-core/src/script/standard.rs` | Add `htlc_script`, `htlc_claim_script_sig`, `htlc_refund_script_sig`, `extract_htlc` |
| `crates/tensorium-core/src/wallet.rs` | Local-verify uses RPC tip height |
| `crates/txmwallet/src/main.rs` | Add `htlc-secret`, `htlc-script`, `htlc-claim`, `htlc-refund` subcommands |
| `docs/integrations/ATOMIC_SWAP_HTLC.md` | New atomic swap guide |

No changes to `block.rs`, `utxo.rs`, `chain.rs`, `state.rs`, `tensorium-node`, or
consensus parameters.

## Tests

~10 unit tests across `vm.rs` and `standard.rs`:

1. `op_0_pushes_empty` — `OP_0` pushes a falsy empty element
2. `cltv_passes_when_height_ge_locktime` — `block_height ≥ N` → continues
3. `cltv_fails_below_locktime` — `block_height < N` → `Err(LockTimeNotMet)`
4. `cltv_leaves_value_on_stack` — operand still present after CLTV (for `OP_DROP`)
5. `htlc_claim_valid` — correct preimage + recipient sig → true
6. `htlc_claim_wrong_preimage_fails` — bad preimage → `EQUALVERIFY` failure
7. `htlc_claim_wrong_sig_fails` — bad signature → false / failure
8. `htlc_refund_valid` — `block_height ≥ locktime` + refund sig → true
9. `htlc_refund_before_locktime_fails` — `block_height < locktime` → `LockTimeNotMet`
10. `htlc_script_roundtrip` — `htlc_script` then `extract_htlc` → same fields

## Constraints

- Locktime operand ≤ 8 bytes, decoded little-endian as `u64`
- Hashlock is exactly 32 bytes (`SHA256`)
- Pubkey hashes are 20 bytes (`OP_HASH160`)
- HTLC uses a single flat `OP_IF/OP_ELSE/OP_ENDIF` (no nesting)
- `OP_CHECKLOCKTIMEVERIFY` evaluates against `ctx.block_height`; correctness during
  block validation relies on `utxo.rs` passing the including block's height
- `OP_CHECKSIG` remains valid only in scriptPubKey context (`allow_checksig = true`)

## What S3 Does Not Include

- P2SH (S4)
- Relative timelocks / `OP_CHECKSEQUENCEVERIFY`
- Time-based (wall-clock) locktimes
- Multi-input HTLC transactions (wallet handles one input at a time)
- On-chain enforcement of swap atomicity beyond the HTLC primitive itself
