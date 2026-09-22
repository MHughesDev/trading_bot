//! Source 4.4: a negative result whose retry condition has come true
//! (AGENT-014 §4.4).
//!
//! # What this subscription ran into, and why it is shaped like this
//!
//! §4.4 says: *"a `negative_result` in scope whose `retry_condition` now
//! evaluates true (a capability, a data window, an adapter)"*. On this platform
//! `retry_condition` is **an English sentence**. It is written by
//! `harness::memory::negative::FailureClass::retry_condition`, which returns
//! strings like *"retry when the data window extends beyond 2026-01-31"* and
//! *"retry if a venue with lower effective cost is added, or the holding period
//! lengthens"*, and it is stored in `extra.retry_condition`. Nothing on this
//! platform can evaluate a sentence.
//!
//! What **is** structured is the failure class it was derived from, and the
//! class is the only input the sentence had. So this module goes back to the
//! class and asks what kind of change each one is waiting for. The answer is
//! uncomfortable and is the finding worth carrying out of this task:
//!
//! | class | the condition names | an operand on the row? | a measurement on the platform? |
//! |---|---|---|---|
//! | `insufficient_data` | the data window extending past the one it was measured on | **yes** — the window's end | **yes** — data coverage |
//! | `capacity_insufficient` | an AUM target at or below the collapse point | no — the detail is prose | no |
//! | `costs_dominate` | a cheaper venue, or a longer holding period | no | no |
//! | `regime_specific` | the supporting regime recurring | no — no regime is recorded | no |
//! | `protocol_unstable` | the protocol becoming deterministic | no | no |
//! | `leakage` | the leak removed and the feature re-derived | no | no |
//! | the other four | *nothing* — there is no retry condition | — | — |
//!
//! **One of six is evaluable.** The other five are not built as guesses and are
//! not built as `false` either: they answer [`Fired::NotEvaluable`], which puts
//! a line on the log naming what is missing. A session carrying five rules it
//! can never evaluate should say so, every iteration, until somebody either
//! records the operand or amends the spec. Silently answering "no" is how a
//! subscription comes to look like it is working when nothing has ever asked it
//! anything.
//!
//! The four classes with no retry condition answer a measured [`Fired::No`],
//! not `NotEvaluable`, and the difference is deliberate. A `weak_effect` has no
//! retry condition because there is nothing that could change to make a
//! measured-small effect large — that is a decision the platform made, with a
//! reason written down beside it, and reporting it as an unanswerable question
//! would lose it.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::{admitted, offer_all, Fired, Ingest};
use crate::agenda::{AgendaItem, Source};
use crate::charter::Charter;
use crate::harvest::{SkipReason, Skipped};
use crate::sources::memory::session_of;

/// The failure-class words this module knows.
///
/// The one definition of the vocabulary *in this crate*. The platform's own is
/// `harness::memory::negative::FailureClass::all()`, and these two must stay
/// equal: a class word this module does not recognise answers
/// [`Fired::NotEvaluable`] rather than being treated as one with no retry
/// condition, so a drift here is loud rather than silent — but it is still a
/// drift, and it wants an invariant test that walks
/// `FailureClass::all()` and looks each word up in [`RetryTrigger::for_class`].
/// That test belongs in `crates/invariants`, which is not this crate;
/// [`the_vocabulary_is_closed_and_every_word_has_a_trigger`] holds the half
/// that can be held from here.
pub const FAILURE_CLASSES: &[&str] = &[
    "weak_effect",
    "capacity_insufficient",
    "costs_dominate",
    "replication_failed",
    "insufficient_data",
    "no_better_than_null",
    "leakage",
    "protocol_unstable",
    "fragile_parameters",
    "regime_specific",
];

