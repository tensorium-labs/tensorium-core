# Phase 9A Contracts Implementation Plan

Status: implementation anchor for turning specs into Solidity code.
Last updated: 2026-06-01

## Current Files

- `interfaces/IWrappedTensorium.sol`
- `interfaces/ITensoriumBridgeController.sol`

## What Exists Now

- contract specs in the repo root docs
- interface skeletons under `contracts/interfaces/`
- local Hardhat workspace under `contracts/`
- concrete `src/` contract implementations
- local Hardhat tests passing

## What Still Needs To Be Added

1. deployment config for Optimism Sepolia
2. broader contract tests
3. ownership transfer script to multisig
4. deployment/release runbook for contracts

## Active Tooling Decision

Current local toolchain:

- Hardhat

Reason:

- available immediately in this environment
- easy local compile/test without waiting on external tooling cleanup
- enough to move Phase 9A from docs into working contract code

## First Implementation Order

1. add concrete token contract
2. add concrete controller contract
3. add unit tests for:
   - operator gating
   - pause behavior
   - replay protection
   - bridge-only mint path
4. add deployment script
5. deploy to Optimism Sepolia
