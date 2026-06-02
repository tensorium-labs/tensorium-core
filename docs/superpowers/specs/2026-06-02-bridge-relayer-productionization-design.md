# Bridge Relayer Productionization — Design Spec

**Date:** 2026-06-02
**Status:** Approved
**Scope:** `/root/tensorium-bridge-relayer/` on VPS 157.230.44.162

---

## Problem

The Phase 9A bridge relayer is live but has several gaps blocking production posture:

1. **RocksDB LOCK conflict** — `withdrawal-watcher.js` calls `txmwallet send` without `TENSORIUM_RPC`, causing a panic when the MC node holds the state.db lock. MC releases are silently broken.
2. **No retry logic** — Transient mint/release failures are logged and skipped permanently until the next poll cycle. Users can lose funds if a single RPC call fails.
3. **No alerting** — Failures go to pm2 logs only; no notification to operators.
4. **No HTTP API** — Deposit recipient mapping requires manual file creation per txid by an operator. Users cannot self-serve.
5. **No health endpoint** — The monitor only knows if the pm2 process is online, not whether it's processing correctly.

---

## Approach: Additive (Approach 1)

Extend the existing flat file structure with two new modules (`api-server.js`, `alerts.js`) and targeted fixes to the existing watchers. The core watcher logic, `op-client.js`, `txm-client.js`, and `state.js` are unchanged except for the LOCK fix and retry wrapper additions.

---

## File Map

| File | Action | What changes |
|---|---|---|
| `withdrawal-watcher.js` | Modify | Add `TENSORIUM_RPC` to `releaseTxm` env; wrap release in `withRetry`; call `sendAlert` on final failure; write to `failedReleases` in state |
| `deposit-watcher.js` | Modify | Wrap `mintFromDeposit` in `withRetry`; call `sendAlert` on final failure; write to `failedMints` in state; upgrade no-pending-file warn to alert |
| `index.js` | Modify | Call `startApiServer()` before starting watchers |
| `api-server.js` | Create | Express API: `POST /deposit`, `GET /health`; API-key middleware |
| `alerts.js` | Create | Discord webhook helper `sendAlert(title, body)` |
| `state.js` | Modify | Extend default state with `failedMints: {}` and `failedReleases: {}` fields |
| `.env` | Modify | Add `RELAYER_API_PORT`, `RELAYER_API_KEY`, `DISCORD_ALERT_WEBHOOK` |

Nginx: new site block for `bridge-api.tensoriumlabs.com` → `127.0.0.1:3004`.

---

## Bug Fix: RocksDB LOCK (withdrawal-watcher.js)

```js
// BEFORE — causes LOCK panic when MC node is running:
const env = {
  ...process.env,
  TENSORIUM_WALLET: process.env.CUSTODY_WALLET_PATH,
  TENSORIUM_WALLET_PASSPHRASE: process.env.CUSTODY_WALLET_PASSPHRASE,
  TENSORIUM_STATE: process.env.TENSORIUM_MC_STATE,
};

// AFTER — txmwallet uses RPC for UTXO lookup, no DB open:
const env = {
  ...process.env,
  TENSORIUM_WALLET: process.env.CUSTODY_WALLET_PATH,
  TENSORIUM_WALLET_PASSPHRASE: process.env.CUSTODY_WALLET_PASSPHRASE,
  TENSORIUM_STATE: process.env.TENSORIUM_MC_STATE,
  TENSORIUM_RPC: process.env.TENSORIUM_MC_RPC_LOCAL || "127.0.0.1:33332",
};
```

---

## Retry Logic (shared helper, added to both watchers)

```js
async function withRetry(label, fn, maxAttempts = 3) {
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (err) {
      const last = attempt === maxAttempts;
      console.error(`[${label}] attempt ${attempt}/${maxAttempts} failed: ${err.message}`);
      if (last) throw err;
      await new Promise(r => setTimeout(r, 5000 * attempt ** 2)); // 5s, 20s, 45s
    }
  }
}
```

Backoff schedule: attempt 1→2: wait 5s, attempt 2→3: wait 20s, then throw.

**Applied to:**
- `mintFromDeposit(...)` in deposit-watcher — 3 attempts; on final failure: call `sendAlert`, write to `state.failedMints[key]`, continue loop
- `releaseTxm(...)` in withdrawal-watcher — 3 attempts; on final failure: call `sendAlert`, write to `state.failedReleases[bridgeEventId]`, mark processed to prevent infinite retry
- `getWithdrawalEvents(...)` — 2 attempts; on failure: log + continue loop (no alert, transient network error)

**Extended state schema** (`relayer-state.json`):

```json
{
  "processedUtxos": {},
  "processedWithdrawals": {},
  "failedMints": {},
  "failedReleases": {},
  "lastProcessedBlock": 0
}
```

Each `failedMints[key]` entry: `{ utxo, recipient, atoms, error, ts, attempts }`.
Each `failedReleases[id]` entry: `{ tensoriumAddr, atoms, evmTx, error, ts, attempts }`.

---

## alerts.js

```js
export async function sendAlert(title, body) {
  const url = process.env.DISCORD_ALERT_WEBHOOK;
  if (!url) { console.warn("[alert] DISCORD_ALERT_WEBHOOK not set"); return; }
  await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      embeds: [{
        title: `🚨 Bridge Relayer: ${title}`,
        description: body,
        color: 0xff4444,
        timestamp: new Date().toISOString(),
      }],
    }),
  }).catch(e => console.error("[alert] Discord webhook failed:", e.message));
}
```

