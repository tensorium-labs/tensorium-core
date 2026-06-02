# Bridge Relayer Productionization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Productionize the live bridge relayer with LOCK fix, retry logic, Discord alerting, HTTP deposit API, and health endpoint.

**Architecture:** Additive — two new modules (`alerts.js`, `api-server.js`) added alongside existing watchers. Existing watcher files get targeted fixes (LOCK env var, retry wrapper). No refactoring of working code paths.

**Tech Stack:** Node.js ESM, Express 4, ethers.js (already installed), `node:test` for tests. All work happens on VPS `157.230.44.162` via SSH.

**Working directory on VPS:** `/root/tensorium-bridge-relayer/`

**Test command:** `node --test test/` (or individual test files)

---

## Task 1: Create `alerts.js` with test

**Files:**
- Create: `/root/tensorium-bridge-relayer/alerts.js`
- Create: `/root/tensorium-bridge-relayer/test/alerts.test.js`

- [ ] **Create `test/` directory and write the failing test:**

```bash
mkdir -p /root/tensorium-bridge-relayer/test
```

```js
// /root/tensorium-bridge-relayer/test/alerts.test.js
import { describe, it, mock, beforeEach } from "node:test";
import assert from "node:assert/strict";

describe("sendAlert", () => {
  it("does nothing when DISCORD_ALERT_WEBHOOK is unset", async () => {
    delete process.env.DISCORD_ALERT_WEBHOOK;
    const { sendAlert } = await import("../alerts.js");
    // Should not throw
    await assert.doesNotReject(() => sendAlert("test title", "test body"));
  });

  it("calls fetch with correct Discord embed shape when webhook is set", async () => {
    const calls = [];
    global.fetch = async (url, opts) => {
      calls.push({ url, body: JSON.parse(opts.body) });
      return { ok: true };
    };
    process.env.DISCORD_ALERT_WEBHOOK = "https://discord.com/webhook/test";
    // Re-import fresh module (cache bust via query param workaround)
    const mod = await import(`../alerts.js?v=${Date.now()}`);
    await mod.sendAlert("Mint failed", "UTXO abc:0 failed after 3 attempts");
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "https://discord.com/webhook/test");
    const embed = calls[0].body.embeds[0];
    assert.ok(embed.title.includes("Mint failed"));
    assert.ok(embed.description.includes("UTXO abc:0"));
    assert.equal(embed.color, 0xff4444);
  });
});
```

- [ ] **Run to confirm failure:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/alerts.test.js 2>&1 | tail -5
```

Expected: `Cannot find module '../alerts.js'`

- [ ] **Create `alerts.js`:**

```js
// /root/tensorium-bridge-relayer/alerts.js

export async function sendAlert(title, body) {
  const url = process.env.DISCORD_ALERT_WEBHOOK;
  if (!url) {
    console.warn("[alert] DISCORD_ALERT_WEBHOOK not set — skipping alert:", title);
    return;
  }
  await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      embeds: [{
        title: `\u{1F6A8} Bridge Relayer: ${title}`,
        description: body,
        color: 0xff4444,
        timestamp: new Date().toISOString(),
      }],
    }),
  }).catch(e => console.error("[alert] Discord webhook failed:", e.message));
}
```

- [ ] **Run tests:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/alerts.test.js 2>&1 | tail -8
```

Expected: `2 pass`

- [ ] **Commit:**

```bash
cd /root/tensorium-bridge-relayer
git add alerts.js test/alerts.test.js 2>/dev/null || true
# If no git repo on VPS, skip git add — just note files are changed
echo "alerts.js created"
```

---

## Task 2: Add `withRetry` helper + test

**Files:**
- Create: `/root/tensorium-bridge-relayer/retry.js`
- Create: `/root/tensorium-bridge-relayer/test/retry.test.js`

- [ ] **Write failing test:**

```js
// /root/tensorium-bridge-relayer/test/retry.test.js
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { withRetry } from "../retry.js";

describe("withRetry", () => {
  it("returns result on first success", async () => {
    const result = await withRetry("test", async () => 42);
    assert.equal(result, 42);
  });

  it("retries and succeeds on second attempt", async () => {
    let attempts = 0;
    const result = await withRetry("test", async () => {
      attempts++;
      if (attempts < 2) throw new Error("transient");
      return "ok";
    }, 3, 0); // 0ms delay for tests
    assert.equal(result, "ok");
    assert.equal(attempts, 2);
  });

  it("throws after maxAttempts", async () => {
    let attempts = 0;
    await assert.rejects(
      withRetry("test", async () => { attempts++; throw new Error("permanent"); }, 3, 0),
      /permanent/
    );
    assert.equal(attempts, 3);
  });
});
```

