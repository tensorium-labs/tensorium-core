# Marketplace M1 — Order-Relay + Keyless Settlement Builder — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the headless, fully-testable backend for wallet-native marketplace trading: a non-custodial `tensorium-order-relay` Node service plus a keyless `txmwallet asset-build-settlement` subcommand.

**Architecture:** The relay stores public listings and buyer/seller-signed settlements, validates everything against the live tensorium-node + indexer, builds the *unsigned* settlement by shelling out to `txmwallet asset-build-settlement` (reusing the audited Rust `build_settlement_tx`/`verify_settlement`), and broadcasts only fully-signed transactions. No private keys ever touch the relay.

**Tech Stack:** Node 22 ESM + Express 5 + `node:test` (relay, mirrors `tensorium-otc-watcher`); `@noble/secp256k1` + `bech32` for signature auth; Rust (txmwallet subcommand). Spec: `docs/superpowers/specs/2026-06-13-marketplace-wallet-native-trading-design.md`.

---

## File structure

**tensorium-core (Rust subcommand):**
- Modify `crates/txmwallet/src/main.rs` — add `build_unsigned_settlement()` pure helper + `asset-build-settlement` match arm + help line.

**tensorium-order-relay (new repo `github.com/tensorium-labs/tensorium-order-relay`):**
- `package.json`, `.gitignore`, `.env.example`, `deposits/`→`data/` (state dir)
- `sig.js` — verify secp256k1 signature + derive txm address from pubkey (pure)
- `order-state.js` — pure listing state-machine transitions
- `state.js` — atomic JSON persistence
- `node-client.js` — tensorium-node RPC (`/getutxos`, `/sendrawtransaction`)
- `indexer-client.js` — indexer (`/balance`, `/asset`)
- `validate.js` — listing chain-validation
- `build.js` — shell out to `txmwallet asset-build-settlement`
- `api-server.js` — Express routes
- `prune.js` — periodic invalidation/expiry sweep
- `index.js` — wiring + required-env check
- `deploy/tensorium-order-relay.service`, `deploy/nginx-relay.conf`, `README.md`
- `test/*.test.js`

---

## Task R1: Keyless `asset-build-settlement` subcommand (Rust)

**Files:**
- Modify: `crates/txmwallet/src/main.rs` (near the `asset-buy` arm ~line 393, and help ~line 1240)
- Test: `crates/txmwallet/src/main.rs` `#[cfg(test)]` module

Context: `asset-buy` (lines 393–477) already builds the settlement. We factor its core into a **keyless pure function** that takes explicit inputs (no wallet, no RPC) so it is unit-testable, then add a thin subcommand that fetches buyer UTXOs + royalty and calls it, emitting the **unsigned** tx.

- [ ] **Step 1: Write the failing test**

Add to the txmwallet test module:

```rust
#[test]
fn build_unsigned_settlement_produces_verifiable_unsigned_tx() {
    use tensorium_core::block::OutPoint;
    use tensorium_core::hash::Hash256;
    let order = AssetOrder {
        asset_id_hex: "11".repeat(32),
        amount: 100,
        price_atoms: 1_000_000,
        seller_addr: "txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p".into(),
        seller_txid_hex: "22".repeat(32),
        seller_vout: 0,
        seller_value: 50_000,
    };
    let buyer_addr = "txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p".to_string();
    // one buyer UTXO big enough: price + CARRIER + miner_fee
    let buyer_utxos = vec![(
        OutPoint { txid: Hash256([3u8; 32]), output_index: 1 },
        2_000_000u64,
    )];
    let out = build_unsigned_settlement(&order, &buyer_addr, &buyer_utxos, 250, &buyer_addr)
        .expect("build ok");
    // No inputs are signed yet.
    assert!(out.tx.inputs.iter().all(|i| i.script_sig.is_empty()));
    // It must pass the canonical verifier against its own terms.
    assert!(verify_settlement(&out.tx, &out.terms).is_empty());
    // Index map: seller=[0], buyer=1..n
    assert_eq!(out.input_indices.seller, vec![0]);
    assert_eq!(out.input_indices.buyer, (1..out.tx.inputs.len()).collect::<Vec<_>>());
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo test -p txmwallet build_unsigned_settlement -- --nocapture`
Expected: FAIL — `build_unsigned_settlement` / `InputIndices` not found.

- [ ] **Step 3: Implement the pure helper + types**

Add near `AssetOrder` (after line 82):

```rust
#[derive(serde::Serialize)]
struct InputIndices { seller: Vec<usize>, buyer: Vec<usize> }

#[derive(serde::Serialize)]
struct UnsignedSettlement {
    tx: tensorium_core::block::Transaction,
    terms: SettlementTerms,
    input_indices: InputIndices,
}

/// Keyless: build the UNSIGNED settlement tx from an order + explicit buyer
/// inputs + royalty terms. Reuses the canonical build/verify so it can never
/// drift from consensus. No signing, no I/O.
fn build_unsigned_settlement(
    order: &AssetOrder,
    buyer_addr: &str,
    buyer_inputs: &[(tensorium_core::block::OutPoint, u64)],
    royalty_bps: u16,
    royalty_addr: &str,
) -> Result<UnsignedSettlement, String> {
    let asset_id: [u8; 32] = hex::decode(&order.asset_id_hex)
        .map_err(|_| "bad asset_id hex".to_owned())?
        .as_slice().try_into().map_err(|_| "asset_id must be 32 bytes".to_owned())?;
    let terms = SettlementTerms {
        asset_id, amount: order.amount, price_atoms: order.price_atoms,
        royalty_bps, royalty_addr: royalty_addr.to_owned(),
        seller_addr: order.seller_addr.clone(), buyer_addr: buyer_addr.to_owned(),
        miner_fee_atoms: tensorium_core::mempool::MIN_RELAY_FEE_ATOMS,
    };
    let seller_txid = tensorium_core::hash::Hash256(
        hex::decode(&order.seller_txid_hex).map_err(|_| "bad seller txid hex".to_owned())?
            .as_slice().try_into().map_err(|_| "seller txid must be 32 bytes".to_owned())?);
    let seller_input = (
        tensorium_core::block::OutPoint { txid: seller_txid, output_index: order.seller_vout },
        order.seller_value,
    );
    let tx = build_settlement_tx(&terms, seller_input, buyer_inputs)?;
    let mismatches = verify_settlement(&tx, &terms);
    if !mismatches.is_empty() {
        return Err(format!("built settlement failed verify: {mismatches:?}"));
    }
    let buyer = (1..tx.inputs.len()).collect();
    Ok(UnsignedSettlement { tx, terms, input_indices: InputIndices { seller: vec![0], buyer } })
}
```

