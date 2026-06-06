# Dynamic Fee System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add congestion-aware 3-tier fee estimation (slow/normal/fast) to the node RPC and a fee selector UI to the Chrome wallet extension.

**Architecture:** A new `fee_tiers()` method on `Mempool` computes percentile-based tiers from live mempool data (falling back to floor multipliers when empty). The `/estimatefee` RPC endpoint is updated to return these tiers. The Chrome wallet fetches the endpoint on Send page load and renders a 3-pill selector with a congestion badge; users can also enter a custom fee.

**Tech Stack:** Rust (tensorium-core, tensorium-node), TypeScript + React 18 (tensorium-wallet-extension)

---

## File Map

| File | Change |
|---|---|
| `crates/tensorium-core/src/mempool.rs` | Add `CongestionLevel`, `FeeTiers`, `Mempool::fee_tiers()`, 4 unit tests |
| `crates/tensorium-node/src/main.rs` | Replace `/estimatefee` handler body (lines 1655–1674) |
| `src/lib/rpc.ts` (wallet-extension) | Add `EstimateFeeResponse` interface + `estimateFee()` to `RpcClient` |
| `src/popup/pages/Send.tsx` (wallet-extension) | Fee selector UI + active fee wiring throughout |

---

## Task 1: Add `CongestionLevel`, `FeeTiers`, and `fee_tiers()` to `mempool.rs`

**Repo:** `tensorium-core`  
**Files:**
- Modify: `crates/tensorium-core/src/mempool.rs`

- [ ] **Step 1: Write the 4 failing tests**

Open `crates/tensorium-core/src/mempool.rs`. At the bottom of the `mod tests` block (currently ends around line 389), add:

```rust
    #[test]
    fn fee_tiers_empty_mempool() {
        let mp = Mempool::new();
        let tiers = mp.fee_tiers();
        assert_eq!(tiers.slow_atoms,   MIN_RELAY_FEE_ATOMS);
        assert_eq!(tiers.normal_atoms, MIN_RELAY_FEE_ATOMS * 2);
        assert_eq!(tiers.fast_atoms,   MIN_RELAY_FEE_ATOMS * 10);
        assert_eq!(tiers.mempool_count, 0);
        assert!(matches!(tiers.congestion_level, CongestionLevel::Low));
    }

    #[test]
    fn fee_tiers_low_congestion() {
        // 3 transactions — all fees at MIN_RELAY_FEE_ATOMS, congestion = Low
        let mut mp = Mempool::new();
        for i in 0u8..3 {
            let fee = MIN_RELAY_FEE_ATOMS + i as u64;
            mp.pending.insert(
                format!("tx{i}"),
                MempoolEntry { tx: Transaction::coinbase(i as u64, 0, "x"), fee_atoms: fee },
            );
        }
        let tiers = mp.fee_tiers();
        assert!(tiers.slow_atoms   >= MIN_RELAY_FEE_ATOMS);
        assert!(tiers.normal_atoms >= MIN_RELAY_FEE_ATOMS * 2);
        assert!(tiers.fast_atoms   >= MIN_RELAY_FEE_ATOMS * 10);
        assert!(tiers.slow_atoms <= tiers.normal_atoms);
        assert!(tiers.normal_atoms <= tiers.fast_atoms);
        assert!(matches!(tiers.congestion_level, CongestionLevel::Low));
    }

    #[test]
    fn fee_tiers_medium_congestion() {
        // 8 transactions with fees 10_000..17_000, congestion = Medium
        let mut mp = Mempool::new();
        for i in 0u8..8 {
            let fee = MIN_RELAY_FEE_ATOMS + i as u64 * 1_000;
            mp.pending.insert(
                format!("tx{i}"),
                MempoolEntry { tx: Transaction::coinbase(i as u64, 0, "x"), fee_atoms: fee },
            );
        }
        let tiers = mp.fee_tiers();
        // P25 index = 8*25/100 = 2 → fee[2] = 12_000 < NORMAL_FLOOR(20_000) → clamped
        assert_eq!(tiers.slow_atoms,   MIN_RELAY_FEE_ATOMS);   // P25=12_000 but floor=10_000
        assert_eq!(tiers.normal_atoms, MIN_RELAY_FEE_ATOMS * 2); // P50=14_000 < 20_000 → floor
        assert_eq!(tiers.fast_atoms,   MIN_RELAY_FEE_ATOMS * 10); // P75=16_000 < 100_000 → floor
        assert!(matches!(tiers.congestion_level, CongestionLevel::Medium));
    }

    #[test]
    fn fee_tiers_high_congestion() {
        // 25 transactions with fees 10_000..34_000, congestion = High
        let mut mp = Mempool::new();
        for i in 0u8..25 {
            let fee = MIN_RELAY_FEE_ATOMS + i as u64 * 1_000;
            mp.pending.insert(
                format!("tx{i:02}"),
                MempoolEntry { tx: Transaction::coinbase(i as u64, 0, "x"), fee_atoms: fee },
            );
        }
        let tiers = mp.fee_tiers();
        assert!(matches!(tiers.congestion_level, CongestionLevel::High));
        // Invariant: slow ≤ normal ≤ fast
        assert!(tiers.slow_atoms <= tiers.normal_atoms);
        assert!(tiers.normal_atoms <= tiers.fast_atoms);
        assert_eq!(tiers.mempool_count, 25);
    }
```

