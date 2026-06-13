# Tensorium Marketplace — Wallet-Native Trading v1 (Design)

**Date:** 2026-06-13
**Status:** Approved (brainstorming) → pending spec review
**Supersedes/extends:** `2026-06-11-marketplace-wallet-connect-design.md`
**Repos touched:** `tensorium-order-relay` (new), `tensorium-core` (txmwallet + `web/marketplace`), `tensorium-wallet-extension`

---

## 1. Problem & goal

The live marketplace (`marketplace.tensoriumlabs.com`, static `web/marketplace/index.html` + read-only indexer API) is a catalog plus address/asset lookup. Every trade action — mint, sell, buy, settle — is shown only as a `txmwallet asset-…` CLI command the user must run by hand and exchange JSON files out-of-band. This is the "CLI-feel" gap.

**Goal of v1:** a wallet-native trading experience — connect wallet, browse real listings, list an asset, buy with a wallet signature, and complete sales in-page — with **no JSON/CLI handoff**, while staying **non-custodial** and not changing consensus.

## 2. Hard protocol constraint (drives the whole design)

Asset settlement is a **2-of-2 interactive co-sign** (verified in `txmwallet` + `crates/tensorium-core/src/settlement.rs`):

- `asset-sell` → produces an **unsigned** `AssetOrder` (terms + the seller's anchor UTXO outpoint; **no signature**).
- `asset-buy` → builds the settlement tx via `build_settlement_tx`, signs **only the buyer's inputs** (indices `1..`), self-checks with `verify_settlement`.
- `asset-accept` → the **seller signs `input[0]` last** and broadcasts.

There is **no seller-signed offer a buyer can fill unilaterally**. A true one-click "buy & own now" would need new sighash semantics (a consensus/script-VM change) and is explicitly **out of scope** for v1. What v1 removes is the manual JSON handoff, via an **order-relay** plus wallet-native partial signing. The seller still co-signs each sale — reduced to one in-dapp click.

## 3. Approved decisions

| Decision | Choice |
|---|---|
| v1 model | Buyer-native + order-relay; seller co-signs |
| Seller accept UX | In-dapp "My Sales" pending list (no external notifier in v1) |
| Listing lifecycle | Auto-expire (7 days) + seller cancel + auto-prune on chain-invalidation |
| Custody | Fully non-custodial — the relay never holds keys; only chain mutation is broadcasting a fully-signed tx |
| Settlement tx construction | Reuse the audited Rust `build_settlement_tx`/`verify_settlement` via a new **keyless** txmwallet subcommand — **no JS reimplementation** (zero drift) |

## 4. Architecture overview

Four components across three repos, built as three milestones:

```
┌────────────────────────┐      ┌─────────────────────────────┐
│  Marketplace frontend  │◀────▶│   tensorium-order-relay     │
│  (web/marketplace)     │ HTTP │  (new Node service)         │
│  window.tensorium      │      │  - listings + settlements   │
└───────────┬────────────┘      │  - validates vs node+indexer│
            │ provider           │  - builds unsigned tx (D)   │
            ▼                    │  - broadcasts signed tx     │
┌────────────────────────┐      └──────────┬──────────────────┘
│  tensorium-wallet-ext  │                 │ shells out / RPC
│  signAssetTxPartial    │                 ▼
│  signMessage           │      ┌─────────────────────────────┐
└────────────────────────┘      │ txmwallet asset-build-       │
                                 │ settlement (keyless) +       │
                                 │ tensorium-node RPC + indexer │
                                 └─────────────────────────────┘
```

Non-custodial invariant: the relay only ever holds **public** data (orders) and **buyer/seller-signed** transactions. Private keys never leave the wallet extension. The relay's single chain-mutating action is `POST /sendrawtransaction` of a tx already fully signed by both parties.

---

## 5. Component A — `tensorium-order-relay` (new service)

Modeled on `tensorium-otc-watcher`: ESM Node, Express API bound to `127.0.0.1:<port>`, JSON state with atomic temp-file+rename writes, systemd unit, fronted by nginx. TDD with `node:test`.

### 5.1 State model (`relay-state.json`, atomic writes)

```jsonc
{
  "listings": {
    "<listing_id>": {
      "listing_id": "lst_<random>",
      "asset_id_hex": "<64hex>",
      "kind": "txm20" | "nft",
      "amount": 100,                // fungible units; 1 for NFT
      "price_atoms": 5000000,
      "seller_addr": "txm1…",
      "anchor": { "txid": "<64hex>", "vout": 0, "value": 12345 },
      "state": "listed" | "pending_settlement" | "broadcast" | "expired" | "cancelled",
      "created_at": 1781…, "expires_at": 1781…,           // +7 days
      "settlement": null | { "signedTx": {…}, "buyer_addr": "txm1…", "ts": … },
      "broadcast_txid": null | "<64hex>"
    }
  }
}
```

State machine: `listed → pending_settlement → broadcast` (terminal `done`), with `listed/pending_settlement → expired|cancelled`. A listing accepts **one** in-flight settlement at a time (first buyer to POST a valid settlement wins the slot); if that settlement is not accepted before a `settlement_ttl` (e.g. 30 min) it reverts to `listed` so another buyer can try.

### 5.2 HTTP API

Public reads (GET, rate-limited):
- `GET /listings` → active `listed` listings (escaped on the client too).
- `GET /listing/:id` → one listing + state.
- `GET /pending?seller=<addr>` → listings in `pending_settlement` for that seller (for "My Sales").

Writes (POST, rate-limited, sig-authed where noted):
- `POST /listing` `{terms, seller_pubkey, sig}` — **sig-authed**. Relay verifies `sig` over the canonical terms string and that `hash(seller_pubkey) == seller_addr`; then validates against chain (§5.3); picks/validates the anchor UTXO; stores `listed`.
- `POST /quote` `{listing_id, buyer_addr}` — relay builds the **unsigned** settlement (§Component D), returns `{unsignedTx, summary, inputIndices:{buyer:[…], seller:[0]}}`. Pure/stateless (no mutation).
- `POST /settlement` `{listing_id, signedTx}` — relay re-derives the expected unsigned tx, checks only buyer inputs got signed and outputs/terms are unchanged (`verify_settlement` via subcommand), moves listing → `pending_settlement`, stores the buyer-signed tx.
- `POST /accept` `{listing_id, fullySignedTx, seller_pubkey, sig}` — **sig-authed** as the seller. Relay `verify_settlement` + confirms `input[0]` now signed, then `POST /sendrawtransaction` to the node. On success → `broadcast` + store txid (idempotent: if already `broadcast`, return the stored txid). On node "already in mempool"/"conflict", treat as success-ish and surface.
- `POST /cancel` `{listing_id, seller_pubkey, sig}` — **sig-authed** as the seller → `cancelled`.

### 5.3 Chain validation (every listing, on create and on prune)

Via tensorium-node RPC (`/getutxos/<seller_addr>`) and indexer (`/balance/<addr>`, `/asset/<id>`):
- anchor outpoint is a real, **mature, unspent** UTXO of `seller_addr`;
- seller's indexed balance of `asset_id` ≥ `amount` (fungible) or owns the NFT;
- asset exists (royalty terms fetched from indexer at quote time — tamper-proof, deterministic).

A background **prune timer** (e.g. every 60 s) drops listings that are expired, whose anchor got spent, or whose asset balance fell below `amount`.

### 5.4 Idempotency & crash-safety
- Mirror the OTC fix: persist state after each mutation (atomic write); broadcast keyed by `listing_id` with the stored `broadcast_txid` so a retry never double-submits.
- `verify_settlement` is run again immediately before broadcast — the relay never broadcasts a tx whose outputs/terms don't match the listing.

### 5.5 Deployment
New repo `github.com/tensorium-labs/tensorium-order-relay`; systemd `tensorium-order-relay.service` on `66.42.120.149`; nginx route under the marketplace vhost, e.g. `location /relay/ { proxy_pass http://127.0.0.1:<port>/; limit_req zone=txmapi; }` with `GET` open and `POST` allowed only on the relay path (separate from the GET-only `/api/` indexer path). Or a `relay.tensoriumlabs.com` subdomain — chosen at implementation time; default to the `/relay/` path to avoid a new cert.

---

## 6. Component D — txmwallet `asset-build-settlement` (keyless)

New subcommand in `crates/txmwallet/src/main.rs`:

```
txmwallet asset-build-settlement <order.json> <buyer_addr>   # no wallet, no passphrase
  → stdout JSON: { unsignedTx, terms (SettlementTerms), summary, inputIndices }
```

It reuses the exact `SettlementTerms` + `build_settlement_tx` + `verify_settlement` already used by `asset-buy` — but **takes the buyer address as an argument instead of a loaded wallet**, fetches buyer UTXOs from the node and royalty from the indexer, builds the tx, runs the self-`verify_settlement`, and emits the **unsigned** tx (no `sign_input` calls). `summary` is a human-readable `{price, miner_fee, royalty, asset, amount, seller, buyer}` for the wallet approval popup.

This keeps settlement math in one audited place; the relay shells out to it (and the Rust tests already cover `build_settlement_tx`/`verify_settlement`). Add unit tests for the new subcommand's argument parsing and that its output round-trips through `verify_settlement`.

---

## 7. Component C — wallet-extension provider additions

Two small, well-bounded methods on `window.tensorium` (each with an approval popup mirroring the existing `SignAssetTx` page; TDD with vitest):

1. **`signAssetTxPartial(unsignedTx, inputIndices, summary)`** → signs **only** `inputIndices` belonging to the active account, returns the partially-signed tx, and **does not broadcast**. (Today's `signAssetTx` signs all inputs and broadcasts — wrong for 2-of-2.) The approval popup shows `summary` (what the user pays/receives) and, as a hardening step, the popup recomputes key figures from the tx outputs so the user isn't trusting an attacker-supplied `summary`. Used by the buyer (their inputs) and the seller (`input[0]`).
2. **`signMessage(message)`** → returns `{ pubkey, sig }` for the active account over `message`. Used for listing-create / accept / cancel auth. The relay verifies `sig` and `hash(pubkey) == claimed seller_addr`.

`signAssetTx` (sign-all + broadcast) stays for single-signer mint/issue/transfer. No breaking change.

---

## 8. Component B — marketplace frontend (`web/marketplace`)

Replace the CLI `<code>` instruction blocks with wallet-native flows (keep the existing design system; **escape every rendered field** — reuse the `esc()` helper added in the XSS fix):

- **Connect** — `requestAccounts()` / `getAddress()`; show wrong-/no-wallet states (no `window.tensorium` → "Install the Tensorium wallet").
- **Browse** — `GET /relay/listings`, render real cards (ticker, price, amount, royalty) with a **Buy** button.
- **List an asset** (seller) — form (asset, amount, price) → `signMessage(canonical terms)` → `POST /relay/listing`. Pre-fill the seller's assets via `getAssets(addr)`.
- **Buy** (buyer) — `POST /relay/quote` → `signAssetTxPartial(unsignedTx, buyerIndices, summary)` → `POST /relay/settlement`; show progress (`quoted → signed → submitted → awaiting seller`).
- **My Sales** (seller) — `GET /relay/pending?seller=addr`: each pending settlement has **Accept** (`signAssetTxPartial(tx, [0], summary)` → `POST /relay/accept` → shows broadcast txid + explorer link) and each listing has **Cancel** (`signMessage` → `POST /relay/cancel`).

State refresh: poll listings + pending on an interval (and after each action), reflecting `pending_settlement`/`broadcast`. Mobile-responsive; all amounts formatted (atoms → TXM).

---

## 9. End-to-end data flow (happy path)

1. Seller: Connect → List → `signMessage` → `POST /relay/listing` → relay validates vs chain → `listed`.
2. Buyer: Browse → Buy → `POST /relay/quote` (relay builds unsigned settlement via §6) → wallet `signAssetTxPartial` buyer inputs → `POST /relay/settlement` (relay re-verifies) → `pending_settlement`.
3. Seller: My Sales shows it → Accept → wallet `signAssetTxPartial` `input[0]` → `POST /relay/accept` → relay `verify_settlement` + `/sendrawtransaction` → `broadcast` (txid shown to both). Asset moves; seller paid; platform fee + creator royalty paid — atomically or not at all.

## 10. Error handling
- Relay rejects any listing/settlement failing chain validation or `verify_settlement`, with a specific reason.
- Quote is stateless; a stale quote (anchor spent meanwhile) fails at `/settlement` re-validation, surfaced to the buyer to retry.
- Broadcast errors (`already in mempool`, `conflict`, node busy 5xx) handled like the bridge/OTC clients: transient 5xx retried, conflicts surfaced without losing state, idempotent on `broadcast_txid`.
- Wallet: user rejection, wrong account (signing indices not owned), timeout — all return clear provider errors to the dapp.

## 11. Testing strategy
- **Relay (TDD, `node:test`):** state machine transitions; chain-validation (mock node/indexer); expiry/prune; sig-verification (valid/forged/wrong-address); single-settlement-slot contention; idempotent + re-verified broadcast; never-broadcast-on-mismatch.
- **txmwallet subcommand (Rust):** arg parsing; unsigned output round-trips `verify_settlement`; reuses existing settlement tests.
- **Wallet (vitest):** `signAssetTxPartial` signs only requested indices and does not broadcast; `signMessage` returns recoverable pubkey; popup recomputes figures from tx.
- **Frontend:** manual happy-path + the documented edge cases (no wallet, rejected sign, stale listing); a light DOM test for escaped rendering.

## 12. Security summary
Non-custodial; sig-auth on list/accept/cancel; relay validates everything against node+indexer and re-`verify_settlement` before the only chain mutation (broadcast of a doubly-signed tx); all rendered fields escaped; nginx GET/POST split + rate-limiting; wallet recomputes approval figures from the tx (don't trust dapp-supplied summary); keys never leave the extension.

## 13. Non-goals (v1)
Unilateral fillable offers / one-click no-seller-online buy (needs a consensus change); auctions/offers/bidding; price charts; fiat; cross-chain; search/filter beyond a simple list; external notifications (Discord/email) for sellers — deferred to v2.

## 14. Milestones (each gets its own spec→plan→build)
- **M1 — Backend, headless & fully testable:** `tensorium-order-relay` + txmwallet `asset-build-settlement`. Deliverable: relay validating, building, storing, broadcasting against the live node/indexer, exercised by tests + curl (no UI).
- **M2 — Wallet:** `signAssetTxPartial` + `signMessage` + approval popups, released as a new extension version.
- **M3 — Frontend:** wallet-native Connect/Browse/List/Buy/My-Sales wired to M1+M2; deploy to `web/marketplace`.

Dependency order M1 → M2 → M3; M1 and M2 can proceed in parallel after M1's API contract is frozen.

## 15. Residual risk
- Seller must be online to complete a sale (protocol limit; mitigated to one click; v2 = auto-accept agent or sighash change).
- Buyer trusts that the listing's seller will accept promptly; settlement slot TTL prevents indefinite lockout but a griefing seller can ignore buys (mitigation: reputation/auto-accept in v2).
- New hot service surface (relay) — mitigated by non-custodial design (no keys, only re-verified signed-tx broadcast) + rate-limiting + chain re-validation.
