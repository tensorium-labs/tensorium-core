# Tensorium Canonical Asset Metadata

Status: Phase 10E canonical data-provider packet
Last updated: 2026-06-02

This file is the single source of truth for third-party integrations, listing forms, wallets, explorers, and data providers.

If another document disagrees with this file, update the other document.

## Project Identity

| Field | Value |
| --- | --- |
| Project name | Tensorium |
| Native ticker | TXM |
| Wrapped ticker | wTXM |
| Project type | Native Layer 1 Proof-of-Work blockchain |
| License | Apache-2.0 |
| Mainnet launch date | 2026-06-02 |
| Mainnet genesis timestamp | 2026-06-01 00:00:00 UTC |
| Website | https://tensoriumlabs.com |
| Docs | https://docs.tensoriumlabs.com |
| Whitepaper | https://whitepaper.tensoriumlabs.com |
| Source code | https://github.com/tensorium-labs/tensorium-core |
| Support contact | dev@tensoriumlabs.com |
| Community | https://discord.gg/KkgGSZKVZw |

## Native Chain Metadata

| Field | Value |
| --- | --- |
| Chain name | Tensorium Mainnet |
| Chain ID | `tensorium-mainnet-candidate-0` |
| Native asset | TXM |
| Consensus | SHA256d Nakamoto PoW |
| State model | UTXO |
| Block time target | 60 seconds |
| P2P seed | `seed.tensoriumlabs.com:33333` |
| Public RPC | `https://mc-rpc.tensoriumlabs.com` |
| Public explorer | https://explorer.tensoriumlabs.com |
| Backup seed | `139.180.137.144:33333` |

## Wrapped Asset Metadata

| Field | Value |
| --- | --- |
| Wrapped asset | wTXM |
| Network | Optimism mainnet |
| EVM chainId | `10` |
| ERC-20 contract | `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` |
| Controller | `0x4b31C557AD64609B975610812273BF82F1475384` |
| Safe | `0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9` |
| Bridge app | https://bridge.tensoriumlabs.com |
| Explorer | https://optimistic.etherscan.io/address/0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e |

## Tokenomics

| Field | Value |
| --- | --- |
| Max supply | 33,000,000 TXM |
| Founder allocation | 1,000,000 TXM |
| Mining allocation | 32,000,000 TXM |
| Initial block reward | 15.23557865 TXM |
| Halving interval | 1,051,200 blocks |
| Halving eras | 10 |
| Decimals | 18 |
| Founder lock | Voluntary 24-month social lock, max 10% of allocation per calendar month |

## Public Service Surface

| Service | URL | Notes |
| --- | --- | --- |
| Website | https://tensoriumlabs.com | Canonical project homepage |
| Docs | https://docs.tensoriumlabs.com | Setup, mining, RPC reference |
| Whitepaper | https://whitepaper.tensoriumlabs.com | Technical/tokenomics reference |
| Explorer | https://explorer.tensoriumlabs.com | Canonical chain visibility |
| Public RPC | https://mc-rpc.tensoriumlabs.com | Mainnet public RPC via nginx reverse proxy |
| Bridge | https://bridge.tensoriumlabs.com | TXM ↔ wTXM bridge |
| Pool | https://pooltxm.tensoriumlabs.com | Official/reference mining pool |

## Integration Assumptions

Use these assumptions when submitting to data providers:

- Native chain asset: `TXM`
- EVM-tracked wrapped asset for price discovery: `wTXM`
- Canonical public chain visibility: `https://explorer.tensoriumlabs.com`
- Canonical public RPC for wallets/integrators: `https://mc-rpc.tensoriumlabs.com`
- Canonical support contact: `dev@tensoriumlabs.com`

## Operational Posture Notes

- Public RPC is fronted by nginx and the node RPC stays on localhost.
- Explorer now persists its incremental index and reloads from disk on restart.
- Chain state is persisted in RocksDB and legacy `state.json` auto-migrates on first open.
- Backup/restore runbooks exist and have been drill-tested.

## SLA / Reliability Assumptions

These are operational expectations, not contractual guarantees:

| Surface | Assumption |
| --- | --- |
| Public RPC | best-effort public endpoint, rate limited, not a contractual SLA |
| Explorer | best-effort public visibility endpoint, expected to track chain tip within a few blocks |
| Bridge | operator-managed service with explicit caps and multisig control |
| Seed node | public bootstrap endpoint for peer discovery, with backup seed available |

## Cross-Reference Sources

The values here are aligned with:

- `CEX_LISTING_PACKAGE.md`
- `MAINNET_READINESS.md`
- `README.md`
- `../project/RISK_DISCLOSURE.md`
- `../operations/PUBLIC_RPC_POSTURE.md`

## Update Rule

When any of these change, update this file in the same change set:

- RPC URL
- explorer URL
- bridge URL
- seed node
- chain ID
- token contract
- supply / tokenomics
- support contact
