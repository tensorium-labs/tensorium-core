# TXM Marketplace — Atomic Settlement (Layer 4) Design

Trustless, on-chain settlement for buying/selling TXM20 + NFT assets for native
TXM. One co-signed transaction atomically moves the asset, pays the seller,
collects a 2.5% platform fee, pays the creator royalty (secondary sales), and
returns the buyer's change. This spec covers **only the settlement protocol
primitives (`tensorium-core`) and the wallet co-sign commands (`txmwallet`)**.
The listings board / order relay and the web frontend are separate later cycles.

## Problem & goal

Assets are an `OP_RETURN` overlay (Layer 1) tracked by a deterministic indexer
(Layer 2); the node does **not** validate asset ownership. So an asset⇄TXM trade
cannot be enforced by a script-level HTLC on the asset side. We want a trade that
is nonetheless **atomic and trustless**: either the asset moves *and* the seller
is paid *and* the fee + royalty are paid, or nothing happens.

**Goal:** settle a trade in a **single transaction** co-signed by seller and
buyer. The node enforces TXM value conservation + signatures (so no one is
under/over-paid and inputs are authorized); the indexer applies the asset
`TRANSFER` (source = `inputs[0]` = seller); the platform fee and royalty are
ordinary TXM outputs that any party can **verify before signing**. No consensus
change, no custody, no trusted escrow.

## Non-goals (this spec)

- No listings database, order book, or HTTP relay — trades are coordinated by
  passing JSON files (or the future relay/backend). Wallet-only here.
- No web frontend (Layer 5).
- No new SIGHASH modes / no consensus change. `signature_hash()` already covers
  the whole tx (all outpoints + all outputs + payload, with signature scripts
  zeroed), so both parties sign the *same* hash and stamp only their own input —
  fully sufficient for an interactive co-sign.
- No cross-chain / wTXM trades.

## Why co-signed single-tx (vs. alternatives)

- **Trusted backend escrow** (buyer→escrow, seller→escrow, backend forwards):
  simplest, but custodial — contradicts "custody on-chain, backend holds only
  metadata," and a compromised backend loses funds. Rejected.
- **HTLC two-tx atomic swap:** the asset leg can't be hashlock-enforced (overlay,
  node-blind), so the swap isn't actually atomic. Rejected.
- **Co-signed single settlement tx (chosen):** atomic by construction (one tx),
  trustless (node + signatures + pre-sign verification), reuses the existing tx
  and signing model. The only new primitives are pure settlement build/verify
  functions and a "sign one input" wallet helper.

## Settlement transaction layout (canonical order)

A fixed output order makes `verify_settlement` deterministic. For an asset sale
of `amount` of `asset_id` at `price` atoms:

| # | Input/Output | Value (atoms) | Notes |
|---|---|---|---|
| in[0]  | **seller** UTXO            | `V_seller`  | establishes asset source = seller; seller signs |
| in[1..]| **buyer** UTXOs            | `V_buyer`   | fund the purchase; buyer signs |
| out[0] | P2PKH **buyer**            | `CARRIER`   | asset destination (dust); `dest_output_index = 0` |
| out[1] | `OP_RETURN` TXMA transfer  | `0`         | `transfer(asset_id, amount, dest_output_index=0)` |
| out[2] | P2PKH **seller**           | `V_seller + price − fee − royalty` | seller's input refund + net proceeds |
| out[3] | P2PKH **platform**         | `fee = floor(price·250/10000)` | 2.5% platform fee |
| out[4] | P2PKH **royalty_addr**     | `royalty = floor(price·royalty_bps/10000)` | **omitted** if `royalty == 0` |
| out[5] | P2PKH **buyer** (change)   | `V_buyer − price − CARRIER − miner_fee` | **omitted** if `0` |

**Conservation:** `Σinputs − Σoutputs = miner_fee` (the implicit miner fee).
Derivation: `V_seller + V_buyer − (CARRIER + 0 + (V_seller+price−fee−royalty) +
fee + royalty + (V_buyer−price−CARRIER−miner_fee)) = miner_fee`. ✓

