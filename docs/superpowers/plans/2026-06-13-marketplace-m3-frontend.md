# Marketplace M3 — Wallet-Native Frontend — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`).

**Goal:** Replace the static CLI-instruction marketplace with wallet-native flows — Connect, Browse, List, Buy, My-Sales — wired to the live order-relay (`/relay/*`) and the wallet provider (`window.tensorium`), then deploy and run the live end-to-end trade.

**Architecture:** Extract the logic into a testable ES module `web/marketplace/marketplace.js` (pure helpers — canonical messages, formatting, escaped card rendering — are unit-tested with `node:test`; the wallet/network flows call `window.tensorium` + `fetch('/relay/...')`). `index.html` becomes the UI shell that imports the module. No build step (static files served from `/var/www/marketplace`).

**Tech Stack:** Vanilla ES modules + `window.tensorium` provider + the relay REST API. Repo: `tensorium-core` (`web/marketplace/`). Spec: `docs/superpowers/specs/2026-06-13-marketplace-wallet-native-trading-design.md` (Component B). Depends on M1 (relay, LIVE) + M2 (wallet v0.1.8, shipped).

## Verified contracts (must match exactly)

**Relay (`api-server.js`) canonical signed messages:**
- listing: `list:${asset_id_hex}:${amount}:${price_atoms}:${seller_addr}`
- accept: `accept:${listing_id}`
- cancel: `cancel:${listing_id}`

**Relay endpoints:**
- `GET /relay/listings` → `{listings:[{listing_id,asset_id_hex,kind,amount,price_atoms,seller_addr,anchor,state,...}]}`
- `GET /relay/pending?seller=<addr>` → `{listings:[{...,settlement:{signedTx,buyer_addr,ts}}]}`
- `POST /relay/listing` `{terms:{asset_id_hex,amount,price_atoms,seller_addr,kind}, seller_pubkey, sig}` → listing
- `POST /relay/quote` `{listing_id, buyer_addr}` → `{listing_id, unsignedTx, summary, input_indices:{seller:[0],buyer:[1,...]}}`
- `POST /relay/settlement` `{listing_id, signedTx, buyer_addr}` → listing (pending_settlement)
- `POST /relay/accept` `{listing_id, fullySignedTx, seller_pubkey, sig}` → listing (broadcast, has `broadcast_txid`)
- `POST /relay/cancel` `{listing_id, seller_pubkey, sig}` → `{cancelled:true}`

**Wallet provider (`window.tensorium`, v0.1.8):**
- `requestAccounts() → [address]`, `getAddress() → address`, `getAssets(addr) → {fungible,nfts}`
- `signMessage(message) → {pubkey, sig}`
- `signAssetTxPartial(unsignedTx, inputIndices, summary) → partiallySignedTx`

---

## Task 1: Pure helpers module + tests (TDD)

**Files:** Create `web/marketplace/marketplace.js`; `web/marketplace/test/marketplace.test.js`

- [ ] **Step 1: Write the failing test** (`web/marketplace/test/marketplace.test.js`)

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { esc, fmtAtoms, listingMessage, acceptMessage, cancelMessage, renderListingCard } from "../marketplace.js";

describe("esc", () => {
  it("neutralizes HTML so a malicious ticker cannot inject script", () => {
    assert.equal(esc(`<img src=x onerror=alert(1)>`), "&lt;img src=x onerror=alert(1)&gt;");
    assert.equal(esc("GOLD"), "GOLD");
  });
});

describe("fmtAtoms", () => {
  it("formats atoms as TXM with 8 decimals", () => {
    assert.equal(fmtAtoms(123_45678900), "123.45678900");
    assert.equal(fmtAtoms(0), "0.00000000");
  });
});

describe("canonical messages (must match relay api-server.js)", () => {
  it("listingMessage", () => {
    assert.equal(listingMessage({ asset_id_hex: "aa", amount: 100, price_atoms: 5000000, seller_addr: "txm1s" }),
      "list:aa:100:5000000:txm1s");
  });
  it("acceptMessage / cancelMessage", () => {
    assert.equal(acceptMessage("lst_1"), "accept:lst_1");
    assert.equal(cancelMessage("lst_1"), "cancel:lst_1");
  });
});

