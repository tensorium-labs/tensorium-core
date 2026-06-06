# Dynamic Fee System Design

**Date:** 2026-06-06  
**Status:** Approved  
**Scope:** `tensorium-core` (mempool) + `tensorium-node` (RPC) + `tensorium-wallet-extension` (Chrome wallet UI)

---

## Problem

The current fee system is static. `/estimatefee` returns a single flat value (`max(median, min_relay)`) that does not reflect network congestion. The Chrome wallet hardcodes `MIN_RELAY_FEE_ATOMS = 10_000` in `Send.tsx` and never fetches the RPC — users have no visibility into network conditions and no way to prioritize their transaction.

---

## Goals

1. `/estimatefee` returns three fee tiers (slow/normal/fast) that adapt to actual mempool backlog.
2. Chrome wallet shows the three tiers as selectable pills with a congestion badge.
3. Users can override with a custom fee amount.
4. When the RPC is unreachable, the wallet falls back gracefully to hardcoded baseline values.

---

## Non-Goals

- Replace-by-fee (RBF) — deferred.
- Fee-per-byte / weight-based fees — Tensorium UTXO fees are implicit (inputs − outputs); no size component needed at this stage.
- Historical fee estimation (Bitcoin `estimatesmartfee` style) — not enough chain history yet.

---

## Architecture

### Layer 1 — `tensorium-core/mempool.rs`

Add two new public types and one method to `Mempool`.

**`CongestionLevel`**

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CongestionLevel { Low, Medium, High }
```

Thresholds: `count < 5` → Low, `5 ≤ count < 20` → Medium, `count ≥ 20` → High.

**`FeeTiers`**

```rust
#[derive(Debug, Serialize)]
pub struct FeeTiers {
    pub slow_atoms:       u64,
    pub normal_atoms:     u64,
    pub fast_atoms:       u64,
    pub congestion_level: CongestionLevel,
    pub mempool_count:    u64,
}
```

**`Mempool::fee_tiers()`**

```rust
pub fn fee_tiers(&self) -> FeeTiers
```

Logic:

- Constants (floor values):
  - `SLOW_FLOOR   = MIN_RELAY_FEE_ATOMS`        (1× = 10_000 atoms)
  - `NORMAL_FLOOR = MIN_RELAY_FEE_ATOMS * 2`    (2× = 20_000 atoms)
  - `FAST_FLOOR   = MIN_RELAY_FEE_ATOMS * 10`   (10× = 100_000 atoms)
- If `mempool.is_empty()` → return floor values, `congestion_level = Low`.
- Otherwise:
  - Collect fees, sort ascending.
  - `slow   = max(fees[len * 25 / 100], SLOW_FLOOR)`
  - `normal = max(fees[len * 50 / 100], NORMAL_FLOOR)`
  - `fast   = max(fees[len * 75 / 100], FAST_FLOOR)`
  - `congestion_level` from count thresholds above.

Invariant guaranteed: `slow ≤ normal ≤ fast` because floors are ordered and percentiles are from the same sorted slice.

---

### Layer 2 — `tensorium-node/main.rs`

Update the `("GET", "/estimatefee")` handler.

**New response format:**

```json
{
  "slow_atoms":       10000,
  "normal_atoms":     20000,
  "fast_atoms":       100000,
  "congestion_level": "low",
  "mempool_count":    2,
  "slow_txm":         0.0001,
  "normal_txm":       0.0002,
  "fast_txm":         0.001
}
```

`_txm` fields are computed in the handler (`atoms as f64 / 1e8`) so the frontend needs no arithmetic.

`/getmempoolinfo` is unchanged — it still returns the full `FeeStats` struct including `min_relay_fee_atoms`, `priority_fee_atoms`, median, min, max.

---

### Layer 3 — `tensorium-wallet-extension/src/lib/rpc.ts`

Add interface and method — no existing code removed:

```typescript
export interface EstimateFeeResponse {
  slow_atoms:       number;
  normal_atoms:     number;
  fast_atoms:       number;
  congestion_level: 'low' | 'medium' | 'high';
  mempool_count:    number;
  slow_txm:         number;
  normal_txm:       number;
  fast_txm:         number;
}

// Added to RpcClient interface:
estimateFee(): Promise<EstimateFeeResponse>;

