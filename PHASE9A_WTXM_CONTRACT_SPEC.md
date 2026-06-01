# Phase 9A wTXM Contract Spec

Status: implementation blueprint for the first wrapped TXM token on Optimism.
Last updated: 2026-06-01

This document defines the minimum expected behavior of the first `wTXM`
contract.

## Goal

Deploy a simple, auditable ERC-20 that represents bridged TXM on Optimism.

## Token Identity

Recommended initial identity:

- name: `Wrapped Tensorium`
- symbol: `wTXM`
- decimals: `18`

Reason:

- matches normal EVM tooling expectations
- easy to integrate with DEXes and wallets

## Ownership Model

Recommended owner:

- multisig, not the deployer EOA

Required ownership steps:

1. deploy contract
2. validate configuration
3. transfer ownership to multisig
4. verify deployer has no lingering privileged ownership path

## Mint Model

Minting must not be public.

Required rule:

- only the bridge controller may mint `wTXM`

Required checks:

- zero-address mint forbidden
- paused-state mint forbidden

## Burn Model

Burn path must support bridge exit flow.

Recommended first-release model:

- bridge controller exposes the burn-facing bridge flow
- user burn is recorded through bridge controller logic rather than free-form
  arbitrary burns

Allowed behavior:

- direct holder burn can be disabled if the bridge controller is the intended
  canonical path

## Required ERC-20 Behavior

The token should support:

- standard balance tracking
- standard transfer behavior
- standard allowance / approval flow

The token should not add unnecessary custom behavior in Phase 9A.

## Required Events

Base ERC-20 events:

- `Transfer`
- `Approval`

Bridge-related operational visibility should come from the bridge controller,
not from hidden off-chain assumptions.

## Pause Behavior

Recommended first release:

- pause should block bridge-controlled mint and burn actions
- ordinary transfers may remain enabled unless there is a strong reason to halt
  all movement

Reason:

- bridge incident response should not unnecessarily freeze every holder unless
  the situation is severe

## Upgrade Philosophy

Preferred first-release posture:

- keep implementation simple
- avoid unnecessary upgradability complexity if not needed

If upgradeability is used:

- ownership and upgrade authority must both be multisig-controlled
- upgrade path must be documented clearly

## Security Constraints

Must not allow:

- unrestricted mint
- hidden admin mint path
- deployer retaining ultimate authority
- bridge controller bypass

## Spec Success Condition

The `wTXM` token spec is ready when:

- token identity is fixed
- owner is multisig
- only the bridge controller can mint
- burn path is defined through the bridge flow
- pause semantics are clear
