# Marketplace — On-chain Create-Asset + Brand Rebrand (Design)

**Date:** 2026-06-13 · **Status:** approved → implementing
**Repos:** `tensorium-core` (txmwallet + `web/marketplace`), `tensorium-order-relay`

## Goal
Let any developer/user create a **TXM20 token** or an **NFT (with creator royalty)** from the marketplace, minted **on-chain** via their wallet, through a clean, non-confusing UI — and rebrand the marketplace to the official Tensorium dark/cyan brand.

## Decisions
- Palette: official brand — dark `#05070D`, cyan `#22D3EE`/`#67E8F9`, green `#34D399` (success), text `#F3F6FB`, muted `#5C6C84`.
- Create UX: a dedicated **Create** section on `marketplace.tensoriumlabs.com` with **Token | NFT** tabs (alongside Connect/Browse/List/My-Sales).
- On-chain mechanism: relay builds the unsigned tx (keyless txmwallet subcommand) → `window.tensorium.signAssetTx(unsignedTx, summary)` signs **and broadcasts** → minted on-chain.

## Components

### A. txmwallet keyless builders (`crates/txmwallet/src/main.rs`)
Refactor the existing `asset-issue`/`asset-mint` arms to factor a keyless builder (no wallet/passphrase; take `creator_addr`, fetch its mature UTXOs for the fee), mirroring `build_unsigned_settlement`:
- `asset-build-issue <ticker> <decimals> <supply> <name...> <creator_addr>` → prints `{ tx, summary }` (unsigned). `summary = {action:"issue", ticker, decimals, supply, name, fee_atoms}`.
- `asset-build-mint <royalty_bps> <royalty_addr> <content_hash_hex> <uri> <creator_addr>` → prints `{ tx, summary }`. `summary = {action:"mint", royalty_bps, royalty_addr, uri, content_hash, fee_atoms}`.

Both reuse the same `AssetOp` encoding + output layout the signed commands already use (the only change: inputs come from `fetch_mature_utxos(creator_addr)` and are left **unsigned**; no `sign_input`). Validation unchanged (royalty_bps ≤ 10000, content_hash 32 bytes, ticker non-empty).

### B. relay endpoints (`tensorium-order-relay`)
- `POST /relay/build-issue` `{ticker, decimals, supply, name, creator_addr}` → run subcommand → `{unsignedTx, summary}`.
- `POST /relay/build-mint` `{royalty_bps, royalty_addr, content_hash, uri, creator_addr}` → `{unsignedTx, summary}`.
- Keyless/public (building is harmless; only the creator's wallet signature broadcasts it). Input-validated (addr regex, royalty 0–10000, decimals 0–18, positive supply, 64-hex content_hash). New `build-asset.js` shells to txmwallet (argv, no shell) — sibling of `build.js`. nginx `/relay/` already allows POST.

### C. frontend (`web/marketplace/index.html` + `marketplace.js`)
- **Rebrand:** rewrite the CSS theme block to the brand palette; apply to every section + the wallet-native UI classes. Keep all escaping.
- **Create section** (Token | NFT tabs):
  - Token form: ticker, decimals (default 8), supply, name.
  - NFT form: name, media URI (ipfs://… or https), content file → sha256 (computed in-browser via SubtleCrypto), royalty % (0–100 → bps) + royalty address (defaults to connected wallet).
  - Flow: validate → `POST /relay/build-issue|build-mint` → `window.tensorium.signAssetTx(unsignedTx, summary)` → on confirm, poll `/api/asset/<id>` until indexed → success card (asset_id + explorer link + "now in Browse").
  - Plain-language helpers, a live preview card, and a 4-step indicator (Fill → Review → Sign in wallet → Minted ✓). Disabled states when no wallet.
- New `marketplace.js` pure helpers (tested): `royaltyPctToBps`, `sha256Hex(file)` wrapper signature, `buildIssueParams(form)`, `buildMintParams(form)`, `createTokenFlow`/`createNftFlow` (injectable `{wallet, api}`).

## Data flow (create token)
Connect → Token tab → fill → `POST /relay/build-issue` → `{unsignedTx, summary}` → `window.tensorium.signAssetTx` (wallet shows summary, signs+broadcasts) → poll `/api/asset/<asset_id>` → success. Asset now in `/api/assets` (Browse).

## Error handling
- No wallet → disable Create with "Install the Tensorium wallet".
- Builder errors (insufficient creator funds, bad params) → surfaced inline.
- Wallet reject / node reject → clear message; nothing left half-done (issue/mint is a single broadcast).
- Indexer lag → poll with timeout, then "minted — appearing shortly".

## Testing
- Rust: `build_unsigned_issue`/`build_unsigned_mint` pure fns — output round-trips `extract_asset_op` + is unsigned; reuse existing asset tests.
- Relay: `build-asset.js` (injected exec) + endpoint validation (node:test).
- Frontend: pure helpers (royalty bps, param builders, escaping) unit-tested; manual + live on-chain (create a token + NFT → confirm in catalog).

## Non-goals
Editing/burning assets; collections/batch mint; file hosting/IPFS pinning (user supplies the URI); marketplace-side media rendering beyond ticker/name.

## Milestones
1. Backend: txmwallet builders + relay endpoints (headless, testable) + deploy.
2. Frontend: rebrand + Create UI wired to the endpoints + deploy + live verify.
