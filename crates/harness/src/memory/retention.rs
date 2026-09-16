//! Retention (AGENT-007 §2.4; Set S task S-4.16).
//!
//! # The failure mode this is about
//!
//! A store that only grows does not fail loudly. Retrieval ranks, so the brief
//! keeps filling — with old, narrowly-true findings — and the agent's
//! understanding gets *worse* over time while every dashboard stays green. That
//! is the worst kind of regression: invisible to the thing that watches for
//! regressions.
//!
//! # `archived` is not `deleted`
//!
//! §2.4's last line, and it is load-bearing: **nothing an evidence chain points
//! at is ever hard-deleted.** Archived means out of retrieval and out of the
//! brief, still addressable by id, still resolvable in the desk — so a dossier
//! written in March still resolves its citations in September. It is the same
//! append-only discipline the ledger has, applied to the knowledge that cites it.

use serde::{Deserialize, Serialize};

use super::{days_between, Confidence, Entry, Kind};

/// Where an entry stands today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    /// In retrieval, in the brief.
    Live,
    /// Past `valid_until`. Still retrieved, flagged in the brief, and queued for
    /// re-verification (P7). A stale finding is not a wrong finding — it is a
    /// finding nobody has checked lately, and the difference matters to whoever
    /// reads the brief.
    Stale,
    /// Out of retrieval, still addressable.
    Archived,
}

/// Days a stale, uncited finding waits before archiving (§2.4).
pub const FINDING_ARCHIVE_DAYS: i64 = 180;
/// Days a superseded instrument profile is kept (§2.4).
pub const SUPERSEDED_PROFILE_DAYS: i64 = 90;
/// Days a closed, uncited open question waits (§2.4, AGENT-013 §9).
pub const CLOSED_QUESTION_DAYS: i64 = 180;

/// What today's retention job should do with one entry.
///
/// `last_cited` is the date a retrieval was marked `cited`, or the creation
/// date when it has never been cited. `today` is supplied, not read: this crate
/// has no clock, and a retention rule whose answer depends on when the test runs
/// is a rule nobody can assert.
#[must_use]
pub fn retention(entry: &Entry, today: &str, last_cited: &str) -> Retention {
    // Rule zero, before anything else. A sealed trial ticket or a dossier
    // pointing at this entry means it is part of an evidence chain, and an
    // evidence chain with a hole in it is not an evidence chain.
    if entry.cited_by_chain {
        return if past_validity(entry, today) {
            Retention::Stale
        } else {
            Retention::Live
        };
    }
    if entry.archived {
        return Retention::Archived;
    }

    match entry.kind {
        // Kept indefinitely, by name. §2.4: the cheapest row in the store and
        // the most likely to save a budget.
        Kind::NegativeResult => Retention::Live,
        // Until the user deletes it; and until someone fixes it.
        Kind::UserPreference | Kind::PlatformDefect => Retention::Live,
        Kind::InstrumentProfile => {
            if entry.superseded_by.is_some() {
                match days_between(last_cited, today) {
                    Some(d) if d >= SUPERSEDED_PROFILE_DAYS => Retention::Archived,
                    _ => Retention::Stale,
                }
            } else {
                Retention::Live
            }
        }
        Kind::OpenQuestion => {
            let closed = entry
                .extra
                .get("status")
                .and_then(|s| s.get("closed"))
                .is_some();
            if closed {
                match days_between(last_cited, today) {
                    Some(d) if d >= CLOSED_QUESTION_DAYS => Retention::Archived,
                    _ => Retention::Live,
                }
            } else {
                Retention::Live
            }
        }
        Kind::Finding | Kind::SignalRegistry => {
            if !past_validity(entry, today) {
                return Retention::Live;
            }
            // Stale AND uncited for the window. Both halves are required: a
            // stale finding that people keep citing is a finding the platform
            // should re-verify, not one it should forget.
            match days_between(last_cited, today) {
                Some(d) if d >= FINDING_ARCHIVE_DAYS => Retention::Archived,
                _ => Retention::Stale,
            }
        }
    }
}

fn past_validity(entry: &Entry, today: &str) -> bool {
    match entry.valid_until.as_deref() {
        Some(v) => today > v,
        None => false,
    }
}

/// Apply the decision to the row. Archiving sets a flag; it never removes.
pub fn apply(entry: &mut Entry, decision: Retention) {
    match decision {
        Retention::Live => {
            entry.stale = false;
            entry.archived = false;
        }
        Retention::Stale => {
            entry.stale = true;
            entry.archived = false;
        }
        Retention::Archived => {
            entry.stale = true;
            entry.archived = true;
        }
    }
}

/// The monthly report (§2.4's last paragraph): store size, archive rate and the
/// share of retrievals that were cited.
///
/// A degrading brief becomes a number before it becomes bad research, which is
/// the only point at which anyone can act on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreReport {
    pub total: usize,
    pub live: usize,
    pub stale: usize,
    pub archived: usize,
    pub by_kind: std::collections::BTreeMap<String, usize>,
    /// Archived ÷ total.
    pub archive_rate: f64,
    /// `memory_retrievals` rows with `cited = true` ÷ all rows (AGENT-007 §4).
    pub cited_share: f64,
    pub established: usize,
    pub suggestive: usize,
}