describe("renderListingCard", () => {
  it("escapes user fields and includes price + a Buy button bound to the listing id", () => {
    const html = renderListingCard({ listing_id: "lst_x", asset_id_hex: "ab".repeat(32), kind: "txm20", amount: 100, price_atoms: 5000000, seller_addr: "txm1s", ticker: "<b>X</b>" });
    assert.ok(html.includes("&lt;b&gt;X&lt;/b&gt;"));        // ticker escaped
    assert.ok(!html.includes("<b>X</b>"));
    assert.ok(html.includes("data-listing=\"lst_x\""));      // buy button target
    assert.ok(html.includes("0.05000000"));                  // price in TXM
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd web/marketplace && node --test test/marketplace.test.js` → FAIL (no module).

- [ ] **Step 3: Implement the pure helpers in `web/marketplace/marketplace.js`**

```js
// ── Pure helpers (unit-tested) ──────────────────────────────────────────────
export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

export const fmtAtoms = (a) =>
  `${Math.floor(Number(a) / 1e8)}.${(Number(a) % 1e8).toString().padStart(8, "0")}`;

// MUST match relay api-server.js listMsg/actMsg exactly.
export const listingMessage = (t) => `list:${t.asset_id_hex}:${t.amount}:${t.price_atoms}:${t.seller_addr}`;
export const acceptMessage = (id) => `accept:${id}`;
export const cancelMessage = (id) => `cancel:${id}`;

const shortId = (h) => (h ? `${h.slice(0, 8)}…${h.slice(-6)}` : "");

export function renderListingCard(l) {
  return `
    <div class="card asset-card">
      <div class="tick">${esc(l.ticker || "NFT")} <span class="tag">${l.kind === "nft" ? "NFT" : "TXM20"}</span></div>
      <div class="id">${esc(shortId(l.asset_id_hex))}</div>
      <div class="row"><span>Amount</span><span>${esc(String(l.amount))}</span></div>
      <div class="row"><span>Price</span><span>${fmtAtoms(l.price_atoms)} TXM</span></div>
      <div class="row"><span>Seller</span><span>${esc(shortId(l.seller_addr))}</span></div>
      <button class="wallet-btn buy-btn" data-listing="${esc(l.listing_id)}" data-price="${esc(String(l.price_atoms))}">Buy</button>
    </div>`;
}
```

- [ ] **Step 4: Run it, verify it passes** → `node --test test/marketplace.test.js` PASS.

- [ ] **Step 5: Commit**

```bash
git add web/marketplace/marketplace.js web/marketplace/test/marketplace.test.js
git commit -m "feat(marketplace): testable pure helpers — esc, fmtAtoms, canonical messages, card render"
```

---

## Task 2: Wallet + relay flow functions (in `marketplace.js`)

**Files:** Modify `web/marketplace/marketplace.js`; Test: extend `web/marketplace/test/marketplace.test.js`

The flows are injectable (`{ wallet, api }`) so they unit-test without a browser. `wallet` = a `window.tensorium`-shaped object; `api` = `(path, body?) => Promise<json>`.

- [ ] **Step 1: Write failing tests** (append)

```js
import { connectWallet, createListing, buyListing, acceptSale, cancelListing } from "../marketplace.js";

const fakeWallet = (over = {}) => ({
  requestAccounts: async () => ["txm1seller00000000000000000000000000000000"],
  getAddress: async () => "txm1seller00000000000000000000000000000000",
  signMessage: async (m) => ({ pubkey: "03aa", sig: "30deadbeef", _msg: m }),
  signAssetTxPartial: async (tx, idx) => ({ ...tx, _signedIndices: idx }),
  ...over,
});
const fakeApi = (routes) => async (path, body) => {
  const r = routes[path.split("?")[0]];
  return typeof r === "function" ? r(body) : r;
};

describe("createListing", () => {
  it("signs the canonical listing message and POSTs terms+pubkey+sig", async () => {
    let posted;
    const api = fakeApi({ "/relay/listing": (b) => { posted = b; return { listing_id: "lst_1", state: "listed" }; } });
    const wallet = fakeWallet();
    const out = await createListing({ asset_id_hex: "aa", amount: 100, price_atoms: 5000000, kind: "txm20" }, { wallet, api });
    assert.equal(out.listing_id, "lst_1");
    assert.equal(posted.sig, "30deadbeef");
    assert.equal(posted.seller_pubkey, "03aa");
    assert.equal(posted.terms.seller_addr, "txm1seller00000000000000000000000000000000");
  });
});

describe("buyListing", () => {
  it("quotes, partial-signs the buyer inputs, and posts the settlement", async () => {
    let settle;
    const api = fakeApi({
      "/relay/quote": { listing_id: "lst_1", unsignedTx: { inputs: [{}, {}] }, summary: {}, input_indices: { seller: [0], buyer: [1] } },
      "/relay/settlement": (b) => { settle = b; return { state: "pending_settlement" }; },
    });
    const wallet = fakeWallet();
    const out = await buyListing("lst_1", { wallet, api });
    assert.equal(out.state, "pending_settlement");
    assert.deepEqual(settle.signedTx._signedIndices, [1]); // buyer inputs signed
    assert.equal(settle.listing_id, "lst_1");
  });
});

describe("acceptSale", () => {
  it("signs input[0], signs the accept message, and posts both", async () => {
    let acc;
    const api = fakeApi({ "/relay/accept": (b) => { acc = b; return { state: "broadcast", broadcast_txid: "beef" }; } });
    const wallet = fakeWallet();
    const out = await acceptSale({ listing_id: "lst_1", settlement: { signedTx: { inputs: [{}, {}] } } }, { wallet, api });
    assert.equal(out.broadcast_txid, "beef");
    assert.deepEqual(acc.fullySignedTx._signedIndices, [0]);
    assert.equal(acc.sig, "30deadbeef");
  });
});

describe("cancelListing", () => {
  it("signs cancel:<id> and posts", async () => {
    let c;
    const api = fakeApi({ "/relay/cancel": (b) => { c = b; return { cancelled: true }; } });
    const out = await cancelListing("lst_1", { wallet: fakeWallet(), api });
    assert.equal(out.cancelled, true);
    assert.equal(c.listing_id, "lst_1");
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement the flows** (append to `marketplace.js`)

```js
// ── Wallet + relay flows (deps injected: { wallet, api }) ────────────────────
export async function connectWallet({ wallet }) {
  const accts = await wallet.requestAccounts();
  if (!accts || !accts.length) throw new Error("No account in wallet");
  return accts[0];
}

export async function createListing(form, { wallet, api }) {
  const seller_addr = await wallet.getAddress();
  const terms = { asset_id_hex: form.asset_id_hex, amount: Number(form.amount), price_atoms: Number(form.price_atoms), seller_addr, kind: form.kind };
  const { pubkey, sig } = await wallet.signMessage(listingMessage(terms));
  return api("/relay/listing", { terms, seller_pubkey: pubkey, sig });
}

export async function buyListing(listing_id, { wallet, api }) {
  const buyer_addr = await wallet.getAddress();
  const quote = await api("/relay/quote", { listing_id, buyer_addr });
  const signedTx = await wallet.signAssetTxPartial(quote.unsignedTx, quote.input_indices.buyer, quote.summary);
  return api("/relay/settlement", { listing_id, signedTx, buyer_addr });
}

export async function acceptSale(listing, { wallet, api }) {
  const fullySignedTx = await wallet.signAssetTxPartial(listing.settlement.signedTx, [0], { description: "Accept sale" });
  const { pubkey, sig } = await wallet.signMessage(acceptMessage(listing.listing_id));
  return api("/relay/accept", { listing_id: listing.listing_id, fullySignedTx, seller_pubkey: pubkey, sig });
}

export async function cancelListing(listing_id, { wallet, api }) {
  const { pubkey, sig } = await wallet.signMessage(cancelMessage(listing_id));
  return api("/relay/cancel", { listing_id, seller_pubkey: pubkey, sig });
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS (all helper + flow tests).

- [ ] **Step 5: Commit**

```bash
git add web/marketplace/marketplace.js web/marketplace/test/marketplace.test.js
git commit -m "feat(marketplace): wallet+relay flows — connect, list, buy, accept, cancel (injectable, tested)"
```

---

## Task 3: `index.html` UI shell wired to the flows

**Files:** Modify `web/marketplace/index.html`

Replace the CLI `<code>` instruction blocks with the wallet-native UI. Keep the existing design-system classes. The inline script imports `marketplace.js` and binds the DOM.

- [ ] **Step 1: Replace the trade-instruction section + script.** Keep `loadStats`/`loadAssets` catalog. Add:
  - A header **Connect Wallet** button: on click `connectWallet({wallet:window.tensorium})`; on success show the address + reveal List/My-Sales; on missing `window.tensorium` show "Install the Tensorium wallet" with a link.
  - **Browse**: `loadListings()` → `GET /relay/listings` → render `renderListingCard` into a grid; delegate clicks on `.buy-btn` → `buyListing(id, {wallet,api})` with a status toast (`quoting → signing → submitted → awaiting seller`).
  - **List an asset** form (asset_id, amount, price in TXM→atoms) → `createListing(form,{wallet,api})`; refresh listings.
  - **My Sales**: `GET /relay/pending?seller=<addr>` → render each with **Accept** (`acceptSale(listing,{wallet,api})` → show `broadcast_txid` + explorer link) and **Cancel** (`cancelListing(id,{wallet,api})`).
  - `const api = async (path, body) => { const r = await fetch(path, body ? {method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)} : undefined); const j = await r.json().catch(()=>({})); if(!r.ok) throw new Error(j.error || ('HTTP '+r.status)); return j; };` with `path` prefixed `/relay`.
  - All dynamic text via `esc`/`fmtAtoms`. Wallet missing → disable List/Buy/My-Sales with a clear prompt.

  Provide the full replacement `<script type="module">` importing from `./marketplace.js` and the corresponding HTML sections. Mobile-responsive (reuse existing grid CSS).

- [ ] **Step 2: Lint check (no build)** — `node --check` won't parse HTML; instead verify the module imports resolve: `cd web/marketplace && node -e "import('./marketplace.js').then(m=>console.log(Object.keys(m).join(',')))"` lists all exports. Open `index.html` structure review: ensure every dynamic insertion uses `esc`/`fmtAtoms`.

- [ ] **Step 3: Commit**

```bash
git add web/marketplace/index.html
git commit -m "feat(marketplace): wallet-native UI — connect, browse, list, buy, my-sales"
```

---

## Task 4: Deploy + live end-to-end (gated, manual)

**Host:** `66.42.120.149`. Static files only; the relay + wallet are already live.

- [ ] **Step 1: Push**

```bash
git push origin main
```

- [ ] **Step 2: Deploy static files** (backup first)

```bash
ssh root@66.42.120.149 'cp -r /var/www/marketplace /root/marketplace.bak_$(date +%Y%m%d-%H%M%S)'
scp web/marketplace/index.html web/marketplace/marketplace.js root@66.42.120.149:/var/www/marketplace/
```

- [ ] **Step 3: Smoke test (public)**

```bash
curl -s -o /dev/null -w "%{http_code}\n" https://marketplace.tensoriumlabs.com/                  # 200
curl -s -o /dev/null -w "%{http_code}\n" https://marketplace.tensoriumlabs.com/marketplace.js    # 200
curl -s https://marketplace.tensoriumlabs.com/relay/listings                                     # {"listings":[]}
```

- [ ] **Step 4: Live end-to-end (browser, manual QA — documents the full path)**
  1. Load the v0.1.8 unpacked wallet in Chrome; create/import a funded mainnet wallet.
  2. On marketplace: **Connect** → address shows.
  3. **List** an owned asset → wallet `signMessage` popup → approve → listing appears in Browse.
  4. From a second wallet/account: **Buy** → wallet `signAssetTxPartial` popup (shows recomputed payouts) → approve → status "awaiting seller".
  5. First wallet: **My Sales** → **Accept** → `signAssetTxPartial` input[0] popup → approve → `broadcast_txid` shown; verify the tx on the explorer (asset moved, seller paid, fee + royalty paid).
  6. Test **Cancel** on a fresh listing; test edge cases: no wallet installed, user rejects a popup, a stale listing (anchor spent).

- [ ] **Step 5: Record outcome** in project memory (UI live, e2e result, any issues) and close out M1–M3.

---

## Self-review

- **Spec coverage (Component B):** Connect (Task 2 `connectWallet` + Task 3 button), Browse (Task 1 `renderListingCard` + Task 3 `loadListings`), List (`createListing`), Buy (`buyListing`), My-Sales accept/cancel (`acceptSale`/`cancelListing`), escaped rendering (`esc` everywhere), mobile (existing CSS) — all covered. Wallet-missing/edge states in Task 3.
- **Contract fidelity:** canonical messages (`listingMessage`/`acceptMessage`/`cancelMessage`) match the relay's `listMsg`/`actMsg`; request bodies match each endpoint; `signAssetTxPartial` receives `input_indices.buyer` (buy) / `[0]` (accept) per the 2-of-2 model.
- **Placeholder scan:** Tasks 1–2 are full code + tests; Task 3 specifies every section + the `api` helper + escaping rule (the implementer writes the HTML against the existing index.html, which already has the catalog + design system). Task 4 is manual deploy/QA.
- **Type consistency:** flow signatures `({wallet, api})`; `createListing(form,…)`, `buyListing(id,…)`, `acceptSale(listing,…)`, `cancelListing(id,…)`; helpers `esc/fmtAtoms/listingMessage/acceptMessage/cancelMessage/renderListingCard`.

## Residual
- Static frontend flows are unit-tested at the pure/flow layer (injected wallet+api); the real-browser click-through is manual QA (Task 4 step 4) — there is no headless wallet-extension harness.
- The end-to-end live trade requires a funded seller + buyer and an existing on-chain asset; if none exists, mint one via `txmwallet asset-issue` first.
