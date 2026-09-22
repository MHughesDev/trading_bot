//! Bar-to-bar history registers, and the three-valued verdict that keeps a
//! condition nobody could evaluate distinct from one that was false.
//!
//! # Why this exists
//!
//! The v1 condition node is a pure function of the current bar: `interpreter`
//! and `bytecode` both read the features and bar fields standing *now* and
//! return a `bool`. There was no previous-value register anywhere, so
//! `crossed_above` had nowhere to read its earlier bar from — which is why
//! `strategy-lang` refuses the name outright (`needs_history`) rather than
//! approximating it with the level test `a > b`.
//!
//! The two are not the same claim. `a > b` is true for as long as the fast
//! line stays above the slow one; `crossed_above(a, b)` is true only on the bar
//! where it *became* so. A strategy written with the first enters once and then
//! re-enters on every subsequent bar until the lines part again.
//!
//! # The first bar is the whole problem
//!
//! A cross needs the previous bar. On the first bar of any window there is no
//! previous bar, and **a cross that cannot be evaluated is not a cross that did
//! not happen.** If the runtime answered `false` there, every re-slice of the
//! same data would lose the entries at its leading edge, and the loss would
//! compound with each re-slice — a walk-forward study of twelve folds would
//! drop twelve entries it never reports dropping.
//!
//! So evaluation returns [`Verdict`], which has three values and not two, and
//! [`Unknown`] says which kind of not-knowing it was. Nothing in this module
//! can produce a `false` from absent data: the only way to reach
//! [`Verdict::False`] is for an expression to evaluate, on real values, to zero.
//!
//! # What a register holds
//!
//! One `f64` per *site* per bar, where a site is one occurrence of a
//! history-bearing function in one condition expression. A cross site stores
//! the difference `a - b`, so the operands may be arbitrary sub-expressions
//! rather than the two bare feature slots FEAT-004 §3.3 sketched; a
//! `rising`/`falling` site stores the value itself and needs `n` bars of it.
//!
//! # Point-in-time
//!
//! A committed past value is never rewritten. There is no method on this type
//! that can reach back and change one, so a late revision of an already-closed
//! bar cannot retroactively alter what the cross on the following bar compared
//! against. Revisions to the bar that is still *open* do update its pending
//! value, which is not a point-in-time violation: that bar has not been read as
//! history by anyone yet.
//!
//! If bars ever arrive out of order, every committed value is discarded rather
//! than trusted — reading a register written by a *later* bar is exactly the
//! lookahead this type exists to prevent, and there is no flag to switch that
//! behaviour off.

/// The deepest history any one site may ask for, in bars.
///
/// Bounds the memory a single compiled expression can demand; `rising(x, n)`
/// beyond this is refused at compile time rather than allocated.
pub const MAX_HISTORY_DEPTH: u16 = 512;

/// Why a condition could not be evaluated on this bar.
///
/// Every variant means "the answer is not known", never "the answer is no".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// A feature the expression reads has not produced a value yet — the
    /// indicator is still inside its warm-up window.
    FeatureNotReady,
    /// A bar the expression reads has not arrived.
    BarNotReady,
    /// A history-bearing function has not accumulated the bars it needs.
    /// This is the first-bar case, and `have < need` says how far short it is.
    HistoryNotReady { have: u16, need: u16 },
    /// The arithmetic is undefined at these values (division by zero).
    Undefined,
    /// A bar arrived with an earlier stamp than one already processed, so the
    /// registers were cleared rather than read.
    OutOfOrderBar,
    /// The expression did not compile, or the compiled program is malformed.
    Malformed,
}

/// The value of a condition on one bar: true, false, or not knowable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    True,
    False,
    Unknown(Unknown),
}

impl Verdict {
    /// A verdict from a value that really was computed.
    #[must_use]
    pub const fn from_bool(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }

    #[must_use]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// The reason, when there is one.
    #[must_use]
    pub const fn reason(self) -> Option<Unknown> {
        match self {
            Self::Unknown(u) => Some(u),
            _ => None,
        }
    }

    /// Collapse three values into two, counting unknown as false.
    ///
    /// Named at length on purpose. Doing this is lossy and it is the exact
    /// mistake this type exists to prevent, so it may not happen silently
    /// inside an evaluator — a caller with a reason to collapse writes it at
    /// its own call site, where a reader can see it.
    #[must_use]
    pub const fn unknown_as_false(self) -> bool {
        matches!(self, Self::True)
    }
}

