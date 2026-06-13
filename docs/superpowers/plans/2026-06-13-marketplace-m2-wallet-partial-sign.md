# Marketplace M2 — Wallet `signAssetTxPartial` + `signMessage` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Extend the Tensorium wallet extension with the two provider methods the order-relay needs: `signAssetTxPartial` (sign only the caller's inputs of a 2-of-2 settlement, no broadcast) and `signMessage` (authorize listing/accept/cancel), each behind a user-approval popup that recomputes figures from the transaction itself.

**Architecture:** Pure crypto additions in `lib/crypto.ts` (unit-tested with vitest), provider surface in `inpage/index.ts`, dispatch + approval-pend in `background/service_worker.ts`, and two React approval pages mirroring the existing `SignAssetTx.tsx` pattern (read pending request from `chrome.storage.session`, decrypt the session key, sign, write the result back).

**Tech Stack:** TypeScript, Vite, React, `@noble/secp256k1` v2, `@noble/hashes`, `@scure/base`, vitest. Repo: `tensorium-wallet-extension`. Spec: `tensorium-core/docs/superpowers/specs/2026-06-13-marketplace-wallet-native-trading-design.md` (Component C). Pairs with the order-relay's `sig.js` (M1) which verifies `signMessage` output.

## Background facts (verified in code)
- `signTransaction(tx, priv)` computes the sighash over **all inputs emptied** (`computeTxId(emptyInputs, outputs, payload)`), signs `sha256(sigHash)`, and stamps **every** input's `signature_script` with `[derSig.len, derSig, pubKey.len, pubKey]`. The sighash is invariant to existing signatures, so a buyer can sign inputs `1..` and a seller can later sign `input[0]` independently and both remain valid.
- The relay's `sig.js verifyOwnership` checks `secp.verify(sig, sha256(message), pubkey)` and `addressFromPubkey(pubkey) == address`. So `signMessage` must sign `sha256(utf8(message))` (single hash) and return DER hex + the compressed pubkey hex.
- Approval pages route via `App.tsx`: a `Page` union, pending-request detection reading `chrome.storage.session` keys, and post-unlock re-detection. The page reads its request, on confirm decrypts the session key via `getSession()`, acts, then writes `{...req, status:'confirmed', <result>}` back to the same session key and clears the badge.

---

## Task 1: `signMessage` + `signAssetTxInputs` in `lib/crypto.ts` (TDD)

**Files:**
- Modify: `src/lib/crypto.ts`
- Test: `src/__tests__/crypto-m2.test.ts` (new)

- [ ] **Step 1: Write the failing test** (`src/__tests__/crypto-m2.test.ts`)

