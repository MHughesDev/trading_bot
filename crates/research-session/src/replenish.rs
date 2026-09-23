//! Replenish — the endless setting (AGENT-014 §4.9, §2.1, §7; ADR-0037 §2).
//!
//! "Keep generating questions" in the desk. With it on and the agenda drained,
//! the session runs one `survey` over its charter and turns what the survey
//! proposes into items. With it off, a drained agenda is a finished session.
//!
//! **Endless does not mean unbounded.** The budget still ends it, `max_runs`
//! still ends it, and `stop_when` still ends it — replenish only decides what
//! happens at the one moment the agenda runs out, which is otherwise the end.
//!
//! # A replenish that proposes nothing is a barren run
//!
//! §4.9's last clause, and it is the rule that stops the endless setting from
//! being a way to spend a budget forever. A survey whose every proposal is
//! already on the agenda has learned that there is nothing left to ask, and a
//! session that keeps surveying an exhausted scope is exactly the motion
//! [`crate::stall`]'s barren streak exists to catch. So this module hands the
//! stall detector a [`crate::stall::FinishedRun`] with `knowledge_units = 0`
//! rather than leaving the driver to decide what a fruitless survey counts as.

use uuid::Uuid;

use crate::agenda::{AgendaItem, Insert, ItemStatus, Source};
use crate::charter::Charter;
use crate::harvest::{SkipReason, Skipped};
use crate::sources::offer_all;
use crate::stall::FinishedRun;

/// The workflow a replenish item runs.
///
/// Named once. §4.9 says "a `survey` run over the charter and everything in
/// scope", and the word has to match what the catalogue dispatches on — a
/// second spelling here would produce an item nothing can run.
pub const SURVEY_WORKFLOW: &str = "survey";

/// What the agenda's state means for a session that has nothing to claim.
#[derive(Debug, Clone, PartialEq)]
pub enum Drain {
    /// There is claimable work; nothing to decide.
    NotDrained,
    /// There is no claimable work but something is still in flight or waiting
    /// on a human. **Not** drained: a session that completed while a child was
    /// running would lose the child's harvest, and one that completed while an
    /// item sat in *Needs you* would complete the moment a human stepped away.
    Waiting { reason: &'static str },
    /// `replenish = true`: run this survey.
    Replenish(Box<AgendaItem>),
    /// `replenish = false`: §7's "agenda drained → complete".
    Complete,
}

impl Drain {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Drain::NotDrained => "not_drained",
            Drain::Waiting { .. } => "waiting",
            Drain::Replenish(_) => "replenish",
            Drain::Complete => "complete",
        }
    }
}

/// What a session does when its agenda runs out (§4.9, §7 rows 4 and 5).
///
/// `replenish` is passed rather than read off the charter because it is a
/// setting on the **session** (§2.1), not on the charter (§2.2) — the user can
/// turn "keep generating questions" off without amending what the session is
/// chartered to do, and reading it from the wrong object would make that
/// impossible.
#[must_use]
pub fn on_drain(session: Uuid, charter: &Charter, agenda: &[AgendaItem], replenish: bool) -> Drain {
    if agenda.iter().any(|i| i.status.is_claimable()) {
        return Drain::NotDrained;
    }
    if agenda.iter().any(|i| {
        matches!(
            i.status,
            ItemStatus::Claimed { .. } | ItemStatus::Running { .. }
        )
    }) {
        return Drain::Waiting {
            reason: "a child is still running; its harvest may refill the agenda",
        };
    }
    if agenda
        .iter()
        .any(|i| matches!(i.status, ItemStatus::Proposed))
    {
        return Drain::Waiting {
            reason: "an item is waiting for a human to approve it",
        };
    }
    if !replenish {
        return Drain::Complete;
    }
    Drain::Replenish(Box::new(survey_item(session, charter)))
}

