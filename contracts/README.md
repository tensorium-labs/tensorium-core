# Phase 9A Contracts

This folder is the starting point for the first Tensorium Phase 9A EVM bridge
contracts.

Current scope:

- Solidity interface skeletons for `wTXM`
- Solidity interface skeletons for the Tensorium bridge controller
- initial Hardhat workspace for local compile/test
- initial contract implementations and tests
- implementation planning notes

Current status:

- Hardhat is wired locally inside this folder
- this folder now anchors both the contract surface and the first local EVM
  toolchain
- local compile and basic tests pass

Recommended next step:

- extend the initial contracts beyond the first local passing tests
- add deployment config for Optimism Sepolia
