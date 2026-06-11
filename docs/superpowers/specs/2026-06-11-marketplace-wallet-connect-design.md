# Marketplace Wallet-Connect + TXM20/NFT Creation (Phase 1)

## Goal

Let users connect their Tensorium wallet (browser extension) to
`marketplace.tensoriumlabs.com` and, entirely from the browser:

- issue a new TXM20 fungible token (BRC20-style: ticker, decimals, fixed supply, name)
- mint an NFT (single or part of a collection, with royalty + media URI/hash)
- transfer a TXM20 balance or NFT to another address
- view their own asset holdings ("My Assets")

Marketplace **trading** (list-for-sale, buy, atomic settlement) is **out of
scope** for this spec — it requires a new order-relay backend and is deferred
to a separate Phase 2 design.

## Background

- The on-chain asset protocol (`crates/tensorium-core/src/assets`) is a pure,
  dependency-free module shared by the wallet and indexer. Operations
  (`Issue`, `NftMint`, `Transfer`) are encoded into a `TXMA` `OP_RETURN`
  payload via `assets::codec::encode_op` / `op_return_script`. The node skips
  `OP_RETURN` outputs from the UTXO set, so these ride as ordinary
  transactions — no hard fork.
- `txmwallet` CLI already implements `asset-issue`, `asset-mint`,
  `asset-transfer` end-to-end (UTXO selection, OP_RETURN construction,
  signing, broadcast) — this logic is the reference implementation.
- `txm-asset-indexer` (binds `localhost:23340`, proxied read-only at
  `/api/` on the marketplace domain) tracks asset state: balances, supply,
  NFT ownership, derived by replaying `OP_RETURN` ops from chain data.
- The wallet extension (`tensorium-wallet-extension`, currently v0.1.5)
  injects `window.tensorium` into pages on `*.tensoriumlabs.com` with
  `isInstalled`, `getAddress()`, `requestAccounts()`, `sendTransaction()`.
  It already holds the user's signing key (`WalletKeypair`) and has a
  popup-approval flow for `sendTransaction`.
- The node's RPC server (`crates/tensorium-node/src/main.rs`,
  `handle_rpc_stream`, exposed publicly at `rpc.tensoriumlabs.com`) already
  serves `/getutxos/:address`, `/sendrawtransaction`, `/getmempoolinfo`, etc.

## Architecture

```
┌─────────────────────┐   requestAccounts()    ┌──────────────────────────┐
│  marketplace.        │ ──────────────────────▶│  Wallet extension         │
│  tensoriumlabs.com   │                         │  (window.tensorium,       │
│  (static site)        │◀──────────────────────│   holds signing key)      │
└──────────┬───────────┘   address               └─────────────┬────────────┘
           │                                                     │
           │ POST /buildAssetTx (op, params)                     │ signAssetTx(unsignedTx)
           ▼                                                     │ → POST /sendrawtransaction
┌─────────────────────┐   GET /balance, /holders  ┌─────────────▼────────────┐
│ rpc.tensoriumlabs.com│ ──────────────────────────▶│ txm-asset-indexer         │
│ (tensorium-node RPC) │   (localhost:23340)        │ (read-only, asset state)  │
└──────────────────────┘◀──────────────────────────└────────────────────────────┘
```

## Components

### 1. Node RPC: `POST /buildAssetTx`

New handler in `crates/tensorium-node/src/main.rs::handle_rpc_stream`,
alongside the existing `/getutxos/:address` and `/sendrawtransaction`.

**Request body** (JSON):

```jsonc
// op = "issue"
{ "op": "issue", "from": "txm1...", "ticker": "GOLD", "decimals": 0,
  "supply": 1000000, "name": "Gold Token" }

// op = "nft_mint"
{ "op": "nft_mint", "from": "txm1...", "collection_id": "00..00" /* hex, optional, default all-zero */,
  "royalty_bps": 500, "royalty_addr": "txm1...", "uri": "ipfs://...",
  "content_hash": "<64-hex-sha256>" }

// op = "transfer"
{ "op": "transfer", "from": "txm1...", "to": "txm1...",
  "asset_id": "<64-hex-txid>", "amount": 100 }
```

**Server-side steps**:

1. Validate field shapes/limits using the same constraints
   `assets::codec::encode_op` already enforces (ticker ≤ 8 bytes, name ≤ 32,
   uri ≤ 200, royalty_bps ≤ 10000, etc.) — return `400` with a field-level
   error message on violation.
2. For `transfer`: query `txm-asset-indexer` (`GET /balance/:from` for
   TXM20, `GET /nft/:asset_id/owner` for NFTs) to confirm `from` holds
   sufficient balance / owns the NFT. If the indexer is unreachable, return
   `503 { "error": "asset index temporarily unavailable, try again shortly" }`
   — do **not** fall back to building an unvalidated transfer.
3. Fetch UTXOs for `from` via the existing `/getutxos/:address` internal path.
4. Select inputs (reuse `txmwallet`'s coin-selection: smallest-first or
   largest-first sufficient to cover fee; asset ops carry zero `amount_atoms`
   beyond dust/fee since the asset itself moves via `OP_RETURN` + a marker
   output).
5. Build outputs: `OP_RETURN` (via `assets::codec::op_return_script`),
   change output back to `from`, and for `transfer`/`nft_mint`/`issue` a
   small dust output (`dest_output_index` per `TransferData`) to the
   recipient/creator address so `dest_output_index` resolves correctly.
6. Compute fee using existing `min_relay_fee_atoms` / `priority_fee_atoms`
   constants.
7. Return:

```jsonc
{
  "unsigned_tx": { /* same shape txmwallet builds pre-signing */ },
  "summary": {
    "op": "issue",
    "description": "Issue token GOLD — supply 1,000,000, decimals 0",
    "fee_atoms": 100000
  }
}
```

