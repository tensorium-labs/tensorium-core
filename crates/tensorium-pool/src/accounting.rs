use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// Pool fee in basis points (500 = 5.00 %).
pub const POOL_FEE_BPS: u64 = 500;

/// One entry in the pool payout ledger — one per accepted block.
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PayoutLedger {
    pub entries: Vec<PayoutEntry>,
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

    /// Aggregate stats: blocks found, total gross, total fee, total pending net.
    pub fn stats(&self) -> LedgerStats {
        let blocks_found = self.entries.len() as u64;
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
