# Tensorium Chrome Wallet Extension — Design Spec
**Date:** 2026-06-01
**Phase:** 8B
**Status:** Approved

---

## 1. Overview

A Chrome extension wallet for Tensorium (TXM) targeting regular users who do not run a node.
Users connect to a public RPC proxy hosted on `tensoriumlabs.com`. All crypto runs in-extension
via JS libraries (no native binary, no WASM). Compatible with `txmwallet` CLI wallet file format.

**Repo:** `tensorium-wallet-extension`
**Local:** `/root/.openclaw/workspace/tensorium-wallet-extension`
**GitHub:** `https://github.com/tensorium-labs/tensorium-wallet-extension`

---

## 2. Architecture

### 2.1 Public RPC Proxy (VPS prerequisite)

Before the extension can serve general users, two HTTPS endpoints must be provisioned on VPS
`157.230.44.162`:

| Subdomain | Proxy target | Node type |
|-----------|-------------|-----------|
| `rpc.tensoriumlabs.com` | `127.0.0.1:33332` | Mainnet |
| `mc-rpc.tensoriumlabs.com` | `127.0.0.1:33332` | Mainnet Candidate |

Requirements:
- nginx reverse proxy with `TENSORIUM_RPC_ALLOW_PUBLIC=1` env on both node services
- `limit_req zone=rpc rate=10r/s burst=20` per IP to prevent abuse
- Let's Encrypt SSL (certbot, same as other subdomains)
- Both RPC services updated to allow non-loopback bind (still bind loopback; nginx proxies externally)

The extension ships with these two URLs hardcoded as defaults. Users can override in Settings
with a custom RPC URL.

### 2.2 Extension Stack

```
Manifest V3 Chrome Extension
├── React 18 + TypeScript (popup UI, 360×580px)
├── Vite (multi-entry build: popup + background service worker)
├── @noble/secp256k1     — k256 ECDSA key generation, signing, public key derivation
├── @scure/bech32        — bech32 address encode/decode (prefix: "txm")
├── argon2-browser       — Argon2id KDF (matching txmwallet CLI params)
├── @noble/ciphers               — XChaCha20Poly1305 encryption (same vendor as secp256k1)
└── chrome.storage.local — persistent encrypted wallet store
```

### 2.3 Source Layout

```
tensorium-wallet-extension/
├── manifest.json
├── src/
│   ├── popup/
│   │   ├── main.tsx
│   │   ├── App.tsx              — router: locked | onboarding | wallet
│   │   └── pages/
│   │       ├── Locked.tsx       — password unlock screen
│   │       ├── Onboarding.tsx   — create or import flow
│   │       ├── Dashboard.tsx    — address, balance, action buttons
│   │       ├── Send.tsx         — send form + confirm step
│   │       ├── History.tsx      — tx list fetched from RPC
│   │       └── Settings.tsx     — network, show privkey, export, lock
│   ├── lib/
│   │   ├── crypto.ts            — keygen, encrypt/decrypt, sign, address derive
│   │   ├── rpc.ts               — typed fetch wrappers for all RPC endpoints
│   │   ├── storage.ts           — chrome.storage.local typed read/write
│   │   └── session.ts           — in-memory session key (never touches storage)
│   └── background/
│       └── service_worker.ts    — clears session key on suspend/idle
├── public/icons/                — 16, 48, 128px PNG icons
├── vite.config.ts
├── tsconfig.json
└── package.json
```

---

## 3. Key Management & Crypto

### 3.1 Key Generation

```
1. private_key_bytes = crypto.getRandomValues(32 bytes)
2. public_key_bytes  = secp256k1.getPublicKey(private_key_bytes, compressed=true) → 33 bytes
3. address           = bech32.encode("txm", convertbits(public_key_bytes, 8, 5))
```

Address derivation is byte-for-byte identical to `WalletKeypair::from_public_key()` in Rust.

### 3.2 Wallet-at-Rest Encryption

Parameters are hardcoded to match `txmwallet` CLI exactly:

| Parameter | Value |
|-----------|-------|
| KDF | Argon2id |
| memory | 19456 KiB |
| iterations | 3 |
| parallelism | 1 |
| cipher | XChaCha20Poly1305 |
| salt | 32 bytes random |
| nonce | 24 bytes random |

Wallet JSON fields: `version`, `address`, `public_key_hex`, `encrypted_private_key` →
`{ kdf, kdf_memory_kib, kdf_iterations, kdf_parallelism, cipher, salt_hex, nonce_hex, ciphertext_hex }`.

A wallet `.json` exported from the extension can be opened with `txmwallet` CLI and vice versa.

### 3.3 Session Unlock (Option B)

- On unlock: Argon2id KDF + XChaCha20Poly1305 decrypt → plaintext private key held in
  `session.ts` module-level variable (in-memory only, never written to storage or disk).
- Background service worker listens to `chrome.runtime.onSuspend` → clears session variable.
- Manual lock (Settings → Lock) → clears session variable, UI returns to Locked screen.
- Session does NOT persist across browser restarts.

