# Phase 9A Swap Roadmap

Status: execution roadmap for the first real TXM liquidity path after chain launch.
Last updated: 2026-06-01

This document turns Phase 9A from a broad idea into a concrete sequence.

## Goal

Create the first usable buy/sell path for TXM without changing Tensorium into an
different chain model.

Target outcome:

- TXM remains a native PoW L1 asset on Tensorium
- users get a realistic liquidity path
- bridge/swap complexity is introduced gradually
- the first market path arrives before any attempt at a native Tensorium DEX or
  stablecoin

## Strategic Decision

Do not build a native Tensorium stablecoin or an Optimism-style L2 now.

Near-term Phase 9A path:

1. keep OTC as the first community trading surface
2. build a functional bridge to an EVM chain
3. mint `wTXM` on the EVM side
4. add liquidity on an existing DEX
5. publish docs, risk disclosures, and operational runbooks

## Recommended Chain For First Bridge

Recommended first destination: Optimism

Why:

- stays inside a mature Ethereum L2 environment
- gives a cleaner long-term path if Tensorium later wants broader OP Superchain
  interoperability
- official Optimism bridge and Superchain token standards are mature enough to
  use as architectural reference points
- lower-friction first step than trying to design a native Tensorium DEX

Important note:

- if a future Pearl integration depends specifically on Arbitrum-native flow,
  Arbitrum remains the closest alternative candidate
- for the current roadmap, the working direction is OP-first, not BSC-first

## Scope Split

### Phase 9A.1 — OTC Hardening

Goal:

- make `otc.tensoriumlabs.com` useful as the first temporary liquidity venue

Tasks:

- add clear escrow / trust disclaimer
- add standard listing template
- add reference pricing guidance
- add anti-scam / verification checklist
- add clear TXM wallet + explorer links

Success condition:

- community can do manual trades with less confusion while bridge work is still
  in progress

### Phase 9A.2 — Bridge Model Decision

Goal:

- choose the first operational bridge trust model
- detailed decision record: `PHASE9A_BRIDGE_MODEL_DECISION.md`

Options:

1. custodial operator bridge
2. multisig operator bridge
3. trust-minimized light-client style bridge

Recommended first step:

- multisig operator bridge

Why:

- much easier than a trust-minimized bridge
- much safer than a single-operator custodial key
- realistic for an early project

Minimum design:

- TXM is locked or custody-tracked on Tensorium side
- `wTXM` is minted on Optimism side
- redemption burns `wTXM` and releases TXM back on Tensorium
- mint/burn actions require operator confirmation

### Phase 9A.3 — OP Side Contract Package

Goal:

- deploy minimal `wTXM` token and bridge controller contracts on Optimism

Required contracts:

- `wTXM` ERC-20
- bridge mint/burn controller
- access control / multisig ownership
- pause switch for incidents

Required safety features:

- max mint authority restricted
- owner / operator roles separated
- emergency pause
- events for every mint and burn
- documented supply reconciliation process

### Phase 9A.4 — Tensorium Side Operator Workflow

Goal:

- define how TXM deposits and withdrawals are observed and processed

Minimum workflow:

1. user sends TXM to a published bridge deposit address
2. operator watches Tensorium chain for confirmed deposit
3. operator mints equivalent `wTXM` on Optimism
4. user burns `wTXM` for withdrawal
5. operator verifies burn event and releases TXM on Tensorium

Required operational controls:

- minimum confirmation policy
- bridge ledger for deposit / mint / burn / release
- daily reconciliation
- published maintenance / outage status

### Phase 9A.5 — Liquidity Venue

Goal:

- give `wTXM` a real trade venue

Recommended first venue:

- Uniswap or Velodrome on Optimism

Pair recommendation:

- `wTXM / USDT`
- `wTXM / WETH`

Decision bias:

- `wTXM / USDT` is easier for casual price comprehension
- `wTXM / WETH` is cleaner if treasury wants to stay closer to Ethereum L2
  liquidity rather than BNB-side assets

### Phase 9A.6 — User Surface

Goal:

- make the bridge and swap path understandable to non-dev users

Deliverables:

- `bridge.tensoriumlabs.com` upgrade from landing page to functional guide
- step-by-step deposit / mint / burn / redeem docs
- risk disclosure
- bridge status page or status section
- FAQ for delay, confirmations, and operator review

## What Not To Do Yet

Do not do these in the first Phase 9A release:

- native Tensorium DEX
- native Tensorium stablecoin / USD asset
- trust-minimized cross-chain bridge
- multi-chain bridge expansion
- automated market maker directly on Tensorium

These belong later, after the first liquidity path proves real demand.

## Milestone Order

Recommended order:

1. OTC hardening
2. bridge trust model decision
3. ERC-20 + bridge contract design
4. operator ledger / reconciliation design
5. bridge docs + risk docs
6. test deployment on Optimism Sepolia
7. internal end-to-end deposit / mint / burn / redeem test
8. main deployment
9. Optimism DEX liquidity bootstrap

## Launch Gates For Phase 9A

Before public bridge launch:

- chain launch is already stable
- deposit and withdrawal runbook exists
- bridge wallet ownership is clearly documented
- reconciliation method is documented
- pause / incident procedure exists
- at least one small-value end-to-end bridge drill succeeds
- public docs and warnings are live

## Practical Recommendation

If the team wants the fastest realistic liquidity path:

- launch Tensorium chain first
- keep OTC active immediately after launch
- use Optimism as the first bridge target
- use multisig-controlled `wTXM`
- list `wTXM` on an established Optimism DEX before attempting any native
  Tensorium DEX
