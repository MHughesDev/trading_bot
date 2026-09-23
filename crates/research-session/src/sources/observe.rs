//! Source 4.7: the subscription that turns market observations into experiments
//! (AGENT-014 §4.7).
//!
//! Three triggers, one rule. A regime flip on an instrument profile and new data
//! coverage in scope become `investigate`; a monitor `divergence` or
//! `scaled_down` on a strategy in scope becomes `diagnose`. §4.7's own
//! constraint is the interesting half: *"rate-limited per subject (one item per
//! regime flip, not one per profiler run)"*.
//!
//! # A rate limit is about the event, not about the reading
//!
//! The profiler runs weekly. If it reports "this instrument is choppy and was
//! trending" on three consecutive runs, that is **one** flip observed three
//! times, and an agenda that grows a card each time is an agenda a human stops
//! reading. Layer one catches the repeat only while the first item is still
//! non-terminal — once the run finishes and the item is `done`, the next
//! profiler run inserts the question again, forever.
//!
//! So the limit is a [`Raised`] set of fingerprints, and every
//! [`Observation`] has to be able to name **the event** rather than the moment
//! it was read. [`Observation::RegimeFlip::since`] is that field, and it is
//! documented, tested and named `since` rather than `at` precisely because a
//! caller passing the read time turns the limit back into one item per profiler
//! run — the exact thing §4.7 rules out, reintroduced by a field name.
//!
//! # What does not exist yet, named rather than faked
//!
//! * **`divergence` is not a monitor event kind.** `monitor::events::EventKind`
//!   is `blip | warning | breach | scaled_down | scale_released | halted |
//!   resumed | tripped | reset | window_opened | window_closed | drill`. §4.7
//!   names two kinds and only `scaled_down` can arrive. [`MONITOR_KINDS`] holds
//!   both words anyway, so the day the kind exists this module already accepts
//!   it, and [`the_kinds_this_watches_are_the_two_the_spec_names`] is what fails
//!   if somebody quietly drops the one that does not.
//! * **No regime is recorded on an instrument profile.** `Kind::InstrumentProfile`
//!   exists and `MemoryStore::supersede` will keep a succession of them, but
//!   nothing writes one — no scheduler enqueues the profiler (S-4.7 records
//!   this) — and no field named `regime` appears anywhere in `crates/`. So
//!   [`Observation::RegimeFlip`] has no producer today. The rule is built and
//!   the producer is named.

use std::collections::BTreeSet;

use super::{admitted, offer_all, Ingest};
use crate::agenda::{AgendaItem, Source};
use crate::charter::Charter;
use crate::harvest::{SkipReason, Skipped};
use crate::sources::memory::session_of;

/// The monitor event kinds §4.7 subscribes to.
///
/// `divergence` is here and is not a kind the monitor can emit; see the module
/// header. Written as the spec names them so the gap is visible in one place
/// rather than being absent in a `match` arm nobody notices.
pub const MONITOR_KINDS: &[&str] = &["divergence", "scaled_down"];

/// Something that happened to the world, which the session may want to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// An instrument profile's regime changed (AGENT-007 §3.3).
    RegimeFlip {
        subject: String,
        from: String,
        to: String,
        /// **When the regime changed**, not when the profiler read it.
        ///
        /// This is what the rate limit keys on. A caller that passes the
        /// profiler's run time makes every weekly run a fresh flip, which is
        /// the behaviour §4.7 rules out in its own parenthesis.
        since: String,
    },
    /// Data the session did not have before.
    CoverageExtended {
        subject: String,
        /// The ISO date coverage now reaches.
        to: String,
    },
    /// A monitor event on a strategy (COMP-007 §6).
    MonitorEvent {
        /// The strategy the event was about, which is the subject the charter
        /// scope is checked against.
        subject: String,
        /// One of [`MONITOR_KINDS`].
        kind: String,
        /// `monitor.events.event_id` — already unique per event, so nothing has
        /// to be invented to tell two apart.
        event_id: String,
        /// For the item's sentence: which metric, or the state it moved to.
        detail: String,
    },
}

impl Observation {
    /// The subject the charter's scope is checked against.
    #[must_use]
    pub fn subject(&self) -> &str {
        match self {
            Observation::RegimeFlip { subject, .. }
            | Observation::CoverageExtended { subject, .. }
            | Observation::MonitorEvent { subject, .. } => subject,
        }
    }

