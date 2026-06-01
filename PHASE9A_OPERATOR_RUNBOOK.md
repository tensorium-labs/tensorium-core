# Phase 9A Operator Runbook

Status: first operating runbook for the public Tensorium <-> Optimism bridge.
Last updated: 2026-06-01

This runbook describes the human/operator flow for deposits, mints, burns, and
releases in the first bridge version.

## Scope

This is for the first public bridge release only:

- Tensorium native TXM on one side
- Optimism `wTXM` on the other side
- multisig-operated bridge

It is not a trustless bridge runbook.

## Preconditions Before Opening The Bridge

Before accepting public flow:

- signer set is active
- custody address is published
- `wTXM` contract is deployed
- bridge controller is deployed
- pause path is tested
- ledger template exists
- public docs exist
- one end-to-end internal drill succeeded

## Daily Roles

Minimum role split:

- operator on duty
- reviewer or second checker
- multisig signer set

One person should not perform every sensitive action alone.

## Deposit Runbook: TXM -> wTXM

### 1. Detect Deposit

- monitor the published Tensorium bridge deposit address
- record any incoming TXM transfer in the ledger as `detected`
- capture the user destination Optimism address from the bridge request flow

### 2. Wait For Confirmations

- apply the current fixed confirmation threshold
- do not mint early for convenience
- update the ledger to `confirmed` once the threshold is reached

### 3. Review Deposit

- verify deposit amount
- verify recipient Optimism address format
- verify the event was not already processed
- verify no incident pause is active

### 4. Prepare Mint

- create the mint request on Optimism
- record the draft mint details in the ledger as `mint_prepared`
- obtain whatever signer/operator approvals are required by policy

### 5. Execute Mint

- submit the mint transaction
- wait for inclusion and confirmation on Optimism
- record mint tx hash and block number
- mark the ledger entry as `minted`

### 6. Post-Mint Check

- confirm the user received `wTXM`
- verify reserve delta still matches supply delta
- include the event in the next reconciliation summary

## Withdrawal Runbook: wTXM -> TXM

### 1. Detect Burn

- monitor the bridge burn flow on Optimism
- capture burn tx hash, amount, and destination Tensorium address
- mark the ledger entry as `burn_detected`

### 2. Wait For Burn Confirmations

- apply the current confirmation threshold
- update the ledger to `burn_confirmed` after threshold is met

### 3. Review Withdrawal

- verify burn amount
- verify destination Tensorium address format
- verify the burn has not already been released
- verify reserve sufficiency before release

### 4. Prepare Release

- prepare the TXM release transaction from custody
- record planned release details in the ledger as `release_prepared`
- obtain required review/approval under the operator policy

### 5. Execute Release

- send TXM to the target Tensorium address
- wait for chain inclusion
- record release txid and block details
- mark the ledger entry as `released`

### 6. Post-Release Check

- verify no duplicate release occurred
- confirm reserve balance still reconciles
- include the event in the next reconciliation summary

## Pause / Incident Runbook

Pause immediately if:

- reserve mismatch is detected
- duplicate mint risk is detected
- invalid burn mapping is detected
- signer compromise is suspected
- contract ownership state is unclear

When paused:

- stop new minting
- stop new releases unless the incident policy explicitly allows exceptions
- mark affected ledger entries as `paused`
- post a status update on the public bridge/status surface

## Reconciliation Runbook

At least once per day:

1. total locked TXM reserve
2. total minted `wTXM`
3. total burned `wTXM`
4. net circulating `wTXM`
5. unresolved entries by status
6. incident or exception notes

Output:

- daily reconciliation note
- explicit statement whether reserve matches net wrapped supply

## No-Go Conditions

Do not open public bridge flow if any of these are missing:

- tested pause procedure
- functioning ledger template
- signer ownership clarity
- custody reserve clarity
- operator/reviewer separation

## First Release Discipline

Phase 9A first release should be intentionally conservative:

- manual review is acceptable
- delayed withdrawals are acceptable
- capacity caps are acceptable
- limited bridge hours are acceptable

What is not acceptable:

- undocumented exceptions
- silent manual overrides
- minting without ledger updates
- releasing without burn verification
