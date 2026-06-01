# Phase 9A Bridge Model Decision

Status: recommended execution model for the first Tensorium bridge release.
Last updated: 2026-06-01

This document narrows the Phase 9A bridge choice from a broad roadmap item into
an operational design that can actually be built and run.

## Decision

Use a multisig-operated bridge for the first public `wTXM` release on
Optimism.

Do not start with:

- a single-key custodial bridge
- a trust-minimized light-client bridge
- a generic multi-chain bridge product

## Why This Model

The first bridge needs to optimize for:

- realistic delivery time
- understandable operating model
- lower key-person risk
- enough controls to pause and recover cleanly

A trust-minimized bridge is better in theory, but it is not the right first
release for Tensorium. It adds too much protocol complexity before there is even
a proven TXM liquidity path.

## High-Level Model

Tensorium side:

- TXM is deposited to a published bridge custody address or a tightly controlled
  custody set
- deposits are not auto-released by contract logic on Tensorium in Phase 9A
- withdrawals are processed by operators after burn verification

Optimism side:

- `wTXM` is a standard ERC-20
- mint and burn are controlled by a dedicated bridge controller
- the bridge controller is owned by a multisig

Human/operator layer:

- operators observe deposits and burn requests
- operators reconcile bridge events against the ledger
- multisig signers authorize sensitive state transitions

## Recommended Roles

### 1. Multisig Signers

Responsibility:

- approve mint authority changes
- approve emergency pause / unpause
- approve contract ownership changes
- approve treasury / bridge wallet rotation

Recommended first setup:

- `2-of-3` for first release if the team is still small
- move to `3-of-5` after the operator set is mature

Constraint:

- signer keys should not all live on one host or with one person

### 2. Bridge Operators

Responsibility:

- watch Tensorium deposits
- prepare mint actions on Optimism
- watch burn events on Optimism
- prepare TXM withdrawal releases
- update the bridge ledger

Constraint:

- operators do not bypass multisig governance for privileged contract changes

### 3. Treasury / Custody Owner

Responsibility:

- custody of locked TXM reserves
- reserve verification
- reconciliation against circulating `wTXM`

Constraint:

- custody accounting must match on-chain and ledger state every day

## Minimum Contract Shape On Optimism

Required pieces:

- `wTXM` ERC-20 token
- bridge controller contract
- pause control
- role separation between operator and owner

Required contract powers:

- mint `wTXM` after confirmed Tensorium deposits
- burn `wTXM` for exit processing
- pause minting/burning in an incident

Required restrictions:

- no unrestricted mint path
- no single hot wallet as final owner
- no hidden admin path outside the multisig

## Deposit Flow

1. user sends TXM to the published Tensorium bridge deposit address
2. bridge operators wait for the required confirmation threshold
3. deposit is recorded in the bridge ledger
4. operator prepares the mint request on Optimism
5. required signer / operator policy is satisfied
6. `wTXM` is minted to the user destination address
7. ledger is updated with the final mint tx hash

## Withdrawal Flow

1. user initiates withdrawal by burning `wTXM`
2. burn event is detected on Optimism
3. operator verifies burn details and recipient mapping
4. burn is recorded in the bridge ledger
5. TXM is released from custody on Tensorium
6. ledger is updated with the final Tensorium tx hash

## Confirmation And Safety Policy

Recommended first-release rules:

- Tensorium deposit confirmations: conservative, fixed threshold
- Optimism burn confirmations: small fixed threshold before release
- withdrawal processing may be delayed during incidents or maintenance windows

Do not market this as instant or trustless in Phase 9A.

## Mandatory Operational Controls

Before public bridge launch, the team must have:

- a written bridge ledger format
- a reserve reconciliation routine
- a signer rotation procedure
- a lost-key procedure
- an emergency pause procedure
- a public incident/status communication path

Supporting docs:

- `PHASE9A_BRIDGE_LEDGER_FORMAT.md`
- `PHASE9A_OPERATOR_RUNBOOK.md`

## Ledger Requirements

Every bridge action should be traceable across both chains.

Minimum ledger fields:

- internal bridge event id
- user TXM deposit tx hash
- confirmed deposit amount
- destination Optimism address
- mint tx hash
- burn tx hash
- release tx hash
- status: pending / minted / burned / released / failed
- operator notes

## Public Risk Disclosure

The first public bridge docs must clearly state:

- this is a multisig-operated bridge
- this is not a trust-minimized bridge
- withdrawals can be delayed for review or incidents
- bridge capacity may be capped initially
- emergency pause can temporarily halt mint or release flows

## What Success Looks Like

Phase 9A bridge model is ready when:

- the signer set is chosen
- custody ownership is documented
- the ledger format exists
- deposit and withdrawal steps are written as a runbook
- one end-to-end drill succeeds on test infrastructure
- public docs explain the trust model honestly
