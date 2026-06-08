# TXM Asset Protocol + Indexer — Design (Layer 1+2 of the Marketplace)

Foundation for `marketplace.tensoriumlabs.com`: a **native, on-chain** token
(TXM20, fungible) and NFT standard on the Tensorium L1, plus the indexer that
tracks balances and ownership. This spec covers **only Layer 1 (asset protocol)
and Layer 2 (indexer)**. Wallet support (Layer 3), marketplace/escrow/fees
(Layer 4), and the frontend (Layer 5) are separate specs that build on this.

## Problem & goal

TXM is a UTXO PoW L1 with a Bitcoin-like script layer and **no smart contracts
or native token concept**. We want fungible tokens (TXM20) and NFTs that are
fully recorded on-chain, verifiable by anyone, and usable in a marketplace —
**without a consensus change / hard fork**.

**Goal:** an overlay protocol where token operations ride inside ordinary TXM
transactions (as `OP_RETURN` metadata), and a deterministic indexer reconstructs
all balances and ownership purely from on-chain data.

## Non-goals (this spec)

- No consensus/L1 change. The node is unmodified; the protocol is an overlay and
  the node never validates token rules (the indexer does, deterministically).
- Marketplace, escrow, fees, royalty *enforcement*, wallet UX, frontend — later specs.
- Cross-chain (wTXM/EVM) assets — out of scope; these are native TXM assets.

## Design overview

An asset operation is encoded in a single `OP_RETURN` output of an otherwise
ordinary TXM transaction. The transaction's real inputs/outputs provide
authorization and anti-replay (every op spends a real UTXO). A standalone
**indexer** scans canonical blocks, parses these outputs, validates them against
the running asset state, and serves balances/ownership over a REST API. Re-running
the indexer from the chain reproduces identical state — there is no off-chain
source of truth for balances.

Ownership is tracked **by address**, Counterparty-style: the **source** of an op
is the address of the transaction's **first input's** spent output (the party who
necessarily signed). This avoids per-UTXO asset binding while keeping every move
authorized by a real signature.

### Codec lives in `tensorium-core`

The encode/decode + validation of the metadata is a set of **pure functions** in
a new `tensorium-core` module (`src/assets/`), so the wallet (Layer 3) and the
indexer (Layer 2) share one canonical implementation. The indexer is a separate
service/crate; it depends on `tensorium-core` for the codec and on the node RPC
for block data.

## Layer 1 — Asset protocol (OP_RETURN metadata)

### Envelope

```
OP_RETURN <push: "TXMA" | version(1) | opcode(1) | payload...>
```

- Magic `TXMA` (4 bytes) identifies the protocol; non-`TXMA` `OP_RETURN`s are ignored.
- `version` = 0x01.
- `opcode`: `0x01 ISSUE`, `0x02 NFT_MINT`, `0x03 TRANSFER`.
- Total data must fit one push (≤ `MAX_ELEMENT_SIZE` = 520 bytes). All multi-byte
  integers are big-endian. Strings are length-prefixed (1-byte length) UTF-8.

### `ISSUE` — create a fungible TXM20

Payload: `ticker(len≤8) | decimals(1) | supply(8) | name(len≤32) | flags(1)`

- `flags` bit 0 = `mintable` (issuer may later mint more — reserved; MVP issues
  fixed supply, bit ignored when 0).
- `asset_id` = the **issuing transaction's txid** (32 bytes) — globally unique.
- Effect: the indexer credits the **source address** with the full `supply` of
  `asset_id`, and records `asset_info{ticker, name, decimals, supply, issuer}`.
- Tickers are **not** unique; the unique key is always `asset_id`. UIs show
  `ticker` + a short `asset_id` to disambiguate.

### `NFT_MINT` — create a non-fungible token

Payload: `collection_id(32) | royalty_bps(2) | royalty_addr(len) | uri(len≤200) | content_hash(32)`

- `asset_id` = the minting **txid** (a single NFT per mint tx in MVP).
- `collection_id` = `asset_id` of a prior `ISSUE`-style collection record, or 32
  zero bytes for a standalone NFT. (A "collection" is an `ISSUE` with `supply=0`,
  `decimals=0` acting as a namespace; reserved — MVP allows zero/standalone.)
- **Royalty (recorded immutably at mint):** `royalty_bps` (0–10000, capped 10000)
  and `royalty_addr` (creator payout address). Stored in `asset_info`. The
  protocol/indexer only **records** royalty; **enforcement happens at sale time**
  in the marketplace (Layer 4) — secondary sales pay `royalty_bps` of the price to
  `royalty_addr`. On-chain provenance means the royalty terms are tamper-proof.
- `uri` + `content_hash` (SHA-256 of the asset content) bind off-chain media to
  the on-chain NFT.
- Effect: indexer sets `nft_owner[asset_id] = source address`, supply 1, indivisible.

### `TRANSFER` — move an asset

Payload: `asset_id(32) | amount(8) | dest_output_index(1)`

- Moves `amount` of `asset_id` (fungible) or `1` (NFT) from the **source address**
  to the address of the transaction output at `dest_output_index`.
- `dest_output_index` must point to a standard P2PKH output in the same tx whose
  address is resolvable; otherwise the op is invalid (ignored).
- Multiple `TXMA` `OP_RETURN`s per tx: only the **first** valid one is processed
  (one asset op per tx in MVP — keeps the source/ordering unambiguous).

## Ownership / balance model

- **Source address** = address derived from the `script_pubkey` of the UTXO spent
  by `inputs[0]`. The indexer resolves this via its own output index (see Layer 2).