```ts
import { describe, it, expect } from 'vitest';
import { sha256 } from '@noble/hashes/sha256';
import * as secp from '@noble/secp256k1';
import {
  signMessage, signAssetTxInputs, keypairFromPrivKeyHex, hexToBytes, bytesToHex,
  type WalletTx,
} from '../lib/crypto';

const PRIV = '11'.repeat(32);

describe('signMessage', () => {
  it('returns a DER signature over sha256(message) that verifies against the pubkey', async () => {
    const { publicKeyHex } = await keypairFromPrivKeyHex(PRIV);
    const msg = 'list:' + 'aa'.repeat(32) + ':100:5000000';
    const { pubkey, sig } = await signMessage(msg, hexToBytes(PRIV));
    expect(pubkey).toBe(publicKeyHex);
    const h = sha256(new TextEncoder().encode(msg));
    expect(secp.verify(sig, h, pubkey)).toBe(true); // sig is DER hex; @noble accepts it here
  });
  it('produces a signature that fails for a different message', async () => {
    const { sig, pubkey } = await signMessage('real', hexToBytes(PRIV));
    const h = sha256(new TextEncoder().encode('tampered'));
    expect(secp.verify(sig, h, pubkey)).toBe(false);
  });
});

describe('signAssetTxInputs', () => {
  const tx = (): WalletTx => ({
    inputs: [
      { previous_output: { txid_bytes: Array(32).fill(1), output_index: 0 }, signature_script: [] },
      { previous_output: { txid_bytes: Array(32).fill(2), output_index: 1 }, signature_script: [] },
    ],
    outputs: [{ value_atoms: 1000, script_pubkey: [0x76, 0xa9, 0x14, ...Array(20).fill(3), 0x88, 0xac] }],
    payload: [],
  });
  it('stamps ONLY the requested input indices, leaving others empty', async () => {
    const out = await signAssetTxInputs(tx(), hexToBytes(PRIV), [1]);
    expect(out.inputs[0].signature_script.length).toBe(0);   // untouched
    expect(out.inputs[1].signature_script.length).toBeGreaterThan(0); // signed
  });
  it('does not broadcast and does not claim a final txid', async () => {
    const out = await signAssetTxInputs(tx(), hexToBytes(PRIV), [0]);
    expect((out as { id?: unknown }).id).toBeUndefined();
  });
  it('two independent partial signs (buyer then seller) both land on the same tx', async () => {
    const buyerSigned = await signAssetTxInputs(tx(), hexToBytes(PRIV), [1]);
    const both = await signAssetTxInputs(buyerSigned, hexToBytes('22'.repeat(32)), [0]);
    expect(both.inputs[0].signature_script.length).toBeGreaterThan(0);
    expect(both.inputs[1].signature_script.length).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `npx vitest run src/__tests__/crypto-m2.test.ts`
Expected: FAIL — `signMessage` / `signAssetTxInputs` not exported.

- [ ] **Step 3: Implement in `src/lib/crypto.ts`** (append; reuse existing `sigToDER`, `computeTxId`, `concatBytes`, `bytesToHex`)

```ts
// Sign an arbitrary message for off-chain auth (listing/accept/cancel on the
// order-relay). Signs sha256(utf8(message)) — matches the relay's sig.js which
// verifies secp.verify(sig, sha256(message), pubkey). Returns DER hex + pubkey.
export async function signMessage(
  message: string,
  privKeyBytes: Uint8Array
): Promise<{ pubkey: string; sig: string }> {
  const h = sha256(new TextEncoder().encode(message));
  const sig = secp256k1.sign(h, privKeyBytes);
  const pubKey = secp256k1.getPublicKey(privKeyBytes, true);
  return { pubkey: bytesToHex(pubKey), sig: bytesToHex(sigToDER(sig)) };
}

// Partial signer for 2-of-2 asset settlement: stamps ONLY the given input
// indices with this key's signature, leaves all other inputs untouched, and does
// NOT broadcast or set a final id (the tx is not yet complete). The sighash is
// computed over all-inputs-emptied (matching signTransaction), so buyer (inputs
// 1..) and seller (input 0) can sign independently and both remain valid.
export async function signAssetTxInputs(
  tx: WalletTx,
  privKeyBytes: Uint8Array,
  inputIndices: number[]
): Promise<WalletTx> {
  const payload = new Uint8Array(tx.payload);
  const emptyInputs = tx.inputs.map((i) => ({ ...i, signature_script: [] as number[] }));
  const sigHash = computeTxId(emptyInputs, tx.outputs, payload);
  const sig = secp256k1.sign(sha256(sigHash), privKeyBytes);
  const pubKey = secp256k1.getPublicKey(privKeyBytes, true);
  const derSig = sigToDER(sig);
  const scriptBytes = Array.from(concatBytes([
    new Uint8Array([derSig.length]), derSig,
    new Uint8Array([pubKey.length]), pubKey,
  ]));
  const want = new Set(inputIndices);
  const inputs = tx.inputs.map((inp, idx) =>
    want.has(idx) ? { ...inp, signature_script: scriptBytes } : { ...inp });
  return { ...tx, inputs, payload: Array.from(payload) };
}
```

- [ ] **Step 4: Run it, verify it passes**

Run: `npx vitest run src/__tests__/crypto-m2.test.ts` → PASS (5 tests).
Then full suite: `npx vitest run` → all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/lib/crypto.ts src/__tests__/crypto-m2.test.ts
git commit -m "feat(wallet): signMessage + signAssetTxInputs (2-of-2 partial signing)"
```

---

## Task 2: pure `summarizeSettlement(tx)` for the approval hardening (TDD)

**Files:**
- Create: `src/lib/settlement-summary.ts`
- Test: `src/__tests__/settlement-summary.test.ts`

The approval popup must show what the user is actually agreeing to, recomputed from the tx (not the dapp-supplied summary). `summarizeSettlement` maps each output to `(address, atoms)` using the existing `extractAddressFromScriptPubKey`, so the UI can render the real destinations.

- [ ] **Step 1: Write the failing test**