/// What would have to change in the world for a dead idea to be worth another
/// look, in the only form a machine can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryTrigger {
    /// `insufficient_data` — the data now has to reach past the window the
    /// negative was measured over. The one trigger with an operand on the row
    /// and a measurement to compare it to.
    DataExtendsBeyond { date: String },
    /// The class names a change, and this platform records nothing that would
    /// answer whether it happened.
    ///
    /// `wants` is the change, in the words of the class's own retry condition,
    /// so the log line and the digest say what is missing rather than that
    /// something is.
    Unobservable {
        class: &'static str,
        wants: &'static str,
    },
    /// The class has no retry condition. Not an absence of information — a
    /// decision, recorded in `FailureClass::retry_condition`'s own comment.
    Never { class: &'static str },
    /// A class word neither this module nor, presumably, the funnel knows.
    UnknownClass { got: String },
}

impl RetryTrigger {
    /// The trigger a failure class implies.
    ///
    /// Derived from the class rather than parsed out of `extra.retry_condition`
    /// because the class is what produced that sentence in the first place, and
    /// because a rule that reads back a sentence it generated is a rule that
    /// breaks the day somebody improves the wording.
    #[must_use]
    pub fn for_class(class: &str, window_end: Option<&str>) -> Self {
        match class {
            "insufficient_data" => match window_end {
                Some(d) => RetryTrigger::DataExtendsBeyond {
                    date: d.to_string(),
                },
                // The class says "retry when the data window extends beyond
                // {window.end}" and the row has no window end. There is nothing
                // to extend beyond, so there is nothing to compare.
                None => RetryTrigger::Unobservable {
                    class: "insufficient_data",
                    wants: "the data window this was measured over, which the row does not carry",
                },
            },
            "capacity_insufficient" => RetryTrigger::Unobservable {
                class: "capacity_insufficient",
                wants: "an AUM target at or below the collapse point; the collapse point is \
                        recorded as prose in the negative's detail, not as a number",
            },
            "costs_dominate" => RetryTrigger::Unobservable {
                class: "costs_dominate",
                wants: "a venue with a lower effective cost, or a longer holding period; no \
                        per-negative cost baseline is recorded to compare a new venue against",
            },
            "regime_specific" => RetryTrigger::Unobservable {
                class: "regime_specific",
                wants: "the supporting regime recurring; no regime is recorded on an instrument \
                        profile, so there is no label to watch",
            },
            "protocol_unstable" => RetryTrigger::Unobservable {
                class: "protocol_unstable",
                wants: "the protocol becoming deterministic across reruns; nothing records \
                        protocol determinism",
            },
            "leakage" => RetryTrigger::Unobservable {
                class: "leakage",
                wants: "the leak removed and the feature re-derived; nothing links a negative to \
                        the feature whose leak produced it",
            },
            "weak_effect" => RetryTrigger::Never {
                class: "weak_effect",
            },
            "replication_failed" => RetryTrigger::Never {
                class: "replication_failed",
            },
            "no_better_than_null" => RetryTrigger::Never {
                class: "no_better_than_null",
            },
            "fragile_parameters" => RetryTrigger::Never {
                class: "fragile_parameters",
            },
            other => RetryTrigger::UnknownClass {
                got: other.to_string(),
            },
        }
    }