- [ ] **Run to confirm failure:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/retry.test.js 2>&1 | tail -5
```

Expected: `Cannot find module '../retry.js'`

- [ ] **Create `retry.js`:**

```js
// /root/tensorium-bridge-relayer/retry.js

// delay: override for tests (pass 0 to skip waiting)
export async function withRetry(label, fn, maxAttempts = 3, delay = null) {
  const getDelay = (attempt) =>
    delay !== null ? delay : 5000 * attempt ** 2; // 5s, 20s, 45s

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (err) {
      const last = attempt === maxAttempts;
      console.error(`[${label}] attempt ${attempt}/${maxAttempts} failed: ${err.message}`);
      if (last) throw err;
      await new Promise(r => setTimeout(r, getDelay(attempt)));
    }
  }
}
```

- [ ] **Run tests:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/retry.test.js 2>&1 | tail -8
```

Expected: `3 pass`

---

## Task 3: Fix LOCK bug in `withdrawal-watcher.js` + add retry + alerts

**Files:**
- Modify: `/root/tensorium-bridge-relayer/withdrawal-watcher.js`

- [ ] **Read the current file first:**

```bash
cat /root/tensorium-bridge-relayer/withdrawal-watcher.js
```

- [ ] **Replace the `releaseTxm` function** — find the env block (around line 15) and add `TENSORIUM_RPC`:

The current env block inside `releaseTxm`:
```js
  const env = {
    ...process.env,
    TENSORIUM_WALLET: process.env.CUSTODY_WALLET_PATH,
    TENSORIUM_WALLET_PASSPHRASE: process.env.CUSTODY_WALLET_PASSPHRASE,
    TENSORIUM_STATE: process.env.TENSORIUM_MC_STATE,
  };
```

Replace with:
```js
  const env = {
    ...process.env,
    TENSORIUM_WALLET: process.env.CUSTODY_WALLET_PATH,
    TENSORIUM_WALLET_PASSPHRASE: process.env.CUSTODY_WALLET_PASSPHRASE,
    TENSORIUM_STATE: process.env.TENSORIUM_MC_STATE,
    TENSORIUM_RPC: process.env.TENSORIUM_MC_RPC_LOCAL || "127.0.0.1:33332",
  };
```

- [ ] **Add imports at top of `withdrawal-watcher.js`** (after existing imports):

```js
import { withRetry } from "./retry.js";
import { sendAlert } from "./alerts.js";
```

- [ ] **Replace the withdrawal processing block** in `checkWithdrawals` — find:

```js
    try {
      const result = releaseTxm(tensoriumAddress, atoms.toString());
      console.log(`[withdrawal] released: ${result}`);
      state.processedWithdrawals[bridgeEventId] = {
        tensoriumAddr: tensoriumAddress,
        atoms: atoms.toString(),
        evmTx: event.transactionHash,
        ts: new Date().toISOString(),
      };
      changed = true;
    } catch (err) {
      console.error(`[withdrawal] release failed for ${bridgeEventId}:`, err.message);
    }
```

Replace with:
```js
    try {
      const result = await withRetry(
        `withdrawal:${bridgeEventId.slice(0, 10)}`,
        () => Promise.resolve(releaseTxm(tensoriumAddress, atoms.toString()))
      );
      console.log(`[withdrawal] released: ${result}`);
      state.processedWithdrawals[bridgeEventId] = {
        tensoriumAddr: tensoriumAddress,
        atoms: atoms.toString(),
        evmTx: event.transactionHash,
        ts: new Date().toISOString(),
      };
      changed = true;
    } catch (err) {
      console.error(`[withdrawal] release failed permanently for ${bridgeEventId}:`, err.message);
      await sendAlert(
        "Release failed",
        `bridgeEventId: ${bridgeEventId}\nto: ${tensoriumAddress}\natoms: ${atoms}\nevmTx: ${event.transactionHash}\nerror: ${err.message}`
      );
      state.failedReleases = state.failedReleases || {};
      state.failedReleases[bridgeEventId] = {
        tensoriumAddr: tensoriumAddress,
        atoms: atoms.toString(),
        evmTx: event.transactionHash,
        error: err.message,
        ts: new Date().toISOString(),
      };
      // Mark processed to prevent infinite retry — operator must handle manually
      state.processedWithdrawals[bridgeEventId] = { failed: true, ts: new Date().toISOString() };
      changed = true;
    }
```

