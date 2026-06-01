# Phase 9A Bridge Ledger Format

Status: required operational record format for the first public `wTXM` bridge.
Last updated: 2026-06-01

This file defines the minimum ledger structure that bridge operators must keep
for every deposit, mint, burn, and release event.

## Purpose

The first Tensorium bridge is not trust-minimized. Because of that, the ledger
is part of the security model.

The ledger must let the team answer these questions at any time:

- how much TXM is locked in custody
- how much `wTXM` has been minted
- which user deposits have already been minted
- which burns have already been released
- whether reserve balance matches circulating wrapped supply

## Required Properties

The ledger must be:

- append-friendly
- auditable by operators
- easy to diff and back up
- explicit about status transitions
- able to map one bridge event across both chains

## Recommended Storage

Recommended first format:

- CSV or JSONL for raw event storage
- daily reconciliation summary in Markdown

Recommended repository stance:

- do not store live operational ledger data in the public repo
- keep only the template and field definitions in git

## Canonical Event Lifecycle

Every bridge record should move through a controlled lifecycle:

1. `detected`
2. `confirmed`
3. `mint_prepared`
4. `minted`
5. `burn_detected`
6. `burn_confirmed`
7. `release_prepared`
8. `released`
9. `failed`
10. `paused`

Not every event will use every state, but the meanings must stay consistent.

## Minimum Event Fields

Each bridge record should contain:

- `bridge_event_id`
- `direction`
- `status`
- `created_at_utc`
- `updated_at_utc`
- `tensorium_address`
- `optimism_address`
- `txm_amount_atoms`
- `txm_amount_display`
- `wtxm_amount_wei`
- `confirmation_policy`
- `operator_id`
- `reviewer_id`
- `notes`

## Deposit-Side Fields

For deposits from Tensorium into Optimism:

- `tensorium_deposit_txid`
- `tensorium_deposit_vout`
- `tensorium_block_hash`
- `tensorium_block_height`
- `tensorium_confirmations`
- `deposit_address`
- `mint_tx_hash`
- `mint_nonce`
- `mint_block_number`

## Burn-Side Fields

For withdrawals from Optimism back to Tensorium:

- `burn_tx_hash`
- `burn_log_index`
- `burn_block_number`
- `burn_confirmations`
- `tensorium_release_txid`
- `tensorium_release_block_hash`
- `tensorium_release_height`
- `release_address`

## Direction Values

Allowed values:

- `txm_to_wtxm`
- `wtxm_to_txm`

## Status Values

Allowed values:

- `detected`
- `confirmed`
- `mint_prepared`
- `minted`
- `burn_detected`
- `burn_confirmed`
- `release_prepared`
- `released`
- `failed`
- `paused`

## Example CSV Header

```text
bridge_event_id,direction,status,created_at_utc,updated_at_utc,tensorium_address,optimism_address,txm_amount_atoms,txm_amount_display,wtxm_amount_wei,confirmation_policy,operator_id,reviewer_id,tensorium_deposit_txid,tensorium_deposit_vout,tensorium_block_hash,tensorium_block_height,tensorium_confirmations,deposit_address,mint_tx_hash,mint_nonce,mint_block_number,burn_tx_hash,burn_log_index,burn_block_number,burn_confirmations,tensorium_release_txid,tensorium_release_block_hash,tensorium_release_height,release_address,notes
```

## Reconciliation Checks

At minimum, operators must run these checks daily:

1. sum of locked TXM reserve
2. sum of minted minus burned `wTXM`
3. unresolved pending deposits
4. unresolved pending withdrawals
5. failed or paused records needing intervention

## Exception Rules

If one user deposit appears twice:

- mark the duplicate candidate clearly
- do not mint again until manual review is complete

If a burn event has invalid recipient mapping:

- mark as `paused`
- escalate to operator review
- do not release TXM automatically

If custody reserve and wrapped supply diverge:

- stop new minting
- investigate before unpausing

## Template Policy

The template used by operators should be versioned.

Recommended fields for template metadata:

- `ledger_version`
- `generated_at_utc`
- `policy_version`

## Success Condition

This ledger format is good enough for Phase 9A when:

- every bridge event can be traced across both chains
- operators can reconcile reserves daily
- failed cases are visible, not hidden in chat or memory
- auditors can understand the lifecycle without guessing
