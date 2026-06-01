# Phase 9A Signer Rotation Procedure

Status: operational procedure for rotating the bridge multisig signer set.
Last updated: 2026-06-01

## When to Rotate a Signer

Rotate a signer key immediately if any of the following:
- Signer device is lost, stolen, or suspected compromised
- Signer operator leaves the project
- Signer key was ever stored on a public VPS (violates layout policy)
- Scheduled rotation (recommended: every 12 months)

Do NOT wait. A suspected compromise must be treated as a confirmed compromise.

## Phase 9A Context

Phase 9A uses a single EOA as owner (not yet a multisig). The signer rotation
procedure below applies when the multisig is activated in Phase 9B.

For Phase 9A EOA rotation, follow the Emergency Owner Transfer section below.

## Standard Signer Rotation (Phase 9B+ Multisig)

### Step 1 — Prepare new signer key

- Generate new keypair on an air-gapped device or hardware wallet
- Confirm new address is not on any VPS
- Record new address, share only with other signers

### Step 2 — Pause bridge (precautionary)

- Call `controller.pause()` from current owner multisig
- Announce pause on Telegram: "Bridge paused for signer rotation. Back online in ~1h."
- Do NOT announce which signer is being rotated

### Step 3 — Add new signer to multisig

- Using remaining valid signers, propose and confirm `addOwner(newAddress)` tx
- Confirm on-chain that new signer is in the signer set

### Step 4 — Remove old signer

- Propose and confirm `removeOwner(oldAddress)` tx
- Verify threshold is still met (2-of-3 or adjusted)

### Step 5 — Unpause bridge

- Call `controller.unpause()` from owner multisig
- Verify bridge is operational: test one small deposit

### Step 6 — Document

- Update PHASE9A_SIGNER_CUSTODY_LAYOUT.md with new signer set
- Add entry to PHASE9A_INCIDENT_LOG.md with rotation details
- Announce resume on Telegram

## Emergency Owner Transfer (Phase 9A — single EOA)

If the Phase 9A EOA owner key (0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB) is compromised:

### Step 1 — Pause immediately

```
controller.pause()  — from compromised key if still accessible
token.pause()       — from compromised key if still accessible
```

If key is inaccessible: the bridge is already at risk. Skip to Step 3.

### Step 2 — Move custody

- If TXM custody address is safe: leave funds there
- If custody key is also compromised: announce emergency publicly, users cannot withdraw

### Step 3 — Deploy new contracts

- Generate new deployer key (air-gapped)
- Deploy new WrappedTensorium + TensoriumBridgeController to Optimism mainnet
- New deployment is the canonical bridge going forward

### Step 4 — Communicate

- Post on Telegram: "Bridge emergency. Old contracts deprecated. New addresses: [new addresses]."
- Update bridge.tensoriumlabs.com immediately with new contract addresses

### Step 5 — Handle old wTXM

- Old wTXM on the compromised contracts cannot be backed by new custody
- Post public statement with exact scope of impact
- Attempt to honor withdrawals manually from custody if funds are safe

## Checklist for Any Rotation

- [ ] New key generated off-VPS
- [ ] Bridge paused before rotation
- [ ] Rotation executed and verified on-chain
- [ ] Bridge unpaused and tested
- [ ] PHASE9A_SIGNER_CUSTODY_LAYOUT.md updated
- [ ] Incident log entry added
- [ ] Telegram announcement posted