Also add `CongestionLevel` to the imports at the top of the `tests` module:
```rust
    use super::{CongestionLevel, Mempool, MempoolEntry, MIN_RELAY_FEE_ATOMS};
```

- [ ] **Step 2: Run tests — expect compile error (types not defined yet)**

```bash
cd /root/.openclaw/workspace/tensorium-core
cargo test -p tensorium-core fee_tiers 2>&1 | head -20
```

Expected: compile error `cannot find type CongestionLevel` or similar.

- [ ] **Step 3: Add `CongestionLevel`, `FeeTiers`, and `fee_tiers()` to `mempool.rs`**

After the existing `FeeStats` struct (currently around line 194–202), add:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CongestionLevel { Low, Medium, High }

#[derive(Debug, Serialize)]
pub struct FeeTiers {
    pub slow_atoms:       u64,
    pub normal_atoms:     u64,
    pub fast_atoms:       u64,
    pub congestion_level: CongestionLevel,
    pub mempool_count:    u64,
}
```

Then add `fee_tiers()` inside `impl Mempool`, after `fee_stats()`:

```rust
    /// Returns three fee tiers based on live mempool percentiles.
    /// Falls back to floor multipliers when the mempool is empty.
    pub fn fee_tiers(&self) -> FeeTiers {
        const SLOW_FLOOR:   u64 = MIN_RELAY_FEE_ATOMS;
        const NORMAL_FLOOR: u64 = MIN_RELAY_FEE_ATOMS * 2;
        const FAST_FLOOR:   u64 = MIN_RELAY_FEE_ATOMS * 10;

        let count = self.pending.len() as u64;

        let congestion_level = if count < 5 {
            CongestionLevel::Low
        } else if count < 20 {
            CongestionLevel::Medium
        } else {
            CongestionLevel::High
        };

        if self.is_empty() {
            return FeeTiers {
                slow_atoms:   SLOW_FLOOR,
                normal_atoms: NORMAL_FLOOR,
                fast_atoms:   FAST_FLOOR,
                congestion_level,
                mempool_count: count,
            };
        }

        let mut fees: Vec<u64> = self.pending.values().map(|e| e.fee_atoms).collect();
        fees.sort_unstable();
        let len = fees.len();

        let slow   = fees[len * 25 / 100].max(SLOW_FLOOR);
        let normal = fees[len * 50 / 100].max(NORMAL_FLOOR);
        let fast   = fees[len * 75 / 100].max(FAST_FLOOR);

        FeeTiers {
            slow_atoms:   slow,
            normal_atoms: normal,
            fast_atoms:   fast,
            congestion_level,
            mempool_count: count,
        }
    }
