//! L2 order-book reconstruction (Set E 5.3).
//!
//! Applies a snapshot and then contiguous deltas to keep a local view of one
//! instrument's book. Pure — no I/O, no clock — so the same code runs live and
//! in replay (the builders-purity rule in this crate's manifest).
//!
//! # The two rules that keep it honest
//!
//! * **A gap is a resync, never a guess.** A delta whose `sequence` is not
//!   exactly `last_sequence + 1` is not applied: the book is cleared, marked
//!   [`BookState::NeedsSnapshot`], and every later delta is refused until a
//!   snapshot arrives. Applying deltas across a gap produces a book that looks
//!   valid and is wrong, which is worse than no book.
//! * **Tentative data does not move the confirmed book.** An update marked
//!   `is_tentative` (low-confirmation on-chain data) is refused with
//!   [`ApplyOutcome::TentativeIgnored`] and leaves the sequence untouched, so the
//!   confirmed version of the same update still applies when it arrives.
//!
//! A crossed book after an update (best bid ≥ best ask) is reported as
//! [`ApplyOutcome::Crossed`] and also forces a resync: an exchange does not
//! publish a crossed book, so one here means a lost or misordered message.

use std::collections::BTreeMap;

use domain::money::{Price, Size};
use domain::payloads::orderbook::{BookLevel, BookUpdateKind, OrderBookPayload};
use rust_decimal::Decimal;

/// Whether the book can be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookState {
    /// No snapshot yet, or the last one was invalidated by a gap. Reads
    /// return `None`.
    NeedsSnapshot,
    /// Built from a snapshot plus contiguous deltas.
    Live,
}

/// What one update did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// A snapshot replaced the book.
    Rebuilt,
    /// A contiguous delta was applied.
    Applied,
    /// A delta at or below the last applied sequence — a redelivery; ignored.
    Stale,
    /// A delta that skipped sequences. The book was cleared and needs a
    /// snapshot.
    GapDetected { expected: u64, got: u64 },
    /// A delta arrived while the book was waiting for a snapshot; ignored.
    AwaitingSnapshot,
    /// The update was tentative and was not applied.
    TentativeIgnored,
    /// The update left the book crossed; it was cleared and needs a snapshot.
    Crossed,
}

/// One instrument's L2 book.
#[derive(Clone, Debug)]
pub struct OrderBookBuilder {
    /// Price → size. Read highest-first.
    bids: BTreeMap<Price, Size>,
    /// Price → size. Read lowest-first.
    asks: BTreeMap<Price, Size>,
    last_sequence: Option<u64>,
    state: BookState,
}

impl OrderBookBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_sequence: None,
            state: BookState::NeedsSnapshot,
        }
    }

    #[must_use]
    pub fn state(&self) -> BookState {
        self.state
    }

    #[must_use]
    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    fn invalidate(&mut self) {
        self.bids.clear();
        self.asks.clear();
        self.state = BookState::NeedsSnapshot;
    }

    fn set_levels(side: &mut BTreeMap<Price, Size>, levels: &[BookLevel]) {
        for l in levels {
            if l.size.inner() <= Decimal::ZERO {
                side.remove(&l.price);
            } else {
                side.insert(l.price, l.size);
            }
        }
    }

    /// Apply one update.
    pub fn feed_update(&mut self, update: &OrderBookPayload) -> ApplyOutcome {
        if update.is_tentative {
            return ApplyOutcome::TentativeIgnored;
        }
        let outcome = match update.kind {
            BookUpdateKind::Snapshot => {
                self.bids.clear();
                self.asks.clear();
                Self::set_levels(&mut self.bids, &update.bids);
                Self::set_levels(&mut self.asks, &update.asks);
                self.last_sequence = Some(update.sequence);
                self.state = BookState::Live;
                ApplyOutcome::Rebuilt
            }
            BookUpdateKind::Delta => {
                if self.state == BookState::NeedsSnapshot {
                    return ApplyOutcome::AwaitingSnapshot;
                }
                let last = self.last_sequence.unwrap_or(0);
                if update.sequence <= last {
                    return ApplyOutcome::Stale;
                }
                if update.sequence != last + 1 {
                    let expected = last + 1;
                    self.invalidate();
                    return ApplyOutcome::GapDetected {
                        expected,
                        got: update.sequence,
                    };
                }
                Self::set_levels(&mut self.bids, &update.bids);
                Self::set_levels(&mut self.asks, &update.asks);
                self.last_sequence = Some(update.sequence);
                ApplyOutcome::Applied
            }
        };
        if let (Some(b), Some(a)) = (self.best_bid_level(), self.best_ask_level()) {
            if b.0 >= a.0 {
                self.invalidate();
                return ApplyOutcome::Crossed;
            }
        }
        outcome
    }

    fn best_bid_level(&self) -> Option<(Price, Size)> {
        self.bids.iter().next_back().map(|(p, s)| (*p, *s))
    }

    fn best_ask_level(&self) -> Option<(Price, Size)> {
        self.asks.iter().next().map(|(p, s)| (*p, *s))
    }

    fn live(&self) -> bool {
        self.state == BookState::Live
    }

    /// Highest bid. `None` while the book needs a snapshot.
    #[must_use]
    pub fn best_bid(&self) -> Option<(Price, Size)> {
        self.live().then(|| self.best_bid_level()).flatten()
    }

    /// Lowest ask. `None` while the book needs a snapshot.
    #[must_use]
    pub fn best_ask(&self) -> Option<(Price, Size)> {
        self.live().then(|| self.best_ask_level()).flatten()
    }

    /// Midpoint of the best bid and ask.
    #[must_use]
    pub fn mid(&self) -> Option<Price> {
        let (b, _) = self.best_bid()?;
        let (a, _) = self.best_ask()?;
        Some(Price::from_decimal((b.inner() + a.inner()) / Decimal::TWO))
    }

    /// Best ask minus best bid.
    #[must_use]
    pub fn spread(&self) -> Option<Decimal> {
        let (b, _) = self.best_bid()?;
        let (a, _) = self.best_ask()?;
        Some(a.inner() - b.inner())
    }

    /// The top `n` levels each side: bids highest-first, asks lowest-first.
    /// Empty while the book needs a snapshot.
    #[must_use]
    pub fn depth(&self, n: usize) -> (Vec<BookLevel>, Vec<BookLevel>) {
        if !self.live() {
            return (vec![], vec![]);
        }
        let bids = self
            .bids
            .iter()
            .rev()
            .take(n)
            .map(|(p, s)| BookLevel {
                price: *p,
                size: *s,
            })
            .collect();
        let asks = self
            .asks
            .iter()
            .take(n)
            .map(|(p, s)| BookLevel {
                price: *p,
                size: *s,
            })
            .collect();
        (bids, asks)
    }
}

