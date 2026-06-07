# Persistent UTXO Set + Validation at Block-Accept Time

**Date:** 2026-06-07
**Status:** Approved (design)
**Severity of the bug this fixes:** Critical (consensus / network-wide DoS)

## Problem

`ChainState::submit_block` (crates/tensorium-core/src/state.rs) accepts a block
after calling only `validate_block` — which checks proof-of-work, chain id,
height, previous-hash, merkle root, future-timestamp, and that the first
transaction is a coinbase. It **never** calls `UtxoSet::apply_block`, so at the
moment a block becomes canonical the node does **not** verify:

- the coinbase amount (subsidy + fees ceiling) — i.e. supply inflation,
- transaction double-spends,
- input signatures / script satisfaction,
- output value not exceeding input value,
- coinbase maturity on spent inputs.

These rules are only enforced lazily, later, inside `build_utxo_set` (a full
replay of the canonical chain via `apply_block`) when something calls
`/getutxos`, `/sendrawtransaction`, or the pool payout path.

### Exploit

A miner produces a single valid-proof-of-work block (~2–3 minutes of GPU time at
the current 40-bit difficulty) whose coinbase is inflated, or that contains a
double-spend / invalid-signature transaction. `submit_block` accepts it; with the
most work it becomes the canonical tip. From that point every node that adopts
it has a corrupt canonical chain: `build_utxo_set`'s replay fails with
`UtxoError`, so `/getutxos` returns 500 **network-wide** — pool payouts halt,
wallet sends fail, and the bridge deposit watcher (which reads UTXOs) stops. The
poison block is stored and canonical; recovery requires manual database surgery
on every node. This is the same failure mode as the canonical-prune bug
(commit 842b2b8), but here it is reachable by any miner, cheaply, on demand.

## Goal

Validate every block's full UTXO consequences **before** it is adopted as
canonical, by maintaining a persistent UTXO set in RocksDB that is updated
incrementally on tip extension and rebuilt on reorg. Invalid blocks are rejected
and never become canonical.

Non-goals: changing the genesis, the consensus parameters, the wire protocol, or
resetting the chain. No undo-log machinery (reorgs replay).

## Design

### 1. Storage (crates/tensorium-core/src/storage/mod.rs)

Add a fourth column family and one meta key:

```rust
pub const CF_UTXO: &str = "utxo";
pub const META_UTXO_TIP: &[u8] = b"utxo_tip";
```

- **Key:** the outpoint, encoded as `txid (32 bytes) || output_index (4 bytes, big-endian)` = 36 bytes.
- **Value:** `bincode`-serialized `UtxoEntry { output: TxOutput, created_height: u64, coinbase: bool }` — the same codec already used for blocks (`encode_block` / `decode_block`).
- `META_UTXO_TIP` stores the hash of the block whose post-state the persistent
  UTXO set currently reflects. It is the single source of truth for "is the UTXO
  set in sync with the canonical tip?"

New helpers: `encode_outpoint(&OutPoint) -> [u8; 36]`, `decode_outpoint(&[u8]) -> OutPoint`, `encode_utxo_entry(&UtxoEntry) -> Vec<u8>`, `decode_utxo_entry(&[u8]) -> UtxoEntry`, with round-trip unit tests.

`cf_options()` in state.rs gains a `CF_UTXO` descriptor.

### 2. Validate by reusing `apply_block` unchanged

`UtxoSet::apply_block` reads each transaction's inputs from `self.entries` and
nothing else from the set (phase 1). Therefore a block can be validated against
the persistent store by **seeding a small in-memory `UtxoSet` with exactly the
inputs the block references** (each looked up from `CF_UTXO`; a missing lookup is
left absent so `apply_block` returns `MissingInput`), then calling the existing
`apply_block`. This is behavior-identical to validating against the full set and
requires **no change to utxo.rs**.