    /// Whether this trigger has fired, given what the world was measured to be.
    ///
    /// # Absence
    ///
    /// A subject with no coverage measurement answers
    /// [`Fired::NotEvaluable`], **not** [`Fired::No`]. Reading a missing
    /// measurement as a date of zero — or as "the window has not extended" —
    /// would make every one of these conditions permanently false the moment a
    /// reader was not looking.
    #[must_use]
    pub fn evaluate(&self, subject: &str, measured: &Measured) -> Fired {
        match self {
            RetryTrigger::DataExtendsBeyond { date } => {
                let Some(now) = measured.data_coverage_end.get(subject) else {
                    return Fired::NotEvaluable {
                        missing: format!("how far the data for `{subject}` now reaches"),
                    };
                };
                // Parsed rather than compared as text. ISO-8601 dates do order
                // lexicographically, but only when both are well formed, and a
                // malformed one would compare wrong rather than fail.
                let (Ok(then), Ok(now_d)) = (
                    NaiveDate::parse_from_str(date, "%Y-%m-%d"),
                    NaiveDate::parse_from_str(now, "%Y-%m-%d"),
                ) else {
                    return Fired::NotEvaluable {
                        missing: format!(
                            "a comparable pair of dates for `{subject}`: the negative was measured \
                             to `{date}` and the data is recorded as reaching `{now}`"
                        ),
                    };
                };
                if now_d > then {
                    Fired::Yes
                } else {
                    Fired::No
                }
            }
            RetryTrigger::Unobservable { class, wants } => Fired::NotEvaluable {
                missing: format!("`{class}` waits for {wants}"),
            },
            // Measured, not missing: the platform decided this class has no
            // retry condition, and the decision has a reason beside it.
            RetryTrigger::Never { .. } => Fired::No,
            RetryTrigger::UnknownClass { got } => Fired::NotEvaluable {
                missing: format!(
                    "`{got}` is not a failure class this platform files; the classes are {}",
                    FAILURE_CLASSES.join(", ")
                ),
            },
        }
    }
}

/// A negative result, as source 4.4 reads it.
///
/// `retry_condition` — the sentence — is deliberately **not** a field. Carrying
/// it would invite a reader to think this module evaluates it, and the module
/// header is the whole reason it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegativeRow {
    pub id: String,
    pub subject: String,
    pub claim: String,
    /// `extra.failure_class`, as the funnel filed it.
    pub failure_class: String,
    /// The end of the data window the negative was measured over
    /// (`data_window.end`).
    pub window_end: Option<String>,
}

impl NegativeRow {
    #[must_use]
    pub fn new(id: &str, subject: &str, claim: &str, failure_class: &str) -> Self {
        Self {
            id: id.to_string(),
            subject: subject.to_string(),
            claim: claim.to_string(),
            failure_class: failure_class.to_string(),
            window_end: None,
        }
    }

    #[must_use]
    pub fn measured_to(mut self, date: &str) -> Self {
        self.window_end = Some(date.to_string());
        self
    }

    #[must_use]
    pub fn trigger(&self) -> RetryTrigger {
        RetryTrigger::for_class(&self.failure_class, self.window_end.as_deref())
    }

    /// The question a retry asks.
    ///
    /// It names the class it died of, because a run that does not know why the
    /// idea was dropped will drop it the same way again — and because
    /// `circling` (§7.1) is the rule that catches a session doing exactly that.
    #[must_use]
    pub fn question(&self) -> String {
        format!(
            "retry, now that the condition it was shelved under has changed — it was refused as \
             `{}`: {}",
            self.failure_class, self.claim
        )
    }
}

/// What the world measures, for the conditions that have something to compare
/// against.
///
/// One map today, because one of the six triggers is evaluable. It is a struct
/// rather than that map so that the day a regime label or a venue cost table
/// exists, the new field lands beside this one and
/// [`RetryTrigger::for_class`]'s `Unobservable` arm becomes a real comparison
/// without every caller changing shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Measured {
    /// Subject → the ISO date the platform's data for it now reaches.
    ///
    /// A subject **absent** from this map is a subject nothing measured, and
    /// every rule reading it treats absence as unanswerable rather than as a
    /// window that has not moved.
    pub data_coverage_end: BTreeMap<String, String>,
}

impl Measured {
    #[must_use]
    pub fn with_coverage(mut self, subject: &str, to: &str) -> Self {
        self.data_coverage_end
            .insert(subject.to_string(), to.to_string());
        self
    }
}