/// What opening a bar did to the registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarTransition {
    /// The first bar this register file has seen.
    Opened,
    /// The same bar as the last call — a revision, or another event arriving
    /// inside it. Nothing was committed.
    SameBar,
    /// A later bar: the previous bar's pending values became history.
    Rolled,
    /// An earlier bar than one already seen. All committed history was
    /// discarded; every site reads as unknown until it warms again.
    OutOfOrder,
}

#[derive(Clone, Debug)]
struct Site {
    depth: u16,
    /// Committed values, index 0 = the most recently closed bar.
    /// `None` means that bar closed without the site being computable.
    past: Vec<Option<f64>>,
    /// The value computed for the bar that is currently open. Becomes
    /// `past[0]` when a later bar opens, and can be overwritten any number of
    /// times before then.
    pending: Option<f64>,
}

impl Site {
    fn new(depth: u16) -> Self {
        let depth = depth.clamp(1, MAX_HISTORY_DEPTH);
        Self {
            depth,
            past: vec![None; depth as usize],
            pending: None,
        }
    }

    fn commit(&mut self) {
        self.past.pop();
        self.past.insert(0, self.pending.take());
    }

    fn clear(&mut self) {
        for slot in &mut self.past {
            *slot = None;
        }
        self.pending = None;
    }

    /// Contiguous run of known values from the most recent bar backwards.
    ///
    /// A gap stops the run: a cross may not step over a bar whose value was
    /// never known and compare against the one before it, because the bar it
    /// skipped might be where the crossing happened.
    fn filled(&self) -> u16 {
        let mut n = 0_u16;
        for slot in &self.past {
            if slot.is_some() {
                n += 1;
            } else {
                break;
            }
        }
        n
    }
}

/// A file of per-site history registers sharing one bar clock.
///
/// Several compiled programs can share one file: each takes a contiguous run
/// of sites starting at the base [`push_sites`](Self::push_sites) returns, so
/// one [`open_bar`](Self::open_bar) call rolls every condition in an instance
/// together and they cannot drift apart.
#[derive(Clone, Debug, Default)]
pub struct HistoryRegisters {
    sites: Vec<Site>,
    /// The bar currently open, as a sequence number supplied by the driver.
    open: Option<i64>,
    /// Set when the last `open_bar` went backwards. Cleared by the next bar
    /// that does not, but the history it discarded stays discarded.
    faulted: bool,
}

impl HistoryRegisters {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A file holding one site per entry of `depths`.
    #[must_use]
    pub fn with_depths(depths: &[u16]) -> Self {
        let mut regs = Self::new();
        regs.push_sites(depths);
        regs
    }

    /// Append `depths.len()` sites and return the base index of the run.
    pub fn push_sites(&mut self, depths: &[u16]) -> usize {
        let base = self.sites.len();
        self.sites.extend(depths.iter().copied().map(Site::new));
        base
    }

