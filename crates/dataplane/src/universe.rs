//! As-of universe membership (Set P P-4.3; DA-08; DATA-005 §8).
//!
//! A universe is not a list. It is a list *with a date*, and the difference is
//! the single most expensive mistake in backtesting: evaluate a strategy on
//! today's S&P 500 over the last ten years and every result is inflated by the
//! companies that were removed from the index for going bust. The instruments
//! that would have hurt are absent because they failed, which is precisely why
//! they are absent.
//!
//! So membership here is an **append-only log of changes**, not a mutable set.
//!
//! ## Why a log rather than intervals with a `valid_to`
//!
//! The obvious shape is one row per membership interval: `(universe, instrument,
//! valid_from, valid_to, reason)`, with a removal setting `valid_to`. P-4.3's
//! acceptance forbids exactly that — *"`valid_to` is never overwritten — a
//! removal is a new row"* — and the reason is not fussiness. An `UPDATE` that
//! closes an interval destroys the previous answer: once `valid_to` is written
//! there is no way to ask what the platform believed membership was *before* the
//! removal was recorded, so a study that ran last week cannot be reproduced. The
//! same failure as overwriting a bar instead of storing a revision (DA-16), in a
//! different table.
//!
//! The log is therefore the write model: [`MembershipEvent`] rows, only ever
//! inserted. Intervals are a *read* model, derived by [`intervals`], and every
//! `valid_to` in them is computed from a later `Removed` event rather than stored.
//!
//! ## Two clocks, and they are not the same
//!
//! Each event carries `effective` (when the change took effect in the market) and
//! `recorded_at` (when the platform learned of it). An index change announced on
//! the 15th and effective on the 20th is knowable on the 15th, and a study whose
//! knowledge cutoff is the 17th may use it. [`members_as_of`] takes the market
//! clock; [`members_known_as_of`] takes both, and refuses to let a membership
//! change reach a study that could not have known it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::identity::InstrumentKey;
use crate::qc::survivorship::Membership;

/// Whether an event admits an instrument or removes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipChange {
    Added,
    Removed,
}

impl MembershipChange {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }
}

/// One append-only membership change.
///
/// `reason` is not decoration. "Removed" covers a delisting, an index demotion, a
/// merger and a data-quality exclusion, and a survivorship audit that cannot tell
/// them apart cannot say whether a universe is biased.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipEvent {
    pub universe: String,
    pub instrument_id: InstrumentKey,
    pub change: MembershipChange,
    /// When the change took effect in the market.
    pub effective: DateTime<Utc>,
    /// When the platform learned of it. Never earlier than it could be known;
    /// commonly *earlier* than `effective` for an announced index change.
    pub recorded_at: DateTime<Utc>,
    pub reason: String,
}

impl MembershipEvent {
    pub fn added(
        universe: impl Into<String>,
        instrument_id: InstrumentKey,
        effective: DateTime<Utc>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            universe: universe.into(),
            instrument_id,
            change: MembershipChange::Added,
            effective,
            recorded_at: effective,
            reason: reason.into(),
        }
    }

    pub fn removed(
        universe: impl Into<String>,
        instrument_id: InstrumentKey,
        effective: DateTime<Utc>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            universe: universe.into(),
            instrument_id,
            change: MembershipChange::Removed,
            effective,
            recorded_at: effective,
            reason: reason.into(),
        }
    }

    /// The same event, learned of at `recorded_at` rather than when it took
    /// effect.
    #[must_use]
    pub fn known_at(mut self, recorded_at: DateTime<Utc>) -> Self {
        self.recorded_at = recorded_at;
        self
    }
}

