# tensorium-sdk-js Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and publish `@tensorium/sdk` — a TypeScript npm package for querying balance, building/signing/broadcasting transactions on the Tensorium chain, compatible with Node.js and browser.

**Architecture:** 4 focused source files (types, rpc, wallet, tx) with a thin index.ts re-export. Dual ESM+CJS build via Vite library mode. All crypto via `@noble/secp256k1` + `@noble/hashes` + `bech32` — zero native deps, browser-safe.

**Tech Stack:** TypeScript, `@noble/secp256k1` v2, `@noble/hashes`, `bech32`, Vite library mode, vitest

---

## Known Test Vectors (computed from Rust source)

```
# transaction_id vector
input:  txid=[0x00]*32, output_index=0, sig_script=empty
output: value_atoms=100_000_000 (1 TXM), address="txm1qtest"
payload: empty
expected txid: f1502ea322ee70ca9761b78cec26c14986c67bfbea11e8435c5441d527893f7a

# address derivation vector
private_key:  0101010101010101010101010101010101010101010101010101010101010101
public_key:   031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f
expected addr: txm178gjqyjqdwr6lvnldhgk4s98dlx6540dtczms0
```

---

## File Map

| File | Action |
|---|---|
| `tensorium-sdk-js/package.json` | Create |
| `tensorium-sdk-js/tsconfig.json` | Create |
| `tensorium-sdk-js/vite.config.ts` | Create |
| `tensorium-sdk-js/src/types.ts` | Create |
| `tensorium-sdk-js/src/rpc.ts` | Create |
| `tensorium-sdk-js/src/wallet.ts` | Create |
| `tensorium-sdk-js/src/tx.ts` | Create |
| `tensorium-sdk-js/src/index.ts` | Create |
| `tensorium-sdk-js/test/sdk.test.ts` | Create |

---

## Task 1: Init project + install deps

**Files:**
- Create: `tensorium-sdk-js/package.json`
- Create: `tensorium-sdk-js/tsconfig.json`
- Create: `tensorium-sdk-js/vite.config.ts`

- [ ] **Step 1: Create directory and package.json**

```bash
mkdir -p /root/.openclaw/workspace/tensorium-sdk-js/src /root/.openclaw/workspace/tensorium-sdk-js/test
```

Write `/root/.openclaw/workspace/tensorium-sdk-js/package.json`:

```json
{
  "name": "@tensorium/sdk",
  "version": "0.1.0",
  "description": "JavaScript/TypeScript SDK for the Tensorium Proof-of-Work chain",
  "type": "module",
  "main": "./dist/index.cjs",
  "module": "./dist/index.mjs",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": {
      "import": "./dist/index.mjs",
      "require": "./dist/index.cjs",
      "types": "./dist/index.d.ts"
    }
  },
  "files": ["dist"],
  "scripts": {
    "build": "vite build && tsc --emitDeclarationOnly --declaration --outDir dist",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "keywords": ["tensorium", "txm", "blockchain", "sdk", "pow"],
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "https://github.com/tensorium-labs/tensorium-sdk-js.git"
  },
  "dependencies": {
    "@noble/hashes": "^1.4.0",
    "@noble/secp256k1": "^2.1.0",
    "bech32": "^2.0.0"
  },
  "devDependencies": {
    "typescript": "^5.4.0",
    "vite": "^5.2.0",
    "vite-plugin-dts": "^3.9.0",
    "vitest": "^1.5.0"
  }
}
```

- [ ] **Step 2: Create tsconfig.json**

Write `/root/.openclaw/workspace/tensorium-sdk-js/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "declaration": true,
    "declarationDir": "dist",
    "outDir": "dist",
    "rootDir": "src",
    "skipLibCheck": true
  },
  "include": ["src"]
}
```

- [ ] **Step 3: Create vite.config.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/vite.config.ts`:

```typescript
import { defineConfig } from 'vite';
import { resolve } from 'path';
import dts from 'vite-plugin-dts';

export default defineConfig({
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      name: 'TensoriumSdk',
      fileName: 'index',
      formats: ['es', 'cjs'],
    },
    rollupOptions: {
      external: ['@noble/secp256k1', '@noble/hashes/sha256', 'bech32'],
    },
  },
  plugins: [dts({ include: ['src'] })],
});
```

- [ ] **Step 4: Install dependencies**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm install
```