- [ ] **Verify file looks correct:**

```bash
node --check /root/tensorium-bridge-relayer/withdrawal-watcher.js && echo "syntax OK"
```

Expected: `syntax OK`

---

## Task 4: Add retry + alerts to `deposit-watcher.js`

**Files:**
- Modify: `/root/tensorium-bridge-relayer/deposit-watcher.js`

- [ ] **Add imports at top** (after existing imports):

```js
import { withRetry } from "./retry.js";
import { sendAlert } from "./alerts.js";
```

- [ ] **Replace the mint block** inside `checkDeposits` — find:

```js
    try {
      const receipt = await mintFromDeposit(
        controller, bridgeEventId, utxo.txid, pending.recipient, utxo.value_atoms
      );
      console.log(`[deposit] minted evmTx=${receipt.hash}`);
      state.processedUtxos[key] = {
        mintTx: receipt.hash,
        recipient: pending.recipient,
        atoms: utxo.value_atoms,
        ts: new Date().toISOString(),
      };
      changed = true;
    } catch (err) {
      console.error(`[deposit] mint failed for ${key}:`, err.message);
    }
```

Replace with:
```js
    try {
      const receipt = await withRetry(
        `deposit:${key}`,
        () => mintFromDeposit(controller, bridgeEventId, utxo.txid, pending.recipient, utxo.value_atoms)
      );
      console.log(`[deposit] minted evmTx=${receipt.hash}`);
      state.processedUtxos[key] = {
        mintTx: receipt.hash,
        recipient: pending.recipient,
        atoms: utxo.value_atoms,
        ts: new Date().toISOString(),
      };
      changed = true;
    } catch (err) {
      console.error(`[deposit] mint failed permanently for ${key}:`, err.message);
      await sendAlert(
        "Mint failed",
        `utxo: ${key}\nrecipient: ${pending.recipient}\natoms: ${utxo.value_atoms}\nerror: ${err.message}`
      );
      state.failedMints = state.failedMints || {};
      state.failedMints[key] = {
        recipient: pending.recipient,
        atoms: utxo.value_atoms,
        error: err.message,
        ts: new Date().toISOString(),
      };
      changed = true;
      // Do NOT mark processedUtxos — allow retry on next poll cycle
    }
```

- [ ] **Upgrade the no-pending-file warn to an alert** — find:

```js
      console.warn(`[deposit] UTXO ${key} confirmed, no pending file — needs operator mapping`);
```

Replace with:
```js
      console.warn(`[deposit] UTXO ${key} confirmed, no pending file — needs operator mapping`);
      await sendAlert(
        "Deposit needs mapping",
        `UTXO: ${key}\natoms: ${utxo.value_atoms}\nconfs: ${confs}\nSend POST /deposit with txid + recipient to process.`
      );
```

- [ ] **Verify syntax:**

```bash
node --check /root/tensorium-bridge-relayer/deposit-watcher.js && echo "syntax OK"
```

---

## Task 5: Extend `state.js` default schema + add lastCheck fields

**Files:**
- Modify: `/root/tensorium-bridge-relayer/state.js`

- [ ] **Find and replace the `DEFAULT` object** — current:

```js
const DEFAULT = {
  processedUtxos: {},
  processedWithdrawals: {},
  lastProcessedBlock: 0,
};
```

Replace with:
```js
const DEFAULT = {
  processedUtxos: {},
  processedWithdrawals: {},
  failedMints: {},
  failedReleases: {},
  lastProcessedBlock: 0,
  lastDepositCheckAt: null,
  lastWithdrawalBlock: 0,
};
```

- [ ] **Add `updateDepositCheck` and `updateWithdrawalBlock` exports** at the bottom of `state.js`:

```js
export function updateDepositCheck() {
  const s = readState();
  s.lastDepositCheckAt = new Date().toISOString();
  writeState(s);
}

export function updateWithdrawalBlock(block) {
  const s = readState();
  s.lastWithdrawalBlock = block;
  writeState(s);
}
```

- [ ] **Update `deposit-watcher.js` to call `updateDepositCheck`** — add import at top:

```js
import { readState, writeState, updateDepositCheck } from "./state.js";
```

At the end of the `checkDeposits` function (after `if (changed) writeState(state);`), add:

```js
  updateDepositCheck();
```

- [ ] **Update `withdrawal-watcher.js` to call `updateWithdrawalBlock`** — add to import line:

```js
import { readState, writeState, updateWithdrawalBlock } from "./state.js";
```

After `state.lastProcessedBlock = currentBlock;` in `checkWithdrawals`, add:

```js
  updateWithdrawalBlock(currentBlock);
```

- [ ] **Verify both files:**

```bash
node --check /root/tensorium-bridge-relayer/deposit-watcher.js && \
node --check /root/tensorium-bridge-relayer/withdrawal-watcher.js && \
echo "both OK"
```

---

## Task 6: Create `api-server.js`

**Files:**
- Create: `/root/tensorium-bridge-relayer/api-server.js`
- Create: `/root/tensorium-bridge-relayer/test/api.test.js`

- [ ] **Check Express is available:**

```bash
node -e "import('express').then(()=>console.log('express OK')).catch(()=>console.log('missing'))"
```

If missing: `npm install express --save`

- [ ] **Write the failing test first:**

```js
// /root/tensorium-bridge-relayer/test/api.test.js
import { describe, it, before, after } from "node:test";
import assert from "node:assert/strict";
import { existsSync, unlinkSync, mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PENDING_DIR = join(__dirname, "../deposits/pending");

// Set required env before import
process.env.RELAYER_API_KEY = "test-key-abc123";
process.env.RELAYER_API_PORT = "13004";

const { startApiServer, stopApiServer } = await import(`../api-server.js?v=${Date.now()}`);

let server;
before(async () => { server = await startApiServer(); });
after(() => server?.close());

const BASE = "http://127.0.0.1:13004";
const AUTH = { "Authorization": "Bearer test-key-abc123", "Content-Type": "application/json" };

describe("POST /deposit", () => {
  const TXID = "a".repeat(64);
  const FILE = join(PENDING_DIR, `${TXID}.json`);

  after(() => { if (existsSync(FILE)) unlinkSync(FILE); });

  it("rejects missing auth", async () => {
    const r = await fetch(`${BASE}/deposit`, { method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ txid: TXID, outputIndex: 0, recipient: "0x" + "1".repeat(40) }) });
    assert.equal(r.status, 401);
  });

  it("rejects invalid txid", async () => {
    const r = await fetch(`${BASE}/deposit`, { method: "POST", headers: AUTH,
      body: JSON.stringify({ txid: "short", outputIndex: 0, recipient: "0x" + "1".repeat(40) }) });
    assert.equal(r.status, 400);
    const d = await r.json();
    assert.ok(d.error.includes("txid"));
  });

  it("rejects invalid recipient", async () => {
    const r = await fetch(`${BASE}/deposit`, { method: "POST", headers: AUTH,
      body: JSON.stringify({ txid: TXID, outputIndex: 0, recipient: "notanaddress" }) });
    assert.equal(r.status, 400);
  });

  it("queues valid deposit", async () => {
    mkdirSync(PENDING_DIR, { recursive: true });
    const r = await fetch(`${BASE}/deposit`, { method: "POST", headers: AUTH,
      body: JSON.stringify({ txid: TXID, outputIndex: 0, recipient: "0x" + "1".repeat(40) }) });
    assert.equal(r.status, 200);
    const d = await r.json();
    assert.equal(d.queued, true);
    assert.ok(existsSync(FILE));
  });

  it("returns 409 on duplicate", async () => {
    const r = await fetch(`${BASE}/deposit`, { method: "POST", headers: AUTH,
      body: JSON.stringify({ txid: TXID, outputIndex: 0, recipient: "0x" + "1".repeat(40) }) });
    assert.equal(r.status, 409);
  });
});

describe("GET /health", () => {
  it("returns 200 with expected fields", async () => {
    const r = await fetch(`${BASE}/health`);
    assert.equal(r.status, 200);
    const d = await r.json();
    assert.equal(d.ok, true);
    assert.ok("uptime" in d);
    assert.ok("failedMints" in d);
    assert.ok("failedReleases" in d);
    assert.ok("pendingDeposits" in d);
  });
});
```

