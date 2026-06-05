# Phase 9A Bridge Policy

Status: first executable policy set for the public Tensorium <-> Optimism bridge.
Last updated: 2026-06-01

This document defines the initial guardrails for operating the first public
bridge release.

## Goal

Set conservative rules that are realistic for launch and easy to enforce.

Phase 9A policy should prefer:

- safety over speed
- manual review over automation
- explicit caps over unlimited flow
- reversible pauses over optimistic assumptions

## Signer Policy

Recommended initial signer model:

- `2-of-3` multisig at first public launch

Why:

- small team can move without adding too much coordination drag
- still avoids a single-key bridge
- fast enough for incident response

Upgrade path:

- move to `3-of-5` once:
  - more than three reliable key holders exist,
  - bridge volume is meaningful,
  - signer rotation procedure is tested

Rules:

- no two signer keys on the same VPS
- no signer key stored on the bridge operator hot host
- at least one signer key must stay cold or hardware-backed

## Confirmation Policy

Initial operational thresholds:

- Tensorium deposit mint threshold: `20` confirmations
- Optimism burn release threshold: `12` confirmations

Rationale:

- TXM side should be conservative because it anchors custody
- OP side can be faster, but still should not release immediately

Policy note:

- thresholds can be raised during incidents or high-risk periods
- thresholds should not be lowered ad hoc in private chat

## Bridge Capacity Policy

Initial caps:

- per-deposit minimum: `10 TXM`
- per-deposit soft max: `10,000 TXM`
- daily bridge mint cap: `50,000 TXM`
- daily bridge release cap: `50,000 TXM`

Interpretation:

- deposits above the soft max require manual approval
- daily caps are aggregate operational caps, not user promises

Why:

- prevents one early mistake from becoming treasury-scale damage
- gives the team time to watch real usage patterns

## Mint Policy

Minting is allowed only when all of these are true:

- deposit reached confirmation threshold
- destination OP address is valid
- event is present in the ledger
- event is not already minted
- bridge is not paused
- daily mint cap is not exceeded
- reserve reconciliation is currently healthy

## Release Policy

Release is allowed only when all of these are true:

- burn reached confirmation threshold
- destination TXM address is valid
- burn is present in the ledger
- burn has not already been released
- bridge is not paused
- daily release cap is not exceeded
- reserve sufficiency is confirmed

## Pause Policy

Pause conditions:

- reserve mismatch
- suspected signer compromise
- duplicate mint risk
- duplicate release risk
- invalid burn destination mapping
- contract ownership uncertainty
- unexplained ledger divergence

Pause effects:

- stop new minting immediately
- stop new releases immediately
- keep incoming deposit detection active for visibility
- publish operator note/status update

Unpause requires:

- root cause understood
- ledger reconciled
- signer review complete
- written note added to incident log

## Reviewer Policy

Minimum separation:

- operator submits
- reviewer confirms

For routine small flow:

- same-day review is acceptable

For large or flagged flow:

- require explicit second-person review before mint/release

## Large Transaction Policy

Treat these as high-review transactions:

- any single deposit above `10,000 TXM`
- any single withdrawal above `10,000 TXM`
- any address with unusual repeated activity
- any transaction that would push the daily cap near exhaustion

High-review rules:

- second operator review required
- signer awareness required
- slower processing acceptable

## Bridge Hours Policy

Recommended first-release posture:

- limited staffed bridge window is acceptable
- off-hours deposits can wait
- off-hours withdrawals can wait

Reason:

- early launch should optimize for correctness, not 24/7 marketing claims

## Policy Change Process

Policy changes should be documented, versioned, and dated.

Changes that require signer review:

- confirmation thresholds
- daily caps
- signer model
- pause/unpause rules
- custody rules

## First Release Recommendation

If the team wants the safest realistic launch posture:

- start with `2-of-3`
- use `20` TXM confirmations for mint
- use `12` OP confirmations for release
- keep daily caps active
- treat anything large as manual review
- do not promise instant bridging

Supporting docs:

- `PHASE9A_WTXM_CONTRACT_SPEC.md`
- `PHASE9A_BRIDGE_CONTROLLER_SPEC.md`