```ts
import { describe, it, expect } from 'vitest';
import { summarizeSettlement } from '../lib/settlement-summary';

const p2pkh = (b: number) => [0x76, 0xa9, 0x14, ...Array(20).fill(b), 0x88, 0xac];

describe('summarizeSettlement', () => {
  it('lists each spendable output as {address, atoms}, skipping OP_RETURN/data outputs', () => {
    const tx = {
      inputs: [],
      outputs: [
        { value_atoms: 5_000_000, script_pubkey: p2pkh(7) },          // seller payout
        { value_atoms: 125_000, script_pubkey: p2pkh(9) },            // royalty
        { value_atoms: 1_000, script_pubkey: [0x6a, 0x04, 1, 2, 3, 4] }, // OP_RETURN data -> skipped
      ],
      payload: [],
    };
    const out = summarizeSettlement(tx as any);
    expect(out.outputs.length).toBe(2);
    expect(out.outputs[0].atoms).toBe(5_000_000);
    expect(out.outputs[0].address.startsWith('txm1')).toBe(true);
    expect(out.total_spendable_atoms).toBe(5_125_000);
  });
});
```

- [ ] **Step 2: Run it, verify it fails** → FAIL.

- [ ] **Step 3: Implement `src/lib/settlement-summary.ts`**

```ts
import { extractAddressFromScriptPubKey, type WalletTx } from './crypto';

export interface SettlementSummary {
  outputs: { address: string; atoms: number }[];
  total_spendable_atoms: number;
}

// Recompute the human-facing effect of a settlement tx from its outputs alone,
// so the approval UI never has to trust a dapp-supplied summary. Outputs that
// are not standard P2PKH (e.g. OP_RETURN asset-overlay data) are skipped.
export function summarizeSettlement(tx: WalletTx): SettlementSummary {
  const outputs: { address: string; atoms: number }[] = [];
  for (const o of tx.outputs) {
    const address = extractAddressFromScriptPubKey(o.script_pubkey);
    if (address) outputs.push({ address, atoms: o.value_atoms });
  }
  return { outputs, total_spendable_atoms: outputs.reduce((n, o) => n + o.atoms, 0) };
}
```

- [ ] **Step 4: Run it, verify it passes** → PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib/settlement-summary.ts src/__tests__/settlement-summary.test.ts
git commit -m "feat(wallet): summarizeSettlement — recompute approval figures from the tx"
```

---

## Task 3: Provider surface in `inpage/index.ts`

**Files:** Modify `src/inpage/index.ts`; Test: extend `src/__tests__/asset-tx.test.ts`

- [ ] **Step 1: Add the two methods** to the `window.tensorium` object (after `getAssets`)

```ts
    signAssetTxPartial: (unsignedTx: unknown, inputIndices: number[], summary: unknown) =>
      request('signAssetTxPartial', { unsignedTx, inputIndices, summary }),
    signMessage: (message: string) => request('signMessage', { message }),
```

- [ ] **Step 2: Add an assertion to `src/__tests__/asset-tx.test.ts`**

```ts
  it('exposes signAssetTxPartial and signMessage on window.tensorium', () => {
    const src = fs.readFileSync(path.join(__dirname, '../inpage/index.ts'), 'utf-8');
    expect(src).toContain('signAssetTxPartial:');
    expect(src).toContain('signMessage:');
  });
```

- [ ] **Step 3: Run + commit**

Run: `npx vitest run src/__tests__/asset-tx.test.ts` → PASS.
```bash
git add src/inpage/index.ts src/__tests__/asset-tx.test.ts
git commit -m "feat(wallet): expose signAssetTxPartial + signMessage to dapps"
```

---

## Task 4: Dispatch + approval-pend in `background/service_worker.ts`

**Files:** Modify `src/background/service_worker.ts`; Test: extend `src/__tests__/asset-tx.test.ts`

- [ ] **Step 1: Add method handling** in `handleDapp` (after the `signAssetTx` branch)

```ts
  if (method === 'signAssetTxPartial') {
    return await pendApproval('txm_partial_req', {
      unsignedTx: params['unsignedTx'], inputIndices: params['inputIndices'], summary: params['summary'],
    });
  }

  if (method === 'signMessage') {
    return await pendApproval('txm_signmsg_req', { message: params['message'] });
  }
