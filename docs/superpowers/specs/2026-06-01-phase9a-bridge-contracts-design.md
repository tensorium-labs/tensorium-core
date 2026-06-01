# Phase 9A Bridge Contracts — Design Spec

Date: 2026-06-01
Status: approved

## Context

Rewrite bersih kedua bridge contracts dari scaffold ke production-ready untuk Phase 9A first public opening. Contracts belum live di manapun sehingga rewrite tidak ada migration cost.

## Decisions

| Topic | Decision |
|---|---|
| bridgeEventId withdrawal | Auto-generate di contract via keccak256 |
| Bridge caps | On-chain `maxPerTx`, owner dapat update |
| Ownership | `Ownable2Step` + deployment script enforce multisig |
| Pause path | `pauser` role: pause-only, unpause tetap onlyOwner |

## WrappedTensorium.sol

Inherits: `ERC20`, `Ownable2Step`, `Pausable`

State:
- `address public bridgeController`
- `address public pauser`

Owner actions: `setBridgeController`, `setPauser`, `unpause`, `transferOwnership`
Pauser actions: `pause`
Controller actions: `bridgeMint(address to, uint256 amount)`, `bridgeBurnFrom(address from, uint256 amount)`

Rules:
- `bridgeMint` dan `bridgeBurnFrom` → `onlyBridgeController whenNotPaused`
- `unpause` → `onlyOwner`
- `pause` → caller must be `pauser` or `owner`
- Token tidak enforce cap — cap ada di controller

Events: `BridgeControllerUpdated`, `PauserUpdated`
Errors: `NotBridgeController`, `InvalidBridgeController`, `NotPauser`

## TensoriumBridgeController.sol

Inherits: `Ownable2Step`, `Pausable`

State:
- `address public immutable token`
- `uint256 public withdrawalNonce`
- `uint256 public maxPerTx`
- `address public pauser`
- `mapping(address => bool) public operators`
- `mapping(bytes32 => bool) public processedEventIds`

Owner actions: `setOperator`, `setPauser`, `setMaxPerTx`, `unpause`, `transferOwnership`
Pauser actions: `pause`
Operator actions: `mintFromTensoriumDeposit(bytes32 bridgeEventId, bytes32 tensoriumTxid, address recipient, uint256 amount)`
User actions: `requestWithdrawalToTensorium(string calldata tensoriumAddress, uint256 amount)`

bridgeEventId auto-generation (withdrawal):
```
withdrawalNonce += 1;
bytes32 bridgeEventId = keccak256(abi.encodePacked(withdrawalNonce, msg.sender, amount, tensoriumAddress));
```

Validasi pada kedua fungsi: `amount <= maxPerTx`, `amount > 0`, `recipient != address(0)`

Events: `OperatorUpdated`, `PauserUpdated`, `MaxPerTxUpdated`, `BridgePaused`, `BridgeUnpaused`, `DepositMinted`, `WithdrawalRequested`
Errors: `NotOperator`, `NotPauser`, `InvalidToken`, `InvalidRecipient`, `InvalidAmount`, `InvalidTensoriumAddress`, `BridgeEventAlreadyProcessed`, `ExceedsMaxPerTx`

## Tests — test/bridge.test.js

~18 test cases menggantikan 3 test lama:

WrappedTensorium:
- owner bisa set bridge controller
- non-owner tidak bisa set bridge controller
- owner bisa set pauser
- pauser bisa pause
- pauser tidak bisa unpause
- non-pauser tidak bisa pause
- owner bisa unpause
- mint/burn blocked saat paused
- Ownable2Step: transfer butuh acceptance

TensoriumBridgeController:
- operator bisa mint, emit DepositMinted
- duplikat bridgeEventId → BridgeEventAlreadyProcessed
- amount > maxPerTx → ExceedsMaxPerTx
- non-operator tidak bisa mint
- user bisa withdraw, balance burned, emit WithdrawalRequested
- dua withdrawal berbeda menghasilkan bridgeEventId berbeda
- withdrawal amount > maxPerTx → revert
- pauser bisa pause, tidak bisa unpause
- mint/withdraw blocked saat paused
- owner bisa update maxPerTx

## Deployment Script — scripts/deploy.js

Flow:
1. Baca env: `MULTISIG_ADDRESS`, `OPERATOR_ADDRESS`, `PAUSER_ADDRESS`, `MAX_PER_TX`
2. Hard-fail jika `MULTISIG_ADDRESS` tidak di-set
3. Deploy `WrappedTensorium(name, symbol, deployer)`
4. Deploy `TensoriumBridgeController(token.address, deployer, maxPerTx)`
5. `token.setBridgeController(controller.address)`
6. `token.setPauser(PAUSER_ADDRESS)`
7. `controller.setPauser(PAUSER_ADDRESS)`
8. `controller.setOperator(OPERATOR_ADDRESS, true)`
9. `token.transferOwnership(MULTISIG_ADDRESS)` — initiate (Ownable2Step)
10. `controller.transferOwnership(MULTISIG_ADDRESS)` — initiate (Ownable2Step)
11. Log addresses + reminder multisig harus call `acceptOwnership()`
12. Tulis `deployments/<network>.json`

Untuk Sepolia test: `MULTISIG_ADDRESS` bisa EOA. Untuk mainnet: Gnosis Safe address.

## Out of Scope (Phase 9A)

- Daily cumulative cap
- Automatic pause triggers
- On-chain signer set
- wTXM → TXM automatic release (tetap manual operator)
