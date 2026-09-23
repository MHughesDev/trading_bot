//! Survivorship (DATA-005 §7 and §8, DA-08, P-2.5).
//!
//! Flag when the set being studied was chosen **with knowledge of which members
//! survived**.
//!
//! ## Why this check refuses to pass on thin metadata
//!
//! Survivorship is invisible in the numbers by construction. A universe of
//! currently-listed instruments has no gaps, no bad ticks, perfect coverage and
//! a beautiful Sharpe — because everything that went to zero was removed from it
//! before anyone looked. Every other check in the battery will pass.
//!
//! So a vacuous pass here is worse than no check at all: it would attach
//! evidence of quality to the one defect that quality checks cannot see. When
//! the membership history needed to answer the question is absent, this reports
//! [`Outcome::InsufficientMetadata`] and **never** `Pass`.
//!
//! Until Set P Phase 4 lands the membership history, that is the answer for
//! every universe. That is correct and is not a gap to be worked around.

use serde::{Deserialize, Serialize};

use super::checks::{CheckKind, CheckResult, Outcome};

/// How a universe's membership was established.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Membership {
    /// Full history: every instrument that was ever a member, with the dates it
    /// joined and left. The only shape that can be free of survivorship.
    Historical {
        members: usize,
        /// Members that left the universe during the study window — delisted,
        /// demoted, merged. **A historical universe with none of these over a
        /// long window is suspicious rather than reassuring**, and the check
        /// says so.
        departed: usize,
        window_days: u32,
    },
    /// Membership taken from today's listing. Post-selected by construction.
    CurrentlyListed { members: usize },
    /// No membership record at all.
    Unknown,
}

/// One instrument's own listing status, for the single-instrument case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingStatus {
    Active,
    /// Retained after delisting — the good case. `P-4.4` requires delisted
    /// instruments be retained rather than deleted, and this is what reads it.
    Delisted,
    /// The catalog does not say.
    Unknown,
}

/// Below this, a long-window historical universe with no departures is flagged
/// as suspicious rather than clean.
///
/// A year of equities with zero departures means the history is not really
/// historical. The number is a judgement, written down rather than inlined.
const SUSPICIOUS_WINDOW_DAYS: u32 = 365;

/// Check a universe.
#[must_use]
pub fn check_universe(membership: &Membership) -> CheckResult {
    let (outcome, detail) = match membership {
        Membership::Unknown => (
            Outcome::InsufficientMetadata,
            "no membership history is recorded for this universe, so survivorship cannot be \
             assessed. This is not a pass: survivorship is invisible in the numbers, so every \
             other check passing says nothing about it. Set P Phase 4 supplies the history."
                .to_string(),
        ),
        Membership::CurrentlyListed { members } => (
            Outcome::Fail,
            format!(
                "the universe's {members} members were taken from today's listing, so anything \
                 that delisted during the study window is absent by construction. Any result \
                 over this universe is biased upward by an unknown amount."
            ),
        ),
        Membership::Historical {
            members,
            departed,
            window_days,
        } => {
            if *departed == 0 && *window_days >= SUSPICIOUS_WINDOW_DAYS {
                (
                    Outcome::Fail,
                    format!(
                        "the universe claims a full membership history over {window_days} days \
                         and records zero departures across {members} members. Over a window \
                         that long that is implausible: the history is probably current \
                         membership wearing a date range."
                    ),
                )
            } else {
                (
                    Outcome::Pass,
                    format!(
                        "full membership history: {members} members, {departed} departed during \
                         a {window_days}-day window"
                    ),
                )
            }
        }
    };

    CheckResult {
        kind: CheckKind::Survivorship,
        outcome,
        detail,
        offending: vec![],
        metric: match membership {
            Membership::Historical { departed, .. } => Some(*departed as f64),
            _ => None,
        },
    }
}