```

- [ ] **Step 4: Run the 4 new tests — expect all pass**

```bash
cargo test -p tensorium-core fee_tiers -- --nocapture
```

Expected output:
```
test mempool::tests::fee_tiers_empty_mempool ... ok
test mempool::tests::fee_tiers_high_congestion ... ok
test mempool::tests::fee_tiers_low_congestion ... ok
test mempool::tests::fee_tiers_medium_congestion ... ok
```

- [ ] **Step 5: Run full workspace tests — expect 117 pass, 0 fail**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: `test result: ok. 117 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add crates/tensorium-core/src/mempool.rs
git commit -m "feat(mempool): add CongestionLevel, FeeTiers, fee_tiers() with 4 tests"
```

---

## Task 2: Update `/estimatefee` endpoint in `tensorium-node/main.rs`

**Repo:** `tensorium-core`  
**Files:**
- Modify: `crates/tensorium-node/src/main.rs` lines 1655–1674

- [ ] **Step 1: Replace the `/estimatefee` handler body**

Find the block starting at line 1655:
```rust
        ("GET", "/estimatefee") => {
            let mempool = load_mempool(&mempool_path);
            let fee_stats = mempool.fee_stats();
            // Recommend max(min_relay, median) so users beat the current queue.
            let recommended = fee_stats.median_fee_atoms
                .max(fee_stats.min_relay_fee_atoms);
            write_json_response(
                stream,
                200,
                &json!({
                    "min_relay_fee_atoms":  fee_stats.min_relay_fee_atoms,
                    "priority_fee_atoms":   fee_stats.priority_fee_atoms,
                    "median_fee_atoms":     fee_stats.median_fee_atoms,
                    "recommended_fee_atoms": recommended,
                    "min_relay_fee_txm":    fee_stats.min_relay_fee_atoms as f64 / 1e8,
                    "priority_fee_txm":     fee_stats.priority_fee_atoms  as f64 / 1e8,
                    "recommended_fee_txm":  recommended as f64 / 1e8,
                }),
            )
        }
```

Replace with:
```rust
        ("GET", "/estimatefee") => {
            let mempool = load_mempool(&mempool_path);
            let tiers = mempool.fee_tiers();
            write_json_response(
                stream,
                200,
                &json!({
                    "slow_atoms":       tiers.slow_atoms,
                    "normal_atoms":     tiers.normal_atoms,
                    "fast_atoms":       tiers.fast_atoms,
                    "congestion_level": tiers.congestion_level,
                    "mempool_count":    tiers.mempool_count,
                    "slow_txm":         tiers.slow_atoms   as f64 / 1e8,
                    "normal_txm":       tiers.normal_atoms as f64 / 1e8,
                    "fast_txm":         tiers.fast_atoms   as f64 / 1e8,
                }),
            )
        }
```

Note: `fee_tiers()` is now used here; `fee_stats()` still used by `/getmempoolinfo` — do NOT remove it.

You will need to add `FeeTiers` and `CongestionLevel` to the `use tensorium_core::mempool::` import at the top of `main.rs`. Find the existing import line and add them:

```rust
use tensorium_core::mempool::{Mempool, MIN_RELAY_FEE_ATOMS, CongestionLevel, FeeTiers};
```

(Check the exact existing import and extend it — do not duplicate the `use` line.)

- [ ] **Step 2: Build the node crate — expect 0 errors**

```bash
cd /root/.openclaw/workspace/tensorium-core
cargo build -p tensorium-node 2>&1 | grep -E "^error|warning\[" | head -20
```

Expected: no `error` lines. Unused import warnings are acceptable.

- [ ] **Step 3: Run full workspace tests — expect 117 pass, 0 fail**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: `test result: ok. 117 passed; 0 failed`

- [ ] **Step 4: Smoke-test the endpoint locally**

```bash
# Start node in background, wait 1 second, query endpoint
cargo run -p tensorium-node -- testnet rpc &>/tmp/node.log &
NODE_PID=$!
sleep 2
curl -s http://127.0.0.1:23332/estimatefee | python3 -m json.tool
kill $NODE_PID 2>/dev/null
```

Expected response shape:
```json
{
  "slow_atoms": 10000,
  "normal_atoms": 20000,
  "fast_atoms": 100000,
  "congestion_level": "low",
  "mempool_count": 0,
  "slow_txm": 0.0001,
  "normal_txm": 0.0002,
  "fast_txm": 0.001
}
```

- [ ] **Step 5: Commit**

```bash
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): update /estimatefee to return 3-tier congestion-based fees"
```

---

## Task 3: Add `EstimateFeeResponse` and `estimateFee()` to `rpc.ts`

**Repo:** `tensorium-wallet-extension`  
**Files:**
- Modify: `src/lib/rpc.ts`

- [ ] **Step 1: Add the interface and method**

Open `src/lib/rpc.ts`. After the `MempoolInfoResponse` interface (currently around line 58–69), add:

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
```

In the `RpcClient` interface (currently around line 71–78), add after `getMempoolInfo`:

```typescript
  estimateFee(): Promise<EstimateFeeResponse>;
```

In `createRpcClient()` return object (currently around line 80–96), add after `getMempoolInfo`:

```typescript
    estimateFee: () => rpcFetch(`${base}/estimatefee`),
```

- [ ] **Step 2: Type-check — expect 0 errors**