/// One derived membership interval. `valid_to` is `None` while the instrument is
/// still a member.
///
/// Derived, never stored: see the module docs for why closing an interval in place
/// destroys the answer a previous study got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipInterval {
    pub instrument_id: InstrumentKey,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub added_reason: String,
    pub removed_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UniverseError {
    #[error("the event log mixes universes ({expected} and {found}); resolve one universe at a time so a membership question cannot be answered from another universe's rows")]
    MixedUniverses { expected: String, found: String },
    #[error("instrument {0} is removed from universe {1} at {2} having never been added; a removal with no matching addition means the log is missing rows, and folding it would silently produce a smaller universe than history had")]
    RemovedWithoutAdd(i64, String, DateTime<Utc>),
    #[error("event for instrument {instrument} claims to have been recorded at {recorded_at}, after taking effect at {effective}, but a later event for the same instrument was recorded earlier; the log is not append-only")]
    NotAppendOnly {
        instrument: i64,
        effective: DateTime<Utc>,
        recorded_at: DateTime<Utc>,
    },
}

/// Membership valid at `as_of` on the market clock.
///
/// Folds every event whose `effective` is at or before `as_of`. Events are sorted
/// here rather than assumed sorted: the log is append-only in *recording* order,
/// which is not effective order.
///
/// # Errors
/// [`UniverseError`] when the log is internally inconsistent. A universe that
/// cannot be resolved is refused, never silently returned smaller than history
/// had it.
pub fn members_as_of(
    log: &[MembershipEvent],
    as_of: DateTime<Utc>,
) -> Result<BTreeSet<InstrumentKey>, UniverseError> {
    resolve(log, |e| e.effective <= as_of)
}

/// Membership valid at `as_of` **and knowable by** `knowledge_cutoff`.
///
/// The second clock is what stops an announced-but-not-yet-recorded change from
/// reaching a study. A universe resolved without it is a small lookahead that
/// nobody sees, because the membership looks right — it is only right *later*.
///
/// # Errors
/// As [`members_as_of`].
pub fn members_known_as_of(
    log: &[MembershipEvent],
    as_of: DateTime<Utc>,
    knowledge_cutoff: DateTime<Utc>,
) -> Result<BTreeSet<InstrumentKey>, UniverseError> {
    resolve(log, |e| {
        e.effective <= as_of && e.recorded_at <= knowledge_cutoff
    })
}

fn resolve(
    log: &[MembershipEvent],
    admit: impl Fn(&MembershipEvent) -> bool,
) -> Result<BTreeSet<InstrumentKey>, UniverseError> {
    let name = one_universe(log)?;
    let mut selected: Vec<&MembershipEvent> = log.iter().filter(|e| admit(e)).collect();
    // Effective order, then recording order, then Added before Removed so a
    // same-instant add-and-remove resolves to "not a member" rather than to
    // whichever row the storage layer happened to return first.
    selected.sort_by(|a, b| {
        (a.effective, a.recorded_at, a.change.as_str()).cmp(&(
            b.effective,
            b.recorded_at,
            b.change.as_str(),
        ))
    });
    let mut members = BTreeSet::new();
    for e in selected {
        match e.change {
            MembershipChange::Added => {
                members.insert(e.instrument_id);
            }
            MembershipChange::Removed => {
                if !members.remove(&e.instrument_id) {
                    return Err(UniverseError::RemovedWithoutAdd(
                        e.instrument_id.0,
                        name.clone(),
                        e.effective,
                    ));
                }
            }
        }
    }
    Ok(members)
}

