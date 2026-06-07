# Tensorium OTC Round — June 2026

A small, transparent OTC round to bootstrap **wTXM/ETH liquidity** on Optimism.
Everything here is on-chain and verifiable.

> **Not investment advice.** TXM is an experimental L1. Liquidity is thin and
> price is volatile. Only participate with funds you can afford to lose.

---

## Why this round

We have plenty of TXM but need the **counter-asset (ETH)** to deepen the
Uniswap pool. Rather than wait, we're selling a small slice of the **ecosystem
allocation** to early backers, and using the proceeds to seed the pool. The
separate **liquidity allocation is reserved for future Arbitrum and Base
pools** and is *not* touched in this round.

## Terms

| Item | Value |
|---|---|
| **Asset sold** | TXM (from ecosystem allocation) |
| **OTC price** | **$0.01 / TXM** |
| **Round size** | up to **300,000 TXM** (= **$3,000** target, paid in ETH) |
| **Payment** | ETH on **Optimism** |
| **Listing price** | **$0.015 / TXM** on Uniswap (Optimism, wTXM/ETH) after the round |
| **Early-backer upside** | buy at $0.01, market opens at $0.015 → **+50% vs listing** |
| **Vesting** | **20% on delivery**, **80% linear over 6 monthly tranches**, locked on-chain via CLTV |
| **Ecosystem impact** | 300K (OTC) + 200K (pool seed) = **500K of 2M ecosystem (25%)**; 1.5M remains |

### Vesting schedule (per buyer, proportional)

Tokens are delivered into **on-chain CLTV time-locks** that only the buyer can
spend, and only after each tranche's block height:

| Tranche | Share | Unlocks at |
|---|---|---|
| TGE / delivery | 20% | immediately (liquid) |
| Month 1 | 13.33% | lock height + ~43,200 blocks |
| Month 2 | 13.33% | + ~86,400 blocks |
| Month 3 | 13.33% | + ~129,600 blocks |
| Month 4 | 13.33% | + ~172,800 blocks |
| Month 5 | 13.33% | + ~216,000 blocks |
| Month 6 | 13.34% | + ~259,200 blocks |

> Locks are enforced by **block height** (the chain's target is 60 s/block, so
> ~43,200 blocks ≈ 1 month). Real unlock timing varies with network hashrate;
> the height is the binding commitment, the date is an estimate.

**Example — a buyer of the full 300,000 TXM ($3,000):**
60,000 TXM liquid on delivery, then ~40,000 TXM unlocking each month for 6 months.

## How to participate

1. Contact the team (Discord) and agree on your amount.
2. Send ETH (Optimism) to the published round address (see Discord / the OTC page).
   *Trustless option:* settle via an **HTLC atomic swap** (TXM ⇄ ETH) so neither
   side has to trust the other — ask for the swap guide.
3. The team builds your CLTV vesting locks on-chain and shares the txids.
   You can verify every lock with `txmwallet` or the explorer before and after.

## Transparency commitments

- Every vesting lock is **on-chain and published** (txids in Discord).
- The ETH raised is paired into the Uniswap pool; the **LP position is public**.
- Treasury/ecosystem movements for this round are disclosed.
- **No team dumping**: the only TXM entering the new pool from us is the
  declared 200K seed; OTC buyers are vested.

## Anti-dump math

A buyer holds at most **20% liquid** at delivery. For the full round that's
60K TXM (~$900 at listing) against a **$3,000 ETH** pool depth — the pool can
absorb it without cratering. The remaining 80% is height-locked and releases
slowly, so no one can flood the pool they just helped seed.

---

*Addresses, the exact lock heights, and the LP position link are published on
`otc.tensoriumlabs.com` and in Discord `#announcements` when the round opens.*