### 3.4 Transaction Signing

```
1. GET /getblocktemplate/<address>  → UTXOs available for address
2. Build tx: { inputs: [{txid, vout}], outputs: [{address, amount_atoms}] }
3. tx_hash = SHA256d(canonical JSON bytes)   — must match Rust exactly
4. signature = secp256k1.sign(tx_hash, private_key_bytes) → DER hex
5. POST /sendrawtransaction { tx, signature_hex, public_key_hex }
```

The canonical JSON serialization and SHA256d hashing must be verified against known test
vectors from `txmwallet` CLI before shipping.

---

## 4. UI Flows

### 4.1 Onboarding (first install)

```
Welcome screen
├── [Create Wallet]
│    → Generate keypair
│    → Set password (min 8 chars, strength meter)
│    → Encrypt + save to chrome.storage.local
│    → REQUIRED: Download backup JSON before proceeding
│         (button disabled until download triggered)
│    → Dashboard
└── [Import Wallet]
     ├── Paste private key hex  (64 hex chars)
     └── Upload .json file      (txmwallet-compatible)
     → Set password (re-encrypts with new password)
     → Dashboard
```

### 4.2 Returning User

```
Popup opens → check chrome.storage.local
├── No wallet → Onboarding
└── Wallet exists, session locked → Locked.tsx
     → Input password → decrypt → session.ts stores privkey
     → Dashboard
```

### 4.3 Dashboard

- Address displayed with one-click copy
- Balance in TXM (fetched from RPC on open, refresh button)
- Buttons: Send | History | Settings
- Network badge (Testnet / MC) top-right

### 4.4 Send Flow

```
Send page
→ Input: recipient address (bech32 validation inline)
→ Input: amount in TXM (validate: > 0, ≤ balance)
→ [Review] button → Confirm screen
   Shows: to, amount, warning "No fee, no reversal"
→ [Confirm & Send]
   → Sign tx → POST /sendrawtransaction
   → Success: show txid
   → Error: show node error message
```

### 4.5 History

- Fetch `/getblockcount` → height N
- Fetch `/getblock/0` through `/getblock/N` sequentially
- Filter outputs/inputs matching current address
- Display as list: height, direction (in/out), amount, block hash (truncated)
- Note: O(N) scan, acceptable for MVP while chain is young
- 10-second timeout per block fetch; skip + show warning if exceeded

### 4.6 Settings

- Network selector: Testnet | Mainnet Candidate | Custom RPC URL
- Show Private Key: re-verify password → display hex with copy button + warning
- Export Wallet JSON: download current wallet file
- Lock Wallet: clear session, return to Locked screen

---

## 5. Error Handling

| Scenario | Behavior |
|----------|----------|
| RPC unreachable | Red banner: "Node unreachable — check network or try again". Retry button. |
| Wrong password on unlock | Inline error: "Incorrect password". Storage not cleared. |
| Wrong password on Show Privkey | Inline error. No lockout. |
| Send: insufficient balance | Inline error before sign attempt. |
| Send: invalid recipient address | Inline bech32 validation, error shown before submit. |
| Send: node rejects tx | Show raw error from node response. |
| Import: bad private key hex | "Invalid private key — must be 64 hex characters". |
| Import: bad JSON format | "Invalid wallet file format". |
| History scan timeout | Skip block, show "N blocks unavailable" warning at top of list. |

---

## 6. Testing

| Layer | Tool | What is tested |
|-------|------|---------------|
| Crypto unit | Vitest | keygen → address roundtrip; encrypt → decrypt; sign → secp256k1 verify |
| CLI compat | Vitest | Import txmwallet-generated JSON, decrypt, verify address matches |
| TX signing compat | Vitest + known vectors | JS-signed tx hash matches Rust-computed hash for same inputs |
| RPC layer | Vitest + MSW | All RPC wrappers mocked; test happy path and error responses |
| UI flows | Vitest + React Testing Library | Onboarding, unlock, send, settings |
| Integration | Manual script | Sign tx in extension JS → submit to mainnet node → verify accepted |

---

## 7. Known Limitations (MVP)

- **History O(N):** Scan all blocks from genesis. Acceptable while MC height is 0. Fix in Phase 9B with a dedicated RPC endpoint or indexed explorer API.
- **Single address:** No HD wallet or derivation paths. One keypair per wallet, same as `txmwallet` CLI.
- **No dApp injection:** No `window.tensorium` provider. This is a pure wallet UI, not a MetaMask-style dApp bridge.
- **No fee estimation:** TXM has no per-tx fee (coinbase-only), so no fee UI needed.
- **No hardware wallet:** No Ledger/Trezor support in MVP.

---

## 8. Deployment

1. `npm run build` → `dist/` folder
2. Load unpacked in Chrome (`chrome://extensions → Load unpacked → dist/`)
3. Future: package as `.crx` or publish to Chrome Web Store after soak test

The extension does not require a server — it is fully static after build.
The only external dependency is the public RPC proxy on `tensoriumlabs.com`.