    #[must_use]
    pub fn site_count(&self) -> usize {
        self.sites.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// Whether any bar has been opened yet. Before the first one there is no
    /// bar clock, and every history site reads unknown.
    #[must_use]
    pub const fn is_clocked(&self) -> bool {
        self.open.is_some()
    }

    /// The bar currently open.
    #[must_use]
    pub const fn open_seq(&self) -> Option<i64> {
        self.open
    }

    /// Whether the last bar transition went backwards.
    #[must_use]
    pub const fn faulted(&self) -> bool {
        self.faulted
    }

    /// Declare which bar the evaluation that follows belongs to.
    ///
    /// `seq` must not decrease. Equal values mean the same bar — a revision, or
    /// any other event arriving inside it — and commit nothing. The driver owns
    /// what a bar is: the live instance buckets by the declared timeframe so
    /// that one minute is one bar and not sixty trade ticks, and a per-bar
    /// replay can pass the bar index straight through.
    pub fn open_bar(&mut self, seq: i64) -> BarTransition {
        match self.open {
            None => {
                self.open = Some(seq);
                self.faulted = false;
                BarTransition::Opened
            }
            Some(cur) if seq == cur => BarTransition::SameBar,
            Some(cur) if seq > cur => {
                for site in &mut self.sites {
                    site.commit();
                }
                self.open = Some(seq);
                self.faulted = false;
                BarTransition::Rolled
            }
            Some(_) => {
                // Going backwards means the driver's clock is not a clock.
                // Anything already committed may have been written by a bar
                // later than this one, and comparing against it would be
                // lookahead, so none of it survives.
                for site in &mut self.sites {
                    site.clear();
                }
                self.faulted = true;
                BarTransition::OutOfOrder
            }
        }
    }

    /// Record this bar's value for `site`. Overwrites any earlier value for the
    /// same bar; never touches a committed one.
    pub fn record(&mut self, site: usize, value: f64) {
        if let Some(s) = self.sites.get_mut(site) {
            s.pending = if value.is_nan() { None } else { Some(value) };
        }
    }

    /// The value `k` closed bars ago, `k = 0` being the most recent.
    ///
    /// `None` whenever it is not known — including every read while the file is
    /// faulted, which is what stops a register written by a later bar from
    /// being read as history.
    #[must_use]
    pub fn past(&self, site: usize, k: u16) -> Option<f64> {
        if self.faulted {
            return None;
        }
        self.sites
            .get(site)
            .and_then(|s| s.past.get(k as usize))
            .copied()
            .flatten()
    }

    /// Bars of contiguous known history `site` holds right now.
    #[must_use]
    pub fn filled(&self, site: usize) -> u16 {
        if self.faulted {
            return 0;
        }
        self.sites.get(site).map_or(0, Site::filled)
    }

    /// The depth `site` was created with.
    #[must_use]
    pub fn depth(&self, site: usize) -> u16 {
        self.sites.get(site).map_or(0, |s| s.depth)
    }

    /// The deepest history any site in this file needs, in bars. This is the
    /// warm-up lead-in a cross adds on top of its features' own.
    #[must_use]
    pub fn warmup_bars(&self) -> u16 {
        self.sites.iter().map(|s| s.depth).max().unwrap_or(0)
    }

    /// Whether `site` can be evaluated now, and why not when it cannot.
    ///
    /// # Errors
    ///
    /// [`Unknown::HistoryNotReady`] while the site is still warming, and
    /// [`Unknown::OutOfOrderBar`] once the bar clock has gone backwards.
    pub fn readiness(&self, site: usize) -> Result<(), Unknown> {
        if self.faulted {
            return Err(Unknown::OutOfOrderBar);
        }
        let need = self.depth(site);
        let have = self.filled(site);
        if have >= need {
            Ok(())
        } else {
            Err(Unknown::HistoryNotReady { have, need })
        }
    }
}

/// A signal that could neither fire nor be ruled out on this event.
///
/// Emitted instead of, and never alongside, the signal itself. A driver that
/// counts entries must count these too, or it is reporting a strategy that did
/// not trade when what happened is that nobody asked it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    /// The signal name the node would have emitted.
    pub signal: String,
    /// The condition node whose verdict was unknown.
    pub node: String,
    pub reason: Unknown,
}

/// Signals that fired, and signals whose conditions were not knowable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalVerdicts {
    /// Signals whose conditions evaluated to true.
    pub fired: Vec<String>,
    /// Signals whose conditions could not be evaluated. **Not** an empty
    /// category in normal operation: every window's leading bars land here for
    /// any strategy that reads a cross, or an indicator still warming.
    pub unresolved: Vec<Unresolved>,
}