// Added to createRpcClient():
estimateFee: () => rpcFetch(`${base}/estimatefee`),
```

---

### Layer 4 — `tensorium-wallet-extension/src/popup/pages/Send.tsx`

**New state:**

```typescript
type FeePreset = 'slow' | 'normal' | 'fast' | 'custom';
const [feePreset,    setFeePreset]    = useState<FeePreset>('normal');
const [customFeeTxm, setCustomFee]   = useState('');
const [feeEstimate,  setFeeEstimate] = useState<EstimateFeeResponse | null>(null);
const [showCustom,   setShowCustom]  = useState(false);
```

**Mount effect** — fetch `estimateFee()` in parallel with `getUtxos()`. On failure, leave `feeEstimate` as `null` (fallback kicks in below).

**Active fee derivation:**

```typescript
const MIN_RELAY = 10_000;
const activeFeeAtoms: number =
  feePreset === 'slow'   ? (feeEstimate?.slow_atoms   ?? MIN_RELAY) :
  feePreset === 'normal' ? (feeEstimate?.normal_atoms  ?? MIN_RELAY * 2) :
  feePreset === 'fast'   ? (feeEstimate?.fast_atoms    ?? MIN_RELAY * 10) :
  Math.max(Math.round(parseFloat(customFeeTxm || '0') * 1e8), MIN_RELAY);
```

**Fee selector UI** (inserted between amount input and Review button):

```
Network Fee                    ● Low activity
[ Slow  ]  [★ Normal ]  [ Fast  ]
 0.0001      0.0002      0.001 TXM
[ Custom ▾ ]
  └─ (expanded) input: "e.g. 0.0005 TXM"  ← min 0.0001 TXM
```

- Active pill: `wallet-btn--primary`. Inactive: `wallet-btn--secondary`.
- Congestion badge colour: Low → `#22c55e` (green), Medium → `#f59e0b` (amber), High → `#ef4444` (red).
- When `feeEstimate` is null: badge omitted silently; floor values shown.
- Custom input: `type="number"`, min `0.0001`. If parsed atoms < `MIN_RELAY`: input border red, note "Below minimum (0.0001 TXM)", Review button disabled.

**Confirm screen** — replace hardcoded `{fmt(MIN_RELAY_FEE_ATOMS)}` with `{fmt(activeFeeAtoms)}`.

**Send logic** — replace all references to `MIN_RELAY_FEE_ATOMS` in UTXO selection, change calculation, and balance check with `activeFeeAtoms`.

---

## Error Handling

| Scenario | Behaviour |
|---|---|
| `/estimatefee` unreachable | Silent fallback to floor values; no banner |
| Custom fee < `MIN_RELAY_FEE_ATOMS` | Red input border + note; Review disabled |
| Custom fee not a number | Treated as 0 → clamped to MIN_RELAY |
| Node rejects tx for low fee | Existing RPC error banner unchanged |

---

## Testing

**`tensorium-core/mempool.rs`** — 4 new unit tests:

| Test | Covers |
|---|---|
| `fee_tiers_empty_mempool` | Floor values returned; `congestion_level = Low` |
| `fee_tiers_low_congestion` | 3 txs; P25/P50/P75 ≥ respective floors; `Low` |
| `fee_tiers_medium_congestion` | 8 txs with varied fees; correct percentile values; `Medium` |
| `fee_tiers_high_congestion` | 25 txs; `High`; `fast ≥ normal ≥ slow` invariant |

Expected workspace test count: 113 → 117.

**Chrome wallet** — no new automated tests. Manual smoke test after deploy:
- Open Send, confirm 3 pills render with correct TXM values.
- Select Fast → confirm screen shows fast fee.
- Enter custom 0.00005 → Review disabled. Enter 0.0005 → Review enabled.
- Disconnect RPC → pills still render with floor fallback values.

---

## Files Changed

| Repo | File | Change |
|---|---|---|
| `tensorium-core` | `crates/tensorium-core/src/mempool.rs` | Add `CongestionLevel`, `FeeTiers`, `fee_tiers()`, 4 tests |
| `tensorium-core` | `crates/tensorium-node/src/main.rs` | Update `/estimatefee` response |
| `tensorium-wallet-extension` | `src/lib/rpc.ts` | Add `EstimateFeeResponse`, `estimateFee()` |
| `tensorium-wallet-extension` | `src/popup/pages/Send.tsx` | Fee selector UI + active fee wiring |