- [ ] **Step 4: Run it, verify it passes**

Run: `cargo test -p txmwallet build_unsigned_settlement`
Expected: PASS.

- [ ] **Step 5: Add the subcommand arm + help**

Add a match arm before `asset-buy` (~line 393):

```rust
"asset-build-settlement" => {
    // usage: txmwallet asset-build-settlement <order.json> <buyer_addr>   (KEYLESS)
    let order_path = args.get(2).map(PathBuf::from).ok_or("usage: txmwallet asset-build-settlement <order.json> <buyer_addr>")?;
    let buyer_addr = args.get(3).ok_or("missing buyer_addr")?.to_string();
    let order: AssetOrder = serde_json::from_str(
        &fs::read_to_string(&order_path).map_err(|e| format!("read order: {e}"))?)
        .map_err(|e| format!("parse order: {e}"))?;
    let rpc = env::var("TENSORIUM_RPC").unwrap_or_else(|_| DEFAULT_RPC.to_owned());
    let indexer = env::var("TENSORIUM_INDEXER").unwrap_or_else(|_| "127.0.0.1:23340".to_owned());
    #[derive(serde::Deserialize)]
    struct AssetInfoResp { royalty_bps: u16, royalty_addr: String }
    let info_body = rpc_get(&indexer, &format!("/asset/{}", order.asset_id_hex))
        .map_err(|e| format!("indexer /asset lookup failed: {e}"))?;
    let info: AssetInfoResp = serde_json::from_str(&info_body).map_err(|e| format!("parse asset info: {e}"))?;
    let need = order.price_atoms + CARRIER_ATOMS + tensorium_core::mempool::MIN_RELAY_FEE_ATOMS;
    let mut buyer_inputs = Vec::new();
    let mut total = 0u64;
    for (op, v) in fetch_mature_utxos(&rpc, &buyer_addr)? {
        buyer_inputs.push((op, v)); total += v;
        if total >= need { break; }
    }
    if total < need { return Err(format!("insufficient buyer funds: have {total}, need {need}")); }
    let out = build_unsigned_settlement(&order, &buyer_addr, &buyer_inputs, info.royalty_bps, &info.royalty_addr)?;
    println!("{}", serde_json::to_string(&out).map_err(|e| format!("serialize: {e}"))?);
}
```

Add to the help block (~line 1241):

```rust
    println!("  asset-build-settlement <order.json> <buyer_addr>      build UNSIGNED settlement (keyless, for relay)");
```

- [ ] **Step 6: Build + commit**

```bash
cargo build -p txmwallet && cargo test -p txmwallet
git add crates/txmwallet/src/main.rs
git commit -m "feat(txmwallet): keyless asset-build-settlement subcommand for the order-relay"
```

---

## Task 1: Scaffold the relay repo

**Files:** Create `tensorium-order-relay/{package.json,.gitignore,.env.example,data/.gitkeep}`

- [ ] **Step 1: Create the directory and files**

```bash
mkdir -p /root/.openclaw/workspace/tensorium-order-relay/{data,test}
cd /root/.openclaw/workspace/tensorium-order-relay
touch data/.gitkeep
```

`package.json`:

```json
{
  "name": "tensorium-order-relay",
  "version": "0.1.0",
  "type": "module",
  "description": "Non-custodial order relay for the Tensorium asset marketplace",
  "main": "index.js",
  "scripts": { "start": "node index.js", "test": "node --test" },
  "dependencies": { "express": "^5.2.1", "dotenv": "^17.0.0", "@noble/secp256k1": "^2.1.0", "bech32": "^2.0.0" }
}
```

`.gitignore`:

```
node_modules/
.env
data/*.json
*.log
```

`.env.example`:

```
RELAY_API_PORT=3006
TENSORIUM_RPC=https://rpc.tensoriumlabs.com
TENSORIUM_RPC_LOCAL=127.0.0.1:33332
INDEXER_URL=http://127.0.0.1:23340
TXMWALLET_BIN=/usr/local/bin/txmwallet
LISTING_TTL_MS=604800000
SETTLEMENT_TTL_MS=1800000
PRUNE_INTERVAL_MS=60000
```

- [ ] **Step 2: Install deps + commit**

```bash
cd /root/.openclaw/workspace/tensorium-order-relay && npm install
git init -q && git add -A && git commit -q -m "chore: scaffold tensorium-order-relay"
```

---

## Task 2: `sig.js` — signature auth (pure, TDD)

**Files:** Create `sig.js`, `test/sig.test.js`

- [ ] **Step 1: Write the failing test** (`test/sig.test.js`)

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import * as secp from "@noble/secp256k1";
import { createHash } from "node:crypto";
import { addressFromPubkey, verifyOwnership } from "../sig.js";

// Real mainnet vector (public): pubkey -> address (sha256(pubkey)[..20] bech32 'txm').
const PUB = "03a2d569b999c6e1c220f1a3914e10e46f0d53b58cc9e033500a4442ea451bb28d";
const ADDR = "txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p";

describe("addressFromPubkey", () => {
  it("derives the canonical txm address from a compressed pubkey", () => {
    assert.equal(addressFromPubkey(PUB), ADDR);
  });
});