Constants: `PLATFORM_FEE_BPS = 250` (2.5%); `PLATFORM_FEE_ADDRESS =
txm13vgxzj5ulrfhe7x0mlzxg0q6veq42tkku4g3jr` (the existing pool-treasury /
operations wallet — reused so no new wallet is introduced); `CARRIER = 1000`
atoms (0.00001 TXM, the asset-destination dust); `miner_fee = MIN_RELAY_FEE_ATOMS`.

Primary vs. secondary sale: royalty is recorded immutably at mint
(`asset_info.royalty_bps` / `royalty_addr`). If the seller **is** the
`royalty_addr` (or `royalty_bps == 0`), the royalty output is omitted (it would
pay the seller themselves). Otherwise it is always included — secondary sales pay
the original creator. The royalty terms come from the indexer's `/asset/<id>`,
which is a deterministic function of the chain (tamper-proof).

## Layer 4a — settlement primitives (`tensorium-core`)

New module `crates/tensorium-core/src/settlement.rs` (pure, no I/O):

```rust
pub const PLATFORM_FEE_BPS: u16 = 250;
pub const PLATFORM_FEE_ADDRESS: &str = "txm13vgxzj5ulrfhe7x0mlzxg0q6veq42tkku4g3jr";
pub const CARRIER_ATOMS: u64 = 1_000;

pub struct SettlementTerms {
    pub asset_id: [u8; 32],
    pub amount: u64,
    pub price_atoms: u64,
    pub royalty_bps: u16,        // from asset_info
    pub royalty_addr: String,    // from asset_info
    pub seller_addr: String,
    pub buyer_addr: String,
    pub miner_fee_atoms: u64,
}

/// floor(price * bps / 10000)
pub fn fee_split(price: u64, royalty_bps: u16, seller_addr: &str, royalty_addr: &str)
    -> (u64 /*platform_fee*/, u64 /*royalty*/);   // royalty=0 if seller==royalty_addr

/// Build the unsigned settlement tx in canonical layout. `seller_input` and
/// `buyer_inputs` carry (OutPoint, value). Errors if the buyer side can't cover
/// price + CARRIER + miner_fee.
pub fn build_settlement_tx(
    terms: &SettlementTerms,
    seller_input: (OutPoint, u64),
    buyer_inputs: &[(OutPoint, u64)],
) -> Result<Transaction, SettlementError>;

/// Assert the supplied tx satisfies the trust-critical invariants derivable from
/// `terms` alone (no input values needed). The buyer/seller runs this before
/// signing. Returns the list of mismatches (empty = valid). Checks:
///   - out[0] = P2PKH(buyer_addr) with value CARRIER_ATOMS (asset destination)
///   - out[1] decodes to TRANSFER(asset_id, amount, dest_output_index=0)
///   - the platform-fee output = P2PKH(PLATFORM_FEE_ADDRESS), value EXACTLY floor(price·250/10000)
///   - the royalty output = P2PKH(royalty_addr), value EXACTLY floor(price·bps/10000) — present iff > 0
///   - a seller output = P2PKH(seller_addr) with value AT LEAST (price − fee − royalty)
///     (the surplus is the seller's own refunded input; ≥ protects the seller)
pub fn verify_settlement(tx: &Transaction, terms: &SettlementTerms) -> Vec<String>;
```

`verify_settlement` is the trust anchor and is **input-value-independent**: the
fee, royalty, asset transfer, and buyer destination are exact functions of
`terms`, so a tampered tx (reduced fee, wrong destination, missing royalty, wrong
amount/asset, underpaid seller) yields a non-empty mismatch list and the
counterparty refuses to sign. Proceeds/change that depend on each party's own
input values are each party's own concern (the seller's `≥` check covers theirs;
the buyer knows their own funding), so verify needs no input amounts.

### TDD (settlement.rs)
1. `fee_split`: 2.5% rounding; royalty math; royalty zeroed when seller == royalty_addr.
2. `build_settlement_tx`: output count/order/values for (a) secondary sale with royalty, (b) primary/no-royalty (out[4] omitted), (c) zero buyer change (out[5] omitted); conservation holds.
3. `build_settlement_tx`: insufficient buyer funds → error.
4. `verify_settlement`: a freshly built tx verifies clean (empty list).
5. `verify_settlement` tamper detection: each of {reduced platform fee, wrong buyer destination, removed royalty output, wrong transfer amount, wrong asset_id} produces a mismatch.

