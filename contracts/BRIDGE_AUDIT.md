# Tensorium Bridge Contract — Internal Security Audit

**Date:** 2026-06-01  
**Contracts:** `WrappedTensorium.sol`, `TensoriumBridgeController.sol`  
**Deployed (op-sepolia):**
- wTXM: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`
- BridgeController: `0x4b31C557AD64609B975610812273BF82F1475384`

**Test coverage after audit:** 28 tests, all passing.

---

## Summary

Both contracts are structurally sound and follow established OpenZeppelin patterns (`Ownable2Step`, `Pausable`). No critical bugs were found. Three low-priority findings are documented below. The main trust risk is inherent to the multisig-operated bridge model and is accepted by design.

---

## Findings

### [MEDIUM] — Trust: `bridgeBurnFrom` burns any address without ERC-20 allowance

**Location:** `WrappedTensorium.sol:70`

```solidity
function bridgeBurnFrom(address from, uint256 amount)
    external
    onlyBridgeController
    whenNotPaused
{
    _burn(from, amount);
}
```

The bridge controller can burn wTXM from **any** holder's balance without an ERC-20 allowance. This is intentional — the withdrawal flow is `user calls controller.requestWithdrawal` → `controller calls token.bridgeBurnFrom(msg.sender)` — so the burn is always from the user who explicitly requested it. However, if the `bridgeController` address is compromised or updated to a malicious contract (via `setBridgeController`), it can drain any holder.

**Mitigation in place:** `setBridgeController` is `onlyOwner`, and `Ownable2Step` requires a 2-step transfer, preventing accidental ownership change. The bridge controller itself is owned by a multisig.

**Action:** Document this trust assumption clearly in bridge policy docs and user-facing documentation. Do not weaken the `setBridgeController` guard.

---

### [LOW] — No on-chain `tensoriumAddress` format validation

**Location:** `TensoriumBridgeController.sol:119`

```solidity
if (bytes(tensoriumAddress).length == 0) revert InvalidTensoriumAddress();
```

Only non-empty is required. A user entering a typo or invalid address format would have their wTXM burned but could lose the corresponding TXM if the operator cannot complete the L1 release.

**Fix if redeploying:** Add a prefix check.

```solidity
bytes memory addrBytes = bytes(tensoriumAddress);
if (addrBytes.length < 5) revert InvalidTensoriumAddress();
if (addrBytes[0] != 't' || addrBytes[1] != 'x' || addrBytes[2] != 'm' ||
    addrBytes[3] != '1') revert InvalidTensoriumAddress();
```

**Current mitigation:** The bridge UI should validate `txm1` prefix client-side before allowing submission.

---

### [LOW / GAS] — Redundant `processedEventIds` entry for withdrawal events

**Location:** `TensoriumBridgeController.sol:128`

```solidity
withdrawalNonce += 1;
bytes32 bridgeEventId = keccak256(
    abi.encodePacked(withdrawalNonce, msg.sender, amount, tensoriumAddress)
);
processedEventIds[bridgeEventId] = true;   // ← this
IWrappedTensoriumToken(token).bridgeBurnFrom(msg.sender, amount);
```

`processedEventIds` is used for **deposit** events to prevent the operator from double-minting for the same L1 tx. For withdrawals, each call generates a guaranteed-unique ID (because `withdrawalNonce` increments), so marking it processed serves no replay prevention purpose. It wastes ~20k gas per withdrawal and can create namespace confusion if a crafted L1 deposit `bridgeEventId` collides with a withdrawal ID.

**Fix if redeploying:** Remove the `processedEventIds[bridgeEventId] = true;` line from `requestWithdrawalToTensorium`. The `WithdrawalRequested` event is the canonical record for the relayer; storing state is not needed.

---

### [INFORMATIONAL] — Decimal mismatch between native TXM and wTXM

`WrappedTensorium` inherits `ERC20` default of 18 decimals. Native TXM uses 8 decimal places (atoms = 1e-8 TXM). The bridge relay is responsible for the conversion:

```
1 TXM on L1 = 1e8 atoms
1 wTXM on EVM = 1e18 wei
```

The relay must convert `amount_atoms * 1e10` when minting and `amount_wei / 1e10` when releasing. A relay bug in this conversion would cause over-minting or under-releasing. The deployed relay has been drilled (see `deployments/drill-phase9a5-reconciliation.json`).

**Action:** Relay unit tests must include boundary cases at the atom/wei boundary. Any relay upgrade must re-run the full drill before production use.

---

### [INFORMATIONAL] — No global daily / periodic mint rate limit

`maxPerTx` limits individual transactions but does not bound total minting per day. If a compromised operator key floods calls to `mintFromTensoriumDeposit`, it can mint up to `maxPerTx` per transaction continuously until the owner or pauser acts.

**Accepted:** Monitoring + pause capability is the intended defense. The multisig should alert on unusual mint volume.

---

## What the Tests Now Cover (28 total)

| Area | Tests |
|---|---|
| Token role access (owner, non-owner, pauser) | 9 |
| Token pause/unpause | 3 |
| Ownable2Step (token + controller) | 2 |
| Deposit: happy path, duplicate, max, non-operator | 4 |
| Deposit: zero recipient, zero amount | 2 (new) |
| Withdrawal: happy path, uniqueness, max | 3 |
| Withdrawal: empty address, zero amount, insufficient balance | 3 (new) |
| Pause on controller | 2 |
| maxPerTx update | 1 |
| Direct bridgeMint/bridgeBurnFrom isolation | 2 (new) |
| setBridgeController zero address | 1 (new) |

---

## Deployment Constraints to Preserve

1. `bridgeController` on wTXM must always point to a controller owned by a multisig, never an EOA.
2. `maxPerTx` must be set conservatively at launch (~500–1000 TXM equivalent) and raised only after proving relay stability.
3. Any new bridge controller deployment must complete a full drill cycle before pointing wTXM at it.
4. Relay must validate `txm1` address prefix before building the withdrawal event.