Expected: `added N packages`

---

## Task 2: types.ts

**Files:**
- Create: `tensorium-sdk-js/src/types.ts`

- [ ] **Step 1: Write types.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/src/types.ts`:

```typescript
export type Utxo = {
  txid: string;            // 64-char hex, no 0x prefix
  output_index: number;    // u32
  value_atoms: bigint;     // u64 — 1 TXM = 100_000_000 atoms
  created_height: number;
  mature: boolean;
};

export type TxOutput = {
  address: string;         // "txm1q..."
  value_atoms: bigint;
};

// Shape sent to /sendrawtransaction — matches Tensorium Rust serde output
export type RawTx = {
  id: number[];            // 32-byte array
  inputs: Array<{
    previous_output: { txid: number[]; output_index: number };
    signature_script: number[];  // UTF-8 bytes of JSON sig script
  }>;
  outputs: Array<{ value_atoms: number; address: string }>;
  payload: number[];
};

export class InsufficientBalance extends Error {
  constructor(have: bigint, need: bigint) {
    super(`Insufficient balance: have ${have} atoms, need ${need} atoms`);
    this.name = 'InsufficientBalance';
  }
}
```

- [ ] **Step 2: Init git repo**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && git init && git branch -M main
```

---

## Task 3: wallet.ts (TDD)

**Files:**
- Create: `tensorium-sdk-js/src/wallet.ts`
- Create: `tensorium-sdk-js/test/sdk.test.ts` (wallet section)

- [ ] **Step 1: Write failing wallet tests**

