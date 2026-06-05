use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Pool fee in basis points (500 = 5.00 %).
pub const POOL_FEE_BPS: u64 = 500;

/// Default PPLNS window size (number of shares to retain).
pub const PPLNS_DEFAULT_N: usize = 4096;

// ---------------------------------------------------------------------------
// Share tracking
// ---------------------------------------------------------------------------

/// One accepted share from a Stratum miner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareRecord {
    pub wallet_address:  String,
    pub worker_name:     String,
    /// Per-worker share difficulty at the time of submission (bits).
    pub share_diff_bits: u8,
    pub submitted_at_unix: u64,
}

impl ShareRecord {
    /// Difficulty weight = 2^share_diff_bits.
    pub fn weight(&self) -> u64 {
        1u64 << self.share_diff_bits
    }
}

/// Sliding window of the last N accepted shares, used for PPLNS reward splits.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShareWindow {
    pub n:      usize,
    pub shares: VecDeque<ShareRecord>,
}

impl Default for ShareWindow {
    fn default() -> Self { Self::new(PPLNS_DEFAULT_N) }
}

impl ShareWindow {
    pub fn new(n: usize) -> Self {
        Self { n, shares: VecDeque::with_capacity(n.min(16_384)) }
    }

    pub fn push(&mut self, share: ShareRecord) {
        if self.shares.len() >= self.n {
            self.shares.pop_front();
        }
        self.shares.push_back(share);
    }

    /// Sum of all share weights in the window.
    pub fn total_weight(&self) -> u64 {
        self.shares.iter().fold(0u64, |s, r| s.saturating_add(r.weight()))
    }

    /// Per-miner difficulty-weighted totals.
    pub fn miner_weights(&self) -> HashMap<String, u64> {
        let mut m: HashMap<String, u64> = HashMap::new();
        for s in &self.shares {
            *m.entry(s.wallet_address.clone()).or_default() += s.weight();
        }
        m
    }

    pub fn len(&self) -> usize { self.shares.len() }
    pub fn is_empty(&self) -> bool { self.shares.is_empty() }
}

// ---------------------------------------------------------------------------
// Window stats (for the /pool/miners API endpoint)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct WindowStats {
    pub window_n:        usize,
    pub shares_in_window: usize,
    pub total_weight:    u64,
    pub miners:          Vec<MinerWindowEntry>,
}

#[derive(Debug, Serialize)]
pub struct MinerWindowEntry {
    pub wallet_address: String,
    pub shares:         usize,
    pub weight:         u64,
    /// Percentage of total window weight (0–100).
    pub pct:            f64,
}

// ---------------------------------------------------------------------------
// Payout entry
// ---------------------------------------------------------------------------

/// One entry in the pool payout ledger.
/// With PPLNS one block may produce multiple entries — one per participating miner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutEntry {
    pub block_height: u64,
    pub block_hash: String,
    pub miner_address: String,
    pub gross_reward_atoms: u64,
    /// pool_fee = gross * POOL_FEE_BPS / 10_000  (rounds down)
    pub pool_fee_atoms: u64,
    /// net_payout = gross - pool_fee
    pub net_payout_atoms: u64,
    /// Whether the net payout has been sent on-chain.
    pub paid_out: bool,
}

impl PayoutEntry {
    pub fn new(
        block_height: u64,
        block_hash: String,
        miner_address: String,
        gross_reward_atoms: u64,
    ) -> Self {
        let (net, fee) = split_fee(gross_reward_atoms, POOL_FEE_BPS);
        Self {
            block_height,
            block_hash,
            miner_address,
            gross_reward_atoms,
            pool_fee_atoms: fee,
            net_payout_atoms: net,
            paid_out: false,
        }
    }
}

/// Split `gross` into `(net, fee)` where fee = gross * fee_bps / 10_000.
/// Always rounds fee down so net ≥ gross * (1 - fee_bps/10_000).
pub fn split_fee(gross: u64, fee_bps: u64) -> (u64, u64) {
    let fee = gross.saturating_mul(fee_bps) / 10_000;
    let net = gross.saturating_sub(fee);
    (net, fee)
}

/// Persistent payout ledger, stored as JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct PayoutLedger {
    pub entries: Vec<PayoutEntry>,
    /// PPLNS share window — persisted so restarts preserve accumulated shares.
    #[serde(default)]
    pub share_window: ShareWindow,
}