/// Source 4.4 — negatives whose retry condition has come true.
///
/// §4.4's workflow is *"`investigate`, as variant"*. The `investigate` half is
/// [`Source::RetryConditionMet`]'s default workflow. The "as variant" half is
/// not a field on [`AgendaItem`] and is not invented as one: it is what the
/// intake precheck decides when the child is about to start, and it reaches
/// [`crate::stall`] as [`crate::stall::Precheck`] on the finished run, which is
/// what the `circling` rule keys on. An item from this source is by
/// construction a second look at a question already answered once, so the
/// source *is* the marker.
#[must_use]
pub fn retry_conditions(
    charter: &Charter,
    rows: &[NegativeRow],
    measured: &Measured,
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    let mut candidates = Vec::new();
    for r in rows {
        if r.claim.trim().is_empty() {
            out.skipped.push(Skipped {
                what: r.id.clone(),
                why: SkipReason::Blank,
            });
            continue;
        }
        if let Err(s) = admitted(charter, &r.subject, &r.id) {
            out.skipped.push(s);
            continue;
        }
        match r.trigger().evaluate(&r.subject, measured) {
            Fired::Yes => candidates.push(
                AgendaItem::new(
                    session_of(agenda),
                    &r.question(),
                    &r.subject,
                    Source::RetryConditionMet,
                )
                .with_source_ref(&r.id),
            ),
            // Measured and false: the ordinary case, and not worth a line. A
            // session with two hundred live negatives would otherwise write two
            // hundred notes an iteration.
            Fired::No => {}
            Fired::NotEvaluable { missing } => out.skipped.push(Skipped {
                what: format!("{}: {missing}", r.id),
                why: SkipReason::NotEvaluable,
            }),
        }
    }
    out.inserted = offer_all(agenda, candidates);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::ItemStatus;
    use crate::sources::tests::{charter_over, session};

    fn seeded() -> Vec<AgendaItem> {
        vec![AgendaItem {
            status: ItemStatus::Done {
                verdict: "accept".into(),
            },
            ..AgendaItem::new(session(), "something settled", "", Source::User)
        }]
    }

    #[test]
    fn the_vocabulary_is_closed_and_every_word_has_a_trigger() {
        // Half of the invariant. The other half — that this list equals
        // `harness::memory::negative::FailureClass::all()` — cannot be asserted
        // from this crate and is named in `FAILURE_CLASSES`'s doc comment.
        for c in FAILURE_CLASSES {
            let t = RetryTrigger::for_class(c, Some("2026-01-01"));
            assert!(
                !matches!(t, RetryTrigger::UnknownClass { .. }),
                "`{c}` is in the vocabulary and has no trigger"
            );
        }
    }

    #[test]
    fn exactly_one_class_in_ten_can_actually_be_evaluated() {
        // The finding, as a test, so that it fails the day somebody records the
        // operand that would make a second one evaluable — which is the day
        // this module's header stops being true.
        let evaluable: Vec<&&str> = FAILURE_CLASSES
            .iter()
            .filter(|c| {
                matches!(
                    RetryTrigger::for_class(c, Some("2026-01-01")),
                    RetryTrigger::DataExtendsBeyond { .. }
                )
            })
            .collect();
        assert_eq!(
            evaluable,
            vec![&"insufficient_data"],
            "if this changed, the module header's table changed with it"
        );
    }

    #[test]
    fn four_classes_have_no_retry_condition_and_that_is_a_measured_no() {
        for c in [
            "weak_effect",
            "replication_failed",
            "no_better_than_null",
            "fragile_parameters",
        ] {
            let fired = RetryTrigger::for_class(c, None).evaluate("btc", &Measured::default());
            assert_eq!(
                fired,
                Fired::No,
                "`{c}` has no retry condition, which is a decision and not an absence"
            );
        }
    }

    #[test]
    fn an_unmeasured_subject_is_not_evaluable_rather_than_false() {
        // The whole of the rule this crate is built on: `0 ?? x` is `0`.
        let t = RetryTrigger::for_class("insufficient_data", Some("2026-01-31"));
        let fired = t.evaluate("btc", &Measured::default());
        assert!(matches!(fired, Fired::NotEvaluable { .. }), "{fired:?}");
        assert_ne!(fired, Fired::No);
    }

    #[test]
    fn the_window_extending_fires_and_the_window_standing_still_does_not() {
        let t = RetryTrigger::for_class("insufficient_data", Some("2026-01-31"));
        let later = Measured::default().with_coverage("btc", "2026-03-01");
        let same = Measured::default().with_coverage("btc", "2026-01-31");
        let earlier = Measured::default().with_coverage("btc", "2025-12-01");
        assert_eq!(t.evaluate("btc", &later), Fired::Yes);
        assert_eq!(t.evaluate("btc", &same), Fired::No, "equal is not beyond");
        assert_eq!(t.evaluate("btc", &earlier), Fired::No);
    }

    #[test]
    fn a_malformed_date_is_refused_rather_than_compared_as_text() {
        let t = RetryTrigger::for_class("insufficient_data", Some("31/01/2026"));
        let m = Measured::default().with_coverage("btc", "2026-03-01");
        assert!(
            matches!(t.evaluate("btc", &m), Fired::NotEvaluable { .. }),
            "`31/01/2026` sorts before `2026-03-01` as text and would have fired"
        );
    }

    #[test]
    fn what_is_missing_is_named_rather_than_reported_as_missing() {
        let Fired::NotEvaluable { missing } =
            RetryTrigger::for_class("regime_specific", None).evaluate("btc", &Measured::default())
        else {
            panic!("regime_specific has no operand on this platform");
        };
        assert!(missing.contains("regime"), "{missing}");
        assert!(
            missing.contains("instrument profile"),
            "it should name where the operand would have to live: {missing}"
        );
    }

    #[test]
    fn an_unknown_class_is_not_evaluable_and_lists_the_ones_that_are() {
        let Fired::NotEvaluable { missing } = RetryTrigger::for_class("vibes", None)
            .evaluate("btc", &Measured::default())
        else {
            panic!("an unknown class cannot be evaluated");
        };
        assert!(missing.contains("vibes"));
        assert!(missing.contains("insufficient_data"));
    }

    #[test]
    fn a_fired_condition_inserts_an_investigate_naming_the_class_it_died_of() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded();
        let out = retry_conditions(
            &charter,
            &[NegativeRow::new(
                "n_1",
                "btc",
                "momentum at 1h",
                "insufficient_data",
            )
            .measured_to("2026-01-31")],
            &Measured::default().with_coverage("btc", "2026-03-01"),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 2);
        assert_eq!(agenda[1].workflow, "investigate", "§4.4's default");
        assert_eq!(agenda[1].sources, vec![Source::RetryConditionMet]);
        assert_eq!(agenda[1].source_refs, vec!["n_1".to_string()]);
        assert!(
            agenda[1].question.contains("insufficient_data"),
            "a run that does not know why the idea was dropped drops it the same \
             way again: {}",
            agenda[1].question
        );
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn a_measured_no_writes_no_line_and_an_unanswerable_one_does() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded();
        let out = retry_conditions(
            &charter,
            &[
                // measured false
                NegativeRow::new("n_no", "btc", "a", "weak_effect"),
                // unanswerable
                NegativeRow::new("n_huh", "btc", "b", "costs_dominate"),
            ],
            &Measured::default(),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 1, "neither inserts");
        assert_eq!(out.skipped.len(), 1, "{:?}", out.skipped);
        assert!(out.skipped[0].what.starts_with("n_huh"));
        assert_eq!(out.skipped[0].why, SkipReason::NotEvaluable);
    }

    #[test]
    fn a_retry_is_weighed_above_an_ordinary_open_question() {
        // §5.1's table: retry_condition_met 1.3, open_question 1.0. A condition
        // the world just satisfied is a cheaper look than a fresh one.
        assert!(Source::RetryConditionMet.weight() > Source::OpenQuestion.weight());
    }
}