The `unsigned_tx` shape must exactly match what `WalletKeypair::sign_input`
(used by `txmwallet` and the extension) expects — no new serialization
format.

### 2. Wallet extension additions

`window.tensorium` gains two methods (in `src/inpage/index.ts`, proxied
through `src/content/inject.ts` to the service worker as today):

- **`signAssetTx(unsignedTx, summary)`** → opens the existing approval
  popup, rendering `summary.description` and `summary.fee_atoms` (formatted
  as TXM) instead of the generic "send X TXM to Y" text used for
  `sendTransaction`. On approval: signs each input with
  `WalletKeypair::sign_input`, `POST`s to
  `rpc.tensoriumlabs.com/sendrawtransaction`, and resolves with `{ txid }`.
  On rejection: rejects the promise with `Error("User rejected")`.

- **`getAssets(address)`** → proxies to the indexer:
  `GET /balance/:address` (all TXM20 holdings) and a holdings-by-owner
  lookup for NFTs. Returns:
  ```ts
  { tokens: [{ asset_id, ticker, balance, decimals }],
    nfts: [{ asset_id, collection_id, uri, royalty_bps }] }
  ```
  If the indexer endpoint for "all assets owned by an address" doesn't
  exist yet, add it to `tensorium-indexer`'s read-only router
  (`crates/tensorium-indexer/src/api.rs`) as `GET /address/:addr/assets` —
  this is a pure read over already-indexed state, no new indexing logic.

Extension version bumps to v0.1.6.

### 3. Marketplace UI (`tensorium-sites/marketplace`)

New sections added to `index.html` (or split into `create.html` /
`assets.html` following the existing multi-page pattern of the site):

- **Connect Wallet** button in the nav, using
  `window.tensorium.requestAccounts()`. Shows the connected address
  (truncated) once connected; persists connection state in
  `localStorage` for the session.
- **Create Token** form: ticker, decimals (0–18), supply, name →
  `POST /buildAssetTx {op: "issue", ...}` → `signAssetTx`. On success, show
  the txid + link to indexer's `/asset/:id` lookup (already on the page per
  the existing "lookup" UI).
- **Mint NFT** form: collection (optional, defaults to standalone),
  royalty %, royalty address, media URI, content hash (computed client-side
  via `crypto.subtle.digest('SHA-256', file)` if the user uploads a file,
  or entered manually) → `op: "nft_mint"`.
- **Transfer** action: a "Send" button on each row of "My Assets" opens a
  small modal (destination address + amount) → `op: "transfer"`.
- **My Assets** page: calls `getAssets(address)` on connect, renders TXM20
  balances and owned NFTs using the existing `.asset-card` styles from
  `assets/core.css`.

All new network calls go to `rpc.tensoriumlabs.com` (already in the
extension's `host_permissions`) and the indexer's `/api/` proxy path on the
marketplace domain (already nginx-proxied).

## Error handling

- Form validation client-side mirrors server limits (ticker length, royalty
  range) for instant feedback, but the server is the source of truth —
  client validation is UX-only.
- `/buildAssetTx` errors (`400` validation, `503` indexer-unavailable, `500`
  insufficient UTXOs for fee) are surfaced verbatim in a dismissible inline
  error banner on the relevant form.
- `signAssetTx` rejection (user closes/declines popup) is caught and shown
  as a neutral "Transaction not sent" message — not treated as an error.
- Broadcast failure from `/sendrawtransaction` (e.g. mempool reject) bubbles
  up the node's error string unchanged, since it's already
  human-readable (e.g. "fee below minimum relay fee").

## Testing

- **Node RPC**: unit tests for `/buildAssetTx` per op type (issue, nft_mint,
  transfer) covering valid input, each validation failure, and the
  insufficient-balance/indexer-unreachable paths. Integration test:
  build → sign with a test `WalletKeypair` → `/sendrawtransaction` against a
  local single-node regtest-style chain → confirm indexer reflects the new
  asset after the block is mined.
- **Indexer**: unit test for new `GET /address/:addr/assets` route (empty,
  single TXM20, single NFT, mixed).
- **Wallet extension**: existing extension test harness extended with
  `signAssetTx` approve/reject paths (mock `unsigned_tx`/`summary`) and
  `getAssets` proxy (mock indexer response).
- **Marketplace UI**: manual browser test on a testnet/local chain — connect
  wallet, issue a token, mint an NFT, transfer both, confirm "My Assets"
  reflects state after each action. No automated UI test framework currently
  in this repo; manual pass is acceptable for this phase.

## Deployment

1. Ship `/buildAssetTx` + indexer's `/address/:addr/assets` as part of the
   next `tensorium-core` release (after this is merged to `main` and the
   live node/indexer are updated — same deploy flow as prior asset-protocol
   work: build → canary against frozen DB → deploy).
2. Ship wallet extension v0.1.6 (adds `signAssetTx`/`getAssets`) — requires
   users to update the extension; existing `sendTransaction` flow remains
   unchanged for backwards compatibility.
3. Ship marketplace UI changes to `tensorium-sites/marketplace` →
   `tensorium-marketplace` GitHub repo → VPS `/var/www/marketplace` (or
   wherever it's currently served — confirm path during implementation).

## Out of scope (Phase 2)

- Order-relay service for sell listings (`asset-sell`/`asset-buy`/
  `asset-accept` currently require manual JSON file exchange).
- Marketplace "Browse listings" / "Buy" UI and `signSettlement()` extension
  method.
- Royalty payout enforcement at settlement time (exists in
  `settlement.rs`, but only reachable via the file-exchange CLI flow today).