```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
npm run typecheck 2>&1 | tail -10
```

Expected: `Found 0 errors.`

- [ ] **Step 3: Commit**

```bash
git add src/lib/rpc.ts
git commit -m "feat(rpc): add EstimateFeeResponse interface and estimateFee() method"
```

---

## Task 4: Fee selector UI in `Send.tsx`

**Repo:** `tensorium-wallet-extension`  
**Files:**
- Modify: `src/popup/pages/Send.tsx`

This is the largest task. Work through it in sub-steps.

- [ ] **Step 1: Add new state variables and import**

At the top of the file, add `EstimateFeeResponse` to the rpc import:

```typescript
import { createRpcClient, type UtxoEntry, type EstimateFeeResponse } from '../../lib/rpc';
```

Inside the `Send` component, after the existing `useState` declarations, add:

```typescript
type FeePreset = 'slow' | 'normal' | 'fast' | 'custom';
const [feePreset,    setFeePreset]    = useState<FeePreset>('normal');
const [customFeeTxm, setCustomFee]   = useState('');
const [feeEstimate,  setFeeEstimate] = useState<EstimateFeeResponse | null>(null);
const [showCustom,   setShowCustom]  = useState(false);
```

- [ ] **Step 2: Fetch `estimateFee()` in the mount effect**

The existing `useEffect` fetches UTXOs. Extend it to also fetch fee estimate in parallel. Replace the existing `useEffect` body with:

```typescript
  useEffect(() => {
    (async () => {
      try {
        const wallet = await loadWallet();
        if (!wallet) return;
        const rpcUrl = await loadSelectedRpcUrl();
        const rpc = createRpcClient(rpcUrl);
        const [utxoResult] = await Promise.allSettled([
          rpc.getUtxos(wallet.address),
          rpc.estimateFee().then(setFeeEstimate).catch(() => { /* silent fallback */ }),
        ]);
        if (utxoResult.status === 'fulfilled') {
          const mature = utxoResult.value.utxos.filter((u: UtxoEntry) => u.mature);
          setUtxos(mature);
          setBalance(mature.reduce((s: number, u: UtxoEntry) => s + u.value_atoms, 0));
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to load balance.');
      }
    })();
  }, []);
```

- [ ] **Step 3: Add `activeFeeAtoms` derivation**

Remove the existing line:
```typescript
const totalSpendAtoms = amountAtoms + MIN_RELAY_FEE_ATOMS;
```

Replace with:
```typescript
const MIN_RELAY = 10_000;
const activeFeeAtoms: number =
  feePreset === 'slow'   ? (feeEstimate?.slow_atoms   ?? MIN_RELAY) :
  feePreset === 'normal' ? (feeEstimate?.normal_atoms  ?? MIN_RELAY * 2) :
  feePreset === 'fast'   ? (feeEstimate?.fast_atoms    ?? MIN_RELAY * 10) :
  Math.max(Math.round(parseFloat(customFeeTxm || '0') * 1e8), MIN_RELAY);

const customFeeAtoms = Math.round(parseFloat(customFeeTxm || '0') * 1e8);
const customFeeTooLow = feePreset === 'custom' && customFeeTxm !== '' && customFeeAtoms < MIN_RELAY;
const totalSpendAtoms = amountAtoms + activeFeeAtoms;
```

- [ ] **Step 4: Replace `MIN_RELAY_FEE_ATOMS` references throughout the component**

Find every occurrence of `MIN_RELAY_FEE_ATOMS` in `Send.tsx` and replace:

| Old | New |
|---|---|
| `amountAtoms + MIN_RELAY_FEE_ATOMS` | `totalSpendAtoms` (already computed above) |
| `selectedAtoms - amountAtoms - MIN_RELAY_FEE_ATOMS` | `selectedAtoms - amountAtoms - activeFeeAtoms` |
| `fmt(MIN_RELAY_FEE_ATOMS)` | `fmt(activeFeeAtoms)` |
| `const MIN_RELAY_FEE_ATOMS = 10_000;` at top | Delete this line (replaced by `MIN_RELAY` above) |

Also update `validate()` — it uses `totalSpendAtoms` which is already correct after Step 3.

- [ ] **Step 5: Add the fee selector UI block**

In the form JSX (the last `return` block), find the section between the amount input and the `wallet-stack` div:

```tsx
      <input placeholder="Amount in TXM (e.g. 1.5)" value={amountTxm}
        onChange={(e) => setAmountTxm(e.target.value)} type="number" min="0" className="wallet-input" />
      <div className="wallet-stack">
```