/// The interval read model: one row per membership span, `valid_to` computed.
///
/// # Errors
/// As [`members_as_of`].
pub fn intervals(log: &[MembershipEvent]) -> Result<Vec<MembershipInterval>, UniverseError> {
    let name = one_universe(log)?;
    let mut sorted: Vec<&MembershipEvent> = log.iter().collect();
    sorted.sort_by(|a, b| {
        (a.effective, a.recorded_at, a.change.as_str()).cmp(&(
            b.effective,
            b.recorded_at,
            b.change.as_str(),
        ))
    });
    let mut open: BTreeMap<InstrumentKey, usize> = BTreeMap::new();
    let mut out: Vec<MembershipInterval> = Vec::new();
    for e in sorted {
        match e.change {
            MembershipChange::Added => {
                open.insert(e.instrument_id, out.len());
                out.push(MembershipInterval {
                    instrument_id: e.instrument_id,
                    valid_from: e.effective,
                    valid_to: None,
                    added_reason: e.reason.clone(),
                    removed_reason: None,
                });
            }
            MembershipChange::Removed => {
                let idx = open.remove(&e.instrument_id).ok_or_else(|| {
                    UniverseError::RemovedWithoutAdd(e.instrument_id.0, name.clone(), e.effective)
                })?;
                out[idx].valid_to = Some(e.effective);
                out[idx].removed_reason = Some(e.reason.clone());
            }
        }
    }
    Ok(out)
}

fn one_universe(log: &[MembershipEvent]) -> Result<String, UniverseError> {
    let Some(first) = log.first() else {
        return Ok(String::new());
    };
    for e in log {
        if e.universe != first.universe {
            return Err(UniverseError::MixedUniverses {
                expected: first.universe.clone(),
                found: e.universe.clone(),
            });
        }
    }
    Ok(first.universe.clone())
}

