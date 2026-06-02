# Phase 9A Signer And Custody Layout

Status: initial host and responsibility layout for the first public bridge release.
Last updated: 2026-06-01

This document turns the bridge policy into a concrete separation of keys,
machines, and responsibilities.

## Goal

Keep the first Phase 9A bridge operationally simple while avoiding obvious
single-host and single-key failure modes.

## Recommended First Layout

### Host A — Primary Tensorium Infra

Current candidate:

- DigitalOcean `157.230.44.162`

Role:

- public RPC and ecosystem services
- not a signer host
- not the only custody decision point

Allowed bridge function:

- monitoring and read-only support

Not allowed:

- long-term signer key storage
- sole bridge reserve authority

## Host B — Backup MC Seed / Infra

Current candidate:

- Vultr `139.180.137.144`

Role:

- backup MC seed node
- not a signer host
- not a custody hot wallet owner

Allowed bridge function:

- read-only monitoring support

Not allowed:

- signer key storage
- bridge mint admin ownership

## Signer Layout

Recommended initial signer model:

- `2-of-3`

Recommended separation:

1. signer A: Angga controlled, cold or hardware-backed
2. signer B: second trusted operator, separate device/location
3. signer C: emergency/recovery signer, separate device/location

Hard rules:

- no two signer keys on one VPS
- no signer key on DO public infra host
- no signer key on Vultr backup seed host
- no signer key embedded in repo or automation

## Custody Layout

Tensorium-side custody should start as:

- one published bridge custody address
- tightly tracked reserve accounting
- manual release flow under operator policy

Recommended separation:

- custody execution environment separate from public infra
- custody key holder not identical to every signer holder

Rule:

- custody key material should not live on public RPC or seed hosts

## Operator Layout

Recommended first split:

- operator workstation: prepares ledger entries, checks deposits, checks burns
- reviewer workstation: validates operator actions
- signer devices: approve only privileged ownership/governance actions

This keeps the first release manual but compartmentalized.

## Optimism Contract Ownership Layout

Recommended shape:

- `wTXM` owner -> multisig
- bridge controller owner -> multisig
- operator role -> limited hot operator address
- pause role -> multisig or tightly governed emergency path

Do not use:

- single EOA as permanent owner
- deployer wallet left as owner after launch

## Suggested Launch-Day Topology

Minimum topology for first public release:

1. Tensorium custody address published
2. Optimism `wTXM` deployed
3. bridge controller deployed
4. controller ownership transferred to multisig
5. operator hot address granted only limited role
6. public infra hosts remain outside signer/custody key storage

## Recovery Layout

If one signer is unavailable:

- `2-of-3` still works

If one signer is suspected compromised:

- pause bridge
- rotate affected signer
- document incident before unpause

If custody key is suspected compromised:

- pause bridge immediately
- move reserve under incident procedure
- do not reopen until reserve position is re-verified

## Practical Recommendation

For the first live Phase 9A rollout:

- keep signers off VPS entirely
- keep custody off public infra entirely
- use VPS hosts only for read-only infra, monitoring, and public services
- keep privileged actions on dedicated human-controlled devices