The persistent mutations for an accepted block are derived directly from the
block, deterministically (no dependence on the seeded set's final contents):

- for every non-coinbase transaction, delete each `input.previous_output` from `CF_UTXO`;
- for every transaction (including coinbase), insert each output that is not an
  `OP_RETURN` script, keyed by its outpoint, with `created_height = block.header.height`
  and `coinbase = tx.is_coinbase()`.

This mirrors phase 3 of `apply_block` exactly.

### 3. `submit_block` integration

After `validate_block` succeeds and the block is stored in `CF_BLOCKS`, compute
`new_work` vs `old_work` as today. Then:

- **Extension** — `block.header.previous_hash == old_tip_hash` (always heavier,
  so always adopted): seed a `UtxoSet` from `CF_UTXO` with the block's referenced
  inputs and run `apply_block`. On `Err`, return that error — the block remains
  stored as an orphan, the canonical chain and UTXO set are untouched. On `Ok`,
  write a single atomic `WriteBatch`: canonical pointer for the new height, the
  UTXO deltas (deletes + inserts), `META_TIP`, `META_HEIGHT`, and
  `META_UTXO_TIP = block_hash`.

- **Reorg** — `new_work > old_work` but the block does not extend the current
  tip: rebuild the UTXO set by replaying `apply_block` over the new canonical
  chain (`build_canonical_chain(block_hash)`), into a fresh `UtxoSet`. On any
  `Err`, reject the reorg: keep the existing canonical chain and UTXO set
  untouched, return the error. On success, write a batch that updates all
  canonical pointers (including the stale-tail pruning from commit 842b2b8),
  clears and rewrites `CF_UTXO` from the rebuilt set, and sets `META_TIP`,
  `META_HEIGHT`, `META_UTXO_TIP`.

- **Not heavier** — `new_work <= old_work`: store the block only (today's
  behavior); no canonical or UTXO change.

Validation always precedes the atomic write, so an invalid block can never leave
the UTXO set partially mutated.

### 4. Migration / self-heal on `open_db`

After loading caches, compare `META_UTXO_TIP` with the canonical tip hash. If it
is absent, or differs, rebuild the UTXO set from the canonical chain (the same
replay as `build_canonical_chain` + `apply_block` from genesis), persist it to
`CF_UTXO`, and set `META_UTXO_TIP` to the tip. This runs:

- once, on the first start after this upgrade (no `CF_UTXO` yet → build it), and
- whenever an inconsistency is detected (e.g. a crash between writes) — it is
  idempotent.

**Safety:** this replay is exactly what `build_utxo_set` performs today, and it
already succeeds on the live mainnet chain (it backs `/getutxos`). The historical
chain is therefore guaranteed to pass — no reset, no genesis change. If a future
historical block ever failed the replay, the node would refuse to start, which is
the correct fail-closed behavior (it means the stored chain is already corrupt).

### 5. Follow-on (sequenced after the core fix)

Once the persistent UTXO set is authoritative, the node's `build_utxo_set`
(O(n) replay per call) can be replaced by reads from `CF_UTXO`:

- `/getutxos/<addr>` scans `CF_UTXO` filtering by `script_pubkey` — no block
  decode/replay,
- `/sendrawtransaction` and mempool acceptance validate against `CF_UTXO`,
- pool payout maturity checks read `CF_UTXO`.

This is a correctness-neutral performance win and removes the dual-source-of-truth
risk. It lands as separate tasks **after** the validation fix, so the security
change ships first and the query-path migration can be verified independently.

## Error Handling

- Invalid block at accept → `Err(StateError::Validation(..))` (via `apply_block`'s
  `UtxoError`, mapped into `StateError`), block not adopted, UTXO untouched.
- Reorg onto an invalid branch → reject reorg, old state preserved.
- Migration replay failure on start → node refuses to start (fail-closed).
- All persistent writes for an accepted block are a single `WriteBatch` (atomic).

## Testing (TDD)

Core unit tests (crates/tensorium-core/src/state.rs and storage/mod.rs):

1. `submit_block` rejects a tip-extending block whose coinbase exceeds
   `reward_at_height(height)` (the headline fix) — written first, must fail
   against current code.
2. `submit_block` rejects a block containing a double-spend transaction.
3. `submit_block` rejects a block containing an invalid-signature transaction.
4. After N valid extensions, the persistent `CF_UTXO` equals a from-scratch
   `build`/replay of the canonical chain.
5. A reorg to a heavier valid branch rebuilds `CF_UTXO` to reflect the new branch
   (entries from the abandoned branch are gone, new branch's entries present).
6. A reorg to a heavier but **invalid** branch is rejected and leaves the previous
   canonical chain and UTXO set unchanged.
7. Opening a state whose `META_UTXO_TIP` is absent builds the UTXO set to match
   the canonical tip (migration).
8. `encode_outpoint` / `decode_outpoint` / `encode_utxo_entry` / `decode_utxo_entry`
   round-trip.

Existing 129 workspace tests must stay green.

## Deployment

1. Build on both VPS (`cargo build --release -p tensorium-node`).
2. Deploy to DO and Vultr together (consensus change — both must run identical
   rules). Backup the old binary; atomic `mv -f` replace; restart `tensorium-mc`.
3. On restart each node runs the one-time migration build (a few seconds for
   ~1600 blocks). Verify: `/getutxos/<treasury>` returns a clean set, heights stay
   synced DO == Vultr, a pool payout cycle succeeds, and a fresh block is accepted.
4. Reuse the established deploy notes: `export PATH="$HOME/.cargo/bin:$PATH"`
   before `cargo`; never pipe `cargo build` through `tail` under `set -e`; do not
   run global `cargo fmt`.

## Risks

- **Migration build time** grows with chain height; today ~seconds, fine. If it
  ever becomes a startup concern, the follow-on query-path work amortizes it.
- **A subtle divergence** between the incremental delta (extension path) and the
  replay (reorg/migration path) would corrupt the UTXO set. Test #4 pins them to
  equality; the delta is derived from the same block-output rules `apply_block`
  uses in phase 3.
- **Both nodes must deploy together.** A node on the old binary would still accept
  an invalid block the new node rejects, briefly diverging — but the new node
  rejecting the poison block is the correct outcome, and the monitor/sync converges
  the honest chain.