```

- [ ] **Step 2: Add a generic `pendApproval` helper** (DRY — refactor `pendSignAssetTx`/`pendSendTransaction` can stay as-is; this serves the new flows). The result returned is whatever the popup writes under `result`:

```ts
// Generic pending-approval: stash a request in chrome.storage.session under
// `key`, raise the badge, open the popup, and resolve with the popup's `result`
// (or reject with its error). Used by signAssetTxPartial (returns the
// partially-signed tx) and signMessage (returns {pubkey, sig}).
async function pendApproval(key: string, payload: Record<string, unknown>): Promise<unknown> {
  const reqId = Date.now().toString();
  await (chrome.storage.session as any).set({ [key]: { reqId, ...payload, status: 'pending' } });
  await chrome.action.setBadgeText({ text: '1' });
  await chrome.action.setBadgeBackgroundColor({ color: '#ef4444' });
  await openApprovalPopup();
  const deadline = Date.now() + 10 * 60 * 1000;
  while (Date.now() < deadline) {
    await sleep(600);
    const data = await (chrome.storage.session as any).get(key);
    const req = data[key] as { reqId: string; status: string; result?: unknown; error?: string } | undefined;
    if (!req || req.reqId !== reqId || req.status === 'pending') continue;
    if (req.status === 'confirmed') return req.result;
    throw new Error(req.error ?? 'Request rejected');
  }
  await (chrome.storage.session as any).remove(key);
  await chrome.action.setBadgeText({ text: '' });
  throw new Error('Confirmation timed out — please try again');
}
```

- [ ] **Step 3: Add assertions** to `src/__tests__/asset-tx.test.ts`

```ts
  it('dispatcher handles signAssetTxPartial and signMessage', () => {
    const src = fs.readFileSync(path.join(__dirname, '../background/service_worker.ts'), 'utf-8');
    expect(src).toContain("method === 'signAssetTxPartial'");
    expect(src).toContain("method === 'signMessage'");
    expect(src).toContain('pendApproval');
  });
```

- [ ] **Step 4: Run + commit**

Run: `npx vitest run src/__tests__/asset-tx.test.ts` → PASS.
```bash
git add src/background/service_worker.ts src/__tests__/asset-tx.test.ts
git commit -m "feat(wallet): background dispatch + pendApproval for partial-sign and signMessage"
```

---

## Task 5: Approval page `SignAssetTxPartial.tsx`

**Files:** Create `src/popup/pages/SignAssetTxPartial.tsx`; Modify `src/popup/App.tsx`

Mirrors `SignAssetTx.tsx`, but: (a) signs only `req.inputIndices` via `signAssetTxInputs`, (b) does **not** broadcast — writes the partially-signed tx back as `result`, (c) renders `summarizeSettlement(req.unsignedTx)` (the recomputed destinations+amounts) instead of trusting `req.summary`.

- [ ] **Step 1: Create the page**

```tsx
import React, { useState } from 'react';
import { loadWallet } from '../../lib/storage';
import { getSession } from '../../lib/session';
import { hexToBytes, signAssetTxInputs, type WalletTx } from '../../lib/crypto';
import { summarizeSettlement } from '../../lib/settlement-summary';
import { BrandMark } from '../components/BrandMark';
import { ErrorBanner } from '../components/ErrorBanner';

export interface PartialReq {
  reqId: string;
  unsignedTx: WalletTx;
  inputIndices: number[];
  summary?: { description?: string; [k: string]: unknown };
  status: 'pending' | 'confirmed' | 'rejected';
  result?: unknown;
  error?: string;
}

const fmt = (a: number) => `${Math.floor(a / 1e8)}.${(a % 1e8).toString().padStart(8, '0')} TXM`;