Write `/root/.openclaw/workspace/tensorium-sdk-js/test/sdk.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import { TxmWallet } from '../src/wallet.js';
import { txId, buildAndSign, selectUtxos } from '../src/tx.js';
import { TxmRPC } from '../src/rpc.js';
import type { Utxo, TxOutput } from '../src/types.js';

// ── Test vectors (computed from Rust source) ─────────────────────────────────
const TEST_PRIV = '0101010101010101010101010101010101010101010101010101010101010101';
const TEST_PUB  = '031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f';
const TEST_ADDR = 'txm178gjqyjqdwr6lvnldhgk4s98dlx6540dtczms0';

// transaction_id vector: 1 input (all-zero txid, index 0, empty sig) →
//   1 output (100_000_000 atoms, "txm1qtest") → empty payload
const TX_VECTOR_EXPECTED = 'f1502ea322ee70ca9761b78cec26c14986c67bfbea11e8435c5441d527893f7a';

// ── Wallet tests ──────────────────────────────────────────────────────────────
describe('TxmWallet', () => {
  it('generates a random wallet with txm1 address', () => {
    const w = TxmWallet.generate();
    expect(w.address).toMatch(/^txm1/);
    expect(w.privateKeyHex).toHaveLength(64);
    expect(w.publicKeyHex).toHaveLength(66);
  });

  it('restores from private key — address matches test vector', () => {
    const w = TxmWallet.fromPrivateKey(TEST_PRIV);
    expect(w.address).toBe(TEST_ADDR);
    expect(w.publicKeyHex).toBe(TEST_PUB);
    expect(w.privateKeyHex).toBe(TEST_PRIV);
  });

  it('two different privkeys produce different addresses', () => {
    const a = TxmWallet.generate();
    const b = TxmWallet.generate();
    expect(a.address).not.toBe(b.address);
  });
});

// ── tx tests ─────────────────────────────────────────────────────────────────
describe('txId', () => {
  it('matches Rust test vector', () => {
    const inputs = [{
      previous_output: { txid: new Uint8Array(32), output_index: 0 },
      signature_script: new Uint8Array(0),
    }];
    const outputs: TxOutput[] = [{ address: 'txm1qtest', value_atoms: 100_000_000n }];
    expect(txId(inputs, outputs)).toBe(TX_VECTOR_EXPECTED);
  });
});

describe('selectUtxos', () => {
  const utxos: Utxo[] = [
    { txid: 'a'.repeat(64), output_index: 0, value_atoms: 50_000_000n, created_height: 1, mature: true },
    { txid: 'b'.repeat(64), output_index: 0, value_atoms: 80_000_000n, created_height: 2, mature: true },
    { txid: 'c'.repeat(64), output_index: 0, value_atoms: 20_000_000n, created_height: 3, mature: false },
  ];

  it('selects minimum sufficient mature UTXOs', () => {
    const selected = selectUtxos(utxos, 50_000_000n);
    expect(selected.length).toBe(1);
    expect(selected[0].value_atoms).toBe(50_000_000n);
  });

  it('skips immature UTXOs', () => {
    const selected = selectUtxos(utxos, 60_000_000n);
    const total = selected.reduce((s, u) => s + u.value_atoms, 0n);
    expect(total).toBeGreaterThanOrEqual(60_000_000n);
    expect(selected.every(u => u.mature)).toBe(true);
  });

  it('throws InsufficientBalance when mature balance too low', () => {
    const { InsufficientBalance } = await import('../src/types.js');
    expect(() => selectUtxos(utxos, 200_000_000n)).toThrow(InsufficientBalance);
  });
});

describe('buildAndSign', () => {
  it('produces a RawTx with correct structure', () => {
    const wallet = TxmWallet.fromPrivateKey(TEST_PRIV);
    const utxos: Utxo[] = [{
      txid: 'a'.repeat(64), output_index: 0,
      value_atoms: 200_000_000n, created_height: 1, mature: true,
    }];
    const outputs: TxOutput[] = [{ address: TEST_ADDR, value_atoms: 100_000_000n }];
    const tx = buildAndSign(wallet, utxos, outputs);
    expect(tx.id).toHaveLength(32);
    expect(tx.inputs).toHaveLength(1);
    expect(tx.outputs).toHaveLength(2); // payment + change
    const sigScript = JSON.parse(Buffer.from(tx.inputs[0].signature_script).toString());
    expect(sigScript.public_key_hex).toBe(TEST_PUB);
    expect(sigScript.signature_hex).toMatch(/^[0-9a-f]+$/);
  });
});

// ── RPC tests ─────────────────────────────────────────────────────────────────
describe('TxmRPC', () => {
  it('getBlockCount calls /getblockcount', async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ height: 100, chain_id: 'tensorium-testnet-0' }),
    });
    const rpc = new TxmRPC('https://rpc.example.com', mockFetch as any);
    const result = await rpc.getBlockCount();
    expect(result.height).toBe(100);
    expect(mockFetch).toHaveBeenCalledWith('https://rpc.example.com/getblockcount');
  });

  it('getUtxos calls correct URL', async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ tip_height: 100, utxo_count: 0, utxos: [], address: 'txm1test' }),
    });
    const rpc = new TxmRPC('https://rpc.example.com', mockFetch as any);
    await rpc.getUtxos('txm1qtest');
    expect(mockFetch).toHaveBeenCalledWith('https://rpc.example.com/getutxos/txm1qtest');
  });

  it('sendRawTransaction POSTs JSON body', async () => {
    const mockFetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ txid: 'abc' }),
    });
    const rpc = new TxmRPC('https://rpc.example.com', mockFetch as any);
    const tx = { id: [], inputs: [], outputs: [], payload: [] };
    const result = await rpc.sendRawTransaction(tx);
    expect(result.txid).toBe('abc');
    const [url, opts] = mockFetch.mock.calls[0];
    expect(url).toBe('https://rpc.example.com/sendrawtransaction');
    expect(opts.method).toBe('POST');
    expect(JSON.parse(opts.body)).toEqual(tx);
  });
});
```

- [ ] **Step 2: Run tests — confirm they all FAIL**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm test 2>&1 | tail -15
```

Expected: all tests fail with import errors (files don't exist yet).

- [ ] **Step 3: Implement wallet.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/src/wallet.ts`:

```typescript
import * as secp from '@noble/secp256k1';
import { sha256 } from '@noble/hashes/sha256';
import { bech32 } from 'bech32';

const ADDRESS_HRP = 'txm';

function pubkeyToAddress(compressedPubkey: Uint8Array): string {
  const hash = sha256(compressedPubkey);
  const payload = hash.slice(0, 20);
  const words = bech32.toWords(payload);
  return bech32.encode(ADDRESS_HRP, words);
}

export class TxmWallet {
  readonly privateKeyHex: string;
  readonly publicKeyHex: string;
  readonly address: string;

  private constructor(privateKeyBytes: Uint8Array) {
    const pubkey = secp.getPublicKey(privateKeyBytes, true); // compressed
    this.privateKeyHex = Buffer.from(privateKeyBytes).toString('hex');
    this.publicKeyHex = Buffer.from(pubkey).toString('hex');
    this.address = pubkeyToAddress(pubkey);
  }

  static generate(): TxmWallet {
    return new TxmWallet(secp.utils.randomPrivateKey());
  }

  static fromPrivateKey(hex: string): TxmWallet {
    const bytes = Uint8Array.from(Buffer.from(hex, 'hex'));
    return new TxmWallet(bytes);
  }
}
```

- [ ] **Step 4: Run wallet tests only — confirm they pass**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm test -- --reporter=verbose 2>&1 | grep -E "✓|✗|PASS|FAIL|wallet" | head -10
```

Expected: `TxmWallet` describe block all passing.

---

## Task 4: tx.ts (TDD)

**Files:**
- Create: `tensorium-sdk-js/src/tx.ts`

- [ ] **Step 1: Implement tx.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/src/tx.ts`:

```typescript
import * as secp from '@noble/secp256k1';
import { sha256 } from '@noble/hashes/sha256';
import type { Utxo, TxOutput, RawTx } from './types.js';
import { InsufficientBalance } from './types.js';
import type { TxmWallet } from './wallet.js';

type UnsignedInput = {
  previous_output: { txid: Uint8Array; output_index: number };
  signature_script: Uint8Array;
};

function doubleSha256(bytes: Uint8Array): Uint8Array {
  return sha256(sha256(bytes));
}

function le32(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n, true);
  return b;
}

function le64(n: bigint): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, n, true);
  return b;
}

export function txId(
  inputs: UnsignedInput[],
  outputs: TxOutput[],
  payload: Uint8Array = new Uint8Array(0)
): string {
  const parts: Uint8Array[] = [];
  for (const inp of inputs) {
    parts.push(inp.previous_output.txid);
    parts.push(le32(inp.previous_output.output_index));
    parts.push(inp.signature_script);
  }
  for (const out of outputs) {
    parts.push(le64(out.value_atoms));
    parts.push(new TextEncoder().encode(out.address));
  }
  parts.push(payload);

  const total = parts.reduce((n, p) => n + p.length, 0);
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const p of parts) { bytes.set(p, offset); offset += p.length; }

  return Buffer.from(doubleSha256(bytes)).toString('hex');
}

export function selectUtxos(utxos: Utxo[], targetAtoms: bigint): Utxo[] {
  const mature = utxos.filter(u => u.mature);
  const sorted = [...mature].sort((a, b) => Number(b.value_atoms - a.value_atoms));
  const selected: Utxo[] = [];
  let total = 0n;
  for (const u of sorted) {
    selected.push(u);
    total += u.value_atoms;
    if (total >= targetAtoms) return selected;
  }
  const have = mature.reduce((s, u) => s + u.value_atoms, 0n);
  throw new InsufficientBalance(have, targetAtoms);
}

export function buildAndSign(
  wallet: TxmWallet,
  utxos: Utxo[],
  outputs: TxOutput[],
  payload: Uint8Array = new Uint8Array(0)
): RawTx {
  // 1. Build unsigned inputs
  const unsignedInputs: UnsignedInput[] = utxos.map(u => ({
    previous_output: {
      txid: Uint8Array.from(Buffer.from(u.txid, 'hex')),
      output_index: u.output_index,
    },
    signature_script: new Uint8Array(0),
  }));

  // 2. Add change output if needed
  const totalIn = utxos.reduce((s, u) => s + u.value_atoms, 0n);
  const totalOut = outputs.reduce((s, o) => s + o.value_atoms, 0n);
  const allOutputs = [...outputs];
  if (totalIn > totalOut) {
    allOutputs.push({ address: wallet.address, value_atoms: totalIn - totalOut });
  }

  // 3. Compute signature hash (txId with empty sig scripts)
  const sigHashHex = txId(unsignedInputs, allOutputs, payload);
  const sigHashBytes = Uint8Array.from(Buffer.from(sigHashHex, 'hex'));

  // 4. Sign
  const privBytes = Uint8Array.from(Buffer.from(wallet.privateKeyHex, 'hex'));
  const sig = secp.sign(sigHashBytes, privBytes);
  const derHex = Buffer.from(sig.toDERRawBytes()).toString('hex');

  // 5. Build signature script
  const sigScriptJson = JSON.stringify({
    public_key_hex: wallet.publicKeyHex,
    signature_hex: derHex,
  });
  const sigScriptBytes = Array.from(new TextEncoder().encode(sigScriptJson));

  // 6. Build signed inputs
  const signedInputs = unsignedInputs.map(inp => ({
    previous_output: {
      txid: Array.from(inp.previous_output.txid),
      output_index: inp.previous_output.output_index,
    },
    signature_script: sigScriptBytes,
  }));

  // 7. Compute final txid (with signatures included)
  const signedForId: UnsignedInput[] = signedInputs.map(inp => ({
    previous_output: {
      txid: Uint8Array.from(inp.previous_output.txid),
      output_index: inp.previous_output.output_index,
    },
    signature_script: Uint8Array.from(inp.signature_script),
  }));
  const finalTxIdHex = txId(signedForId, allOutputs, payload);

  return {
    id: Array.from(Uint8Array.from(Buffer.from(finalTxIdHex, 'hex'))),
    inputs: signedInputs,
    outputs: allOutputs.map(o => ({
      value_atoms: Number(o.value_atoms),
      address: o.address,
    })),
    payload: Array.from(payload),
  };
}

export async function send(
  rpc: import('./rpc.js').TxmRPC,
  wallet: TxmWallet,
  to: string,
  atoms: bigint,
  payload?: Uint8Array
): Promise<string> {
  const { utxos } = await rpc.getUtxos(wallet.address);
  const parsedUtxos: Utxo[] = utxos.map((u: any) => ({
    ...u,
    value_atoms: BigInt(u.value_atoms),
  }));
  const selected = selectUtxos(parsedUtxos, atoms);
  const tx = buildAndSign(wallet, selected, [{ address: to, value_atoms: atoms }], payload);
  const result = await rpc.sendRawTransaction(tx);
  return result.txid;
}
```