Insert the fee selector between them:

```tsx
      <input placeholder="Amount in TXM (e.g. 1.5)" value={amountTxm}
        onChange={(e) => setAmountTxm(e.target.value)} type="number" min="0" className="wallet-input" />

      {/* Fee selector */}
      <div className="wallet-card" style={{ marginTop: 10 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <div className="wallet-section-label" style={{ margin: 0 }}>Network Fee</div>
          {feeEstimate && (
            <span style={{
              fontSize: 11, fontWeight: 600, borderRadius: 99, padding: '2px 8px',
              background:
                feeEstimate.congestion_level === 'high'   ? '#ef444422' :
                feeEstimate.congestion_level === 'medium' ? '#f59e0b22' : '#22c55e22',
              color:
                feeEstimate.congestion_level === 'high'   ? '#ef4444' :
                feeEstimate.congestion_level === 'medium' ? '#f59e0b' : '#22c55e',
            }}>
              ●&nbsp;{feeEstimate.congestion_level === 'high' ? 'Congested' :
                      feeEstimate.congestion_level === 'medium' ? 'Busy' : 'Low activity'}
            </span>
          )}
        </div>
        <div style={{ display: 'flex', gap: 6, marginBottom: 6 }}>
          {(['slow', 'normal', 'fast'] as const).map((tier) => {
            const atoms = feeEstimate?.[`${tier}_atoms` as keyof EstimateFeeResponse] as number
              ?? (tier === 'slow' ? 10_000 : tier === 'normal' ? 20_000 : 100_000);
            const active = feePreset === tier;
            return (
              <button
                key={tier}
                onClick={() => { setFeePreset(tier); setShowCustom(false); }}
                className={`wallet-btn ${active ? 'wallet-btn--primary' : 'wallet-btn--secondary'}`}
                style={{ flex: 1, padding: '6px 4px', fontSize: 12 }}
              >
                <div style={{ fontWeight: 700, textTransform: 'capitalize' }}>{tier}</div>
                <div style={{ fontSize: 10, opacity: 0.85 }}>{fmt(atoms)}</div>
              </button>
            );
          })}
        </div>
        <button
          onClick={() => { setShowCustom((v) => !v); setFeePreset(showCustom ? 'normal' : 'custom'); }}
          className={`wallet-btn ${feePreset === 'custom' ? 'wallet-btn--primary' : 'wallet-btn--secondary'}`}
          style={{ width: '100%', fontSize: 12, padding: '5px 0' }}
        >
          Custom {showCustom ? '▲' : '▾'}
        </button>
        {showCustom && (
          <div style={{ marginTop: 8 }}>
            <input
              type="number"
              min="0.0001"
              step="0.0001"
              placeholder="e.g. 0.0005"
              value={customFeeTxm}
              onChange={(e) => setCustomFee(e.target.value)}
              className="wallet-input"
              style={{ borderColor: customFeeTooLow ? '#ef4444' : undefined }}
            />
            {customFeeTooLow && (
              <span className="wallet-note" style={{ color: '#ef4444' }}>
                Below minimum (0.0001 TXM)
              </span>
            )}
            <span className="wallet-note">min 0.0001 TXM</span>
          </div>
        )}
      </div>

      <div className="wallet-stack">
```

- [ ] **Step 6: Disable Review button when custom fee is too low**

Find the Review button:
```tsx
        <button onClick={review} disabled={!toAddress || !amountTxm} className="wallet-btn wallet-btn--primary">Review</button>
```

Update `disabled` condition:
```tsx
        <button onClick={review} disabled={!toAddress || !amountTxm || customFeeTooLow} className="wallet-btn wallet-btn--primary">Review</button>
```

- [ ] **Step 7: Type-check — expect 0 errors**

```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
npm run typecheck 2>&1 | tail -10
```

Expected: `Found 0 errors.`

- [ ] **Step 8: Lint**

```bash
npm run lint 2>&1 | tail -10
```

Expected: no errors (warnings acceptable).

- [ ] **Step 9: Build**

```bash
npm run build 2>&1 | tail -10
```

Expected: build succeeds, `dist/` updated.