## Layer 4b — wallet co-sign flow (`txmwallet`)

New primitive — sign a single input (the existing `sign_transaction` stamps the
same P2PKH script onto *every* input, which is correct only for single-owner
txs; co-signing needs per-input stamping over the shared whole-tx hash):

```rust
// In WalletKeypair (wallet.rs): sign signature_hash(), stamp ONLY input `index`.
pub fn sign_input(&self, tx: &mut Transaction, index: usize) -> Result<(), WalletError>;
```

Three commands and two file handoffs (peer-to-peer; no backend):

1. `txmwallet asset-sell <asset_id_hex> <amount> <price_atoms>` → **`asset-order.json`**
   - Records `{asset_id, amount, price, seller_addr, seller_outpoint, seller_value}`.
   - Picks one mature seller UTXO (smallest that exists) via `/getutxos` to be `inputs[0]`.
   - No signing yet (the full tx isn't built). Seller sends this file to the buyer.

2. `txmwallet asset-buy <asset-order.json>` → **`asset-settlement.json`**
   - Fetches the buyer's mature UTXOs (`/getutxos`) and the asset's royalty terms
     from the indexer (`GET <TXM_INDEXER_URL>/asset/<id>`; env `TENSORIUM_INDEXER`,
     default `127.0.0.1:23340`).
   - Builds `SettlementTerms`, calls `build_settlement_tx`, then `verify_settlement`
     (must be empty), signs **only the buyer inputs** (`sign_input` per buyer index),
     writes the partial tx + terms. Buyer sends this back to the seller.

3. `txmwallet asset-accept <asset-settlement.json>` → broadcasts
   - Re-runs `verify_settlement` against the embedded terms (seller confirms they
     receive `price − fee − royalty` and it's their asset moving), signs **only
     `inputs[0]`**, and submits via `/sendrawtransaction`. Prints the settlement txid.

After confirmation, the indexer applies the transfer; `/balance/<buyer>` shows
the asset, `/balance/<seller>` is debited, and the fee + royalty outputs are
visible on-chain.

### TDD (txmwallet)
1. `sign_input` stamps only the target input; the other inputs' `signature_script`
   stay empty; a two-input tx signed by two keypairs (each its own index) has both
   inputs populated and a stable `signature_hash`.
2. Build→verify a settlement end-to-end with generated keypairs (no RPC): assert
   `verify_settlement` is clean and the asset transfer extracts to buyer.

(The RPC-bound command bodies follow the existing `asset-issue`/`send` pattern and
are covered by the pure build/verify tests + `cargo build`, as with prior layers.)

## Error handling

- `build_settlement_tx` errors on insufficient buyer funds (`price + CARRIER +
  miner_fee`) or an unresolvable address.
- `asset-buy` aborts if `verify_settlement` is non-empty (should never happen for
  a self-built tx — a guard against logic drift) or if the indexer is unreachable
  (royalty terms unknown ⇒ cannot settle safely).
- `asset-accept` aborts if `verify_settlement` is non-empty (seller is protected
  from a malicious buyer-built tx) — this is the seller's trust anchor.
- Asset-balance sufficiency (does the seller actually hold `amount`?) is **not**
  enforced by this tx; the indexer rejects/ignores an over-balance transfer at
  apply time, so an under-funded sale simply fails to move the asset while TXM
  still moves. The wallet SHOULD warn by checking `/balance/<seller>` in
  `asset-sell`; final correctness rests on the indexer (documented limitation).

## What this spec delivers

1. `tensorium-core::settlement` — `build_settlement_tx`, `verify_settlement`,
   `fee_split`, constants (pure, TDD).
2. `txmwallet` — `WalletKeypair::sign_input` + `asset-sell` / `asset-buy` /
   `asset-accept` commands implementing the trustless co-sign settlement flow.

## Next cycles

- **Layer 4b (relay/backend):** a listings board + order relay (DB of
  price/status metadata only; brokers the order→partial→signed handoff) so the
  two parties don't pass files manually.
- **Layer 5 (frontend):** `marketplace.tensoriumlabs.com` UI over the indexer +
  relay.
