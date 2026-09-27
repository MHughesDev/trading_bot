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
//! * **Nothing publishes a coverage *crossing*.** Coverage itself is recorded —
//!   the data plane knows how far every instrument reaches — but a frontier is
//!   a reading, not an event, and [`Observation::CoverageExtended`] now refuses
//!   a reading rather than raising one card a day off it. What it wants is the
//!   boundary a question was blocked at, which source 4.4 already holds as a
//!   `negative_result`'s `data_window.end`. The rule is built and the join is
//!   named. This is the one gap here that was a **defect** rather than a
//!   missing producer: see the variant's own docs.
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
    ///
    /// # Coverage extending is not an event, and keying on it flooded the agenda
    ///
    /// `since` on [`Observation::RegimeFlip`] is documented, named and tested
    /// so that the rate limit keys on **the change** rather than on the moment
    /// it was read. This variant had the same obligation and did not meet it:
    /// the fingerprint was `coverage:{subject}:{to}`, and `to` is the frontier
    /// the platform's data reaches *now*. On a live feed that advances every
    /// day, so the fingerprint advanced every day, and §4.7's *"one item per
    /// event"* became one item per subject per day, forever.
    ///
    /// Layer one did not catch it either, which is the part that makes it
    /// worth a type rather than a comment: the question named `to` as well, so
    /// [`crate::agenda::dedupe_key`] also differed every day and the second
    /// day's item merged onto nothing. **Both defences failed at once**, and
    /// neither had ever been exercised — this variant had no test and no
    /// producer, so the flood was armed for whoever wired the feed.
    ///
    /// So the discrete thing that happens is not "coverage reached `to`". It
    /// is "coverage crossed a boundary some question was blocked at", and
    /// [`Self::CoverageExtended::past`] is that boundary. Without one there is
    /// no event to key on — only a reading — and
    /// [`Observation::not_an_event`] says so rather than raising an item.
    CoverageExtended {
        subject: String,
        /// The ISO date coverage now reaches.
        ///
        /// **Deliberately not in the fingerprint and not in the question.** It
        /// is the reading, and it moves on its own.
        to: String,
        /// The boundary the extension crossed: the ISO date a question could
        /// not be asked past until now.
        ///
        /// `None` is the honest common case, not a default to be filled in:
        /// nothing on this platform publishes "coverage moved past something
        /// that mattered". The producer that exists is source 4.4's own
        /// operand — a `negative_result`'s `data_window.end`, which
        /// [`crate::sources::ledger::RetryTrigger::DataExtendsBeyond`] already
        /// compares against — so a caller with a blocked question in hand can
        /// name the boundary it was blocked at. A caller reading only the data
        /// plane cannot, and gets a skipped line instead of a daily card.
        past: Option<String>,
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
            // Keyed on the boundary crossed, never on `to`. See the variant.
            Observation::CoverageExtended { subject, past, to } => match past {
                Some(b) => format!("coverage:{subject}:past:{b}"),
                // Only ever reaches a skipped line, and says which reading it
                // was so the log names the day rather than the subject alone.
                None => format!("coverage:{subject}:reading@{to}"),
            },
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
            // `to` is **not** in this sentence, and that is load-bearing rather
            // than terse. The sentence is the dedupe key (`agenda::dedupe_key`
            // normalises it), so a frontier date in it would give the same
            // crossing a different key every day and layer one would stop
            // merging the repeat it exists to merge.
            Observation::CoverageExtended { subject, past, .. } => match past {
                Some(b) => format!(
                    "what can be asked about {subject} now that its data reaches past {b}, which a \
                     question was blocked at"
                ),
                // Unreachable through `observations`, which refuses a reading
                // before it asks for a question. Written rather than
                // `unreachable!` because a panic in a sentence-builder is a
                // worse failure than a sentence that names the gap.
                None => format!(
                    "what can be asked about {subject} now that its data reaches further — no \
                     boundary was named, so there is nothing to say it reaches past"
                ),
            },
            Observation::MonitorEvent {
                subject,
                kind,
                detail,
                ..
            } => format!("why did {subject} go {kind} in production: {detail}"),
        }
    }

    /// Why §4.7's rate limit cannot key on this observation, if it cannot.
    ///
    /// `None` for anything that names something that happened — a regime flip
    /// has its `since` and a monitor event has its `event_id`, both stable
    /// however often they are read.
    ///
    /// `Some` only for a [`Observation::CoverageExtended`] carrying no
    /// boundary. That is a *reading* of a frontier rather than a crossing of
    /// one, and §4.7's rate limit has nothing stable to key on: every read
    /// would be a fresh event. This is the same distinction
    /// [`crate::sources::Fired::NotEvaluable`] draws one module over — a
    /// subscription that cannot answer is not a subscription answering no — and
    /// it reaches the log as [`SkipReason::NotEvaluable`] for the same reason.
    #[must_use]
    pub fn not_an_event(&self) -> Option<String> {
        match self {
            Observation::CoverageExtended { subject, to, past } if past.is_none() => Some(format!(
                "coverage for `{subject}` was read as reaching `{to}`, which is where it is now \
                 and not something that happened; §4.7's limit keys on an event, so this wants \
                 the boundary the extension crossed — a blocked question's `data_window.end`"
            )),
            Observation::CoverageExtended { .. }
            | Observation::RegimeFlip { .. }
            | Observation::MonitorEvent { .. } => None,
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
        // Before the `already` check, because an observation with no stable key
        // makes that check meaningless rather than false: it would pass every
        // time and insert every time.
        if o.not_an_event().is_some() {
            out.skipped.push(Skipped {
                what: fp,
                why: SkipReason::NotEvaluable,
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

    // ── Source 4.7's third trigger (§4.7: "new data coverage in scope") ─────
    //
    // Until 2026-09-23 this variant had no test and no producer, and it keyed
    // §4.7's rate limit on `to` — where the data reaches *now*. Every test
    // below fails against that version, and the first two fail by inserting
    // one card per day for as long as the feed runs.

    /// Coverage reaching `to`, read on three consecutive days.
    fn reading(to: &str) -> Observation {
        Observation::CoverageExtended {
            subject: "btc".into(),
            to: to.into(),
            past: None,
        }
    }

    /// One crossing of `past`, read on a day when coverage reaches `to`.
    fn crossing(past: &str, to: &str) -> Observation {
        Observation::CoverageExtended {
            subject: "btc".into(),
            to: to.into(),
            past: Some(past.into()),
        }
    }

    #[test]
    fn a_coverage_reading_with_no_boundary_is_refused_rather_than_raised_daily() {
        // The defect this variant shipped with. `to` moves every day on a live
        // feed, so a fingerprint keyed on it is a fresh event every day and the
        // rate limit protects nothing.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let mut already = Raised::default();
        for day in ["2026-09-21", "2026-09-22", "2026-09-23"] {
            let out = observations(&charter, &[reading(day)], &already, &mut agenda);
            assert!(
                out.added().next().is_none(),
                "a reading of the frontier is not an event: {out:?}"
            );
            assert_eq!(out.skipped[0].why, SkipReason::NotEvaluable);
            for item in out.added() {
                already.record(&item.source_refs[0]);
            }
        }
        assert_eq!(agenda.len(), 1, "only the seeded item: {agenda:?}");
    }

    #[test]
    fn what_a_reading_is_missing_is_named_and_is_not_reported_as_a_measured_no() {
        let missing = reading("2026-09-23")
            .not_an_event()
            .expect("a frontier reading names no event");
        assert!(missing.contains("data_window.end"), "{missing}");
        // The distinction `Fired` draws one module over: absent is not false.
        assert_ne!(SkipReason::NotEvaluable, SkipReason::NotSubscribed);
        assert_ne!(SkipReason::NotEvaluable, SkipReason::AlreadyRaised);
    }

    #[test]
    fn an_event_that_names_itself_is_not_refused_as_a_reading() {
        // The guard must catch only the variant it was written for; a flip and
        // a monitor event carry stable identities of their own.
        assert!(flip("2026-09-01").not_an_event().is_none());
        assert!(crossing("2026-01-31", "2026-09-23")
            .not_an_event()
            .is_none());
        assert!(Observation::MonitorEvent {
            subject: "s_1".into(),
            kind: "scaled_down".into(),
            event_id: "e_1".into(),
            detail: "drawdown".into(),
        }
        .not_an_event()
        .is_none());
    }

    #[test]
    fn three_daily_readings_of_one_crossing_are_one_item() {
        // §4.7's own parenthesis, for this trigger: one item per event, not one
        // per read. The boundary is the same crossing each day; only `to` moves.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let mut already = Raised::default();
        for to in ["2026-09-21", "2026-09-22", "2026-09-23"] {
            let out = observations(
                &charter,
                &[crossing("2026-01-31", to)],
                &already,
                &mut agenda,
            );
            for item in out.added() {
                already.record(&item.source_refs[0]);
            }
            // The item finishes between reads, which is the case layer one
            // cannot cover: a terminal item stops absorbing offers.
            for item in agenda.iter_mut().skip(1) {
                item.status = ItemStatus::Done {
                    verdict: "accept".into(),
                };
            }
        }
        assert_eq!(
            agenda.len(),
            2,
            "one seeded item and one crossing: {agenda:?}"
        );
    }

    #[test]
    fn the_frontier_is_in_no_fingerprint_and_in_no_question_so_both_defences_hold() {
        // Both refusals keyed on the same drifting date, so both failed at
        // once. The fingerprint is layer §4.7 and the sentence is layer one.
        let monday = crossing("2026-01-31", "2026-09-21");
        let wednesday = crossing("2026-01-31", "2026-09-23");
        assert_eq!(monday.fingerprint(), wednesday.fingerprint());
        assert_eq!(
            crate::agenda::dedupe_key(&monday.question(), monday.subject()),
            crate::agenda::dedupe_key(&wednesday.question(), wednesday.subject()),
            "a frontier date in the sentence makes layer one stop merging the repeat"
        );
        assert!(
            !monday.question().contains("2026-09-21"),
            "the question named the reading: {}",
            monday.question()
        );
        assert!(
            monday.question().contains("2026-01-31"),
            "the question must name the boundary it crossed: {}",
            monday.question()
        );
    }

    #[test]
    fn a_later_crossing_is_a_second_item_because_it_is_a_second_opening() {
        // The limit is per event. Coverage passing January and then passing
        // June unblocked two different questions.
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let already = Raised::from_refs([crossing("2026-01-31", "2026-09-21").fingerprint()]);
        let _ = observations(
            &charter,
            &[crossing("2026-06-30", "2026-09-23")],
            &already,
            &mut agenda,
        );
        assert_eq!(agenda.len(), 2, "{agenda:?}");
    }

    #[test]
    fn coverage_out_of_the_charters_scope_inserts_nothing() {
        let mut agenda = empty_agenda();
        let out = observations(
            &charter_over(&["eth"]),
            &[crossing("2026-01-31", "2026-09-23")],
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::OutOfScope);
    }

    #[test]
    fn new_coverage_is_an_opening_and_asks_what_can_be_done_not_why() {
        // §4.7's table: `investigate` for a flip and for coverage, `diagnose`
        // only for a monitor event, which is a fault rather than an opening.
        let c = crossing("2026-01-31", "2026-09-23");
        assert_eq!(c.workflow(), "investigate");
        assert!(c.is_subscribed(), "coverage has no kind filter of its own");
        let charter = charter_over(&["btc"]);
        let mut agenda = empty_agenda();
        let _ = observations(
            &charter,
            std::slice::from_ref(&c),
            &Raised::default(),
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, "investigate");
        assert_eq!(agenda[1].sources, vec![Source::Observation]);
        assert_eq!(agenda[1].source_refs, vec![c.fingerprint()]);
    }

    #[test]
    fn an_observation_is_weighed_above_a_seed_and_below_a_contradiction() {
        // §5.1's table: observation 1.2, charter_seed 1.0, contradiction 1.5.
        assert!(Source::Observation.weight() > Source::CharterSeed.weight());
        assert!(Source::Observation.weight() < Source::Contradiction.weight());
    }
}