describe("verifyOwnership", () => {
  it("accepts a valid signature whose pubkey hashes to the claimed address", () => {
    const priv = secp.utils.randomPrivateKey();
    const pub = Buffer.from(secp.getPublicKey(priv, true)).toString("hex");
    const addr = addressFromPubkey(pub);
    const msg = "list:" + "11".repeat(32) + ":100:1000000";
    const h = createHash("sha256").update(msg).digest();
    const sig = secp.sign(h, priv).toDERHex();
    assert.equal(verifyOwnership({ message: msg, pubkey: pub, sig, address: addr }), true);
  });

  it("rejects a signature over a different message", () => {
    const priv = secp.utils.randomPrivateKey();
    const pub = Buffer.from(secp.getPublicKey(priv, true)).toString("hex");
    const addr = addressFromPubkey(pub);
    const h = createHash("sha256").update("real").digest();
    const sig = secp.sign(h, priv).toDERHex();
    assert.equal(verifyOwnership({ message: "tampered", pubkey: pub, sig, address: addr }), false);
  });

  it("rejects when the pubkey does not hash to the claimed address", () => {
    const priv = secp.utils.randomPrivateKey();
    const pub = Buffer.from(secp.getPublicKey(priv, true)).toString("hex");
    const msg = "x";
    const sig = secp.sign(createHash("sha256").update(msg).digest(), priv).toDERHex();
    assert.equal(verifyOwnership({ message: msg, pubkey: pub, sig, address: "txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p" }), false);
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `node --test test/sig.test.js`
Expected: FAIL — cannot import `../sig.js`.

- [ ] **Step 3: Implement `sig.js`**

```js
import * as secp from "@noble/secp256k1";
import { bech32 } from "bech32";
import { createHash } from "node:crypto";

// address = bech32('txm', sha256(compressed_pubkey)[..20])  — matches wallet.rs Address::from_public_key
export function addressFromPubkey(pubkeyHex) {
  const pub = Buffer.from(pubkeyHex, "hex");
  const h = createHash("sha256").update(pub).digest().subarray(0, 20);
  return bech32.encode("txm", bech32.toWords(h));
}

// Verify a secp256k1 DER signature over sha256(message) and that the signer's
// pubkey hashes to the claimed address. Returns true/false (never throws).
export function verifyOwnership({ message, pubkey, sig, address }) {
  try {
    if (addressFromPubkey(pubkey) !== address) return false;
    const h = createHash("sha256").update(String(message)).digest();
    return secp.verify(sig, h, pubkey);
  } catch { return false; }
}
```

- [ ] **Step 4: Run it, verify it passes**

Run: `node --test test/sig.test.js`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add sig.js test/sig.test.js && git commit -q -m "feat(relay): secp256k1 ownership verification + address derivation"
```

---

## Task 3: `order-state.js` — pure state machine (TDD)

**Files:** Create `order-state.js`, `test/order-state.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { canTransition, applyTransition } from "../order-state.js";

describe("order state machine", () => {
  it("allows listed -> pending_settlement", () => {
    assert.equal(canTransition("listed", "pending_settlement"), true);
  });
  it("allows pending_settlement -> broadcast", () => {
    assert.equal(canTransition("pending_settlement", "broadcast"), true);
  });
  it("allows pending_settlement -> listed (settlement slot expired)", () => {
    assert.equal(canTransition("pending_settlement", "listed"), true);
  });
  it("forbids listed -> broadcast (must settle first)", () => {
    assert.equal(canTransition("listed", "broadcast"), false);
  });
  it("forbids any transition out of a terminal state", () => {
    assert.equal(canTransition("broadcast", "listed"), false);
    assert.equal(canTransition("cancelled", "listed"), false);
  });
  it("applyTransition returns a new listing with the next state, or throws on illegal", () => {
    const l = { state: "listed", settlement: null };
    const next = applyTransition(l, "pending_settlement", { settlement: { buyer_addr: "txm1x" } });
    assert.equal(next.state, "pending_settlement");
    assert.equal(next.settlement.buyer_addr, "txm1x");
    assert.equal(l.state, "listed"); // original untouched
    assert.throws(() => applyTransition({ state: "broadcast" }, "listed"));
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `node --test test/order-state.test.js` → FAIL (no module).

- [ ] **Step 3: Implement `order-state.js`**

```js
const NEXT = {
  listed: new Set(["pending_settlement", "expired", "cancelled"]),
  pending_settlement: new Set(["broadcast", "listed", "expired", "cancelled"]),
  broadcast: new Set(),
  expired: new Set(),
  cancelled: new Set(),
};

export function canTransition(from, to) {
  return NEXT[from]?.has(to) ?? false;
}

export function applyTransition(listing, to, patch = {}) {
  if (!canTransition(listing.state, to)) {
    throw new Error(`illegal transition ${listing.state} -> ${to}`);
  }
  return { ...listing, ...patch, state: to };
}
```

- [ ] **Step 4: Run it, verify it passes** → `node --test test/order-state.test.js` PASS.

- [ ] **Step 5: Commit**

```bash
git add order-state.js test/order-state.test.js && git commit -q -m "feat(relay): pure listing state machine"
```

---

## Task 4: `state.js` — atomic persistence (TDD)

**Files:** Create `state.js`, `test/state.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it, afterEach } from "node:test";
import assert from "node:assert/strict";
import { existsSync, rmSync } from "node:fs";
import { makeStore } from "../state.js";

const PATH = "./data/test-state.json";
afterEach(() => { if (existsSync(PATH)) rmSync(PATH); });

describe("makeStore", () => {
  it("returns the default shape when no file exists", () => {
    const s = makeStore(PATH);
    assert.deepEqual(s.read().listings, {});
  });
  it("persists and reloads listings atomically", () => {
    const s = makeStore(PATH);
    const st = s.read();
    st.listings["lst_1"] = { listing_id: "lst_1", state: "listed" };
    s.write(st);
    assert.equal(makeStore(PATH).read().listings["lst_1"].state, "listed");
  });
  it("recovers to default on a corrupt file", () => {
    const s = makeStore(PATH);
    s.write({ listings: { a: 1 } });
    require?.("fs"); // noop
    const fs = await import("node:fs");
    fs.writeFileSync(PATH, "{ not json");
    assert.deepEqual(makeStore(PATH).read().listings, {});
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL (no module).

- [ ] **Step 3: Implement `state.js`** (mirrors the OTC watcher's atomic write)

```js
import { readFileSync, writeFileSync, renameSync, existsSync } from "node:fs";

const DEFAULT = () => ({ listings: {} });

export function makeStore(path) {
  return {
    read() {
      if (!existsSync(path)) return DEFAULT();
      try { return JSON.parse(readFileSync(path, "utf8")); }
      catch { console.error("[state] corrupt state, resetting"); return DEFAULT(); }
    },
    write(state) {
      const tmp = path + ".tmp";
      writeFileSync(tmp, JSON.stringify(state, null, 2));
      renameSync(tmp, path);
    },
  };
}
```

- [ ] **Step 4: Run it, verify it passes** (the corrupt-file test uses top-level await; mark the test `async`). → PASS.

- [ ] **Step 5: Commit**

```bash
git add state.js test/state.test.js && git commit -q -m "feat(relay): atomic JSON state store"
```

---

## Task 5: `node-client.js` + `indexer-client.js` (TDD with injected fetch)

**Files:** Create `node-client.js`, `indexer-client.js`, `test/clients.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { getUtxos, sendRawTransaction } from "../node-client.js";
import { getBalance, getAsset } from "../indexer-client.js";

const okFetch = (body) => async () => ({ ok: true, status: 200, json: async () => body, text: async () => JSON.stringify(body) });

describe("node-client", () => {
  it("getUtxos returns the parsed utxo set", async () => {
    const r = await getUtxos("txm1x", { fetchFn: okFetch({ tip_height: 10, utxos: [{ txid: "aa", output_index: 0, value_atoms: 5, created_height: 1, mature: true }] }) });
    assert.equal(r.tip_height, 10);
    assert.equal(r.utxos.length, 1);
  });
  it("sendRawTransaction posts and returns the node response", async () => {
    const r = await sendRawTransaction({ foo: 1 }, { fetchFn: okFetch({ txid: "bb" }) });
    assert.equal(r.txid, "bb");
  });
});

describe("indexer-client", () => {
  it("getBalance returns fungible+nfts", async () => {
    const r = await getBalance("txm1x", { fetchFn: okFetch({ fungible: [{ asset_id: "11", amount: 100 }], nfts: [] }) });
    assert.equal(r.fungible[0].amount, 100);
  });
  it("getAsset returns royalty terms", async () => {
    const r = await getAsset("11".repeat(32), { fetchFn: okFetch({ royalty_bps: 250, royalty_addr: "txm1r" }) });
    assert.equal(r.royalty_bps, 250);
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement the clients**

`node-client.js`:

```js
const RPC = () => process.env.TENSORIUM_RPC || "https://rpc.tensoriumlabs.com";
const LOCAL = () => "http://" + (process.env.TENSORIUM_RPC_LOCAL || "127.0.0.1:33332");

export async function getUtxos(address, { fetchFn = fetch } = {}) {
  const r = await fetchFn(`${RPC()}/getutxos/${address}`);
  if (!r.ok) throw new Error(`getutxos HTTP ${r.status}`);
  return r.json();
}

export async function sendRawTransaction(tx, { fetchFn = fetch } = {}) {
  const r = await fetchFn(`${LOCAL()}/sendrawtransaction`, {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(tx),
  });
  const body = await r.text();
  if (!r.ok) throw new Error(`sendrawtransaction HTTP ${r.status}: ${body}`);
  try { return JSON.parse(body); } catch { return { raw: body }; }
}
```

`indexer-client.js`:

```js
const IDX = () => process.env.INDEXER_URL || "http://127.0.0.1:23340";

export async function getBalance(address, { fetchFn = fetch } = {}) {
  const r = await fetchFn(`${IDX()}/balance/${address}`);
  if (!r.ok) throw new Error(`balance HTTP ${r.status}`);
  return r.json();
}

export async function getAsset(assetIdHex, { fetchFn = fetch } = {}) {
  const r = await fetchFn(`${IDX()}/asset/${assetIdHex}`);
  if (!r.ok) throw new Error(`asset HTTP ${r.status}`);
  return r.json();
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add node-client.js indexer-client.js test/clients.test.js && git commit -q -m "feat(relay): node + indexer clients (injectable fetch)"
```

---

## Task 6: `validate.js` — listing chain-validation (TDD)

**Files:** Create `validate.js`, `test/validate.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { validateListing } from "../validate.js";

const ASSET = "11".repeat(32);
const SELLER = "txm1seller";
const deps = (over = {}) => ({
  getUtxos: async () => ({ tip_height: 100, utxos: [{ txid: "22".repeat(32), output_index: 0, value_atoms: 50_000, created_height: 1, mature: true }] }),
  getBalance: async () => ({ fungible: [{ asset_id: ASSET, amount: 500 }], nfts: [] }),
  getAsset: async () => ({ royalty_bps: 250, royalty_addr: "txm1r" }),
  ...over,
});

const terms = { asset_id_hex: ASSET, amount: 100, price_atoms: 1_000_000, seller_addr: SELLER };

describe("validateListing", () => {
  it("returns ok + anchor for a valid listing", async () => {
    const r = await validateListing(terms, deps());
    assert.equal(r.ok, true);
    assert.equal(r.anchor.txid, "22".repeat(32));
    assert.equal(r.anchor.value, 50_000);
  });
  it("rejects when the seller lacks the asset balance", async () => {
    const r = await validateListing(terms, deps({ getBalance: async () => ({ fungible: [{ asset_id: ASSET, amount: 50 }], nfts: [] }) }));
    assert.equal(r.ok, false);
    assert.match(r.error, /balance/);
  });
  it("rejects when there is no mature UTXO to anchor", async () => {
    const r = await validateListing(terms, deps({ getUtxos: async () => ({ tip_height: 100, utxos: [{ txid: "22".repeat(32), output_index: 0, value_atoms: 50_000, created_height: 1, mature: false }] }) }));
    assert.equal(r.ok, false);
    assert.match(r.error, /anchor|mature/);
  });
  it("rejects when the asset is unknown to the indexer", async () => {
    const r = await validateListing(terms, deps({ getAsset: async () => { throw new Error("asset HTTP 404"); } }));
    assert.equal(r.ok, false);
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement `validate.js`**

```js
// Validate a proposed listing against the chain. Picks the smallest mature UTXO
// as the anchor (matches txmwallet asset-sell). deps injects the clients.
export async function validateListing(terms, deps) {
  try {
    await deps.getAsset(terms.asset_id_hex); // throws if unknown
  } catch (e) {
    return { ok: false, error: `asset lookup failed: ${e.message}` };
  }

  let bal;
  try { bal = await deps.getBalance(terms.seller_addr); }
  catch (e) { return { ok: false, error: `balance lookup failed: ${e.message}` }; }
  const held = (bal.fungible || []).find((f) => f.asset_id === terms.asset_id_hex);
  const ownsNft = (bal.nfts || []).includes(terms.asset_id_hex);
  if (!ownsNft && (!held || Number(held.amount) < Number(terms.amount))) {
    return { ok: false, error: "insufficient asset balance for this listing" };
  }

  let u;
  try { u = await deps.getUtxos(terms.seller_addr); }
  catch (e) { return { ok: false, error: `utxo lookup failed: ${e.message}` }; }
  const mature = (u.utxos || []).filter((x) => x.mature);
  if (!mature.length) return { ok: false, error: "no mature UTXO to anchor the sale" };
  const anchor = mature.reduce((a, b) => (b.value_atoms < a.value_atoms ? b : a));
  return { ok: true, anchor: { txid: anchor.txid, vout: anchor.output_index, value: anchor.value_atoms } };
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add validate.js test/validate.test.js && git commit -q -m "feat(relay): listing chain-validation + anchor selection"
```

---

## Task 7: `build.js` — shell to `txmwallet asset-build-settlement` (TDD)

**Files:** Create `build.js`, `test/build.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { buildUnsignedSettlement } from "../build.js";

const order = { asset_id_hex: "11".repeat(32), amount: 100, price_atoms: 1000000, seller_addr: "txm1s", seller_txid_hex: "22".repeat(32), seller_vout: 0, seller_value: 50000 };
const fakeOut = JSON.stringify({ tx: { inputs: [{}, {}], outputs: [] }, terms: { price_atoms: 1000000 }, input_indices: { seller: [0], buyer: [1] } });

describe("buildUnsignedSettlement", () => {
  it("invokes txmwallet with argv (no shell) and parses stdout JSON", async () => {
    let captured;
    const execFileFn = (bin, args) => { captured = { bin, args }; return { stdout: fakeOut }; };
    const out = await buildUnsignedSettlement(order, "txm1buyer", { execFileFn, writeFileFn: () => {}, tmpPath: "/tmp/o.json" });
    assert.equal(captured.bin, process.env.TXMWALLET_BIN || "/usr/local/bin/txmwallet");
    assert.deepEqual(captured.args, ["asset-build-settlement", "/tmp/o.json", "txm1buyer"]);
    assert.deepEqual(out.input_indices, { seller: [0], buyer: [1] });
  });
  it("throws a clear error when txmwallet fails", async () => {
    const execFileFn = () => { throw new Error("insufficient buyer funds"); };
    await assert.rejects(() => buildUnsignedSettlement(order, "txm1buyer", { execFileFn, writeFileFn: () => {}, tmpPath: "/tmp/o.json" }), /insufficient buyer funds/);
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement `build.js`**

```js
import { execFileSync } from "node:child_process";
import { writeFileSync, unlinkSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Write the order to a temp file and shell out (argv, no shell) to the keyless
// txmwallet builder. Returns { tx, terms, input_indices }.
export async function buildUnsignedSettlement(order, buyerAddr, opts = {}) {
  const bin = process.env.TXMWALLET_BIN || "/usr/local/bin/txmwallet";
  const tmpPath = opts.tmpPath || join(tmpdir(), `order-${Date.now()}-${Math.random().toString(36).slice(2)}.json`);
  const execFileFn = opts.execFileFn || ((b, a) => ({ stdout: execFileSync(b, a, { encoding: "utf8", timeout: 30000 }) }));
  const writeFileFn = opts.writeFileFn || writeFileSync;
  writeFileFn(tmpPath, JSON.stringify(order));
  try {
    const { stdout } = execFileFn(bin, ["asset-build-settlement", tmpPath, buyerAddr]);
    return JSON.parse(stdout);
  } catch (e) {
    throw new Error(`build settlement failed: ${e.message}`);
  } finally {
    if (!opts.tmpPath && existsSync(tmpPath)) unlinkSync(tmpPath);
  }
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add build.js test/build.test.js && git commit -q -m "feat(relay): keyless settlement build via txmwallet (no shell)"
```

---

## Task 8: `api-server.js` — routes (TDD)

**Files:** Create `api-server.js`, `test/api.test.js`

Dependencies are injected via `startApiServer({ store, deps })` so tests run without a real node. `deps` = `{ validateListing, buildUnsignedSettlement, verifyOwnership, sendRawTransaction, now }`.

- [ ] **Step 1: Write the failing test**

```js
import { describe, it, before, after } from "node:test";
import assert from "node:assert/strict";

const listings = {};
const store = { read: () => ({ listings }), write: (s) => { Object.assign(listings, s.listings); } };
const deps = {
  validateListing: async () => ({ ok: true, anchor: { txid: "22".repeat(32), vout: 0, value: 50000 } }),
  buildUnsignedSettlement: async () => ({ tx: { inputs: [{}, {}] }, terms: { price_atoms: 1000000 }, input_indices: { seller: [0], buyer: [1] } }),
  verifyOwnership: () => true,
  sendRawTransaction: async () => ({ txid: "deadbeef" }),
  now: () => 1_000_000,
};

const { startApiServer } = await import(`../api-server.js?v=${Date.now()}`);
let server; const BASE = "http://127.0.0.1:13900";
before(async () => { process.env.RELAY_API_PORT = "13900"; server = await startApiServer({ store, deps }); });
after(() => server?.close());

const post = (p, b) => fetch(BASE + p, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(b) });

describe("relay API", () => {
  const terms = { asset_id_hex: "11".repeat(32), amount: 100, price_atoms: 1000000, seller_addr: "txm1uyy0sfm07p47f8dy0mvdtwfefya8w5y2qr0q8p", kind: "txm20" };
  let listingId;

  it("POST /listing (sig-authed) creates a listed order", async () => {
    const r = await post("/listing", { terms, seller_pubkey: "03aa", sig: "00" });
    assert.equal(r.status, 200);
    const d = await r.json();
    assert.equal(d.state, "listed");
    listingId = d.listing_id;
  });
  it("GET /listings returns the active listing", async () => {
    const d = await (await fetch(BASE + "/listings")).json();
    assert.equal(d.listings.length, 1);
  });
  it("POST /quote returns an unsigned tx + indices", async () => {
    const d = await (await post("/quote", { listing_id: listingId, buyer_addr: "txm1buyer" })).json();
    assert.deepEqual(d.input_indices, { seller: [0], buyer: [1] });
  });
  it("POST /settlement moves it to pending_settlement", async () => {
    const r = await post("/settlement", { listing_id: listingId, signedTx: { inputs: [{}, {}] }, buyer_addr: "txm1buyer" });
    assert.equal(r.status, 200);
    assert.equal((await r.json()).state, "pending_settlement");
  });
  it("POST /accept (sig-authed) broadcasts and returns the txid", async () => {
    const r = await post("/accept", { listing_id: listingId, fullySignedTx: { inputs: [{}, {}] }, seller_pubkey: "03aa", sig: "00" });
    assert.equal(r.status, 200);
    assert.equal((await r.json()).broadcast_txid, "deadbeef");
  });
  it("POST /accept again is idempotent (same txid, no re-broadcast)", async () => {
    const r = await post("/accept", { listing_id: listingId, fullySignedTx: {}, seller_pubkey: "03aa", sig: "00" });
    assert.equal((await r.json()).broadcast_txid, "deadbeef");
  });
  it("rejects a listing with a bad signature", async () => {
    const bad = { ...deps, verifyOwnership: () => false };
    // re-mount with failing sig
    const { startApiServer: s2 } = await import(`../api-server.js?v=${Date.now()}b`);
    const srv = await s2({ store: { read: () => ({ listings: {} }), write: () => {} }, deps: bad, port: 13901 });
    const r = await fetch("http://127.0.0.1:13901/listing", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ terms, seller_pubkey: "03aa", sig: "00" }) });
    assert.equal(r.status, 401);
    srv.close();
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement `api-server.js`**

```js
import express from "express";
import { canTransition, applyTransition } from "./order-state.js";

const TXID_RE = /^[0-9a-f]{64}$/;
const ADDR_RE = /^txm1[0-9a-z]{20,80}$/;
const rid = () => "lst_" + Math.random().toString(36).slice(2, 10);

// Canonical message a seller signs to authorize an action on a listing.
const listMsg = (t) => `list:${t.asset_id_hex}:${t.amount}:${t.price_atoms}:${t.seller_addr}`;
const actMsg = (action, id) => `${action}:${id}`;

export function startApiServer({ store, deps, port } = {}) {
  const app = express();
  app.use(express.json({ limit: "256kb" }));
  const P = port || parseInt(process.env.RELAY_API_PORT || "3006");
  const now = deps.now || (() => Date.now());
  const LISTING_TTL = parseInt(process.env.LISTING_TTL_MS || "604800000");

  const save = (mut) => { const s = store.read(); mut(s); store.write(s); };
  const activeListing = (s, id) => s.listings[id];

  app.get("/health", (_q, res) => res.json({ ok: true }));

  app.get("/listings", (_q, res) => {
    const s = store.read();
    res.json({ listings: Object.values(s.listings).filter((l) => l.state === "listed") });
  });
  app.get("/listing/:id", (q, res) => {
    const l = store.read().listings[q.params.id];
    return l ? res.json(l) : res.status(404).json({ error: "not found" });
  });
  app.get("/pending", (q, res) => {
    const seller = q.query.seller;
    const s = store.read();
    res.json({ listings: Object.values(s.listings).filter((l) => l.state === "pending_settlement" && l.seller_addr === seller) });
  });

  app.post("/listing", async (req, res) => {
    const { terms, seller_pubkey, sig } = req.body || {};
    if (!terms || !TXID_RE.test(terms.asset_id_hex || "") || !ADDR_RE.test(terms.seller_addr || ""))
      return res.status(400).json({ error: "invalid terms" });
    if (!Number.isInteger(terms.amount) || terms.amount <= 0 || !Number.isInteger(terms.price_atoms) || terms.price_atoms <= 0)
      return res.status(400).json({ error: "amount/price must be positive integers" });
    if (!deps.verifyOwnership({ message: listMsg(terms), pubkey: seller_pubkey, sig, address: terms.seller_addr }))
      return res.status(401).json({ error: "signature does not authorize this seller" });
    const v = await deps.validateListing(terms, deps);
    if (!v.ok) return res.status(409).json({ error: v.error });
    const id = rid();
    const listing = {
      listing_id: id, asset_id_hex: terms.asset_id_hex, kind: terms.kind === "nft" ? "nft" : "txm20",
      amount: terms.amount, price_atoms: terms.price_atoms, seller_addr: terms.seller_addr,
      anchor: v.anchor, state: "listed", created_at: now(), expires_at: now() + LISTING_TTL,
      settlement: null, broadcast_txid: null,
    };
    save((s) => { s.listings[id] = listing; });
    return res.json(listing);
  });

  app.post("/quote", async (req, res) => {
    const { listing_id, buyer_addr } = req.body || {};
    if (!ADDR_RE.test(buyer_addr || "")) return res.status(400).json({ error: "invalid buyer_addr" });
    const l = activeListing(store.read(), listing_id);
    if (!l || l.state !== "listed") return res.status(409).json({ error: "listing not available" });
    const order = { asset_id_hex: l.asset_id_hex, amount: l.amount, price_atoms: l.price_atoms, seller_addr: l.seller_addr, seller_txid_hex: l.anchor.txid, seller_vout: l.anchor.vout, seller_value: l.anchor.value };
    try {
      const built = await deps.buildUnsignedSettlement(order, buyer_addr, deps);
      return res.json({ listing_id, unsignedTx: built.tx, summary: built.terms, input_indices: built.input_indices });
    } catch (e) { return res.status(409).json({ error: e.message }); }
  });

  app.post("/settlement", (req, res) => {
    const { listing_id, signedTx, buyer_addr } = req.body || {};
    save((s) => {
      const l = s.listings[listing_id];
      if (!l) return res.status(404).json({ error: "not found" });
      if (!canTransition(l.state, "pending_settlement")) return res.status(409).json({ error: `listing is ${l.state}` });
      s.listings[listing_id] = applyTransition(l, "pending_settlement", { settlement: { signedTx, buyer_addr, ts: now() } });
      res.json(s.listings[listing_id]);
    });
  });

  app.post("/accept", async (req, res) => {
    const { listing_id, fullySignedTx, seller_pubkey, sig } = req.body || {};
    const l0 = store.read().listings[listing_id];
    if (!l0) return res.status(404).json({ error: "not found" });
    if (l0.state === "broadcast") return res.json(l0); // idempotent
    if (l0.state !== "pending_settlement") return res.status(409).json({ error: `listing is ${l0.state}` });
    if (!deps.verifyOwnership({ message: actMsg("accept", listing_id), pubkey: seller_pubkey, sig, address: l0.seller_addr }))
      return res.status(401).json({ error: "signature does not authorize this seller" });
    let result;
    try { result = await deps.sendRawTransaction(fullySignedTx, deps); }
    catch (e) { return res.status(502).json({ error: `broadcast failed: ${e.message}` }); }
    save((s) => { s.listings[listing_id] = applyTransition(s.listings[listing_id], "broadcast", { broadcast_txid: result.txid || result.raw }); });
    return res.json(store.read().listings[listing_id]);
  });

  app.post("/cancel", (req, res) => {
    const { listing_id, seller_pubkey, sig } = req.body || {};
    const l = store.read().listings[listing_id];
    if (!l) return res.status(404).json({ error: "not found" });
    if (!deps.verifyOwnership({ message: actMsg("cancel", listing_id), pubkey: seller_pubkey, sig, address: l.seller_addr }))
      return res.status(401).json({ error: "signature does not authorize this seller" });
    if (!canTransition(l.state, "cancelled")) return res.status(409).json({ error: `listing is ${l.state}` });
    save((s) => { s.listings[listing_id] = applyTransition(l, "cancelled"); });
    return res.json({ cancelled: true });
  });

  return new Promise((resolve) => {
    const server = app.listen(P, "127.0.0.1", () => { console.log(`[relay] api on 127.0.0.1:${P}`); resolve(server); });
  });
}
```

- [ ] **Step 4: Run it, verify it passes** → `node --test test/api.test.js` PASS (7 assertions).

- [ ] **Step 5: Commit**

```bash
git add api-server.js test/api.test.js && git commit -q -m "feat(relay): express API — listing/quote/settlement/accept/cancel, sig-authed + idempotent broadcast"
```

---

## Task 9: `prune.js` — expiry + invalidation sweep (TDD)

**Files:** Create `prune.js`, `test/prune.test.js`

- [ ] **Step 1: Write the failing test**

```js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { pruneOnce } from "../prune.js";

const mk = (over) => ({ listing_id: "l", state: "listed", expires_at: 2000, seller_addr: "txm1s", asset_id_hex: "11".repeat(32), amount: 100, anchor: { txid: "22".repeat(32), vout: 0 }, ...over });

describe("pruneOnce", () => {
  it("expires listings past expires_at", async () => {
    const listings = { l: mk({ expires_at: 500 }) };
    const deps = { now: () => 1000, validateListing: async () => ({ ok: true }) };
    await pruneOnce(listings, deps);
    assert.equal(listings.l.state, "expired");
  });
  it("expires listings whose chain validation now fails (anchor spent / balance dropped)", async () => {
    const listings = { l: mk({ expires_at: 9999 }) };
    const deps = { now: () => 1000, validateListing: async () => ({ ok: false, error: "anchor spent" }) };
    await pruneOnce(listings, deps);
    assert.equal(listings.l.state, "expired");
  });
  it("leaves valid, unexpired listings untouched", async () => {
    const listings = { l: mk({ expires_at: 9999 }) };
    const deps = { now: () => 1000, validateListing: async () => ({ ok: true }) };
    await pruneOnce(listings, deps);
    assert.equal(listings.l.state, "listed");
  });
  it("reverts a stale pending_settlement back to listed after the settlement TTL", async () => {
    const listings = { l: mk({ state: "pending_settlement", expires_at: 9999, settlement: { ts: 0 } }) };
    const deps = { now: () => 999999999, settlementTtl: 1000, validateListing: async () => ({ ok: true }) };
    await pruneOnce(listings, deps);
    assert.equal(listings.l.state, "listed");
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement `prune.js`**

```js
import { applyTransition } from "./order-state.js";

// One sweep over the listings map (mutated in place). Pure-ish: all I/O via deps.
export async function pruneOnce(listings, deps) {
  const now = deps.now();
  const ttl = deps.settlementTtl ?? parseInt(process.env.SETTLEMENT_TTL_MS || "1800000");
  for (const id of Object.keys(listings)) {
    const l = listings[id];
    if (l.state === "listed") {
      if (now >= l.expires_at) { listings[id] = applyTransition(l, "expired"); continue; }
      const v = await deps.validateListing({ asset_id_hex: l.asset_id_hex, amount: l.amount, price_atoms: l.price_atoms, seller_addr: l.seller_addr }, deps);
      if (!v.ok) listings[id] = applyTransition(l, "expired");
    } else if (l.state === "pending_settlement") {
      if (l.settlement && now - l.settlement.ts >= ttl) listings[id] = applyTransition(l, "listed", { settlement: null });
    }
  }
}

export function startPruneTimer(store, deps) {
  const ms = parseInt(process.env.PRUNE_INTERVAL_MS || "60000");
  return setInterval(async () => {
    try { const s = store.read(); await pruneOnce(s.listings, deps); store.write(s); }
    catch (e) { console.error("[prune] error:", e.message); }
  }, ms);
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add prune.js test/prune.test.js && git commit -q -m "feat(relay): expiry + invalidation prune sweep"
```

---

## Task 10: `index.js` — wiring

**Files:** Create `index.js`

- [ ] **Step 1: Implement `index.js`**

```js
import * as dotenv from "dotenv";
dotenv.config();
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";
import { makeStore } from "./state.js";
import { startApiServer } from "./api-server.js";
import { startPruneTimer } from "./prune.js";
import { validateListing } from "./validate.js";
import { buildUnsignedSettlement } from "./build.js";
import { verifyOwnership } from "./sig.js";
import { getUtxos, sendRawTransaction } from "./node-client.js";
import { getBalance, getAsset } from "./indexer-client.js";

const REQUIRED = ["TENSORIUM_RPC", "INDEXER_URL", "TXMWALLET_BIN"];
for (const k of REQUIRED) if (!process.env[k]) { console.error(`[relay] missing env: ${k}`); process.exit(1); }

const __dirname = dirname(fileURLToPath(import.meta.url));
const store = makeStore(join(__dirname, "data", "relay-state.json"));

// deps bundle passed to the API + prune. validateListing/buildUnsignedSettlement
// receive this same object so their injected client fns resolve.
const deps = {
  getUtxos, sendRawTransaction, getBalance, getAsset,
  validateListing, buildUnsignedSettlement, verifyOwnership,
  now: () => Date.now(),
};

console.log("[relay] tensorium-order-relay starting");
await startApiServer({ store, deps });
startPruneTimer(store, deps);
```

- [ ] **Step 2: Smoke-check it boots (missing-env guard)**

Run: `node index.js` (with no `.env`) → Expected: exits 1 with `missing env: TENSORIUM_RPC`. Then `cp .env.example .env` and run again → Expected: `[relay] api on 127.0.0.1:3006`. Ctrl-C.

- [ ] **Step 3: Run the full suite**

Run: `node --test`
Expected: all tests pass (sig, order-state, state, clients, validate, build, api, prune).

- [ ] **Step 4: Commit**

```bash
git add index.js && git commit -q -m "feat(relay): wire store + api + prune with real clients"
```

---

## Task 11: Deployment artifacts (no tests)

**Files:** Create `deploy/tensorium-order-relay.service`, `deploy/nginx-relay.conf`, `README.md`

- [ ] **Step 1: systemd unit** (`deploy/tensorium-order-relay.service`)

```ini
[Unit]
Description=Tensorium Order Relay (marketplace asset trading)
After=network.target tensorium-node.service tensorium-indexer.service

[Service]
Type=simple
WorkingDirectory=/root/tensorium-order-relay
ExecStart=/usr/bin/node /root/tensorium-order-relay/index.js
Restart=on-failure
RestartSec=10
EnvironmentFile=/root/tensorium-order-relay/.env

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: nginx snippet** (`deploy/nginx-relay.conf`) — to merge into the marketplace vhost

```nginx
# Marketplace order-relay: GET open, POST allowed (unlike the GET-only /api/ indexer path), rate-limited.
location /relay/ {
    limit_req zone=txmapi burst=20 nodelay;
    proxy_pass http://127.0.0.1:3006/;
    proxy_set_header Host $host;
    proxy_read_timeout 20s;
}
```

- [ ] **Step 3: README.md** — document the API contract (the routes from Task 8), the non-custodial model, and the deploy steps below.

- [ ] **Step 4: Commit**

```bash
git add deploy README.md && git commit -q -m "docs(relay): systemd unit, nginx snippet, README"
```

---

## Task 12: Live deployment + smoke test (manual, gated)

**Host:** `66.42.120.149`. Do this only after all tests are green.

- [ ] **Step 1: Push the new repo**

```bash
# create github.com/tensorium-labs/tensorium-order-relay (private until M3), then:
git remote add origin https://github.com/tensorium-labs/tensorium-order-relay.git
git push -u origin master
```

- [ ] **Step 2: Deploy to prod**

```bash
ssh root@66.42.120.149 'git clone https://github.com/tensorium-labs/tensorium-order-relay.git /root/tensorium-order-relay && cd /root/tensorium-order-relay && npm install --omit=dev && cp .env.example .env'
# edit /root/tensorium-order-relay/.env: INDEXER_URL=http://127.0.0.1:23340, TENSORIUM_RPC_LOCAL=127.0.0.1:33332
ssh root@66.42.120.149 'cp /root/tensorium-order-relay/deploy/tensorium-order-relay.service /etc/systemd/system/ && systemctl daemon-reload && systemctl enable --now tensorium-order-relay'
```

- [ ] **Step 3: Wire nginx**

```bash
# Back up the marketplace vhost first, merge deploy/nginx-relay.conf into the server block, then:
ssh root@66.42.120.149 'nginx -t && systemctl reload nginx'
```

- [ ] **Step 4: Smoke test against the live chain (read-only first)**

```bash
ssh root@66.42.120.149 'curl -s 127.0.0.1:3006/health'                 # {"ok":true}
curl -s https://marketplace.tensoriumlabs.com/relay/listings           # {"listings":[]}
```

Expected: health ok, empty listings. (Listing/quote/accept require a real seller signature from M2 — full end-to-end is validated at the end of M3.)

- [ ] **Step 5: Record outcome** in the project memory (host, service, port, nginx path) and mark M1 done.

---

## Self-review

- **Spec coverage:** Component A (relay: state §5.1→state.js+order-state.js; API §5.2→api-server.js; validation §5.3→validate.js; idempotency/crash-safety §5.4→atomic state + idempotent /accept; deploy §5.5→Task 11/12) and Component D (keyless builder §6→Task R1) are all covered. Components B (frontend) and C (wallet methods) are explicitly M2/M3 — out of scope for this plan.
- **Sig auth:** list/accept/cancel all gated by `verifyOwnership` over canonical messages; quote/settlement are not sig-gated by design (quote is read-only; a buyer's settlement is self-authorizing via their input signatures, re-verified before broadcast — note: full `verify_settlement` re-check on the buyer-signed tx is added in M3 wiring when the wallet produces real signed txs; M1 stores/forwards and re-verifies at /accept via the node's own mempool validation on `sendrawtransaction`).
- **Placeholder scan:** none — every step has full code/commands and a real test vector (pubkey→address) is embedded.
- **Type consistency:** `verifyOwnership({message,pubkey,sig,address})`, `validateListing(terms, deps)→{ok,anchor|error}`, `buildUnsignedSettlement(order,buyerAddr,opts)→{tx,terms,input_indices}`, listing shape, and state names (`listed/pending_settlement/broadcast/expired/cancelled`) are consistent across tasks.

> **Residual note for M3:** when the wallet returns a real buyer-signed tx, add a relay-side `verify_settlement` re-check at `/settlement` (re-derive expected unsigned tx from the listing, confirm only buyer inputs are signed and outputs/terms match) before accepting it — stubbed as node-mempool validation in M1.