/// The synthetic item a replenish claims.
///
/// It is an ordinary [`AgendaItem`] and not a special shape, so the board shows
/// it, the digest counts it, a human can veto it and the fold records its run
/// like any other. A survey that ran outside the agenda would be a run the
/// session's own scorecard could not see.
///
/// Its source is [`Source::Replenish`], **not** [`Source::CharterSeed`], even
/// though the charter is what it surveys: the difference is what lets a reader
/// tell a session that was given twelve questions from one that generated them.
#[must_use]
pub fn survey_item(session: Uuid, charter: &Charter) -> AgendaItem {
    let subject = charter
        .scope
        .instruments
        .first()
        .or_else(|| charter.scope.families.first())
        .or_else(|| charter.scope.asset_classes.first())
        .cloned()
        .unwrap_or_default();
    AgendaItem::new(
        session,
        &survey_question(charter),
        &subject,
        Source::Replenish,
    )
    .with_workflow(SURVEY_WORKFLOW)
}

/// The question the survey is asked.
///
/// It names the charter version, and that is what makes two surveys of an
/// amended charter two items rather than one. Without it the dedupe key is
/// identical every time, and the second survey of a session whose scope the
/// user just widened would merge onto the first and never run.
#[must_use]
pub fn survey_question(charter: &Charter) -> String {
    format!(
        "the agenda is empty: what is worth asking next under this charter (version {})",
        charter.version
    )
}

/// A question a `survey` run proposed (§4.9).
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedQuestion {
    pub question: String,
    pub subject: String,
    /// §4.9's default workflow column is "per proposal".
    pub workflow: Option<String>,
    /// The survey's own estimate of the question's value.
    ///
    /// `Option`, and a proposal without one does **not** become 1.0 or 0.0.
    /// See [`Replenished::value_estimates`] for where it goes and what is
    /// missing for it to go anywhere.
    pub value_estimate: Option<f64>,
}

impl ProposedQuestion {
    #[must_use]
    pub fn new(question: &str, subject: &str) -> Self {
        Self {
            question: question.to_string(),
            subject: subject.to_string(),
            workflow: None,
            value_estimate: None,
        }
    }

    #[must_use]
    pub fn worth(mut self, estimate: f64) -> Self {
        self.value_estimate = Some(estimate);
        self
    }

    #[must_use]
    pub fn running(mut self, workflow: &str) -> Self {
        self.workflow = Some(workflow.to_string());
        self
    }
}

/// What the `survey` run came back with.
#[derive(Debug, Clone, PartialEq)]
pub struct SurveyReport {
    pub session: Uuid,
    /// The synthetic item the survey ran as.
    pub item: Uuid,
    pub run: Uuid,
    pub proposed_questions: Vec<ProposedQuestion>,
}

/// What absorbing a survey report did to the agenda.
#[derive(Debug, Clone, PartialEq)]
pub struct Replenished {
    pub inserted: Vec<Insert>,
    pub skipped: Vec<Skipped>,
    /// The survey's estimate for each item it actually created, in the order
    /// they were created.
    ///
    /// **This is here because there is nowhere else to put it.**
    /// [`AgendaItem`] has no `value_estimate` field and `agenda_items` has no
    /// such column, so §4.9's "items with the survey's value estimates" has no
    /// store. The field was not added: `crates/api/src/sessions.rs` builds an
    /// `AgendaItem` with a struct literal, so a new field breaks a crate
    /// another track is compiling against, and inventing a `Score` from a
    /// single estimate would put three signals nobody measured on a decision
    /// row. What is needed, exactly:
    ///
    /// * `research_session::AgendaItem.value_estimate: Option<f64>`;
    /// * `agenda_items.value_estimate double precision` (nullable — a
    ///   proposal without an estimate is not a proposal worth zero);
    /// * `select::Inputs::value` to prefer it over `Objective::weight_for`
    ///   when it is present, with [`crate::select::Provenance::Measured`].
    ///
    /// Until then the estimates come back here and the driver puts them on the
    /// `agenda_item_added` event's payload, where they are readable and are not
    /// silently treated as a score.
    pub value_estimates: Vec<(Uuid, Option<f64>)>,
}

impl Replenished {
    /// The items the survey genuinely added.
    pub fn added(&self) -> impl Iterator<Item = &AgendaItem> {
        self.inserted.iter().filter_map(|i| match i {
            Insert::Added(item) => Some(&**item),
            Insert::Merged { .. } => None,
        })
    }

    /// How many new questions this replenish produced.
    #[must_use]
    pub fn new_questions(&self) -> usize {
        self.added().count()
    }