/// Summarise the log for the survivorship check (`qc::survivorship`).
///
/// This is the function that turns P-2.5's answer from
/// `Outcome::InsufficientMetadata` into a real verdict: the check refuses to pass
/// on thin metadata, and until a universe could produce this summary, thin
/// metadata was all there was.
///
/// `window` is the study window. `departed` counts instruments whose membership
/// *ended* inside it — the number whose absence would otherwise be invisible.
///
/// # Errors
/// As [`members_as_of`].
pub fn survivorship_membership(
    log: &[MembershipEvent],
    window: (DateTime<Utc>, DateTime<Utc>),
) -> Result<Membership, UniverseError> {
    let (from, to) = window;
    let spans = intervals(log)?;
    if spans.is_empty() {
        return Ok(Membership::Unknown);
    }
    // Everything that was a member at any point in the window, not just at its
    // end: counting only survivors is the bias itself.
    let members = spans
        .iter()
        .filter(|s| s.valid_from < to && s.valid_to.is_none_or(|end| end > from))
        .count();
    let departed = spans
        .iter()
        .filter(|s| s.valid_to.is_some_and(|end| end > from && end <= to))
        .count();
    let window_days = u32::try_from((to - from).num_days().max(0)).unwrap_or(u32::MAX);
    Ok(Membership::Historical {
        members,
        departed,
        window_days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qc::checks::Outcome;
    use crate::qc::survivorship::check_universe;
    use chrono::TimeZone;

    const U: &str = "crypto_majors";

    fn d(y: i32, m: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, day, 0, 0, 0).unwrap()
    }

    fn i(n: i64) -> InstrumentKey {
        InstrumentKey(n)
    }

    /// BTC from the start; ETH joined in 2024; LUNA joined in 2021 and was
    /// removed in 2022 for going to zero — the instrument whose absence from a
    /// present-day list is the entire survivorship problem.
    fn log() -> Vec<MembershipEvent> {
        vec![
            MembershipEvent::added(U, i(1), d(2020, 1, 1), "founding member: BTC-USD"),
            MembershipEvent::added(U, i(2), d(2021, 3, 1), "listed on two venues: LUNA-USD"),
            MembershipEvent::removed(U, i(2), d(2022, 5, 13), "delisted; price went to zero"),
            MembershipEvent::added(U, i(3), d(2024, 6, 1), "listed on two venues: ETH-USD"),
        ]
    }

    // ── the acceptance clause: a past date is a past answer ──────────────────

    /// P-4.3 clause 1, both halves. Membership as of 2022-01-01 **excludes** the
    /// instrument added in 2024 and **includes** the one removed in 2022 — and
    /// the second half is the one that matters, because it is the member a
    /// present-day list silently drops.
    #[test]
    fn membership_as_of_a_past_date_excludes_later_additions_and_includes_since_removed() {
        let past = members_as_of(&log(), d(2022, 1, 1)).unwrap();
        assert_eq!(past, [i(1), i(2)].into_iter().collect());
        assert!(
            !past.contains(&i(3)),
            "an instrument added in 2024 was not in the 2022 universe"
        );
        assert!(
            past.contains(&i(2)),
            "the instrument that later went to zero WAS in the 2022 universe, and dropping it \
             is survivorship bias"
        );

        let today = members_as_of(&log(), d(2026, 1, 1)).unwrap();
        assert_eq!(today, [i(1), i(3)].into_iter().collect());
        assert_ne!(past, today, "the two answers differ, which is the point");
    }

    /// The boundary is inclusive on the effective instant: a change effective at
    /// midnight is in force at midnight. Off by one here silently shifts every
    /// universe by a day.
    #[test]
    fn a_change_is_in_force_from_the_instant_it_takes_effect() {
        let just_before =
            members_as_of(&log(), d(2021, 3, 1) - chrono::Duration::nanoseconds(1)).unwrap();
        assert!(!just_before.contains(&i(2)));
        let at = members_as_of(&log(), d(2021, 3, 1)).unwrap();
        assert!(at.contains(&i(2)));

        let removal_instant = members_as_of(&log(), d(2022, 5, 13)).unwrap();
        assert!(
            !removal_instant.contains(&i(2)),
            "a removal effective now has already happened"
        );
    }

    /// Before the first event there is no universe, and that is an empty answer
    /// rather than an error: a study window that predates the universe is a
    /// legitimate question with the answer "nothing".
    #[test]
    fn a_date_before_the_first_event_has_no_members() {
        assert!(members_as_of(&log(), d(2019, 1, 1)).unwrap().is_empty());
    }

    // ── the acceptance clause: valid_to is never overwritten ─────────────────

    /// P-4.3 clause 3. A removal appends; nothing is mutated. The log after a
    /// removal still contains the addition, unchanged, so the answer a study got
    /// before the removal was recorded is still reconstructible.
    #[test]
    fn a_removal_appends_a_row_and_leaves_every_earlier_row_untouched() {
        let before = vec![MembershipEvent::added(U, i(7), d(2025, 1, 1), "listed")];
        let mut after = before.clone();
        after.push(MembershipEvent::removed(U, i(7), d(2025, 7, 1), "delisted"));

        assert_eq!(
            after[0], before[0],
            "the addition row is byte-for-byte what it was; a valid_to UPDATE would have \
             changed it"
        );
        assert_eq!(after.len(), before.len() + 1);

        // And the interval read model computes the closure without storing it.
        let spans = intervals(&after).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].valid_from, d(2025, 1, 1));
        assert_eq!(spans[0].valid_to, Some(d(2025, 7, 1)));
        assert_eq!(spans[0].removed_reason.as_deref(), Some("delisted"));

        // The pre-removal answer survives, which is what an UPDATE would destroy.
        assert!(members_as_of(&after, d(2025, 3, 1))
            .unwrap()
            .contains(&i(7)));
        assert!(!members_as_of(&after, d(2025, 8, 1))
            .unwrap()
            .contains(&i(7)));
    }

    /// An instrument can leave and rejoin, and the log holds both spans. An
    /// interval table with one row per `(universe, instrument)` cannot represent
    /// this at all, which is the second reason not to use one.
    #[test]
    fn an_instrument_that_leaves_and_rejoins_has_two_spans() {
        let log = vec![
            MembershipEvent::added(U, i(5), d(2021, 1, 1), "listed"),
            MembershipEvent::removed(U, i(5), d(2022, 1, 1), "volume below floor"),
            MembershipEvent::added(U, i(5), d(2023, 1, 1), "volume recovered"),
        ];
        let spans = intervals(&log).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].valid_to, Some(d(2022, 1, 1)));
        assert_eq!(spans[1].valid_to, None);
        assert!(members_as_of(&log, d(2021, 6, 1)).unwrap().contains(&i(5)));
        assert!(!members_as_of(&log, d(2022, 6, 1)).unwrap().contains(&i(5)));
        assert!(members_as_of(&log, d(2024, 6, 1)).unwrap().contains(&i(5)));
    }

    // ── the two clocks ──────────────────────────────────────────────────────

    /// An index change announced on the 15th and effective on the 20th is
    /// knowable on the 15th. A study cutting off on the 17th may act on it; one
    /// cutting off on the 14th may not, and letting it would be lookahead that
    /// looks like correct membership.
    #[test]
    fn a_change_recorded_after_a_studys_cutoff_does_not_reach_it() {
        let log = vec![
            MembershipEvent::added(U, i(1), d(2025, 1, 1), "founding"),
            MembershipEvent::added(U, i(9), d(2025, 6, 20), "index addition, announced 06-15")
                .known_at(d(2025, 6, 15)),
        ];
        // Effective date passed and the announcement was known: a member.
        let known = members_known_as_of(&log, d(2025, 6, 25), d(2025, 6, 16)).unwrap();
        assert!(known.contains(&i(9)));

        // Effective date passed but the study's knowledge stops before the
        // announcement: NOT a member, because nothing could have known.
        let blind = members_known_as_of(&log, d(2025, 6, 25), d(2025, 6, 14)).unwrap();
        assert!(
            !blind.contains(&i(9)),
            "a study that could not have known of the change must not be given it"
        );

        // The market clock alone would have admitted it.
        assert!(members_as_of(&log, d(2025, 6, 25)).unwrap().contains(&i(9)));
    }

    // ── the survivorship bridge ─────────────────────────────────────────────

    /// The point of building this: `qc::survivorship` has been returning
    /// `InsufficientMetadata` for every universe because nothing could produce a
    /// `Membership::Historical`. This is what produces one, and the check now
    /// reaches a verdict.
    #[test]
    fn a_resolved_log_lets_the_survivorship_check_stop_saying_insufficient() {
        let before = check_universe(&Membership::Unknown);
        assert_eq!(before.outcome, Outcome::InsufficientMetadata);

        let m = survivorship_membership(&log(), (d(2021, 1, 1), d(2023, 1, 1))).unwrap();
        assert_eq!(
            m,
            Membership::Historical {
                members: 2,
                departed: 1,
                window_days: 730
            }
        );
        let after = check_universe(&m);
        assert_eq!(after.outcome, Outcome::Pass);
        assert!(after.detail.contains("1 departed"), "{}", after.detail);
    }

    /// A member counts if it was in the universe at *any* point in the window.
    /// Counting only the ones still there at the end is the bias itself, computed
    /// by the function meant to detect it.
    #[test]
    fn a_member_that_left_mid_window_still_counts_as_a_member_of_it() {
        let m = survivorship_membership(&log(), (d(2022, 1, 1), d(2022, 12, 31))).unwrap();
        match m {
            Membership::Historical {
                members, departed, ..
            } => {
                assert_eq!(members, 2, "BTC all year, LUNA until May");
                assert_eq!(departed, 1);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A long window with no departures is what the survivorship check calls
    /// suspicious, and the bridge must hand it the honest zero rather than
    /// rounding it up to something that passes.
    #[test]
    fn a_long_window_with_no_departures_reaches_the_check_as_zero() {
        let quiet = vec![MembershipEvent::added(U, i(1), d(2015, 1, 1), "founding")];
        let m = survivorship_membership(&quiet, (d(2020, 1, 1), d(2024, 1, 1))).unwrap();
        assert_eq!(
            m,
            Membership::Historical {
                members: 1,
                departed: 0,
                window_days: 1_461
            }
        );
        assert_eq!(
            check_universe(&m).outcome,
            Outcome::Fail,
            "four years with nobody leaving is current membership wearing a date range"
        );
    }

    /// An empty log is `Unknown`, not a historical universe with zero members. The
    /// difference matters: `Historical { members: 0 }` would claim a full history
    /// that happens to be empty, and would pass.
    #[test]
    fn an_empty_log_is_unknown_rather_than_an_empty_history() {
        let m = survivorship_membership(&[], (d(2020, 1, 1), d(2024, 1, 1))).unwrap();
        assert_eq!(m, Membership::Unknown);
        assert_eq!(
            check_universe(&m).outcome,
            Outcome::InsufficientMetadata,
            "no history is not a clean history"
        );
    }

    // ── adversarial ─────────────────────────────────────────────────────────

    /// A removal with no matching addition means rows are missing. Folding it as
    /// a no-op would silently produce a universe smaller than history had, which
    /// is survivorship arriving through a bug rather than through a policy.
    #[test]
    fn a_removal_with_no_addition_is_refused_rather_than_ignored() {
        let broken = vec![MembershipEvent::removed(U, i(4), d(2022, 1, 1), "delisted")];
        let err = members_as_of(&broken, d(2023, 1, 1)).unwrap_err();
        assert!(matches!(err, UniverseError::RemovedWithoutAdd(4, _, _)));
        assert!(err.to_string().contains("smaller universe"));
        assert!(intervals(&broken).is_err());
    }

    /// Two universes in one log is a bug in the caller's query, and answering it
    /// would resolve one universe's membership from another's rows.
    #[test]
    fn a_log_mixing_two_universes_is_refused() {
        let mut mixed = log();
        mixed.push(MembershipEvent::added(
            "equity_large_cap",
            i(50),
            d(2022, 1, 1),
            "listed",
        ));
        assert!(matches!(
            members_as_of(&mixed, d(2023, 1, 1)),
            Err(UniverseError::MixedUniverses { .. })
        ));
    }

    /// The answer must not depend on the order the storage layer returned rows
    /// in. A universe whose membership changes when a query's `ORDER BY` changes
    /// is not a universe.
    #[test]
    fn the_answer_does_not_depend_on_the_order_the_rows_arrived_in() {
        let forward = log();
        let mut reversed = forward.clone();
        reversed.reverse();
        let mut shuffled = forward.clone();
        shuffled.swap(0, 2);
        for other in [reversed, shuffled] {
            assert_eq!(
                members_as_of(&other, d(2022, 1, 1)).unwrap(),
                members_as_of(&forward, d(2022, 1, 1)).unwrap()
            );
            assert_eq!(
                intervals(&other).unwrap().len(),
                intervals(&forward).unwrap().len()
            );
        }
    }

    /// An instrument added and removed at the same instant is not a member. The
    /// alternative is an answer that depends on which row the database returned
    /// first.
    #[test]
    fn an_add_and_a_removal_at_the_same_instant_resolve_to_not_a_member() {
        let log = vec![
            MembershipEvent::added(U, i(8), d(2025, 1, 1), "listed"),
            MembershipEvent::removed(U, i(8), d(2025, 1, 1), "listing withdrawn same day"),
        ];
        assert!(!members_as_of(&log, d(2025, 1, 1)).unwrap().contains(&i(8)));
        assert!(!members_as_of(&log, d(2026, 1, 1)).unwrap().contains(&i(8)));
    }

    #[test]
    fn an_event_round_trips_through_json() {
        let e =
            MembershipEvent::removed(U, i(2), d(2022, 5, 13), "delisted").known_at(d(2022, 5, 10));
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"change\":\"removed\""), "{s}");
        assert_eq!(serde_json::from_str::<MembershipEvent>(&s).unwrap(), e);
    }
}