    /// The identity of the **event**, which is what the session raises at most
    /// one item for.
    ///
    /// Also the item's `source_ref`, which is what makes [`Raised`] derivable
    /// from `agenda_items.source_refs` rather than being a fourth table.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        match self {
            Observation::RegimeFlip {
                subject,
                from,
                to,
                since,
            } => format!("regime:{subject}:{from}->{to}@{since}"),
            Observation::CoverageExtended { subject, to } => format!("coverage:{subject}:{to}"),
            // Not keyed on the subject and the kind: two `scaled_down` events a
            // week apart on one strategy are two things that happened, and
            // collapsing them would lose the second.
            Observation::MonitorEvent { event_id, .. } => format!("monitor:{event_id}"),
        }
    }

    /// The workflow §4.7's table asks for.
    ///
    /// `diagnose` for monitor events — something deployed is behaving in a way
    /// nobody predicted, and that is a question about *why* — and `investigate`
    /// for a regime flip or new coverage, which are openings rather than faults.
    #[must_use]
    pub fn workflow(&self) -> &'static str {
        match self {
            Observation::MonitorEvent { .. } => "diagnose",
            Observation::RegimeFlip { .. } | Observation::CoverageExtended { .. } => "investigate",
        }
    }

    /// The question the item asks.
    #[must_use]
    pub fn question(&self) -> String {
        match self {
            Observation::RegimeFlip {
                subject,
                from,
                to,
                since,
            } => format!(
                "what changed for {subject} when the regime moved from {from} to {to} on {since}, \
                 and does anything established under {from} still hold"
            ),
            Observation::CoverageExtended { subject, to } => format!(
                "what can be asked about {subject} now that data reaches {to} that could not be \
                 asked before"
            ),
            Observation::MonitorEvent {
                subject,
                kind,
                detail,
                ..
            } => format!("why did {subject} go {kind} in production: {detail}"),
        }
    }

    /// Whether this is an event kind the session subscribes to.
    ///
    /// Only monitor events are filtered — a regime flip and new coverage are
    /// the whole of their own triggers. A monitor event of any other kind is a
    /// reading the session was never asked to react to, and a `blip` in
    /// particular is one COMP-007 §4.1 says is *acted on by nothing*.
    #[must_use]
    pub fn is_subscribed(&self) -> bool {
        match self {
            Observation::MonitorEvent { kind, .. } => MONITOR_KINDS.contains(&kind.as_str()),
            Observation::RegimeFlip { .. } | Observation::CoverageExtended { .. } => true,
        }
    }
}

/// The event fingerprints this session has already raised an item for.
///
/// **Derived, not a fourth table.** Every item this module creates carries its
/// observation's fingerprint as a `source_ref`, so the driver builds this with
/// `SELECT source_refs FROM agenda_items WHERE session_id = $1` — including
/// terminal items, which is the whole point: a `done` item is exactly the one
/// layer one stops protecting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Raised(BTreeSet<String>);

impl Raised {
    /// Build from whatever the driver read out of `agenda_items.source_refs`.
    #[must_use]
    pub fn from_refs<I, S>(refs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(refs.into_iter().map(Into::into).collect())
    }

    #[must_use]
    pub fn contains(&self, fingerprint: &str) -> bool {
        self.0.contains(fingerprint)
    }