**Alert triggers:**

| Event | Title | Body content |
|---|---|---|
| Mint failed (3 attempts) | `"Mint failed"` | UTXO key, recipient, atoms, error message |
| Release failed (3 attempts) | `"Release failed"` | bridgeEventId, tensoriumAddr, atoms, error |
| Deposit confirmed, no pending file | `"Deposit needs mapping"` | UTXO key, atoms, created_height |
| Contract paused (detected before mint) | `"Bridge paused"` | Contract address, block number |

---

## api-server.js

**Port:** `process.env.RELAYER_API_PORT || 3004` (localhost only, nginx in front)

**Middleware:** All routes except `GET /health` require `Authorization: Bearer <RELAYER_API_KEY>`.

### `POST /deposit`

Request:
```json
{ "txid": "<64 hex chars>", "outputIndex": 0, "recipient": "0x<40 hex chars>" }
```

Validation:
- txid: `/^[0-9a-f]{64}$/` — reject if not 64 lowercase hex chars
- outputIndex: integer, 0–99
- recipient: `/^0x[0-9a-fA-F]{40}$/` — valid EVM address

Responses:
- `200 { queued: true, txid, recipient }` — file written to `deposits/pending/<txid>.json`
- `400 { error: "invalid txid" | "invalid recipient" | "invalid outputIndex" }` — validation failed
- `401 { error: "unauthorized" }` — missing or wrong API key
- `409 { error: "already queued" }` — pending file already exists

Pending file format (unchanged from existing schema):
```json
{ "recipient": "0x...", "txid": "...", "outputIndex": 0, "ts": "ISO" }
```

### `GET /health`

No auth required.

Response `200`:
```json
{
  "ok": true,
  "uptime": 3600,
  "lastDepositCheck": "2026-06-02T10:00:00Z",
  "lastWithdrawalBlock": 152500000,
  "pendingDeposits": 0,
  "failedMints": 0,
  "failedReleases": 0
}
```

`lastDepositCheck` and `lastWithdrawalBlock` are updated by the watchers after each successful poll cycle and stored in state.

---

## index.js change

```js
import { startApiServer } from "./api-server.js";
// ... existing env checks ...
startApiServer();   // non-blocking, starts Express
Promise.allSettled([
  runDepositWatcher().catch(...),
  runWithdrawalWatcher().catch(...),
]);
```

---

## Environment Variables (additions to .env)

```
RELAYER_API_PORT=3004
RELAYER_API_KEY=<generate: node -e "console.log(require('crypto').randomBytes(32).toString('hex'))">
DISCORD_ALERT_WEBHOOK=<Discord server webhook URL>
```

---

## Nginx (new site)

File: `/etc/nginx/sites-available/bridge-api.tensoriumlabs.com`

```nginx
server {
    listen 443 ssl;
    server_name bridge-api.tensoriumlabs.com;

    ssl_certificate     /etc/letsencrypt/live/tensoriumlabs.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/tensoriumlabs.com/privkey.pem;

    location / {
        proxy_pass         http://127.0.0.1:3004;
        proxy_set_header   Host $host;
        proxy_set_header   X-Real-IP $remote_addr;
        proxy_read_timeout 30s;
    }
}
```

Then:
```bash
ln -s /etc/nginx/sites-available/bridge-api.tensoriumlabs.com /etc/nginx/sites-enabled/
nginx -t && nginx -s reload                                   # enable HTTP first
certbot --nginx -d bridge-api.tensoriumlabs.com               # get cert
nginx -s reload                                               # reload with SSL
```
DNS A record `bridge-api.tensoriumlabs.com → 157.230.44.162` must exist before certbot.

---

## bridge.tensoriumlabs.com integration

The deposit form currently shows the custody address and tells users to send manually. After this change, the form submits to `https://bridge-api.tensoriumlabs.com/deposit` with the txid the user provides after sending.

The API key is stored as an env var on the VPS and never exposed to the browser — the bridge website backend makes the call server-side (or via a hidden server action).

If the bridge site is static HTML (no backend), the API key cannot be hidden. In that case: use a separate lightweight proxy on the VPS that accepts the form POST without a key and forwards it internally with the key added server-side.

---

## Tests

| Test | What |
|---|---|
| `POST /deposit` valid input | Returns 200, file written |
| `POST /deposit` bad txid | Returns 400 |
| `POST /deposit` bad address | Returns 400 |
| `POST /deposit` duplicate | Returns 409 |
| `POST /deposit` no auth | Returns 401 |
| `GET /health` | Returns 200 with expected shape |
| `withRetry` — first attempt succeeds | Returns immediately |
| `withRetry` — first fails, second succeeds | Returns on second try |
| `withRetry` — all fail | Throws after 3 attempts |
| LOCK fix | `releaseTxm` env contains `TENSORIUM_RPC` |

---

## What is NOT in scope

- Private key encryption / vault integration (deferred — requires operator workflow change)
- Automatic recipient address derivation (user still provides Optimism address)
- Bridge fee charging (no protocol fee in Phase 12)
- Multi-deposit batching