- [ ] **Step 2: Run tx tests — confirm they pass**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm test -- --reporter=verbose 2>&1 | grep -E "✓|✗|PASS|FAIL|txId|selectUtxos|buildAndSign" | head -15
```

Expected: all txId, selectUtxos, buildAndSign tests pass.

---

## Task 5: rpc.ts (TDD)

**Files:**
- Create: `tensorium-sdk-js/src/rpc.ts`

- [ ] **Step 1: Implement rpc.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/src/rpc.ts`:

```typescript
import type { RawTx } from './types.js';

type FetchFn = typeof fetch;

export class TxmRPC {
  private readonly url: string;
  private readonly fetchFn: FetchFn;

  constructor(url: string, fetchFn: FetchFn = fetch) {
    this.url = url.replace(/\/$/, '');
    this.fetchFn = fetchFn;
  }

  private async get<T>(path: string): Promise<T> {
    const res = await this.fetchFn(`${this.url}${path}`);
    if (!res.ok) throw new Error(`RPC ${path} → HTTP ${res.status}`);
    return res.json();
  }

  async getBlockCount(): Promise<{ height: number; chain_id: string }> {
    return this.get('/getblockcount');
  }

  async getUtxos(address: string): Promise<{
    tip_height: number;
    utxo_count: number;
    utxos: Array<{
      txid: string;
      output_index: number;
      value_atoms: number;
      created_height: number;
      mature: boolean;
    }>;
  }> {
    return this.get(`/getutxos/${encodeURIComponent(address)}`);
  }

  async sendRawTransaction(tx: RawTx): Promise<{ txid: string }> {
    const res = await this.fetchFn(`${this.url}/sendrawtransaction`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(tx),
    });
    if (!res.ok) throw new Error(`sendRawTransaction → HTTP ${res.status}`);
    return res.json();
  }
}
```