impl SignalVerdicts {
    /// Whether every signal in this graph got a real answer.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.unresolved.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_with_no_history_is_unknown_not_false() {
        let regs = HistoryRegisters::with_depths(&[1]);
        assert_eq!(regs.past(0, 0), None);
        assert_eq!(
            regs.readiness(0),
            Err(Unknown::HistoryNotReady { have: 0, need: 1 })
        );
    }

    #[test]
    fn pending_becomes_history_only_when_a_later_bar_opens() {
        let mut regs = HistoryRegisters::with_depths(&[1]);
        assert_eq!(regs.open_bar(10), BarTransition::Opened);
        regs.record(0, 1.5);
        // Still the same bar: nothing is history yet.
        assert_eq!(regs.open_bar(10), BarTransition::SameBar);
        assert_eq!(regs.past(0, 0), None);

        assert_eq!(regs.open_bar(11), BarTransition::Rolled);
        assert_eq!(regs.past(0, 0), Some(1.5));
    }

    #[test]
    fn a_revision_of_the_open_bar_overwrites_pending_and_commits_once() {
        let mut regs = HistoryRegisters::with_depths(&[1]);
        regs.open_bar(10);
        regs.record(0, 1.0);
        regs.open_bar(10);
        regs.record(0, 2.0);
        regs.open_bar(11);
        assert_eq!(
            regs.past(0, 0),
            Some(2.0),
            "the last word on an open bar wins"
        );
    }

    /// The point-in-time guarantee: once a bar has closed, what the next bar
    /// compared against cannot be changed by anything arriving afterwards.
    #[test]
    fn a_committed_value_is_never_rewritten_by_a_later_event() {
        let mut regs = HistoryRegisters::with_depths(&[1]);
        regs.open_bar(10);
        regs.record(0, 1.0);
        regs.open_bar(11);
        assert_eq!(regs.past(0, 0), Some(1.0));
        // Everything that can happen inside bar 11 …
        regs.record(0, 99.0);
        regs.open_bar(11);
        // … leaves bar 10's committed value exactly as it was.
        assert_eq!(regs.past(0, 0), Some(1.0));
    }

    #[test]
    fn a_bar_that_could_not_be_computed_commits_a_gap_not_a_zero() {
        let mut regs = HistoryRegisters::with_depths(&[2]);
        regs.open_bar(1);
        regs.record(0, 5.0);
        regs.open_bar(2); // bar 1 commits 5.0
                          // bar 2 records nothing — the feature was not ready
        regs.open_bar(3); // bar 2 commits None
        assert_eq!(regs.past(0, 0), None, "absent, not 0.0");
        assert_eq!(regs.past(0, 1), Some(5.0));
        assert_eq!(regs.filled(0), 0, "history does not step over a gap");
    }

    #[test]
    fn going_backwards_discards_every_committed_value() {
        let mut regs = HistoryRegisters::with_depths(&[1]);
        regs.open_bar(10);
        regs.record(0, 1.0);
        regs.open_bar(11);
        assert_eq!(regs.past(0, 0), Some(1.0));

        assert_eq!(regs.open_bar(9), BarTransition::OutOfOrder);
        assert!(regs.faulted());
        assert_eq!(regs.past(0, 0), None, "a later bar's value is not history");
        assert_eq!(regs.readiness(0), Err(Unknown::OutOfOrderBar));
    }

    #[test]
    fn nan_records_as_absent() {
        let mut regs = HistoryRegisters::with_depths(&[1]);
        regs.open_bar(1);
        regs.record(0, f64::NAN);
        regs.open_bar(2);
        assert_eq!(regs.past(0, 0), None);
    }

    #[test]
    fn deep_sites_report_the_warmup_they_need() {
        let regs = HistoryRegisters::with_depths(&[1, 4, 2]);
        assert_eq!(regs.warmup_bars(), 4);
    }

    #[test]
    fn depth_is_clamped_to_the_maximum() {
        let regs = HistoryRegisters::with_depths(&[u16::MAX]);
        assert_eq!(regs.depth(0), MAX_HISTORY_DEPTH);
    }

    #[test]
    fn programs_sharing_a_file_get_disjoint_runs_and_roll_together() {
        let mut regs = HistoryRegisters::new();
        let a = regs.push_sites(&[1, 1]);
        let b = regs.push_sites(&[1]);
        assert_eq!((a, b), (0, 2));

        regs.open_bar(1);
        regs.record(a, 1.0);
        regs.record(b, 3.0);
        regs.open_bar(2);
        assert_eq!(regs.past(a, 0), Some(1.0));
        assert_eq!(regs.past(a + 1, 0), None);
        assert_eq!(regs.past(b, 0), Some(3.0));
    }

    #[test]
    fn unknown_is_not_false() {
        assert!(!Verdict::Unknown(Unknown::FeatureNotReady).is_true());
        assert_ne!(Verdict::Unknown(Unknown::FeatureNotReady), Verdict::False);
        assert!(Verdict::Unknown(Unknown::Undefined).is_unknown());
        assert!(!Verdict::False.is_unknown());
    }
}