    /// §4.9: a replenish that proposes nothing new counts toward the barren
    /// streak.
    ///
    /// "Nothing new" means every proposal was deduped **or** there were no
    /// proposals at all. A survey that came back empty has told the session the
    /// same thing as one whose every idea was already on the board.
    #[must_use]
    pub fn was_barren(&self) -> bool {
        self.new_questions() == 0
    }

    /// The run the stall detector should see.
    ///
    /// Built here rather than in the driver so that "a fruitless replenish is
    /// barren" is a property of this module and not of whoever wires it. The
    /// cost is passed in because this module does not know it; the knowledge
    /// units are **not** passed in, because that is the number §4.9 decides.
    #[must_use]
    pub fn as_finished_run(&self, report: &SurveyReport, cost: f64) -> FinishedRun {
        FinishedRun::without_family(report.item, report.run)
            .with_workflow(SURVEY_WORKFLOW)
            .from_source(Source::Replenish)
            .producing(u32::try_from(self.new_questions()).unwrap_or(u32::MAX), 0)
            .costing(cost)
    }
}

/// Turn a survey's proposals into agenda items (§4.9).
///
/// Every proposal goes through [`crate::agenda::offer`] — layer one — which is
/// what makes "a fully deduped replenish" a thing that can happen and be
/// counted, rather than a thing that silently doubles the agenda.
///
/// The charter is consulted the same way every subscription consults it: a
/// survey that proposes work outside its own scope has misread the charter, and
/// the item would sit on the board being skipped forever.
#[must_use]
pub fn absorb(
    charter: &Charter,
    report: &SurveyReport,
    agenda: &mut Vec<AgendaItem>,
) -> Replenished {
    let mut skipped = Vec::new();
    let mut candidates = Vec::new();
    let mut estimates = Vec::new();

    for p in &report.proposed_questions {
        if p.question.trim().is_empty() {
            skipped.push(Skipped {
                what: p.question.clone(),
                why: SkipReason::Blank,
            });
            continue;
        }
        if let Err(s) = crate::sources::admitted(charter, &p.subject, &p.question) {
            skipped.push(s);
            continue;
        }
        let workflow = p
            .workflow
            .clone()
            .unwrap_or_else(|| Source::Replenish.default_workflow().to_string());
        if !charter.permits(&workflow) {
            skipped.push(Skipped {
                what: format!("{}: {workflow}", p.question),
                why: SkipReason::Excluded,
            });
            continue;
        }
        let item = AgendaItem::new(report.session, &p.question, &p.subject, Source::Replenish)
            .with_workflow(&workflow)
            .with_source_ref(&format!("survey:{}", report.run));
        estimates.push((item.id, p.value_estimate));
        candidates.push(item);
    }

    let inserted = offer_all(agenda, candidates);
    // Only the ones that actually became items keep their estimate. A merged
    // proposal's estimate belongs to no row: the item it merged onto was
    // created by another source and scored under that source's value.
    let added: Vec<Uuid> = inserted
        .iter()
        .filter_map(|i| match i {
            Insert::Added(item) => Some(item.id),
            Insert::Merged { .. } => None,
        })
        .collect();
    estimates.retain(|(id, _)| added.contains(id));

    Replenished {
        inserted,
        skipped,
        value_estimates: estimates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::{ExplorationShare, Objective, Scope};

    fn session() -> Uuid {
        Uuid::from_u128(0x5e55_1041)
    }

    fn run() -> Uuid {
        Uuid::from_u128(0x2202)
    }

    fn charter() -> Charter {
        Charter {
            scope: Scope {
                instruments: vec!["btc".into()],
                ..Scope::default()
            },
            objective: Objective::BuildKnowledge {
                about: vec!["btc".into()],
            },
            seeds: vec![],
            permitted_workflows: vec![],
            exclusions: vec![],
            standing_constraints: vec![],
            exploration_share: ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        }
    }

    fn done(q: &str) -> AgendaItem {
        AgendaItem {
            status: ItemStatus::Done {
                verdict: "accept".into(),
            },
            ..AgendaItem::new(session(), q, "btc", Source::CharterSeed)
        }
    }

    fn report(qs: Vec<ProposedQuestion>) -> SurveyReport {
        SurveyReport {
            session: session(),
            item: Uuid::from_u128(0x1101),
            run: run(),
            proposed_questions: qs,
        }
    }

    #[test]
    fn a_drained_agenda_with_replenish_off_completes() {
        // §12.9's second clause.
        let agenda = vec![done("one")];
        assert_eq!(
            on_drain(session(), &charter(), &agenda, false),
            Drain::Complete
        );
    }

    #[test]
    fn a_drained_agenda_with_replenish_on_runs_a_survey() {
        // §12.9's first clause.
        let agenda = vec![done("one")];
        let Drain::Replenish(item) = on_drain(session(), &charter(), &agenda, true) else {
            panic!("replenish means keep generating questions");
        };
        assert_eq!(item.workflow, SURVEY_WORKFLOW);
        assert_eq!(item.sources, vec![Source::Replenish]);
        assert_eq!(item.session_id, session());
        assert_eq!(item.status, ItemStatus::Proposed);
    }

    #[test]
    fn claimable_work_is_not_a_drained_agenda() {
        let mut agenda = vec![done("one")];
        agenda.push(AgendaItem {
            status: ItemStatus::Queued,
            ..AgendaItem::new(session(), "two", "btc", Source::CharterSeed)
        });
        assert_eq!(
            on_drain(session(), &charter(), &agenda, false),
            Drain::NotDrained
        );
    }

    #[test]
    fn a_session_does_not_complete_out_from_under_a_running_child() {
        // Completing here would lose the child's harvest, which is exactly the
        // thing that refills an agenda.
        let agenda = vec![AgendaItem {
            status: ItemStatus::Running {
                run_id: Uuid::from_u128(9),
            },
            ..AgendaItem::new(session(), "one", "btc", Source::CharterSeed)
        }];
        let d = on_drain(session(), &charter(), &agenda, false);
        assert!(matches!(d, Drain::Waiting { .. }), "{d:?}");
    }

    #[test]
    fn a_session_does_not_complete_while_an_item_waits_for_a_human() {
        // Under `supervised` every item sits `proposed` until somebody clicks.
        // Completing would end the session the moment a human stepped away.
        let agenda = vec![AgendaItem::new(
            session(),
            "one",
            "btc",
            Source::CharterSeed,
        )];
        let d = on_drain(session(), &charter(), &agenda, true);
        assert!(matches!(d, Drain::Waiting { .. }), "{d:?}");
    }

    #[test]
    fn two_surveys_of_one_charter_version_are_one_question() {
        // And so the second merges rather than running again, which is what
        // stops a drained session surveying in a loop.
        let a = survey_item(session(), &charter());
        let b = survey_item(session(), &charter());
        assert_eq!(a.dedupe_key, b.dedupe_key);
    }

    #[test]
    fn amending_the_charter_makes_the_next_survey_a_new_question() {
        let mut c2 = charter();
        c2.version = 2;
        assert_ne!(
            survey_item(session(), &charter()).dedupe_key,
            survey_item(session(), &c2).dedupe_key,
            "a user who just widened the scope is owed a fresh survey"
        );
    }

    #[test]
    fn a_surveys_proposals_become_scored_items_under_their_own_source() {
        let mut agenda = vec![done("something settled")];
        let r = absorb(
            &charter(),
            &report(vec![
                ProposedQuestion::new("does funding predict returns", "btc").worth(0.7),
                ProposedQuestion::new("does basis predict returns", "btc"),
            ]),
            &mut agenda,
        );
        assert_eq!(r.new_questions(), 2);
        assert!(r.added().all(|i| i.sources == vec![Source::Replenish]));
        assert!(r
            .added()
            .all(|i| i.source_refs == vec![format!("survey:{}", run())]));
        assert!(!r.was_barren());
    }

    #[test]
    fn a_proposal_with_no_estimate_does_not_get_one() {
        // `0 ?? x` is `0`. An unestimated proposal scored at zero is one that
        // never runs; scored at one it outranks everything the survey rated.
        let mut agenda = vec![done("settled")];
        let r = absorb(
            &charter(),
            &report(vec![ProposedQuestion::new("unrated", "btc")]),
            &mut agenda,
        );
        assert_eq!(r.value_estimates.len(), 1);
        assert_eq!(r.value_estimates[0].1, None);
    }

    #[test]
    fn a_merged_proposals_estimate_belongs_to_no_row() {
        // The item it merged onto was created by another source and is scored
        // under that source's value; attaching the survey's guess to it would
        // rewrite a number somebody else measured.
        let mut agenda = vec![AgendaItem {
            status: ItemStatus::Queued,
            ..AgendaItem::new(session(), "already asked", "btc", Source::CharterSeed)
        }];
        let r = absorb(
            &charter(),
            &report(vec![
                ProposedQuestion::new("already asked", "btc").worth(0.9)
            ]),
            &mut agenda,
        );
        assert!(r.value_estimates.is_empty());
        assert_eq!(r.new_questions(), 0);
    }

    #[test]
    fn a_fully_deduped_replenish_is_barren_and_says_so_to_the_stall_detector() {
        // §12.9's third clause. This is the rule that stops "endless" from
        // being a way to spend a budget on motion.
        let mut agenda = vec![AgendaItem {
            status: ItemStatus::Queued,
            ..AgendaItem::new(session(), "already asked", "btc", Source::CharterSeed)
        }];
        let rep = report(vec![ProposedQuestion::new("already asked", "btc")]);
        let r = absorb(&charter(), &rep, &mut agenda);
        assert!(r.was_barren());
        let finished = r.as_finished_run(&rep, 1.5);
        assert_eq!(finished.knowledge_units, 0);
        assert!(finished.produced_nothing());
    }

    #[test]
    fn a_survey_that_came_back_empty_is_barren_too() {
        let mut agenda = vec![done("settled")];
        let rep = report(vec![]);
        let r = absorb(&charter(), &rep, &mut agenda);
        assert!(
            r.was_barren(),
            "a survey with no ideas has said the same thing as one whose ideas \
             were all already on the board"
        );
    }

    #[test]
    fn a_productive_replenish_is_not_barren() {
        let mut agenda = vec![done("settled")];
        let rep = report(vec![ProposedQuestion::new("a new question", "btc")]);
        let r = absorb(&charter(), &rep, &mut agenda);
        let finished = r.as_finished_run(&rep, 1.5);
        assert_eq!(finished.knowledge_units, 1);
        assert!(!finished.produced_nothing());
    }

    #[test]
    fn a_replenish_run_is_never_a_refinement_followup() {
        // `stall::degenerating` keys on a window entirely of
        // `refinement_followup`. Filing the survey under that source would make
        // a session that keeps surveying look degenerate; filing its *output*
        // under it would too. Both are `replenish`.
        let mut agenda = vec![done("settled")];
        let rep = report(vec![ProposedQuestion::new("a new question", "btc")]);
        let r = absorb(&charter(), &rep, &mut agenda);
        assert_eq!(
            r.as_finished_run(&rep, 1.0).sources,
            vec![Source::Replenish]
        );
        assert!(r
            .added()
            .all(|i| !i.sources.contains(&Source::RefinementFollowup)));
    }

    #[test]
    fn a_proposal_outside_the_charters_scope_is_refused_with_a_line() {
        let mut agenda = vec![done("settled")];
        let r = absorb(
            &charter(),
            &report(vec![ProposedQuestion::new("something about eth", "eth")]),
            &mut agenda,
        );
        assert_eq!(r.new_questions(), 0);
        assert_eq!(r.skipped[0].why, SkipReason::OutOfScope);
    }

    #[test]
    fn a_proposal_naming_a_workflow_the_charter_forbids_is_refused() {
        let mut c = charter();
        c.permitted_workflows = vec!["investigate".into()];
        let mut agenda = vec![done("settled")];
        let r = absorb(
            &c,
            &report(vec![
                ProposedQuestion::new("q1", "btc").running("optimise"),
                ProposedQuestion::new("q2", "btc").running("investigate"),
            ]),
            &mut agenda,
        );
        assert_eq!(r.new_questions(), 1);
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].what.contains("optimise"));
    }

    #[test]
    fn a_proposal_keeps_the_workflow_it_named() {
        // §4.9's workflow column is "per proposal".
        let mut agenda = vec![done("settled")];
        let _ = absorb(
            &charter(),
            &report(vec![ProposedQuestion::new("q", "btc").running("diagnose")]),
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, "diagnose");
    }

    #[test]
    fn a_proposal_naming_no_workflow_takes_the_sources_default() {
        let mut agenda = vec![done("settled")];
        let _ = absorb(
            &charter(),
            &report(vec![ProposedQuestion::new("q", "btc")]),
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, Source::Replenish.default_workflow());
    }
}
