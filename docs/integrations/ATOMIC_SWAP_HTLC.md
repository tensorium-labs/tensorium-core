# Atomic Swaps with Tensorium HTLC

Tensorium's script VM (S3) supports Hash Time Locked Contracts (HTLC), the
primitive behind trustless cross-chain atomic swaps. This guide walks through a
TXM ⇄ wTXM (Optimism) swap. No trusted third party is involved.

## The HTLC primitive

An HTLC output can be spent two ways:

- **Claim** — anyone who knows the secret `preimage` (where `SHA256(preimage)`
  equals the hashlock) AND holds the recipient key can spend it immediately.
- **Refund** — after a block-height deadline (`locktime`), the original sender
  can reclaim the funds with the refund key.

The hashlock uses **SHA256**, which also exists as an EVM precompile, so the same
secret unlocks both sides of a cross-chain swap.

## Roles

- **Alice** holds TXM, wants wTXM.
- **Bob** holds wTXM (Optimism), wants TXM.

## Steps

1. **Alice generates the secret.**
   ```
   txmwallet htlc-secret
   # preimage: <64 hex>   (Alice keeps this private)
   # sha256:   <64 hex>   (Alice shares this hash with Bob)
   ```

2. **Alice locks TXM on Tensorium.** Recipient = Bob, refund = Alice,
   `locktime = H1` (a Tensorium block height comfortably in the future).
   ```
   txmwallet htlc-script <sha256> <bob_txm_addr> <alice_txm_addr> H1
   # scriptpubkey: <hex>
   ```
   Alice funds the printed scriptpubkey with the swap amount of TXM.

3. **Bob locks wTXM on Optimism** in an EVM HTLC using the **same** `sha256`
   hashlock, recipient = Alice, refund = Bob, with an EVM timeout **earlier** than
   H1 in wall-clock terms (see the safety note).

4. **Alice claims the wTXM** on Optimism by revealing `preimage`. This publishes
   the preimage on the EVM chain.

5. **Bob reads the preimage** from Alice's Optimism claim and uses it to claim the
   TXM:
   ```
   txmwallet htlc-claim <tensorium_spk_hex> <bob_txm_addr> <preimage> <rpc>
   txmwallet broadcast htlc-claim-tx.json <rpc>
   ```

If the swap is abandoned, each party reclaims their own funds after their
respective timeout (Alice via `txmwallet htlc-refund` once Tensorium height ≥ H1).

## Safety: order the timeouts correctly

Alice's TXM refund deadline (H1) **must be later** than Bob's wTXM timeout. Bob
must be able to claim TXM (after learning the preimage) before Alice can refund it.
A common rule of thumb is H1 ≈ 2× Bob's timeout.

## Height ↔ time conversion

Tensorium targets ≈ **132 seconds per block**.

| Duration | ~Blocks |
|----------|---------|
| 1 hour   | ~27     |
| 6 hours  | ~164    |
| 24 hours | ~655    |
| 48 hours | ~1310   |

Pick H1 (TXM) and Bob's EVM timeout so the TXM refund window is strictly the
longer of the two.

## Limitations

- HTLC enforces the hashlock and timelock; it does not enforce that both legs of a
  swap actually exist. Each party must verify the counterparty's on-chain lock
  before revealing or committing further.
- Timelocks are **absolute block heights** (no relative `OP_CSV` in S3).
- Spend is single-UTXO, full-value (fund one HTLC output per swap leg).
