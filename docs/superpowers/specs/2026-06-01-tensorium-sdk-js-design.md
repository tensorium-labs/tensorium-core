# tensorium-sdk-js Design Spec

Date: 2026-06-01
Status: approved

## Goal

npm package `@tensorium/sdk` — query balance, build + sign + broadcast transactions on the Tensorium chain. Works in Node.js and browser. Zero native dependencies.

## Crypto Layer

Serialization (from Rust source, exactly):

**transaction_id / signature_hash:**
```
bytes = []
for each input:  txid(32B raw) + output_index(4B LE u32) + sig_script(variable, empty for hash)
for each output: value_atoms(8B LE u64) + address(UTF-8 string bytes, no length prefix)
+ payload bytes
txid = SHA256(SHA256(bytes))
```

**Address derivation:**
```
compressed_pubkey (33B) → SHA256 → take first 20 bytes → bech32("txm", base32, Bech32 variant)
```

**Signature script:**
```json
{ "public_key_hex": "03...", "signature_hex": "<DER hex>" }
```
JSON-serialized to UTF-8 bytes, stored as signature_script field.

## Libraries

- `@noble/secp256k1` — secp256k1 ECDSA sign/verify, DER serialization
- `@noble/hashes` — SHA-256
- `bech32` — address encode/decode
- `vite` + `vitest` — build + test

## Package Structure

```
tensorium-sdk-js/
├── src/
│   ├── types.ts       — shared types only
│   ├── rpc.ts         — HTTP RPC client, zero crypto
│   ├── wallet.ts      — key generation + address derivation, no network
│   ├── tx.ts          — build, sign, select UTXOs, send
│   └── index.ts       — re-exports + createClient factory
├── test/
│   └── sdk.test.ts    — vitest, all offline
├── package.json       — @tensorium/sdk
├── tsconfig.json
└── vite.config.ts     — dual ESM+CJS library build
```

## API

```typescript
// types.ts
type Utxo = { txid: string; output_index: number; value_atoms: bigint; created_height: number; mature: boolean }
type TxOutput = { address: string; value_atoms: bigint }
type SignedTx = object  // raw JSON sent to /sendrawtransaction

// rpc.ts
class TxmRPC {
  constructor(url: string)
  getBlockCount(): Promise<{ height: number; chain_id: string }>
  getUtxos(address: string): Promise<{ tip_height: number; utxos: Utxo[] }>
  sendRawTransaction(tx: SignedTx): Promise<{ txid: string }>
}

// wallet.ts
class TxmWallet {
  static generate(): TxmWallet
  static fromPrivateKey(hex: string): TxmWallet
  get address(): string         // "txm1q..."
  get publicKeyHex(): string    // "03..." compressed
  get privateKeyHex(): string
}

// tx.ts
function selectUtxos(utxos: Utxo[], targetAtoms: bigint): Utxo[]
function buildAndSign(wallet: TxmWallet, utxos: Utxo[], outputs: TxOutput[], payload?: Uint8Array): SignedTx
async function send(rpc: TxmRPC, wallet: TxmWallet, to: string, atoms: bigint, payload?: Uint8Array): Promise<string>
```

## Test Coverage (all offline)

- `TxmWallet.generate()` → address starts with "txm1"
- `TxmWallet.fromPrivateKey()` → roundtrip matches
- `transaction_id` output matches known Rust test vector (hardcoded)
- `signature_script` JSON structure correct
- `selectUtxos` picks minimum sufficient UTXOs
- `selectUtxos` throws `InsufficientBalance` when short
- `TxmRPC.getUtxos` calls correct URL (mocked fetch)
- `TxmRPC.sendRawTransaction` POSTs correct body (mocked fetch)

## Build Output

`vite library mode` → `dist/index.mjs` (ESM) + `dist/index.cjs` (CJS) + `dist/index.d.ts`

## Publish

`npm publish --access public` → `@tensorium/sdk` on npmjs.com

## Out of Scope

- Python SDK (Phase 9D next)
- Hardware wallet support
- Multi-sig
- Fee estimation (Phase 9D next)
