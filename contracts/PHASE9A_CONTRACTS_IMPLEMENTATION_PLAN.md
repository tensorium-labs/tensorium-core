# Phase 9A Contracts Implementation Plan

Status: implementation anchor for turning specs into Solidity code.
Last updated: 2026-06-01

## Current Files

- `interfaces/IWrappedTensorium.sol`
- `interfaces/ITensoriumBridgeController.sol`

## What Exists Now

- contract specs in the repo root docs
- interface skeletons under `contracts/interfaces/`

## What Still Needs To Be Added

1. concrete `wTXM` implementation
2. concrete bridge controller implementation
3. deployment config for Optimism Sepolia
4. contract tests
5. ownership transfer script to multisig

## Recommended Tooling Decision

Preferred default:

- Foundry

Why:

- faster local test loop
- easy scripting for deployment and ownership transfer
- clean Solidity-first repo structure

## First Implementation Order

1. choose toolchain
2. add concrete token contract
3. add concrete controller contract
4. add unit tests for:
   - operator gating
   - pause behavior
   - replay protection
   - bridge-only mint path
5. add deployment script
6. deploy to Optimism Sepolia