impl Default for PayoutLedger {
    fn default() -> Self {
        Self { entries: vec![], share_window: ShareWindow::default() }
    }
}

impl PayoutLedger {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize ledger: {e}"))?;
        fs::write(path, json).map_err(|e| format!("write ledger: {e}"))
    }

    pub fn push(&mut self, entry: PayoutEntry) {
        self.entries.push(entry);
    }

    /// Total pool fee collected across all entries.
    pub fn total_fee_atoms(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |sum, e| sum.saturating_add(e.pool_fee_atoms))
    }

    /// Total net pending (unpaid) for a given miner.
    pub fn pending_atoms(&self, miner_address: &str) -> u64 {
        self.entries
            .iter()
            .filter(|e| !e.paid_out && e.miner_address == miner_address)
            .fold(0u64, |sum, e| sum.saturating_add(e.net_payout_atoms))
    }

    /// Mark all entries for a miner as paid.
    pub fn mark_paid(&mut self, miner_address: &str) {
        for e in &mut self.entries {
            if e.miner_address == miner_address {
                e.paid_out = true;
            }
        }
    }

    /// Aggregate stats: blocks found (distinct hashes), total gross, fee, pending.
    pub fn stats(&self) -> LedgerStats {
        let blocks_found = {
            let mut seen: HashSet<&str> = HashSet::new();
            for e in &self.entries { seen.insert(e.block_hash.as_str()); }
            seen.len() as u64
        };
        let total_gross = self
            .entries
            .iter()
            .fold(0u64, |s, e| s.saturating_add(e.gross_reward_atoms));
        let total_fee = self.total_fee_atoms();
        let total_pending_net = self
            .entries
            .iter()
            .filter(|e| !e.paid_out)
            .fold(0u64, |s, e| s.saturating_add(e.net_payout_atoms));
        LedgerStats {
            blocks_found,
            total_gross_atoms: total_gross,
            total_fee_atoms: total_fee,
            total_pending_net_atoms: total_pending_net,
        }
    }

    // ── PPLNS ────────────────────────────────────────────────────────────────

    /// Record an accepted share into the PPLNS window.
    pub fn push_share(&mut self, share: ShareRecord) {
        self.share_window.push(share);
    }

    /// Distribute `gross` reward using PPLNS based on the current window.
    /// Returns `(wallet_address, miner_gross_atoms)` pairs sorted by amount desc.
    /// Rounding dust (a few atoms) stays in the pool treasury.
    /// Falls back to 100% for `fallback_addr` when the window is empty.
    pub fn pplns_split(&self, gross: u64, fallback_addr: &str) -> Vec<(String, u64)> {
        let weights = self.share_window.miner_weights();
        let total   = self.share_window.total_weight();
        if total == 0 || weights.is_empty() {
            return vec![(fallback_addr.to_string(), gross)];
        }
        let mut splits: Vec<(String, u64)> = weights
            .into_iter()
            .map(|(addr, w)| {
                let miner_gross = (gross as u128 * w as u128 / total as u128) as u64;
                (addr, miner_gross)
            })
            .filter(|(_, g)| *g > 0)
            .collect();
        splits.sort_by(|a, b| b.1.cmp(&a.1));
        splits
    }

    /// Window stats for the `/pool/miners` API endpoint.
    pub fn window_stats(&self) -> WindowStats {
        let weights = self.share_window.miner_weights();
        let total_w = self.share_window.total_weight() as f64;
        let mut share_counts: HashMap<&str, usize> = HashMap::new();
        for s in &self.share_window.shares {
            *share_counts.entry(s.wallet_address.as_str()).or_default() += 1;
        }
        let mut miners: Vec<MinerWindowEntry> = weights
            .iter()
            .map(|(addr, &w)| MinerWindowEntry {
                wallet_address: addr.clone(),
                shares: *share_counts.get(addr.as_str()).unwrap_or(&0),
                weight: w,
                pct:    if total_w > 0.0 { w as f64 / total_w * 100.0 } else { 0.0 },
            })
            .collect();
        miners.sort_by(|a, b| b.weight.cmp(&a.weight));
        WindowStats {
            window_n:         self.share_window.n,
            shares_in_window: self.share_window.len(),
            total_weight:     self.share_window.total_weight(),
            miners,
        }
    }
}

