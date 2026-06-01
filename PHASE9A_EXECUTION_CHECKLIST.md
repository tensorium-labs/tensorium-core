# Phase 9A Execution Checklist

Status: operational checklist for moving the first bridge from docs to implementation.
Last updated: 2026-06-01

## Phase 9A.0 — Governance And Keys

- [ ] Select initial `2-of-3` signer set
- [ ] Assign signer A / B / C owners
- [ ] Confirm no signer key is stored on public VPS hosts
- [ ] Confirm at least one signer is cold or hardware-backed
- [ ] Select Tensorium custody owner
- [ ] Document custody handling rules

## Phase 9A.1 — Contracts

- [x] Define `wTXM` ERC-20 contract shape
- [x] Define bridge controller contract shape
- [x] Create Solidity interface skeletons
- [x] Define operator role vs owner role separation
- [x] Define pause control path
- [x] Review contract ownership transfer path

## Phase 9A.2 — Policy And Operations

- [x] Bridge policy written
- [x] Bridge ledger format written
- [x] Operator runbook written
- [x] Reconciliation template written
- [ ] Incident log location defined
- [ ] Signer rotation procedure written
- [ ] Lost-key procedure written

## Phase 9A.3 — User Flow Design

- [ ] Define deposit request UX
- [ ] Define how user submits destination Optimism address
- [ ] Define withdrawal request UX
- [ ] Define bridge status page shape
- [ ] Define public risk disclosure text

## Phase 9A.4 — Test Deployment

- [x] Deploy bridge contracts to Optimism Sepolia
  - WrappedTensorium: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`
  - TensoriumBridgeController: `0x4b31C557AD64609B975610812273BF82F1475384`
  - Deployed: 2026-06-01, network: op-sepolia (chainId 11155420)
- [x] Transfer ownership to test multisig (deployer EOA for Sepolia, acceptOwnership done)
- [ ] Create sample custody flow for test TXM handling
- [ ] Prepare operator hot address with limited role
- [ ] Create first test ledger entries

## Phase 9A.5 — Internal Drill

- [x] Drill one test deposit end-to-end
- [x] Drill one test mint end-to-end
- [x] Drill one test burn end-to-end
- [x] Drill one test release end-to-end (WithdrawalRequested event emitted, operator notified)
- [x] Run reconciliation after the drill (supply balanced, saved to deployments/drill-phase9a5-reconciliation.json)
- [x] Verify pause path during a simulated incident (mint blocked ✓, unpause works ✓)

## Phase 9A.6 — Launch Preparation

- [x] Publish custody address (txm13ydx0hc8g3e07qfcecznt0u3jcw6y386e28qhq — bridge.tensoriumlabs.com)
- [x] Publish bridge FAQ and risk disclosure (bridge.tensoriumlabs.com)
- [x] Publish current limits and confirmation thresholds (10K wTXM/tx, 6 blocks ~12 min)
- [x] Publish bridge hours / review expectations (~15 min auto relayer)
- [x] Publish incident/status communication path (status.tensoriumlabs.com + Telegram)

## Phase 9A.7 — First Public Opening

- [ ] Open with conservative caps only
- [ ] Watch first deposits manually
- [ ] Watch first withdrawals manually
- [ ] Produce first daily reconciliation note
- [ ] Review whether thresholds or caps need tightening

## Current Status

Already done in docs:

- [x] roadmap
- [x] bridge model decision
- [x] bridge policy
- [x] contract specs
- [x] initial contract scaffold
- [x] basic local contract tests
- [x] ledger format
- [x] operator runbook
- [x] templates

Already done in implementation:

- [x] contracts (WrappedTensorium + TensoriumBridgeController — production-ready rewrite)
- [x] 20 tests passing (Ownable2Step, pauser role, maxPerTx cap, auto-generated bridgeEventId)
- [x] deployment script with MULTISIG_ADDRESS enforcement
- [x] test deployment live on Optimism Sepolia (2026-06-01)
- [x] contracts live on Optimism mainnet (2026-06-01)
  - WrappedTensorium: 0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e
  - TensoriumBridgeController: 0x4b31C557AD64609B975610812273BF82F1475384
- [x] bridge relayer live on VPS (pm2: tensorium-bridge-relayer)
- [x] Phase 9A.6 Launch Preparation DONE

Still not done:

- [ ] signer set selection
- [ ] custody assignment
- [ ] test deployment
- [ ] internal drill
- [ ] public bridge opening