    /// Record a fingerprint, so a batch containing the same event twice raises
    /// one item rather than two.
    pub fn record(&mut self, fingerprint: &str) {
        self.0.insert(fingerprint.to_string());
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Source 4.7 — observations in scope become items, at most one per event.
#[must_use]
pub fn observations(
    charter: &Charter,
    seen: &[Observation],
    already: &Raised,
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    // A copy, so a batch carrying the same event twice — which two overlapping
    // reads of `monitor.events` will do — raises one item, and so that a caller
    // is never handed a `Raised` this function mutated behind its back.
    let mut already = already.clone();
    let mut candidates = Vec::new();

    for o in seen {
        let fp = o.fingerprint();
        if !o.is_subscribed() {
            out.skipped.push(Skipped {
                what: fp,
                why: SkipReason::NotSubscribed,
            });
            continue;
        }
        if already.contains(&fp) {
            out.skipped.push(Skipped {
                what: fp,
                why: SkipReason::AlreadyRaised,
            });
            continue;
        }
        if let Err(s) = admitted(charter, o.subject(), &fp) {
            out.skipped.push(s);
            continue;
        }
        already.record(&fp);
        candidates.push(
            AgendaItem::new(
                session_of(agenda),
                &o.question(),
                o.subject(),
                Source::Observation,
            )
            .with_workflow(o.workflow())
            .with_source_ref(&fp),
        );
    }
    out.inserted = offer_all(agenda, candidates);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::ItemStatus;
    use crate::sources::tests::{charter_over, session};

    fn flip(since: &str) -> Observation {
        Observation::RegimeFlip {
            subject: "btc".into(),
            from: "trending".into(),
            to: "choppy".into(),
            since: since.into(),
        }
    }

    fn empty_agenda() -> Vec<AgendaItem> {
        vec![AgendaItem {
            status: ItemStatus::Done {
                verdict: "accept".into(),
            },
            ..AgendaItem::new(session(), "something settled", "", Source::User)
        }]
    }

    #[test]
    fn the_kinds_this_watches_are_the_two_the_spec_names() {
        // `divergence` is not a kind `monitor::events::EventKind` can emit. It
        // is kept here because §4.7 names it, and this test is what stops it
        // being quietly dropped to match what the monitor happens to send.
        assert_eq!(MONITOR_KINDS, &["divergence", "scaled_down"]);
    }

    #[test]
    fn a_blip_is_not_something_the_session_reacts_to() {
        let o = Observation::MonitorEvent {
            subject: "s_1".into(),
            kind: "blip".into(),
            event_id: "e_1".into(),
            detail: "one cadence".into(),
        };
        assert!(!o.is_subscribed());
        let mut agenda = empty_agenda();
        let out = observations(
            &charter_over(&["s_1"]),
            &[o],
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::NotSubscribed);
    }

    #[test]
    fn one_flip_read_by_three_profiler_runs_is_one_item() {
        // §12.8's own clause: "a regime flip inserts exactly one item per flip
        // across three profiler runs".
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let mut already = Raised::default();

        for _run in 0..3 {
            let out = observations(&charter, &[flip("2026-09-01")], &already, &mut agenda);
            for item in out.added() {
                already = Raised::from_refs(
                    already
                        .0
                        .iter()
                        .cloned()
                        .chain(item.source_refs.iter().cloned()),
                );
            }
            // The item finishes between profiler runs, which is the case layer
            // one cannot catch: a terminal item stops absorbing offers.
            for item in agenda.iter_mut().skip(1) {
                item.status = ItemStatus::Done {
                    verdict: "accept".into(),
                };
            }
        }
        assert_eq!(agenda.len(), 2, "one seeded item and one flip: {agenda:?}");
    }

    #[test]
    fn a_second_flip_is_a_second_item_even_on_the_same_subject() {
        // The limit is per event, not per subject. An instrument that flips in
        // September and again in November has changed twice.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let already = Raised::from_refs([flip("2026-09-01").fingerprint()]);
        let _ = observations(&charter, &[flip("2026-11-14")], &already, &mut agenda);
        assert_eq!(agenda.len(), 2);
    }

    #[test]
    fn the_fingerprint_is_the_items_source_ref_so_the_limit_needs_no_fourth_table() {
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let o = flip("2026-09-01");
        let _ = observations(
            &charter,
            std::slice::from_ref(&o),
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda[1].source_refs, vec![o.fingerprint()]);
        assert!(Raised::from_refs(agenda[1].source_refs.clone()).contains(&o.fingerprint()));
    }

    #[test]
    fn one_batch_carrying_an_event_twice_raises_one_item() {
        // Two overlapping reads of `monitor.events` will do this.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let out = observations(
            &charter,
            &[flip("2026-09-01"), flip("2026-09-01")],
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 2);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::AlreadyRaised);
    }

    #[test]
    fn the_callers_raised_set_is_not_mutated_behind_its_back() {
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let already = Raised::default();
        let _ = observations(&charter, &[flip("2026-09-01")], &already, &mut agenda);
        assert!(already.is_empty(), "the driver's copy is its own to update");
    }

    #[test]
    fn a_monitor_event_asks_why_and_a_flip_asks_what_can_be_done() {
        let charter = charter_over(&["s_1", "btc"]);
        let mut agenda = empty_agenda();
        let _ = observations(
            &charter,
            &[
                Observation::MonitorEvent {
                    subject: "s_1".into(),
                    kind: "scaled_down".into(),
                    event_id: "e_9".into(),
                    detail: "drawdown over limit".into(),
                },
                flip("2026-09-01"),
            ],
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, "diagnose");
        assert!(agenda[1].question.contains("drawdown over limit"));
        assert_eq!(agenda[2].workflow, "investigate");
    }

    #[test]
    fn an_out_of_scope_strategy_inserts_nothing() {
        // §12.8's third clause.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let out = observations(
            &charter,
            &[Observation::MonitorEvent {
                subject: "s_elsewhere".into(),
                kind: "scaled_down".into(),
                event_id: "e_9".into(),
                detail: "".into(),
            }],
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::OutOfScope);
    }

    #[test]
    fn two_monitor_events_a_week_apart_are_two_things_that_happened() {
        let charter = charter_over(&["s_1"]);
        let mut agenda = empty_agenda();
        let ev = |id: &str| Observation::MonitorEvent {
            subject: "s_1".into(),
            kind: "scaled_down".into(),
            event_id: id.into(),
            detail: "drawdown".into(),
        };
        let _ = observations(
            &charter,
            &[ev("e_1"), ev("e_2")],
            &Raised::default(),
            &mut agenda,
        );
        // Both are offered; layer one merges the second onto the first because
        // the *sentence* is the same, and the item ends up carrying both refs —
        // which is the right answer: one question, two pieces of evidence.
        assert_eq!(agenda.len(), 2, "{agenda:?}");
        assert_eq!(agenda[1].source_refs.len(), 2, "both events on the card");
    }

    #[test]
    fn an_observation_is_weighed_above_a_seed_and_below_a_contradiction() {
        // §5.1's table: observation 1.2, charter_seed 1.0, contradiction 1.5.
        assert!(Source::Observation.weight() > Source::CharterSeed.weight());
        assert!(Source::Observation.weight() < Source::Contradiction.weight());
    }
}