/// Build the report from the store and the retrieval log's two counts.
#[must_use]
pub fn store_report(entries: &[Entry], retrievals: usize, cited: usize) -> StoreReport {
    let mut r = StoreReport {
        total: entries.len(),
        ..StoreReport::default()
    };
    for e in entries {
        *r.by_kind.entry(e.kind.as_str().to_string()).or_default() += 1;
        if e.archived {
            r.archived += 1;
        } else if e.stale {
            r.stale += 1;
        } else {
            r.live += 1;
        }
        match e.confidence {
            Confidence::Established => r.established += 1,
            Confidence::Suggestive => r.suggestive += 1,
            Confidence::Refuted => {}
        }
    }
    r.archive_rate = if r.total == 0 {
        0.0
    } else {
        r.archived as f64 / r.total as f64
    };
    r.cited_share = if retrievals == 0 {
        0.0
    } else {
        cited as f64 / retrievals as f64
    };
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DataWindow, Entry, Provenance};

    fn f(id: &str) -> Entry {
        Entry::finding(
            id,
            "BTC-USD",
            "x",
            DataWindow::new("2026-01-01", "2026-06-30"),
            Provenance::new("run_1", "t", "v", "h"),
        )
        .expiring()
    }

    #[test]
    fn a_fresh_finding_is_live_and_an_expired_one_is_stale() {
        let e = f("f_1");
        assert_eq!(e.valid_until.as_deref(), Some("2026-09-28"));
        assert_eq!(retention(&e, "2026-09-01", "2026-06-30"), Retention::Live);
        assert_eq!(retention(&e, "2026-10-01", "2026-06-30"), Retention::Stale);
    }

    /// S-4.16's acceptance: an archived finding leaves the brief but still
    /// resolves by id.
    #[test]
    fn a_stale_and_uncited_finding_archives_after_a_hundred_and_eighty_days() {
        let mut e = f("f_1");
        // Stale since 2026-09-28; last cited on its creation date.
        assert_eq!(retention(&e, "2027-01-01", "2026-06-30"), Retention::Archived);
        apply(&mut e, Retention::Archived);
        assert!(e.archived, "out of retrieval");
        assert_eq!(e.id, "f_1", "still addressable by id");
        assert!(!e.claim.is_empty(), "nothing is removed, only flagged");
    }

    /// Both halves are required. A stale finding people keep citing is one to
    /// re-verify, not one to forget.
    #[test]
    fn a_stale_finding_that_is_still_cited_does_not_archive() {
        let e = f("f_1");
        assert_eq!(retention(&e, "2027-01-01", "2026-12-20"), Retention::Stale);
    }

    /// The rule that outranks the timer.
    #[test]
    fn nothing_an_evidence_chain_points_at_is_ever_archived() {
        let mut e = f("f_1");
        e.cited_by_chain = true;
        assert_eq!(
            retention(&e, "2030-01-01", "2026-06-30"),
            Retention::Stale,
            "a sealed ticket's citation must still resolve in 2030"
        );
    }

    #[test]
    fn negative_results_and_preferences_are_kept_indefinitely() {
        let mut n = f("n_1");
        n.kind = crate::memory::Kind::NegativeResult;
        assert_eq!(retention(&n, "2099-01-01", "2026-01-01"), Retention::Live);
        let mut p = f("p_1");
        p.kind = crate::memory::Kind::UserPreference;
        assert_eq!(retention(&p, "2099-01-01", "2026-01-01"), Retention::Live);
    }

    #[test]
    fn a_superseded_profile_is_kept_ninety_days_and_the_latest_is_kept() {
        let mut latest = f("p_now");
        latest.kind = crate::memory::Kind::InstrumentProfile;
        assert_eq!(
            retention(&latest, "2099-01-01", "2026-01-01"),
            Retention::Live
        );
        let mut old = latest.clone();
        old.id = "p_old".into();
        old.superseded_by = Some("p_now".into());
        assert_eq!(retention(&old, "2026-08-01", "2026-06-30"), Retention::Stale);
        assert_eq!(
            retention(&old, "2026-12-01", "2026-06-30"),
            Retention::Archived
        );
    }

    #[test]
    fn the_monthly_report_gives_size_archive_rate_and_cited_share() {
        let mut a = f("f_1");
        let mut b = f("f_2");
        apply(&mut a, Retention::Archived);
        apply(&mut b, Retention::Stale);
        let r = store_report(&[a, b, f("f_3")], 10, 4);
        assert_eq!((r.total, r.archived, r.stale, r.live), (3, 1, 1, 1));
        assert!((r.archive_rate - 1.0 / 3.0).abs() < 1e-9);
        assert!((r.cited_share - 0.4).abs() < 1e-9);
        assert_eq!(r.by_kind["finding"], 3);
    }
}