fn _unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[derive(Debug, Serialize)]
pub struct LedgerStats {
    pub blocks_found: u64,
    pub total_gross_atoms: u64,
    pub total_fee_atoms: u64,
    pub total_pending_net_atoms: u64,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_share(addr: &str, bits: u8) -> ShareRecord {
        ShareRecord {
            wallet_address:    addr.to_string(),
            worker_name:       "w".to_string(),
            share_diff_bits:   bits,
            submitted_at_unix: 0,
        }
    }

    // ── PPLNS window tests ────────────────────────────────────────────────

    #[test]
    fn window_evicts_oldest_when_full() {
        let mut w = ShareWindow::new(3);
        w.push(make_share("alice", 20));
        w.push(make_share("alice", 20));
        w.push(make_share("alice", 20));
        w.push(make_share("bob", 20)); // alice's first share evicted
        assert_eq!(w.len(), 3);
        let wts = w.miner_weights();
        assert_eq!(*wts.get("alice").unwrap_or(&0), (1u64 << 20) * 2);
        assert_eq!(*wts.get("bob").unwrap_or(&0), 1u64 << 20);
    }

    #[test]
    fn pplns_split_equal_hashrate() {
        let mut ledger = PayoutLedger::default();
        for _ in 0..10 {
            ledger.push_share(make_share("alice", 20));
            ledger.push_share(make_share("bob",   20));
        }
        let gross = 1_000_000u64;
        let splits = ledger.pplns_split(gross, "alice");
        let alice = splits.iter().find(|(a, _)| a == "alice").map(|(_, g)| *g).unwrap_or(0);
        let bob   = splits.iter().find(|(a, _)| a == "bob").map(|(_, g)| *g).unwrap_or(0);
        assert_eq!(alice, 500_000);
        assert_eq!(bob,   500_000);
        assert!(alice + bob <= gross);
    }

    #[test]
    fn pplns_split_weighted_by_diff() {
        let mut ledger = PayoutLedger::default();
        // alice has 2x higher diff → 2x weight
        ledger.push_share(make_share("alice", 21)); // weight 2M
        ledger.push_share(make_share("bob",   20)); // weight 1M
        let gross = 3_000_000u64;
        let splits = ledger.pplns_split(gross, "alice");
        let alice = splits.iter().find(|(a, _)| a == "alice").map(|(_, g)| *g).unwrap_or(0);
        let bob   = splits.iter().find(|(a, _)| a == "bob").map(|(_, g)| *g).unwrap_or(0);
        assert_eq!(alice, 2_000_000);
        assert_eq!(bob,   1_000_000);
    }

