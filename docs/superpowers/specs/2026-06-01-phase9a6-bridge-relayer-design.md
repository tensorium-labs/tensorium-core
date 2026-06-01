# Phase 9A.6 Bridge Relayer + Launch Page — Design Spec

Date: 2026-06-01
Status: approved

## Context

Phase 9A.6 Launch Preparation. Build auto bridge relayer (TXM ↔ wTXM Optimism) + update bridge.tensoriumlabs.com dari "Coming Soon BSC" ke live Phase 9A Beta page. Deploy contracts ke Optimism mainnet sekalian.

## Approach

Approach A: txmwallet subprocess. Node.js handles deposit monitoring + Optimism minting. TXM release via `txmwallet send && broadcast` subprocess. Bridge page static HTML, no backend.

## Architecture

```
bridge-relayer/          (repo: tensorium-labs/tensorium-bridge-relayer)
├── index.js              entry point, start kedua loop
├── deposit-watcher.js    loop A: Tensorium → wTXM mint
├── withdrawal-watcher.js loop B: wTXM burn → TXM release
├── txm-client.js         wrapper GET /getutxos, /getblockcount
├── op-client.js          wrapper mintFromTensoriumDeposit + event polling
├── state.js              baca/tulis relayer-state.json (atomic write)
├── relayer-state.json    persisted state
├── deposits/pending/     <txid>.json files for recipient mapping
├── .env                  keys + config (gitignored)
└── package.json
```

## Deposit Watcher (Loop A, 60s interval)

1. `GET /getutxos/<custody_addr>` → filter UTXO baru
2. Skip if `tip_height - created_height < MIN_CONFIRMATIONS` (6 blocks, ~12 min)
3. Skip if already in `state.processedUtxos`
4. Read `deposits/pending/<txid>.json` → get recipient Optimism address
5. `bridgeEventId = keccak256(abi.encodePacked(txid, outputIndex))`
6. Call `controller.mintFromTensoriumDeposit(bridgeEventId, txid, recipient, amount)`
7. Save to state

## Withdrawal Watcher (Loop B, 30s interval)

1. Poll Optimism from `state.lastProcessedBlock` for `WithdrawalRequested` events
2. Skip if `bridgeEventId` in `state.processedWithdrawals`
3. Convert: `atoms = (amount * 1e8n) / 1e18n` (wTXM 18 decimals → TXM 8 decimals)
4. Shell: `txmwallet send <tensoriumAddress> <atoms>`
5. Shell: `txmwallet broadcast`
6. Save to state, update `lastProcessedBlock`

## State Schema

```json
{
  "processedUtxos": {
    "<txid>:<outputIndex>": { "mintTx": "0x...", "recipient": "0x...", "ts": "ISO" }
  },
  "processedWithdrawals": {
    "<bridgeEventId>": { "txmTxid": "...", "tensoriumAddr": "...", "atoms": 0, "ts": "ISO" }
  },
  "lastProcessedBlock": 0
}
```

Atomic write: write to `relayer-state.json.tmp` then rename.

## Decimal Conversion

- TXM: 8 decimal places → 1 TXM = 1e8 atoms
- wTXM: 18 decimal places (ERC-20 standard) → 1 wTXM = 1e18 wei
- Conversion: `atoms = (weiAmount * BigInt(1e8)) / BigInt(1e18)`

## Deposit Recipient Mapping

User submits deposit request via bridge page (static form):
- Input: Optimism address + expected TXM amount
- Output: custody address + instructions
- Bridge page generates a `txid placeholder` instruction: "send TXM, then submit txid here"
- For Phase 9A: operator manually creates `deposits/pending/<txid>.json` after user confirms txid
- File: `{ "recipient": "0x...", "amount_atoms": 0, "submitted_at": "ISO" }`

## .env Config

```
TENSORIUM_MC_RPC=https://mc-rpc.tensoriumlabs.com
CUSTODY_ADDRESS=<generated TXM address>
CUSTODY_WALLET_PATH=/root/.tensorium-bridge/custody-wallet.json
TENSORIUM_MC_STATE=/root/tensorium-mc-state.json
OP_RPC_URL=https://mainnet.optimism.io
OPERATOR_PRIVATE_KEY=<operator key for minting>
CONTROLLER_ADDRESS=<mainnet OP controller>
TOKEN_ADDRESS=<mainnet OP token>
MIN_CONFIRMATIONS=6
POLL_INTERVAL_DEPOSIT_MS=60000
POLL_INTERVAL_WITHDRAWAL_MS=30000
```

## Bridge Page Updates

- Title: "Tensorium Bridge — TXM ↔ wTXM (Optimism)"
- Remove "Coming Soon" / BSC references
- Add "Phase 9A Beta" banner with conservative caps notice
- Sections: How It Works, Custody Address, Limits (10K wTXM/tx), Risk Disclosure, Bridge Hours, Incident Path
- Deposit form: enter Optimism address → show custody address + instructions
- Chain display: Tensorium ↔ Optimism (not BSC)
- Bridge hours: Mon–Sat 08:00–22:00 WIB, review 2x/day
- Incident path: status.tensoriumlabs.com + Telegram community

## Deploy Sequence

1. Deploy contracts ke Optimism mainnet
2. Generate custody TXM wallet (`txmwallet create`)
3. Build relayer repo, push to tensorium-labs/tensorium-bridge-relayer
4. Deploy relayer ke VPS, pm2 start
5. Update bridge page with custody address + contract addresses
6. Update nginx if needed

## Out of Scope

- Automatic deposit recipient mapping (Phase 9B)
- Multi-sig custody (Phase 9B)
- Telegram bot notifications (Phase 9B)
- wTXM supply cap on-chain (already handled by maxPerTx)