- [ ] **Step 2: Run all tests — confirm all pass**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm test 2>&1 | tail -10
```

Expected:
```
Test Files  1 passed (1)
Tests       8 passed (8)
```

---

## Task 6: index.ts + build

**Files:**
- Create: `tensorium-sdk-js/src/index.ts`

- [ ] **Step 1: Write index.ts**

Write `/root/.openclaw/workspace/tensorium-sdk-js/src/index.ts`:

```typescript
export { TxmWallet } from './wallet.js';
export { TxmRPC } from './rpc.js';
export { txId, selectUtxos, buildAndSign, send } from './tx.js';
export type { Utxo, TxOutput, RawTx } from './types.js';
export { InsufficientBalance } from './types.js';

/** Convenience factory */
export function createClient(rpcUrl: string): TxmRPC {
  return new (require('./rpc.js').TxmRPC)(rpcUrl);
}
```

Wait — the `createClient` function uses `require` which won't work in ESM. Fix:

```typescript
export { TxmWallet } from './wallet.js';
export { TxmRPC } from './rpc.js';
export { txId, selectUtxos, buildAndSign, send } from './tx.js';
export type { Utxo, TxOutput, RawTx } from './types.js';
export { InsufficientBalance } from './types.js';
```

- [ ] **Step 2: Run build**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm run build 2>&1
```

Expected:
```
dist/index.mjs    ~15KB
dist/index.cjs    ~15KB
dist/index.d.ts   generated
```

If build fails due to `vite-plugin-dts` issues, run tsc separately:
```bash
npx tsc --declaration --emitDeclarationOnly --outDir dist
```

- [ ] **Step 3: Smoke test the built package from Node.js**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && node --input-type=module << 'EOF'
import { TxmWallet, TxmRPC, txId } from './dist/index.mjs';

const wallet = TxmWallet.fromPrivateKey('0101010101010101010101010101010101010101010101010101010101010101');
console.log('address:', wallet.address);
console.log('expected:', 'txm178gjqyjqdwr6lvnldhgk4s98dlx6540dtczms0');
console.log('match:', wallet.address === 'txm178gjqyjqdwr6lvnldhgk4s98dlx6540dtczms0' ? 'OK' : 'FAIL');

const id = txId(
  [{ previous_output: { txid: new Uint8Array(32), output_index: 0 }, signature_script: new Uint8Array(0) }],
  [{ address: 'txm1qtest', value_atoms: 100_000_000n }]
);
console.log('txid:', id);
console.log('expected:', 'f1502ea322ee70ca9761b78cec26c14986c67bfbea11e8435c5441d527893f7a');
console.log('match:', id === 'f1502ea322ee70ca9761b78cec26c14986c67bfbea11e8435c5441d527893f7a' ? 'OK' : 'FAIL');
EOF
```

Expected: both `OK`.

- [ ] **Step 4: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && git add -A && git commit -m "$(cat <<'EOF'
feat: @tensorium/sdk v0.1.0

- TxmWallet: generate, fromPrivateKey, address derivation (bech32 txm1)
- TxmRPC: getBlockCount, getUtxos, sendRawTransaction
- tx: txId (matches Rust test vector), selectUtxos, buildAndSign, send
- 8 tests passing, dual ESM+CJS build

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Push to GitHub + publish to npm

- [ ] **Step 1: Create GitHub repo**

```bash
GH_TOKEN=$(grep "^token=" /root/.openclaw/password.txt | head -1 | cut -d= -f2) && \
curl -s -X POST https://api.github.com/user/repos \
  -H "Authorization: token $GH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"tensorium-sdk-js","description":"JavaScript/TypeScript SDK for the Tensorium PoW chain","private":false}' \
  | python3 -c "import sys,json; r=json.load(sys.stdin); print(r.get('html_url', r.get('message')))"