impl Default for OrderBookBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn lvl(p: &str, s: &str) -> BookLevel {
        BookLevel {
            price: Price::from_decimal(Decimal::from_str(p).unwrap()),
            size: Size::from_decimal(Decimal::from_str(s).unwrap()),
        }
    }

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn seeded() -> OrderBookBuilder {
        let mut b = OrderBookBuilder::new();
        let snap = OrderBookPayload::new_snapshot(
            vec![lvl("99", "1"), lvl("98", "2"), lvl("97", "3")],
            vec![lvl("101", "1"), lvl("102", "2")],
            10,
        );
        assert_eq!(b.feed_update(&snap), ApplyOutcome::Rebuilt);
        b
    }

    #[test]
    fn snapshot_then_deltas_reconstructs_a_known_book() {
        let mut b = seeded();
        // Seq 11: bid 99 removed, new bid 100; ask 101 resized.
        let d11 = OrderBookPayload::new_delta(
            vec![lvl("99", "0"), lvl("100", "4")],
            vec![lvl("101", "5")],
            11,
        );
        assert_eq!(b.feed_update(&d11), ApplyOutcome::Applied);
        // Seq 12: ask 101 removed.
        let d12 = OrderBookPayload::new_delta(vec![], vec![lvl("101", "0")], 12);
        assert_eq!(b.feed_update(&d12), ApplyOutcome::Applied);

        assert_eq!(b.best_bid().unwrap().0.inner(), d("100"));
        assert_eq!(b.best_ask().unwrap().0.inner(), d("102"));
        assert_eq!(b.mid().unwrap().inner(), d("101"));
        assert_eq!(b.spread().unwrap(), d("2"));
        let (bids, asks) = b.depth(10);
        let prices: Vec<Decimal> = bids.iter().map(|l| l.price.inner()).collect();
        assert_eq!(prices, vec![d("100"), d("98"), d("97")]);
        assert_eq!(asks.len(), 1);
        assert_eq!(b.last_sequence(), Some(12));
    }

    #[test]
    fn a_sequence_gap_forces_a_resync_instead_of_a_silent_corruption() {
        let mut b = seeded();
        let skipped = OrderBookPayload::new_delta(vec![lvl("100", "9")], vec![], 12);
        assert_eq!(
            b.feed_update(&skipped),
            ApplyOutcome::GapDetected {
                expected: 11,
                got: 12
            }
        );
        assert_eq!(b.state(), BookState::NeedsSnapshot);
        assert!(
            b.best_bid().is_none() && b.mid().is_none(),
            "no reads from a broken book"
        );
        // Later deltas are refused until a snapshot.
        let next = OrderBookPayload::new_delta(vec![], vec![], 13);
        assert_eq!(b.feed_update(&next), ApplyOutcome::AwaitingSnapshot);
        let snap = OrderBookPayload::new_snapshot(vec![lvl("50", "1")], vec![lvl("51", "1")], 20);
        assert_eq!(b.feed_update(&snap), ApplyOutcome::Rebuilt);
        assert_eq!(b.mid().unwrap().inner(), d("50.5"));
    }

    #[test]
    fn redelivered_and_tentative_updates_do_not_move_the_book() {
        let mut b = seeded();
        let old = OrderBookPayload::new_delta(vec![lvl("99", "0")], vec![], 10);
        assert_eq!(b.feed_update(&old), ApplyOutcome::Stale);
        let mut tentative = OrderBookPayload::new_delta(vec![lvl("99", "0")], vec![], 11);
        tentative.is_tentative = true;
        assert_eq!(b.feed_update(&tentative), ApplyOutcome::TentativeIgnored);
        assert_eq!(b.best_bid().unwrap().0.inner(), d("99"));
        // The confirmed version of the same update still applies.
        tentative.is_tentative = false;
        assert_eq!(b.feed_update(&tentative), ApplyOutcome::Applied);
        assert_eq!(b.best_bid().unwrap().0.inner(), d("98"));
    }

    #[test]
    fn a_crossed_book_is_refused() {
        let mut b = seeded();
        let cross = OrderBookPayload::new_delta(vec![lvl("103", "1")], vec![], 11);
        assert_eq!(b.feed_update(&cross), ApplyOutcome::Crossed);
        assert_eq!(b.state(), BookState::NeedsSnapshot);
    }
}