export function SignAssetTxPartial({ req, onDone }: { req: PartialReq; onDone: () => void }) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const recomputed = summarizeSettlement(req.unsignedTx);

  const confirm = async () => {
    setError(''); setBusy(true);
    try {
      const wallet = await loadWallet();
      const privKeyHex = getSession();
      if (!wallet || !privKeyHex) { setError('Wallet is locked. Re-unlock to retry.'); return; }
      const signed = await signAssetTxInputs(req.unsignedTx, hexToBytes(privKeyHex), req.inputIndices);
      await (chrome.storage.session as any).set({ txm_partial_req: { ...req, status: 'confirmed', result: signed } });
      await chrome.action.setBadgeText({ text: '' });
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Signing failed.');
    } finally { setBusy(false); }
  };
  const reject = async () => {
    await (chrome.storage.session as any).set({ txm_partial_req: { ...req, status: 'rejected', error: 'User rejected' } });
    await chrome.action.setBadgeText({ text: '' });
    onDone();
  };

  return (
    <div className="wallet-page">
      <div className="wallet-topbar"><div className="wallet-brand"><BrandMark />
        <div className="wallet-brand-copy">
          <div className="wallet-eyebrow">marketplace.tensoriumlabs.com</div>
          <h2>Confirm Marketplace Trade</h2>
        </div></div></div>
      <div className="wallet-card" style={{ marginBottom: 10 }}>
        <div className="wallet-note">You are signing your part of an asset settlement. These are the exact payouts in the transaction:</div>
      </div>
      <div className="wallet-surface" style={{ padding: 16 }}>
        {recomputed.outputs.map((o, i) => (
          <div key={i} style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 6 }}>
            <span className="wallet-subtle" style={{ fontFamily: 'monospace', fontSize: 12 }}>
              {o.address.slice(0, 10)}…{o.address.slice(-6)}
            </span>
            <span>{fmt(o.atoms)}</span>
          </div>
        ))}
        <div className="wallet-divider"></div>
        <div style={{ display: 'flex', justifyContent: 'space-between' }}>
          <span className="wallet-section-label">Total in tx</span><strong>{fmt(recomputed.total_spendable_atoms)}</strong>
        </div>
      </div>
      {error && <ErrorBanner message={error} />}
      <div className="wallet-stack">
        <button onClick={confirm} disabled={busy} className="wallet-btn wallet-btn--primary">
          {busy ? 'Signing…' : 'Approve & Sign'}
        </button>
        <button onClick={reject} disabled={busy} className="wallet-btn wallet-btn--secondary">Reject</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire into `App.tsx`** — add `'asset-partial'` to the `Page` union; import `SignAssetTxPartial, type PartialReq`; add a `partialReq` state; in BOTH the initial-load and post-unlock pending-detection blocks read `txm_partial_req` and route to `'asset-partial'`; add `if (page === 'asset-partial' && partialReq) return <SignAssetTxPartial req={partialReq} onDone={() => nav('dashboard')} />;` to `content`. Follow the exact pattern already used for `txm_asset_req`/`asset-tx`.

- [ ] **Step 3: Build check + commit**

Run: `npx vitest run` (all green) and `npm run build` (compiles).
```bash
git add src/popup/pages/SignAssetTxPartial.tsx src/popup/App.tsx
git commit -m "feat(wallet): SignAssetTxPartial approval page (recomputes payouts from tx)"
```

---

## Task 6: Approval page `SignMessage.tsx`

**Files:** Create `src/popup/pages/SignMessage.tsx`; Modify `src/popup/App.tsx`

- [ ] **Step 1: Create the page** (shows the message + signing address; on confirm calls `signMessage`, writes `{pubkey, sig}` as `result`)