```

- [ ] **Step 2: Push to GitHub**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && \
GH_TOKEN=$(grep "^token=" /root/.openclaw/password.txt | head -1 | cut -d= -f2) && \
git remote add origin https://tensorium-labs:${GH_TOKEN}@github.com/tensorium-labs/tensorium-sdk-js.git && \
git push -u origin main
```

- [ ] **Step 3: Create README.md before publish**

Write `/root/.openclaw/workspace/tensorium-sdk-js/README.md`:

```markdown
# @tensorium/sdk

JavaScript / TypeScript SDK for the [Tensorium](https://tensoriumlabs.com) Proof-of-Work chain.

## Install

```bash
npm install @tensorium/sdk
```

## Usage

```typescript
import { TxmWallet, TxmRPC, send } from '@tensorium/sdk';

const rpc    = new TxmRPC('https://rpc.tensoriumlabs.com');
const wallet = TxmWallet.fromPrivateKey(process.env.TXM_PRIVATE_KEY!);

// Check balance
const { utxos, tip_height } = await rpc.getUtxos(wallet.address);
const balance = utxos
  .filter(u => u.mature)
  .reduce((s, u) => s + BigInt(u.value_atoms), 0n);
console.log('Balance:', balance, 'atoms');

// Send 1 TXM
const txid = await send(rpc, wallet, 'txm1q...destination', 100_000_000n);
console.log('Sent:', txid);
```

## API

### `TxmWallet`
| Method | Description |
|---|---|
| `TxmWallet.generate()` | Create a new random wallet |
| `TxmWallet.fromPrivateKey(hex)` | Restore from 64-char hex private key |
| `.address` | `txm1q…` bech32 address |
| `.publicKeyHex` | 66-char compressed secp256k1 pubkey |
| `.privateKeyHex` | 64-char private key (keep secret) |

### `TxmRPC`
| Method | Description |
|---|---|
| `new TxmRPC(url)` | Create RPC client |
| `.getBlockCount()` | Current chain height |
| `.getUtxos(address)` | UTXOs for address |
| `.sendRawTransaction(tx)` | Broadcast signed tx |

### `send(rpc, wallet, to, atoms)`
High-level: select UTXOs, build, sign, broadcast. Returns txid.

### `selectUtxos(utxos, targetAtoms)`
UTXO selection — greedy, mature UTXOs only. Throws `InsufficientBalance`.

### `txId(inputs, outputs, payload?)`
Compute transaction hash (double SHA-256). Matches Tensorium node exactly.

## Chain

- 1 TXM = 100,000,000 atoms
- Addresses: `txm1q…` (bech32)
- RPC: `https://rpc.tensoriumlabs.com`
- Docs: `https://docs.tensoriumlabs.com`

## License

MIT
```

- [ ] **Step 4: Add .gitignore + commit README**

Write `/root/.openclaw/workspace/tensorium-sdk-js/.gitignore`:

```
node_modules/
dist/
*.tgz
```

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && git add README.md .gitignore && git commit -m "docs: add README and .gitignore" && git push
```

- [ ] **Step 5: Log in to npm and publish**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js && npm login
```

This will prompt for username/password/email. After login:

```bash
npm publish --access public
```

Expected:
```
npm notice Publishing to https://registry.npmjs.org/
+ @tensorium/sdk@0.1.0
```

If npm org `@tensorium` doesn't exist yet, first create it at https://www.npmjs.com/org/create — OR publish without scope first as `tensorium-sdk`:

```bash
# Alternative if @tensorium org not yet created:
# Edit package.json "name" → "tensorium-sdk", then:
npm publish
```

- [ ] **Step 6: Verify published package**

```bash
npm info @tensorium/sdk version
```

Expected: `0.1.0`

- [ ] **Step 7: Update Phase 9D checklist in myProject_PoW.md**

In `/root/.openclaw/workspace/myProject_PoW.md`, update Sprint 9D checklist:

```markdown
- [x] `tensorium-sdk-js`: npm package, query balance dan kirim TX
- [x] GitHub: github.com/tensorium-labs/tensorium-sdk-js
- [ ] `tensorium-sdk-py`: pip package, sama dengan JS (Phase 9D next)
- [ ] RPC API reference di docs.tensoriumlabs.com/api
- [ ] Contoh aplikasi dApp sederhana pakai SDK
```
