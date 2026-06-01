# Phase 9A Bridge Controller Spec

Status: implementation blueprint for the first Tensorium <-> Optimism bridge controller.
Last updated: 2026-06-01

This document defines the minimum expected behavior of the first bridge
controller contract on Optimism.

## Goal

Provide a small, auditable control layer that:

- mints `wTXM` after validated Tensorium deposits
- records bridge burn requests for withdrawals
- enforces pause and role controls

## Design Philosophy

The first controller should be intentionally conservative:

- simple role model
- explicit events
- minimal surface area
- off-chain operator workflow kept visible, not hidden

## Required Roles

### Owner

Recommended owner:

- multisig

Owner responsibilities:

- assign or revoke operator role
- assign or revoke pause authority if separated
- update policy-controlled parameters if contract includes them
- pause/unpause if owner is the emergency authority

### Operator

Recommended use:

- limited hot address for routine bridge actions

Operator responsibilities:

- submit mint after validated deposit
- confirm burn-side workflow inputs where applicable

### Pause Authority

Possible model:

- same as owner for first release

Acceptable alternative:

- dedicated emergency authority under tight governance

## Required State

Minimum tracked state:

- `wTXM` token address
- owner / role assignments
- pause flag
- processed deposit ids or processed bridge event ids
- processed burn ids if the controller enforces them

## Mint Flow

Expected flow:

1. off-chain operator validates a Tensorium deposit
2. operator calls controller mint function
3. controller verifies:
   - caller has operator role
   - bridge is not paused
   - bridge event id has not been used
4. controller mints `wTXM` to the target address
5. controller emits mint event with bridge metadata

## Burn / Withdrawal Flow

Expected first-release flow:

1. user initiates bridge withdrawal on Optimism
2. controller records or emits canonical burn request data
3. `wTXM` amount is burned or escrow transition is finalized according to token
   design
4. controller emits burn / withdrawal request event
5. off-chain operators process the Tensorium-side release

## Required Events

The controller should emit explicit bridge-level events, for example:

- deposit-mint event
- burn-withdrawal-request event
- pause event
- unpause event
- operator role update event

Exact Solidity event names can be finalized at implementation time, but bridge
operations must be observable from chain data.

## Replay Protection

The controller must prevent reusing the same bridge event twice.

Minimum requirement:

- every mint uses a unique bridge event id
- repeated use of the same id must revert

## Pause Semantics

When paused:

- minting must stop
- new withdrawal requests may stop if policy requires it
- privileged role changes should remain tightly controlled

## Non-Goals For First Release

Do not put these into the first controller unless clearly necessary:

- trustless verification of Tensorium headers
- generalized multi-chain routing
- fee markets
- automatic rebalancing
- complex governance modules

## Spec Success Condition

The bridge controller spec is ready when:

- owner/operator role split is clear
- mint path is defined
- burn request path is defined
- replay protection is defined
- pause behavior is defined
- expected emitted events are defined
