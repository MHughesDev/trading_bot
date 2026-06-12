//! Append-only internal transaction journal for paper accounts.
//!
//! Every cash movement on a paper account writes exactly one entry here, so
//! `sum(cash_delta) == cash - starting_cash` holds for each account at all
//! times.  The journal is the internal replacement for venue transaction
//! history — account pages read it instead of calling any external API.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Maximum entries retained per account (oldest evicted first).
const LEDGER_CAPACITY: usize = 100_000;

/// What kind of cash movement an entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperLedgerKind {
    /// Initial (or manual) cash seed.
    Deposit,
    /// Principal cash leg of a fill on a cash/binary account.
    Trade,
    /// Commission or exchange fee.
    Fee,
    /// Realized P&L settled to cash on a margin account.
    RealizedPnl,
    /// Perpetual-swap funding payment.
    Funding,
    /// Contract settlement (binary outcome, futures/option expiry).
    Settlement,
    /// Order rejected by the paper engine (zero cash movement; audit only).
    Rejection,
}

impl PaperLedgerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PaperLedgerKind::Deposit => "deposit",
            PaperLedgerKind::Trade => "trade",
            PaperLedgerKind::Fee => "fee",
            PaperLedgerKind::RealizedPnl => "realized_pnl",
            PaperLedgerKind::Funding => "funding",
            PaperLedgerKind::Settlement => "settlement",
            PaperLedgerKind::Rejection => "rejection",
        }
    }
}

/// One journal line on a paper account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperLedgerEntry {
    /// Monotonic per-account sequence number.
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub asset_class: AssetClass,
    pub kind: PaperLedgerKind,
    pub instrument_id: Option<String>,
    /// Engine order id that caused this entry (None for deposits/funding).
    pub order_id: Option<String>,
    /// Signed cash movement in the account's quote currency
    /// (positive credits the account, negative debits it).
    pub cash_delta: Decimal,
    /// Short human-readable context (e.g. `"buy 1 @ 50000"`).
    pub note: String,
}

/// Bounded append-only journal for one paper account.
#[derive(Debug)]
pub struct PaperLedger {
    asset_class: AssetClass,
    entries: VecDeque<PaperLedgerEntry>,
    next_seq: u64,
    capacity: usize,
}

impl PaperLedger {
    pub fn new(asset_class: AssetClass) -> Self {
        Self {
            asset_class,
            entries: VecDeque::new(),
            next_seq: 0,
            capacity: LEDGER_CAPACITY,
        }
    }

    /// Append an entry; evicts the oldest entry once at capacity.
    pub fn record(
        &mut self,
        kind: PaperLedgerKind,
        instrument_id: Option<String>,
        order_id: Option<String>,
        cash_delta: Decimal,
        note: String,
    ) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(PaperLedgerEntry {
            seq: self.next_seq,
            at: Utc::now(),
            asset_class: self.asset_class,
            kind,
            instrument_id,
            order_id,
            cash_delta,
            note,
        });
        self.next_seq += 1;
    }

    /// All retained entries at or after `since` (all entries when `None`).
    pub fn entries_since(&self, since: Option<DateTime<Utc>>) -> Vec<PaperLedgerEntry> {
        self.entries
            .iter()
            .filter(|e| since.is_none_or(|s| e.at >= s))
            .cloned()
            .collect()
    }

    /// Sum of all retained cash deltas (equals `cash - starting_cash` while
    /// nothing has been evicted).
    pub fn net_cash_flow(&self) -> Decimal {
        self.entries.iter().map(|e| e.cash_delta).sum()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn entries_get_monotonic_sequence_numbers() {
        let mut ledger = PaperLedger::new(AssetClass::Equity);
        ledger.record(
            PaperLedgerKind::Deposit,
            None,
            None,
            dec!(100),
            "seed".into(),
        );
        ledger.record(
            PaperLedgerKind::Trade,
            Some("AAPL".into()),
            Some("o-1".into()),
            dec!(-50),
            "buy".into(),
        );
        let all = ledger.entries_since(None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].seq, 0);
        assert_eq!(all[1].seq, 1);
        assert_eq!(ledger.net_cash_flow(), dec!(50));
    }

    #[test]
    fn since_filter_excludes_older_entries() {
        let mut ledger = PaperLedger::new(AssetClass::Equity);
        ledger.record(
            PaperLedgerKind::Deposit,
            None,
            None,
            dec!(100),
            "seed".into(),
        );
        let cutoff = Utc::now() + chrono::Duration::seconds(1);
        assert!(ledger.entries_since(Some(cutoff)).is_empty());
    }

    #[test]
    fn capacity_is_bounded() {
        let mut ledger = PaperLedger::new(AssetClass::Equity);
        ledger.capacity = 3;
        for i in 0..5 {
            ledger.record(
                PaperLedgerKind::Fee,
                None,
                None,
                Decimal::from(i),
                String::new(),
            );
        }
        assert_eq!(ledger.len(), 3);
        // Oldest evicted: surviving seqs are 2, 3, 4.
        assert_eq!(ledger.entries_since(None)[0].seq, 2);
    }
}
