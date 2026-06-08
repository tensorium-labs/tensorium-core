# Tensorium OTC — Seed &amp; Private Rounds

Two transparent, on-chain OTC rounds to onboard early backers and bootstrap
**wTXM/ETH liquidity** on Optimism. Everything here is verifiable on-chain.

> **Not investment advice.** TXM is an experimental L1. Liquidity is thin and
> price is volatile. Only participate with funds you can afford to lose.

---

## Structure

**Total allocation: 600,000 TXM** (from the ecosystem allocation). Two rounds run
**sequentially** — the **Seed round opens first**, and once its 300,000 TXM is
filled, the **Private round opens automatically** at the higher price. The
liquidity allocation reserved for future Arbitrum &amp; Base pools is *not* touched.

| Round | Allocation | Price | Unlock at TGE | Vesting |
|---|---|---|---|---|
| **Seed** | 300,000 TXM | **$0.05 / TXM** | 20% | 80% monthly over 6 months |
| **Private** | 300,000 TXM | **$0.12 / TXM** | 20% | 80% monthly over 6 months |

**Estimated listing price:** **$0.20 – $0.30 / TXM** on Uniswap (Optimism,
wTXM/ETH) after the rounds — early backers buy well below listing.

- **Payment:** ETH on **Optimism**.
- **Round transition:** automatic and on-chain — the live status (current round,
  price, sold / remaining) is shown on `otc.tensoriumlabs.com`. No manual switch.
- **Vesting** is identical for both rounds: **20% liquid on delivery**, the
  remaining **80% in 6 monthly tranches**, each locked on-chain via **CLTV** so
  only the buyer can spend it, and only after its block height.

### Vesting schedule (per buyer, proportional)

| Tranche | Share | Unlocks at |
|---|---|---|
| TGE / delivery | 20% | immediately (liquid) |
| Month 1 | 13.33% | lock height + ~43,200 blocks |
| Month 2 | 13.33% | + ~86,400 blocks |
| Month 3 | 13.33% | + ~129,600 blocks |
| Month 4 | 13.33% | + ~172,800 blocks |
| Month 5 | 13.33% | + ~216,000 blocks |
| Month 6 | 13.34% | + ~259,200 blocks |

> Locks are enforced by **block height** (target 60 s/block, so ~43,200 blocks ≈
> 1 month). Real unlock timing varies with network hashrate; the height is the
> binding commitment, the date is an estimate.

**Example — a $1,000 buy in the Seed round at $0.05 = 20,000 TXM:**
4,000 TXM liquid on delivery, then ~2,666 TXM unlocking each month for 6 months.

## How to participate (self-service — no DM needed)

1. Open **`otc.tensoriumlabs.com`** and **register** your TXM address against the
   Optimism address you'll send from.
2. **Send ETH (Optimism)** from that registered address to the published OTC
   receive address.
3. Your **CLTV vesting locks are built on-chain automatically** at the current
   round's price, and the txids are recorded — verify every lock with
   `txmwallet` or the explorer. Claim matured tranches in the **Tensorium Wallet**
   extension (Vesting tab).

*Trustless option:* settle via an **HTLC atomic swap** (TXM ⇄ ETH) so neither
side has to trust the other — ask for the swap guide.

## Transparency commitments

- Every vesting lock is **on-chain and published** (txids verifiable).
- The ETH raised is paired into the Uniswap pool; the **LP position is public**.
- Treasury/ecosystem movements for these rounds are disclosed.
- **No team dumping**: OTC buyers are vested (20% liquid, 80% height-locked).

## Anti-dump math

A buyer holds at most **20% liquid** at delivery; the other **80% is height-locked
via CLTV** and releases monthly over 6 months — enforced on-chain, not by trust.
No one can flood the pool they just helped seed.

> **Liquidity disclosure:** the pool is thin and the listing range ($0.20–0.30)
> reflects a small initial pool. Price is volatile and a large sell can move it
> sharply. This is an experimental L1 — participate accordingly.

---

*Live round status (active round, price, sold / remaining), the OTC receive
address, and the LP position link are published on `otc.tensoriumlabs.com` and in
Discord `#announcements`.*