- State:
  - `asset_info[asset_id]` = { kind: FT|NFT, ticker, name, decimals, supply,
    issuer, royalty_bps, royalty_addr, uri, content_hash, mint_height }
  - `ft_balances[address][asset_id]` = u64 (fungible)
  - `nft_owner[asset_id]` = address (non-fungible)
- A `TRANSFER` debits `ft_balances[source][asset_id]` and credits the destination
  address (or reassigns `nft_owner`).

## Validation rules (deterministic; indexer-enforced)

An op is **applied** only if all hold, else **ignored** (no state change):

1. Envelope parses cleanly (`TXMA`, known version, known opcode, well-formed payload).
2. `ISSUE`: `asset_id` (txid) not already an asset; `supply ≤ u64::MAX`; `decimals ≤ 18`.
3. `NFT_MINT`: `asset_id` not already an asset; `royalty_bps ≤ 10000`; `royalty_addr`
   valid (or empty → no royalty); `uri` length ≤ 200.
4. `TRANSFER`: `asset_id` exists; for FT, `ft_balances[source][asset_id] ≥ amount`
   and `amount > 0`; for NFT, `nft_owner[asset_id] == source` and `amount == 1`;
   `dest_output_index` resolves to a valid address.
5. Source address resolvable (inputs[0] prev-output is a known indexed P2PKH output).

Determinism: every indexer applying these rules to the same canonical chain
reaches the same state. There is no time-, network-, or order-of-arrival-dependent
behavior beyond canonical block/tx order.

## Layer 2 — Indexer

A standalone service (new crate `txm-asset-indexer`, or a module added to the
existing explorer indexer — decided in its plan) that:

1. **Scans canonical blocks** from a protocol **activation height** (the block at
   which the protocol goes live; before it, no asset ops exist) to the tip, via the
   node RPC (`/getblockcount`, `/getblock/<h>`).
2. **Maintains an output index** `outpoint(txid:vout) → address` for all P2PKH
   outputs, so it can resolve each tx's source (`inputs[0]`'s prev-output → address).
   (This is the same capability the explorer indexer already has; reuse if merging.)
3. For each tx, in canonical order: find the first `TXMA` `OP_RETURN`, decode,
   validate against current state, apply if valid.
4. **Persists** state + `last_scanned_height` to a DB (RocksDB or JSON snapshots,
   matching project conventions), with **atomic** writes.
5. **Reorg handling:** track per-height deltas (or snapshot at intervals); on a
   reorg detected via the node's canonical chain changing below the last-scanned
   tip, roll back to the common ancestor and re-apply — mirrors `state.rs` /
   explorer reorg handling.
6. **REST API** (read-only):
   - `GET /asset/<asset_id>` → asset_info (incl. royalty)
   - `GET /balance/<address>` → all FT balances + owned NFTs
   - `GET /nft/<asset_id>/owner` → current owner
   - `GET /assets?kind=ft|nft&limit&offset` → list/browse
   - `GET /holders/<asset_id>` → holder distribution (FT) / owner (NFT)
   - `GET /history/<address>` → asset op history
   - `GET /status` → last_scanned_height, tip, in_sync

The indexer is the query backend for the wallet and marketplace. It never holds
keys or funds.

## On-chain guarantee

Every issue, mint, and transfer is a real TXM transaction. Balances and ownership
are a pure function of the canonical chain — **fully reconstructable** by anyone
running the open indexer. The marketplace's database (Layer 4) will hold only
listing UX metadata (price, status); all asset custody and movement remain on-chain.

## Testing (TDD)

Codec (`tensorium-core/src/assets/`), pure-function unit tests:
1. `ISSUE` encode→decode round-trip (ticker/decimals/supply/name/flags).
2. `NFT_MINT` round-trip incl. `royalty_bps`/`royalty_addr`/`uri`/`content_hash`.
3. `TRANSFER` round-trip.
4. Envelope rejects: wrong magic, unknown version/opcode, truncated payload,
   over-size element, `royalty_bps > 10000`.

State machine (indexer apply logic, deterministic, no I/O):
5. `ISSUE` credits source full supply; duplicate `asset_id` ignored.
6. `TRANSFER` FT debits source / credits dest; over-balance transfer ignored
   (state unchanged).
7. `NFT_MINT` sets owner; `TRANSFER` NFT only by current owner; non-owner ignored.
8. Source = `inputs[0]` address; op with unresolvable source ignored.
9. Replaying the same block twice is idempotent; reorg rollback restores prior state.

Indexer integration (against a dev chain / fixtures): scan blocks containing
issue+mint+transfer ops and assert the API returns correct balances/owners.

## Risks

- **Indexer = source of truth for balances** (not node-validated). Mitigated by
  determinism + open implementation + anti-replay (each op spends a real UTXO).
  A future hardening path is consensus-level validation (separate decision).
- **`OP_RETURN` size** caps metadata (520 B) — fine for the fields above; long
  media goes via `uri` + `content_hash`.
- **Source = first input** means wallets must fund asset txs from the asset
  owner's address as `inputs[0]` (a wallet rule in Layer 3).
- **Reorgs** must roll back asset state; the indexer must not serve unconfirmed
  state as final (expose confirmations).

## What this spec delivers

1. `tensorium-core/src/assets/` codec + validation pure functions (TDD).
2. `txm-asset-indexer` service: scanner, output-index, state, reorg, persistence, REST API.

Layers 3–5 (wallet asset commands, marketplace + escrow + 2.5% platform fee +
royalty enforcement, frontend) follow as their own spec→plan→implement cycles.