/// Check a single instrument.
///
/// Weaker than the universe case and honest about it: one instrument cannot be
/// post-selected relative to itself, but studying only instruments that still
/// exist is survivorship one layer up, and the catalog is the only thing that
/// can say whether delisted instruments are retained at all.
#[must_use]
pub fn check_instrument(status: ListingStatus) -> CheckResult {
    let (outcome, detail) = match status {
        ListingStatus::Unknown => (
            Outcome::InsufficientMetadata,
            "the catalog does not record whether this instrument is still listed, so it cannot \
             be said whether the platform retains delisted instruments at all (P-4.4)."
                .to_string(),
        ),
        ListingStatus::Delisted => (
            Outcome::Pass,
            "this instrument is delisted and retained, which is evidence the catalog does not \
             drop what stops trading"
                .to_string(),
        ),
        ListingStatus::Active => (
            // Not a failure: an active instrument is not itself evidence of
            // survivorship. Not a pass either, because a single active
            // instrument says nothing about whether the catalog retains the
            // dead ones.
            Outcome::NotApplicable,
            "a single active instrument; survivorship is a property of a universe, not of one \
             series that is still trading"
                .to_string(),
        ),
    };

    CheckResult {
        kind: CheckKind::Survivorship,
        outcome,
        detail,
        offending: vec![],
        metric: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the rule this module exists for ──────────────────────────────────────

    /// A vacuous pass here attaches evidence of quality to the one defect
    /// quality checks cannot see.
    #[test]
    fn missing_metadata_reports_insufficient_and_never_pass() {
        let r = check_universe(&Membership::Unknown);
        assert_eq!(r.outcome, Outcome::InsufficientMetadata);
        assert!(!r.outcome.is_pass());
        assert!(r.detail.contains("This is not a pass"), "{}", r.detail);
        assert!(
            r.detail.contains("Phase 4"),
            "it should say where the history comes from"
        );
    }

    #[test]
    fn a_universe_of_currently_listed_members_is_flagged() {
        let r = check_universe(&Membership::CurrentlyListed { members: 500 });
        assert_eq!(r.outcome, Outcome::Fail);
        assert!(r.detail.contains("absent by construction"), "{}", r.detail);
        assert!(r.detail.contains("biased upward"), "{}", r.detail);
    }

    #[test]
    fn a_full_membership_history_with_departures_passes() {
        let r = check_universe(&Membership::Historical {
            members: 500,
            departed: 37,
            window_days: 1_826,
        });
        assert_eq!(r.outcome, Outcome::Pass);
        assert!((r.metric.unwrap() - 37.0).abs() < f64::EPSILON);
    }

    /// A year with zero departures means the history is not really historical.
    #[test]
    fn a_long_history_with_no_departures_is_suspicious_rather_than_clean() {
        let r = check_universe(&Membership::Historical {
            members: 500,
            departed: 0,
            window_days: 730,
        });
        assert_eq!(r.outcome, Outcome::Fail);
        assert!(
            r.detail.contains("current membership wearing a date range"),
            "{}",
            r.detail
        );
    }

    /// Over a short window zero departures is ordinary.
    #[test]
    fn a_short_history_with_no_departures_is_not_suspicious() {
        let r = check_universe(&Membership::Historical {
            members: 500,
            departed: 0,
            window_days: 30,
        });
        assert_eq!(r.outcome, Outcome::Pass);
    }

    #[test]
    fn the_suspicion_boundary_is_a_year() {
        let just_under = check_universe(&Membership::Historical {
            members: 10,
            departed: 0,
            window_days: SUSPICIOUS_WINDOW_DAYS - 1,
        });
        assert_eq!(just_under.outcome, Outcome::Pass);

        let at_limit = check_universe(&Membership::Historical {
            members: 10,
            departed: 0,
            window_days: SUSPICIOUS_WINDOW_DAYS,
        });
        assert_eq!(at_limit.outcome, Outcome::Fail);
    }

    // ── the single-instrument case ───────────────────────────────────────────

    /// P-4.4 requires delisted instruments be retained rather than deleted, and
    /// finding one is evidence the catalog obeys that.
    #[test]
    fn a_retained_delisted_instrument_is_evidence_the_catalog_keeps_the_dead() {
        let r = check_instrument(ListingStatus::Delisted);
        assert_eq!(r.outcome, Outcome::Pass);
        assert!(
            r.detail.contains("does not drop what stops trading"),
            "{}",
            r.detail
        );
    }

    /// An active instrument is neither evidence of survivorship nor evidence
    /// against it.
    #[test]
    fn a_single_active_instrument_is_not_applicable_rather_than_passing() {
        let r = check_instrument(ListingStatus::Active);
        assert_eq!(r.outcome, Outcome::NotApplicable);
        assert!(!r.outcome.is_evidence());
    }

    #[test]
    fn an_unknown_listing_status_reports_insufficient_metadata() {
        let r = check_instrument(ListingStatus::Unknown);
        assert_eq!(r.outcome, Outcome::InsufficientMetadata);
        assert!(r.detail.contains("P-4.4"), "{}", r.detail);
    }

    // ── shape ────────────────────────────────────────────────────────────────

    #[test]
    fn every_result_is_tagged_as_the_survivorship_check_with_a_real_detail() {
        for r in [
            check_universe(&Membership::Unknown),
            check_universe(&Membership::CurrentlyListed { members: 1 }),
            check_universe(&Membership::Historical {
                members: 1,
                departed: 1,
                window_days: 10,
            }),
            check_instrument(ListingStatus::Active),
            check_instrument(ListingStatus::Delisted),
            check_instrument(ListingStatus::Unknown),
        ] {
            assert_eq!(r.kind, CheckKind::Survivorship);
            assert!(
                r.detail.len() > 40,
                "a grade with no reasons is unappealable: {}",
                r.detail
            );
        }
    }

    #[test]
    fn membership_round_trips_through_json() {
        for m in [
            Membership::Unknown,
            Membership::CurrentlyListed { members: 3 },
            Membership::Historical {
                members: 3,
                departed: 1,
                window_days: 90,
            },
        ] {
            let s = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Membership>(&s).unwrap(), m);
        }
    }
}