```tsx
import React, { useState, useEffect } from 'react';
import { loadWallet } from '../../lib/storage';
import { getSession } from '../../lib/session';
import { hexToBytes, signMessage } from '../../lib/crypto';
import { BrandMark } from '../components/BrandMark';
import { ErrorBanner } from '../components/ErrorBanner';

export interface SignMsgReq {
  reqId: string; message: string;
  status: 'pending' | 'confirmed' | 'rejected';
  result?: { pubkey: string; sig: string }; error?: string;
}

export function SignMessage({ req, onDone }: { req: SignMsgReq; onDone: () => void }) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [addr, setAddr] = useState('');
  useEffect(() => { loadWallet().then((w) => setAddr(w?.address ?? '')); }, []);

  const confirm = async () => {
    setError(''); setBusy(true);
    try {
      const privKeyHex = getSession();
      if (!privKeyHex) { setError('Wallet is locked. Re-unlock to retry.'); return; }
      const result = await signMessage(req.message, hexToBytes(privKeyHex));
      await (chrome.storage.session as any).set({ txm_signmsg_req: { ...req, status: 'confirmed', result } });
      await chrome.action.setBadgeText({ text: '' });
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Signing failed.');
    } finally { setBusy(false); }
  };
  const reject = async () => {
    await (chrome.storage.session as any).set({ txm_signmsg_req: { ...req, status: 'rejected', error: 'User rejected' } });
    await chrome.action.setBadgeText({ text: '' });
    onDone();
  };

  return (
    <div className="wallet-page">
      <div className="wallet-topbar"><div className="wallet-brand"><BrandMark />
        <div className="wallet-brand-copy">
          <div className="wallet-eyebrow">marketplace.tensoriumlabs.com</div>
          <h2>Signature Request</h2>
        </div></div></div>
      <div className="wallet-card" style={{ marginBottom: 10 }}>
        <div className="wallet-note">A dapp is asking you to sign a message to authorize a marketplace action. No funds move.</div>
      </div>
      <div className="wallet-surface" style={{ padding: 16 }}>
        <div className="wallet-section-label">Message</div>
        <div style={{ fontFamily: 'monospace', fontSize: 12, wordBreak: 'break-all', marginTop: 6 }}>{req.message}</div>
        <div className="wallet-divider"></div>
        <div className="wallet-section-label">Signing address</div>
        <div className="wallet-subtle" style={{ fontFamily: 'monospace', fontSize: 12 }}>{addr}</div>
      </div>
      {error && <ErrorBanner message={error} />}
      <div className="wallet-stack">
        <button onClick={confirm} disabled={busy} className="wallet-btn wallet-btn--primary">
          {busy ? 'Signing…' : 'Sign'}
        </button>
        <button onClick={reject} disabled={busy} className="wallet-btn wallet-btn--secondary">Reject</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire into `App.tsx`** — add `'sign-message'` to `Page`; import `SignMessage, type SignMsgReq`; add `signMsgReq` state; read `txm_signmsg_req` in both pending-detection blocks and route to `'sign-message'`; add the content branch. Same pattern as Task 5.

- [ ] **Step 3: Build + commit**

Run: `npx vitest run` (green) + `npm run build`.
```bash
git add src/popup/pages/SignMessage.tsx src/popup/App.tsx
git commit -m "feat(wallet): SignMessage approval page for marketplace auth"
```

---

## Task 7: Version bump + release build

**Files:** Modify `manifest.json`, `package.json`

- [ ] **Step 1:** Bump `version` to `0.1.8` in both `manifest.json` and `package.json`.
- [ ] **Step 2:** `npx vitest run` (full suite green) and `npm run build` (clean production build).
- [ ] **Step 3: Commit + push**

```bash
git add manifest.json package.json
git commit -m "chore(wallet): v0.1.8 — marketplace partial-sign + signMessage"
git push origin main
```

---

## Self-review

- **Spec coverage (Component C):** `signAssetTxPartial` (Task 1 crypto + Task 3 provider + Task 4 dispatch + Task 5 page) ✓; `signMessage` (Task 1 + Task 3 + Task 4 + Task 6) ✓; approval popups recompute from tx (Task 2 + Task 5 renders `summarizeSettlement`) ✓; non-breaking (existing `signAssetTx` untouched) ✓.
- **Interop with M1:** `signMessage` signs `sha256(message)` and returns DER hex + compressed pubkey — exactly what the relay `sig.js verifyOwnership` expects (it derives `addressFromPubkey` and `secp.verify(sig, sha256(message), pubkey)`).
- **2-of-2 correctness:** `signAssetTxInputs` computes the sighash over all-inputs-emptied (invariant to existing sigs) and stamps only requested indices — buyer (`1..`) then seller (`[0]`) compose. Verified by the third crypto test.
- **Placeholder scan:** none — full code for crypto, summary, provider, dispatch, both pages; App.tsx wiring described against the concrete existing pattern (Task 5/6 step 2).
- **Type consistency:** `signMessage → {pubkey, sig}` (matches relay), `signAssetTxInputs(tx, priv, indices) → WalletTx` (no `id`), `summarizeSettlement(tx) → {outputs[], total_spendable_atoms}`, session keys `txm_partial_req`/`txm_signmsg_req`, result-bearing `pendApproval`.

## Residual / follow-ups
- App.tsx wiring (Tasks 5/6 step 2) is described, not code-blocked, because it interleaves with existing JSX — the implementer follows the existing `txm_asset_req`/`asset-tx` pattern exactly. Build must pass.
- After release, M3 (frontend) consumes these methods; end-to-end (list → quote → partial-sign → settle → accept) is validated in M3 against the live relay + redeployed txmwallet.
- Loading the unpacked extension for a real browser click-through is a manual QA step at M3.