    #[test]
    fn pplns_split_single_miner_gets_all() {
        let mut ledger = PayoutLedger::default();
        for _ in 0..5 { ledger.push_share(make_share("solo", 20)); }
        let gross = 1_190_279_581u64;
        let splits = ledger.pplns_split(gross, "solo");
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].0, "solo");
        assert_eq!(splits[0].1, gross);
    }

    #[test]
    fn pplns_split_empty_window_fallback() {
        let ledger = PayoutLedger::default();
        let splits = ledger.pplns_split(1_000_000, "finder");
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].0, "finder");
        assert_eq!(splits[0].1, 1_000_000);
    }

    #[test]
    fn pplns_split_sum_never_exceeds_gross() {
        let mut ledger = PayoutLedger::default();
        for i in 0..100u8 {
            ledger.push_share(make_share(if i % 3 == 0 { "alice" } else if i % 3 == 1 { "bob" } else { "carol" }, 16 + (i % 8)));
        }
        let gross = 1_190_279_581u64;
        let splits = ledger.pplns_split(gross, "alice");
        let sum: u64 = splits.iter().map(|(_, g)| g).sum();
        assert!(sum <= gross, "sum {sum} > gross {gross}");
    }

    #[test]
    fn blocks_found_counts_distinct_hashes() {
        let mut ledger = PayoutLedger::default();
        // One block → two PPLNS entries (two miners)
        ledger.push(PayoutEntry::new(100, "hash_a".into(), "txm1alice".into(), 600_000));
        ledger.push(PayoutEntry::new(100, "hash_a".into(), "txm1bob".into(),   400_000));
        // Second block
        ledger.push(PayoutEntry::new(101, "hash_b".into(), "txm1alice".into(), 1_000_000));
        let stats = ledger.stats();
        assert_eq!(stats.blocks_found, 2); // distinct hashes, not entry count
    }

    // ── Existing fee/ledger tests ─────────────────────────────────────────

    #[test]
    fn fee_split_5_percent() {
        let gross = 1_523_557_865u64; // initial block reward
        let (net, fee) = split_fee(gross, POOL_FEE_BPS);
        // fee = 5% of gross (rounded down)
        assert_eq!(fee, 76_177_893);
        assert_eq!(net, 1_447_379_972);
        assert_eq!(net + fee, gross);
    }

    #[test]
    fn fee_split_zero_gross() {
        let (net, fee) = split_fee(0, POOL_FEE_BPS);
        assert_eq!(net, 0);
        assert_eq!(fee, 0);
    }

    #[test]
    fn fee_split_1_atom() {
        // 5% of 1 atom rounds down to 0, net = 1
        let (net, fee) = split_fee(1, POOL_FEE_BPS);
        assert_eq!(fee, 0);
        assert_eq!(net, 1);
    }

    #[test]
    fn fee_split_20_atoms() {
        // 5% of 20 = 1
        let (net, fee) = split_fee(20, POOL_FEE_BPS);
        assert_eq!(fee, 1);
        assert_eq!(net, 19);
    }

    #[test]
    fn payout_entry_construction() {
        let gross = 1_523_557_865u64;
        let entry = PayoutEntry::new(
            100,
            "abc123".to_string(),
            "txm1miner".to_string(),
            gross,
        );
        assert_eq!(entry.gross_reward_atoms, gross);
        assert_eq!(entry.pool_fee_atoms + entry.net_payout_atoms, gross);
        assert!(!entry.paid_out);
        // fee is 5%
        let expected_fee = gross * 500 / 10_000;
        assert_eq!(entry.pool_fee_atoms, expected_fee);
    }

    #[test]
    fn ledger_stats_accumulate_correctly() {
        let mut ledger = PayoutLedger::default();
        // Two blocks, two different miners
        ledger.push(PayoutEntry::new(1, "h1".into(), "txm1alice".into(), 1_000_000));
        ledger.push(PayoutEntry::new(2, "h2".into(), "txm1bob".into(), 2_000_000));
        let stats = ledger.stats();
        assert_eq!(stats.blocks_found, 2);
        assert_eq!(stats.total_gross_atoms, 3_000_000);
        // fee = 5% of 1M + 5% of 2M = 50_000 + 100_000 = 150_000
        assert_eq!(stats.total_fee_atoms, 150_000);
        // all unpaid → pending = net of both
        assert_eq!(
            stats.total_pending_net_atoms,
            950_000 + 1_900_000
        );
    }

    #[test]
    fn ledger_pending_per_miner() {
        let mut ledger = PayoutLedger::default();
        ledger.push(PayoutEntry::new(1, "h1".into(), "txm1alice".into(), 1_000_000));
        ledger.push(PayoutEntry::new(2, "h2".into(), "txm1alice".into(), 1_000_000));
        ledger.push(PayoutEntry::new(3, "h3".into(), "txm1bob".into(), 1_000_000));
        // alice has 2 blocks pending
        assert_eq!(ledger.pending_atoms("txm1alice"), 1_900_000);
        assert_eq!(ledger.pending_atoms("txm1bob"), 950_000);
    }

    #[test]
    fn ledger_mark_paid_clears_pending() {
        let mut ledger = PayoutLedger::default();
        ledger.push(PayoutEntry::new(1, "h1".into(), "txm1alice".into(), 1_000_000));
        assert_eq!(ledger.pending_atoms("txm1alice"), 950_000);
        ledger.mark_paid("txm1alice");
        assert_eq!(ledger.pending_atoms("txm1alice"), 0);
        // stats: no pending
        assert_eq!(ledger.stats().total_pending_net_atoms, 0);
    }

    #[test]
    fn fee_does_not_exceed_gross() {
        // Property: fee ≤ gross for any value
        for gross in [0u64, 1, 99, 100, 1_000, u64::MAX / 10_000] {
            let (net, fee) = split_fee(gross, POOL_FEE_BPS);
            assert!(fee <= gross, "fee {fee} > gross {gross}");
            assert_eq!(net + fee, gross);
        }
    }
}