- [ ] **Run to confirm failure:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/api.test.js 2>&1 | tail -5
```

Expected: `Cannot find module '../api-server.js'`

- [ ] **Create `api-server.js`:**

```js
// /root/tensorium-bridge-relayer/api-server.js
import express from "express";
import { writeFileSync, existsSync, readdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import { readState } from "./state.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PENDING_DIR = join(__dirname, "deposits", "pending");
const START_TIME = Date.now();

const TXID_RE    = /^[0-9a-f]{64}$/;
const ADDR_RE    = /^0x[0-9a-fA-F]{40}$/;

function apiKeyMiddleware(req, res, next) {
  const header = req.headers["authorization"] || "";
  const token  = header.startsWith("Bearer ") ? header.slice(7) : "";
  if (token !== process.env.RELAYER_API_KEY) {
    return res.status(401).json({ error: "unauthorized" });
  }
  next();
}

export function startApiServer() {
  const app = express();
  app.use(express.json());

  app.post("/deposit", apiKeyMiddleware, (req, res) => {
    const { txid, outputIndex, recipient } = req.body || {};

    if (!txid || !TXID_RE.test(txid))
      return res.status(400).json({ error: "invalid txid — must be 64 lowercase hex chars" });
    if (typeof outputIndex !== "number" || outputIndex < 0 || outputIndex > 99)
      return res.status(400).json({ error: "invalid outputIndex — must be integer 0–99" });
    if (!recipient || !ADDR_RE.test(recipient))
      return res.status(400).json({ error: "invalid recipient — must be 0x-prefixed 40-char hex" });

    const file = join(PENDING_DIR, `${txid}.json`);
    if (existsSync(file))
      return res.status(409).json({ error: "already queued" });

    const pending = { recipient, txid, outputIndex, ts: new Date().toISOString() };
    writeFileSync(file, JSON.stringify(pending, null, 2));
    console.log(`[api] deposit queued txid=${txid.slice(0, 12)}... recipient=${recipient}`);
    return res.json({ queued: true, txid, recipient });
  });

  app.get("/health", (req, res) => {
    const state = readState();
    let pendingDeposits = 0;
    try {
      pendingDeposits = readdirSync(PENDING_DIR).filter(f => f.endsWith(".json")).length;
    } catch {}
    res.json({
      ok:                  true,
      uptime:              Math.floor((Date.now() - START_TIME) / 1000),
      lastDepositCheckAt:  state.lastDepositCheckAt || null,
      lastWithdrawalBlock: state.lastWithdrawalBlock || 0,
      pendingDeposits,
      failedMints:         Object.keys(state.failedMints  || {}).length,
      failedReleases:      Object.keys(state.failedReleases || {}).length,
    });
  });

  const port = parseInt(process.env.RELAYER_API_PORT || "3004");
  return new Promise(resolve => {
    const server = app.listen(port, "127.0.0.1", () => {
      console.log(`[api] listening on http://127.0.0.1:${port}`);
      resolve(server);
    });
  });
}
```

- [ ] **Run tests:**

```bash
cd /root/tensorium-bridge-relayer && node --test test/api.test.js 2>&1 | tail -12
```

Expected: `6 pass`

---

## Task 7: Wire up in `index.js` + update `.env`

**Files:**
- Modify: `/root/tensorium-bridge-relayer/index.js`
- Modify: `/root/tensorium-bridge-relayer/.env`

- [ ] **Add `startApiServer` import and call to `index.js`:**

Current `index.js` content to replace:

```js
import * as dotenv from "dotenv";
dotenv.config();

import { runDepositWatcher } from "./deposit-watcher.js";
import { runWithdrawalWatcher } from "./withdrawal-watcher.js";

const REQUIRED = [
  "CUSTODY_ADDRESS", "CUSTODY_WALLET_PATH", "CUSTODY_WALLET_PASSPHRASE",
  "TENSORIUM_MC_STATE", "OP_RPC_URL", "OPERATOR_PRIVATE_KEY",
  "CONTROLLER_ADDRESS", "TOKEN_ADDRESS",
];
for (const k of REQUIRED) {
  if (!process.env[k]) { console.error(`[relayer] missing env: ${k}`); process.exit(1); }
}

console.log("[relayer] Phase 9A bridge relayer starting");
console.log("[relayer] custody :", process.env.CUSTODY_ADDRESS);
console.log("[relayer] controller:", process.env.CONTROLLER_ADDRESS);

Promise.allSettled([
  runDepositWatcher().catch((err) => { console.error("[deposit] fatal:", err); process.exit(1); }),
  runWithdrawalWatcher().catch((err) => { console.error("[withdrawal] fatal:", err); process.exit(1); }),
]);
```

New `index.js`:
```js
import * as dotenv from "dotenv";
dotenv.config();

import { runDepositWatcher } from "./deposit-watcher.js";
import { runWithdrawalWatcher } from "./withdrawal-watcher.js";
import { startApiServer } from "./api-server.js";
import { mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

const REQUIRED = [
  "CUSTODY_ADDRESS", "CUSTODY_WALLET_PATH", "CUSTODY_WALLET_PASSPHRASE",
  "TENSORIUM_MC_STATE", "OP_RPC_URL", "OPERATOR_PRIVATE_KEY",
  "CONTROLLER_ADDRESS", "TOKEN_ADDRESS",
  "RELAYER_API_KEY",
];
for (const k of REQUIRED) {
  if (!process.env[k]) { console.error(`[relayer] missing env: ${k}`); process.exit(1); }
}

// Ensure pending deposits directory exists
mkdirSync(join(__dirname, "deposits", "pending"), { recursive: true });

console.log("[relayer] Phase 9A bridge relayer starting");
console.log("[relayer] custody    :", process.env.CUSTODY_ADDRESS);
console.log("[relayer] controller :", process.env.CONTROLLER_ADDRESS);
console.log("[relayer] api port   :", process.env.RELAYER_API_PORT || "3004");

await startApiServer();

Promise.allSettled([
  runDepositWatcher().catch((err) => { console.error("[deposit] fatal:", err); process.exit(1); }),
  runWithdrawalWatcher().catch((err) => { console.error("[withdrawal] fatal:", err); process.exit(1); }),
]);
```

- [ ] **Generate API key and add to `.env`:**

```bash
cd /root/tensorium-bridge-relayer
API_KEY=$(node -e "console.log(require('crypto').randomBytes(32).toString('hex'))")
echo "" >> .env
echo "RELAYER_API_PORT=3004" >> .env
echo "RELAYER_API_KEY=${API_KEY}" >> .env
echo "DISCORD_ALERT_WEBHOOK=" >> .env   # fill in after Discord webhook URL obtained
echo "Generated RELAYER_API_KEY: ${API_KEY}"
```

Save the API_KEY output — needed for bridge website integration.

- [ ] **Verify index.js syntax:**

```bash
node --check /root/tensorium-bridge-relayer/index.js && echo "syntax OK"
```

---

## Task 8: Run all tests + restart pm2

- [ ] **Run all tests:**

```bash
cd /root/tensorium-bridge-relayer
node --test test/alerts.test.js test/retry.test.js test/api.test.js 2>&1 | tail -15
```

Expected: all pass (alerts: 2, retry: 3, api: 6 = 11 total)

- [ ] **Restart pm2:**

```bash
pm2 restart tensorium-bridge-relayer
sleep 5
pm2 logs tensorium-bridge-relayer --lines 12 --nostream 2>/dev/null | grep -v "^$" | tail -12
```

Expected output includes:
```
[relayer] Phase 9A bridge relayer starting
[api] listening on http://127.0.0.1:3004
[deposit] watching custody address: txm13ydx...
[withdrawal] watching from block: ...
```

- [ ] **Smoke test health endpoint:**

```bash
curl -sf http://127.0.0.1:3004/health | python3 -m json.tool
```

Expected:
```json
{
  "ok": true,
  "uptime": 5,
  "lastDepositCheckAt": null,
  "lastWithdrawalBlock": 0,
  "pendingDeposits": 0,
  "failedMints": 0,
  "failedReleases": 0
}
```

- [ ] **Smoke test deposit API:**

```bash
curl -sf -X POST http://127.0.0.1:3004/deposit \
  -H "Authorization: Bearer $(grep RELAYER_API_KEY /root/tensorium-bridge-relayer/.env | cut -d= -f2)" \
  -H "Content-Type: application/json" \
  -d "{\"txid\":\"$(head -c 32 /dev/urandom | xxd -p)\",\"outputIndex\":0,\"recipient\":\"0x1234567890123456789012345678901234567890\"}" | python3 -m json.tool
```

Expected: `{ "queued": true, "txid": "...", "recipient": "0x..." }`

---

## Task 9: Nginx + DNS + certbot for `bridge-api.tensoriumlabs.com`

- [ ] **Create nginx site config:**

```bash
cat > /etc/nginx/sites-available/bridge-api.tensoriumlabs.com << 'EOF'
server {
    listen 80;
    server_name bridge-api.tensoriumlabs.com;

    location / {
        proxy_pass         http://127.0.0.1:3004;
        proxy_set_header   Host $host;
        proxy_set_header   X-Real-IP $remote_addr;
        proxy_read_timeout 30s;
    }
}
EOF
```

- [ ] **Enable site and reload nginx:**

```bash
ln -sf /etc/nginx/sites-available/bridge-api.tensoriumlabs.com \
       /etc/nginx/sites-enabled/
nginx -t && nginx -s reload && echo "nginx OK"
```

- [ ] **Add DNS A record** (manual step — do this in your DNS provider):

```
bridge-api.tensoriumlabs.com  A  157.230.44.162
```

Wait 1–2 minutes for propagation, then verify:
```bash
dig +short bridge-api.tensoriumlabs.com
```
Expected: `157.230.44.162`

- [ ] **Run certbot:**

```bash
certbot --nginx -d bridge-api.tensoriumlabs.com --non-interactive --agree-tos \
  -m dev@tensoriumlabs.com 2>&1 | tail -5
```

Expected: `Successfully deployed certificate`

- [ ] **Final smoke test via public HTTPS:**

```bash
curl -sf https://bridge-api.tensoriumlabs.com/health | python3 -m json.tool
```

Expected: `{ "ok": true, ... }`

---

## Task 10: Add Discord webhook URL to `.env` + final verification

- [ ] **Get Discord webhook URL** (manual step):
  1. Go to Discord server → Settings → Integrations → Webhooks
  2. Create new webhook in `#bridge-wtxm` channel
  3. Copy URL

- [ ] **Add webhook to .env:**

```bash
# Replace empty DISCORD_ALERT_WEBHOOK= line
sed -i "s|DISCORD_ALERT_WEBHOOK=|DISCORD_ALERT_WEBHOOK=<paste-url-here>|" \
  /root/tensorium-bridge-relayer/.env
```

- [ ] **Restart and verify alert fires:**

```bash
pm2 restart tensorium-bridge-relayer
sleep 3

# Test alert (temporarily trigger by calling sendAlert directly)
node -e "
import('./alerts.js').then(({sendAlert}) =>
  sendAlert('Test alert', 'Bridge relayer productionization complete. Alerts working.')
).catch(console.error)
"
```

Expected: Discord message appears in `#bridge-wtxm` channel.

- [ ] **Run monitor to confirm all green:**

```bash
bash /usr/local/bin/tensorium-monitor.sh && tail -5 /var/log/tensorium-monitor.log
```

Expected: `STATUS: OK`

---

## Self-Review Checklist

- [x] **Spec coverage:** LOCK fix ✓ | withRetry ✓ | sendAlert triggers ✓ | POST /deposit ✓ | GET /health ✓ | state schema ✓ | nginx ✓ | certbot ✓ | env vars ✓
- [x] **No placeholders:** All tasks have complete code
- [x] **Type consistency:** `withRetry(label, fn, maxAttempts, delay)` — 4-param signature used consistently. `sendAlert(title, body)` — 2-param used consistently. `startApiServer()` — returns `Promise<Server>` — consistent in tests and index.js
- [x] **`updateDepositCheck` / `updateWithdrawalBlock`** defined in Task 5, imported in Task 4/5 steps
- [x] **`failedMints` / `failedReleases`** added to DEFAULT in Task 5, referenced in Task 3/4
- [x] **`lastDepositCheckAt` / `lastWithdrawalBlock`** in DEFAULT (Task 5), returned in `/health` (Task 6)
