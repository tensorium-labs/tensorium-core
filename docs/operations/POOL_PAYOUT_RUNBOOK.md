# Pool Payout Runbook

> **Pool treasury address, custody address, and bridge Safe:** see `../integrations/CANONICAL_ASSET_METADATA.md`.

This runbook defines how the reference pool handles custody separation between:

- founder cold wallet
- pool treasury wallet
- payout hot wallet

## Roles

### Founder Cold Wallet

- never used for routine pool operations
- never used to pay miners
- never stored on pool, node, or explorer hosts

### Pool Treasury Wallet

- receives pool fee revenue and pool-mined block rewards
- acts as reserve capital for pool operations
- should be treated as a higher-trust wallet than the payout hot wallet
- should not be the day-to-day wallet used to broadcast miner payouts

### Payout Hot Wallet

- operational wallet used to send miner payouts
- balance must be capped
- if compromised, operator loss should be bounded to the configured cap

## Policy

### Hard rules

1. Founder cold wallet must never fund routine miner payouts.
2. Pool treasury wallet and payout hot wallet must be distinct addresses.
3. Payout hot wallet must have a documented soft cap.
4. Refill from treasury to payout hot wallet must be explicit and logged.
5. `tensorium-pool mark-paid` must only be run after the payout transaction is actually broadcast and recorded.

### Recommended initial cap

- Start with a payout hot wallet cap of 3 to 7 days of expected payouts.
- If uncertain, choose the lower bound and refill more often.

## Suggested Environment Variables

```bash
TENSORIUM_POOL_TREASURY=<treasury_address>
TENSORIUM_POOL_PAYOUT_HOT_WALLET=<hot_wallet_address>
TENSORIUM_POOL_PAYOUT_HOT_MAX_ATOMS=<soft_cap_atoms>
TENSORIUM_POOL_LEDGER=/root/pool/pool-ledger.json
```

## Daily Operator Flow

1. Check pending payouts:

```bash
tensorium-pool stats
tensorium-pool accounting
```

2. Check custody metadata:

```bash
tensorium-pool custody
```

3. If hot wallet balance is below the operating threshold:
   - refill from treasury
   - record txid and amount in operator notes

4. Build and broadcast miner payouts from the payout hot wallet.

5. Only after payout broadcast:

```bash
tensorium-pool mark-paid <miner_address>
```

## Refill Flow

1. Review total pending net payouts.
2. Decide refill amount based on:
   - current hot wallet balance
   - next payout batch
   - configured soft cap
3. Send treasury -> payout hot wallet.
4. Record:
   - date/time
   - refill amount
   - txid
   - operator

## Incident Rules

### If payout hot wallet is suspected compromised

1. Stop issuing payouts from the hot wallet.
2. Move remaining funds to a fresh wallet if safe to do so.
3. Generate a new payout hot wallet.
4. Update `TENSORIUM_POOL_PAYOUT_HOT_WALLET`.
5. Resume payouts only after balance cap and operator notes are reset.

### If treasury wallet is suspected compromised

1. Pause refills.
2. Stop pool payout operations except emergency settlement decisions.
3. Escalate to treasury rotation procedure.

## Runtime Checks

At pool startup, confirm:

- `TENSORIUM_POOL_TREASURY` is set
- `TENSORIUM_POOL_PAYOUT_HOT_WALLET` is set
- `TENSORIUM_POOL_PAYOUT_HOT_WALLET != TENSORIUM_POOL_TREASURY`
- `TENSORIUM_POOL_PAYOUT_HOT_MAX_ATOMS` matches operator policy

Useful checks:

```bash
tensorium-pool custody
curl -fsS http://127.0.0.1:23336/pool/custody
```

## Status

Documented for Phase 10C on 2026-06-02.