- [ ] **Step 10: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
git add src/popup/pages/Send.tsx src/lib/rpc.ts
git commit -m "feat(wallet): 3-tier dynamic fee selector with congestion badge and custom input"
```

---

## Task 5: Build & deploy to VPS + smoke test

**Repos:** `tensorium-core`, `tensorium-wallet-extension`

- [ ] **Step 1: Push tensorium-core to GitHub**

```bash
cd /root/.openclaw/workspace/tensorium-core
git push origin main
```

- [ ] **Step 2: Pull and rebuild node binary on DO VPS (157.230.44.162)**

```bash
ssh root@157.230.44.162 "cd /root/tensorium-core && git pull && cargo build --release -p tensorium-node 2>&1 | tail -5"
```

Expected: `Compiling tensorium-node`, then `Finished release`.

- [ ] **Step 3: Deploy new binary and restart MC service on DO**

```bash
ssh root@157.230.44.162 "
  cp /usr/local/bin/tensorium-node /usr/local/bin/tensorium-node.bak-pre-dynfee &&
  cp /root/tensorium-core/target/release/tensorium-node /usr/local/bin/tensorium-node &&
  systemctl restart tensorium-mc &&
  sleep 3 &&
  curl -s http://127.0.0.1:33332/estimatefee
"
```

Expected: JSON with `slow_atoms`, `normal_atoms`, `fast_atoms`, `congestion_level`.

- [ ] **Step 4: Pull and rebuild on Vultr VPS (139.180.137.144)**

```bash
ssh root@139.180.137.144 "cd /root/tensorium-core && git pull && cargo build --release -p tensorium-node 2>&1 | tail -5"
ssh root@139.180.137.144 "
  cp /usr/local/bin/tensorium-node /usr/local/bin/tensorium-node.bak-pre-dynfee &&
  cp /root/tensorium-core/target/release/tensorium-node /usr/local/bin/tensorium-node &&
  systemctl restart tensorium-mc &&
  sleep 3 &&
  curl -s http://127.0.0.1:33332/estimatefee
"
```

- [ ] **Step 5: Verify public RPC endpoint**

```bash
curl -s https://mc-rpc.tensoriumlabs.com/estimatefee | python3 -m json.tool
```

Expected response:
```json
{
  "slow_atoms": 10000,
  "normal_atoms": 20000,
  "fast_atoms": 100000,
  "congestion_level": "low",
  "mempool_count": 0,
  "slow_txm": 0.0001,
  "normal_txm": 0.0002,
  "fast_txm": 0.001
}
```

- [ ] **Step 6: Push wallet extension to GitHub**

```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
git push origin main
```

- [ ] **Step 7: Manual smoke test in browser**

Load the extension from `dist/` in Chrome (`chrome://extensions` → Load unpacked).

Checklist:
- [ ] Open Send page → 3 fee pills render (Slow / Normal / Fast) with TXM values
- [ ] "Normal" is pre-selected (highlighted)
- [ ] Green "Low activity" badge visible (when connected to mainnet RPC)
- [ ] Click "Fast" → confirm screen shows fast fee (0.001 TXM)
- [ ] Click "Custom ▾" → input appears
- [ ] Enter `0.00005` → input turns red, note "Below minimum", Review button disabled
- [ ] Enter `0.0005` → Review button enabled
- [ ] Disconnect from internet → reload Send → pills still show floor fallback values, no error banner

- [ ] **Step 8: Create GitHub release v0.3.5-mainnet (tensorium-core)**

```bash
cd /root/.openclaw/workspace/tensorium-core

# Update version references
sed -i 's/v0\.3\.4-mainnet/v0.3.5-mainnet/g' install.sh

# Add CHANGELOG entry
# Edit CHANGELOG.md: add v0.3.5 entry above v0.3.4 entry:
# ## v0.3.5-mainnet (2026-06-06)
# - Dynamic fee estimation: `/estimatefee` now returns slow/normal/fast tiers based on mempool congestion
# - Chrome wallet: 3-pill fee selector with congestion badge and custom fee input

git add install.sh CHANGELOG.md
git commit -m "chore: bump to v0.3.5-mainnet"
git tag v0.3.5-mainnet
git push origin main --tags
```

Then build release binaries on VPS and upload to GitHub release:
```bash
ssh root@157.230.44.162 "
  cd /root/tensorium-core &&
  cargo build --release 2>&1 | tail -3 &&
  cp target/release/tensorium-node /root/releases/tensorium-node-v0.3.5 &&
  cp target/release/txmwallet /root/releases/txmwallet-v0.3.5
"
```

Upload via `gh release create v0.3.5-mainnet` or through GitHub web UI with binaries + updated CHECKSUMS.
