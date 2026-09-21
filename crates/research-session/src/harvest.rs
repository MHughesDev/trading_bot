//! The harvest: what a finished run puts back into the session (AGENT-014 P6,
//! §6.5; the pure half of Set S task S-11.5).
//!
//! A child run ends and the driver reads its dossier: the questions it opened,
//! the moves it thinks are worth making next, and the proposals it wants a
//! decision on. This module is the part of that which is a *decision* — what
//! becomes an agenda item, what becomes a human's problem, and what the item the
//! run was attached to turns into. The reading of the dossier and the writing of
//! the rows are plumbing and live in the driver, which is why [`harvest`] is a
//! function over records with no store, no dossier reader and no clock in it.
//!
//! # A session never approves its own proposals
//!
//! P6, and the reason [`AwaitingHuman`] exists as a type rather than as a flag
//! on a proposal. There is no field on it that could say a session decided
//! something, no constructor that could set one, and
//! [`AwaitingHuman::digest_record`] has no branch that could write
//! `awaiting: false`. A reader holding the output of a harvest cannot mistake a
//! proposal for an approval, because the only shape it comes back in is the one
//! that means *somebody still owes this a decision*.
//!
//! The cost of getting this wrong is not abstract: rings 1 and 2 are the
//! registry and the paper book (AGENT-013 §4), and a session that could clear
//! its own queue would be a session that deploys.
//!
//! # The dedupe here is layer one, and there is only one of it
//!
//! Every harvested question goes through [`crate::agenda::offer`] — the same
//! call every other source uses — so a question this run suggested and a
//! contradiction wanted extends one item rather than making two. Nothing in this
//! module compares two questions itself. Layer two is `hypothesis_precheck` at
//! intake and layer three is the ledger's trial count in `novelty`; a fourth
//! comparison written here would be a fourth answer to give when they disagree.
//!
//! The offers are made **before** the item is marked `done`, which is the order
//! §6.5 writes them in and which matters: `offer` merges onto non-terminal items
//! only, so a run that suggests re-asking its own question extends the item it
//! just ran instead of opening a second one.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agenda::{offer, AgendaItem, Insert, ItemStatus, Source, MAX_ATTEMPTS};
use crate::digest::ProposalRecord;

/// The verdicts §6.6 counts as a child failing rather than concluding.
///
/// A run that ends this way did not answer its question, so the item goes back
/// on the agenda rather than to `done` — a `done{failed}` item is a question the
/// board would show as settled.
pub const FAILURE_VERDICTS: &[&str] = &["failed", "fenced", "interrupted"];

/// What a harvest could not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HarvestError {
    #[error(
        "ring {got} is not a ring anything can be proposed into: ring 0 is the run's own workspace \
         and it writes there directly, and ring 3 is unreachable by any path"
    )]
    NotAProposableRing { got: i16 },
}

/// The rings a proposal can target (AGENT-013 §4, AGENT-014 §6.5).
///
/// Two variants, because "proposals into rings 1 and 2 go to *Needs you*" is the
/// whole set. Ring 0 is what the run wrote for itself and is not proposed; ring
/// 3 is live and is unreachable from any mode. Both are refused at
/// [`Ring::parse`], which is the boundary where a driver maps a dossier row —
/// so no code downstream has to hold an opinion about a ring that cannot exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ring {
    /// Platform artifacts: the registry, playbooks, limits.
    Artifacts,
    /// The paper book.
    Paper,
}

impl Ring {
    /// # Errors
    ///
    /// [`HarvestError::NotAProposableRing`] for anything but 1 or 2.
    pub fn parse(n: i16) -> Result<Self, HarvestError> {
        match n {
            1 => Ok(Ring::Artifacts),
            2 => Ok(Ring::Paper),
            got => Err(HarvestError::NotAProposableRing { got }),
        }
    }

    /// The number the digest and the desk render.
    #[must_use]
    pub fn number(self) -> i16 {
        match self {
            Ring::Artifacts => 1,
            Ring::Paper => 2,
        }
    }
}

/// A question the run opened into memory (AGENT-013 §9), as source 4.3 needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenQuestion {
    /// The memory row's id, which becomes the item's `source_ref` so the agenda
    /// card opens the question.
    pub id: String,
    pub question: String,
    pub subject: String,
    /// `blocking` questions come in under a heavier source (§5.1's table) rather
    /// than under a flag the scorer would have to know about separately.
    pub blocking: bool,
}

impl OpenQuestion {
    #[must_use]
    pub fn new(id: &str, question: &str, subject: &str) -> Self {
        Self {
            id: id.to_string(),
            question: question.to_string(),
            subject: subject.to_string(),
            blocking: false,
        }
    }

    #[must_use]
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }
}

/// Something the run said to ask next, as source 4.8 needs it.
///
/// **Both** halves of §6.5's reading land here: the dossier's `next_moves` and
/// its proposed next questions. They arrive as one kind because they become one
/// source — `refinement_followup` — and that is not a tidying-up. It is what
/// [`crate::stall`]'s degenerating rule reads: a window every one of whose runs
/// came from a follow-up and established nothing is a programme adjusting rather
/// than predicting, and routing half of a session's self-generated work under
/// another source would hide exactly the sessions that rule exists to catch.
/// It is also why the source is the lowest-weighted one in §5.1's table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextQuestion {
    pub question: String,
    pub subject: String,
    /// §4.8: a secondary hypothesis becomes an agenda item only if the run
    /// marked it worth pursuing. The rest are in the dossier for a human and are
    /// reported in [`Harvest::skipped`] rather than dropped.
    pub worth_pursuing: bool,
}

impl NextQuestion {
    /// A next move the run did **not** mark worth pursuing.
    ///
    /// The unmarked case is the constructor's default for the reason
    /// [`crate::stall::FinishedRun::new`] defaults to barren: the state that
    /// costs money should be the one somebody typed on purpose.
    #[must_use]
    pub fn new(question: &str, subject: &str) -> Self {
        Self {
            question: question.to_string(),
            subject: subject.to_string(),
            worth_pursuing: false,
        }
    }

    #[must_use]
    pub fn worth_pursuing(mut self) -> Self {
        self.worth_pursuing = true;
        self
    }
}

/// A proposal the run made into ring 1 or ring 2 (AGENT-013 §7.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProposal {
    pub id: String,
    /// `promote_strategy`, `deploy_paper`, `adopt_playbook`, `change_limits`,
    /// `start_continuous_research` — a string, because the vocabulary is
    /// AGENT-013 §7.5's and this module reports what the dossier said rather
    /// than keeping a second copy of a list it does not own.
    pub kind: String,
    pub what: String,
    pub ring: Ring,
}

impl RunProposal {
    #[must_use]
    pub fn new(id: &str, kind: &str, what: &str, ring: Ring) -> Self {
        Self {
            id: id.to_string(),
            kind: kind.to_string(),
            what: what.to_string(),
            ring,
        }
    }
}

/// What a finished run produced, as the harvest needs to see it.
///
/// Not the dossier and not the run row: these are the facts §6.5 reads. A record
/// carrying the run's prose would invite a harvest that summarised, and
/// summarising is the digest's job and the model's output is not what it reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunHarvest {
    pub session: Uuid,
    /// The agenda item this run was claimed for.
    pub item: Uuid,
    pub run: Uuid,
    /// `supported`, `refuted`, `inconclusive`, or one of [`FAILURE_VERDICTS`].
    pub verdict: String,
    /// How many times this item had already been attempted before this run, as
    /// the row holds it. Passed in rather than looked up so that
    /// [`outcome`] is a function of what the caller knows.
    pub attempts: i32,
    pub open_questions: Vec<OpenQuestion>,
    pub next_questions: Vec<NextQuestion>,
    pub proposals: Vec<RunProposal>,
}

impl RunHarvest {
    /// A run that finished with a verdict and brought nothing back.
    #[must_use]
    pub fn new(session: Uuid, item: Uuid, run: Uuid, verdict: &str) -> Self {
        Self {
            session,
            item,
            run,
            verdict: verdict.to_string(),
            attempts: 0,
            open_questions: Vec::new(),
            next_questions: Vec::new(),
            proposals: Vec::new(),
        }
    }

    #[must_use]
    pub fn opening(mut self, q: OpenQuestion) -> Self {
        self.open_questions.push(q);
        self
    }

    #[must_use]
    pub fn suggesting(mut self, q: NextQuestion) -> Self {
        self.next_questions.push(q);
        self
    }

    #[must_use]
    pub fn proposing(mut self, p: RunProposal) -> Self {
        self.proposals.push(p);
        self
    }

    #[must_use]
    pub fn after_attempts(mut self, attempts: i32) -> Self {
        self.attempts = attempts;
        self
    }
}

/// A proposal on its way to *Needs you*, and the only shape a proposal leaves a
/// harvest in.
///
/// **It carries no decision and it cannot be given one** (P6). The proposal, the
/// run that made it and the item it came from, and nothing else: a session that
/// could write `approved` here is a session that promotes strategies and deploys
/// paper by itself, which is the thing every ring exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaitingHuman {
    proposal: RunProposal,
    run: Uuid,
    item: Uuid,
}

impl AwaitingHuman {
    #[must_use]
    pub fn proposal(&self) -> &RunProposal {
        &self.proposal
    }

    #[must_use]
    pub fn run(&self) -> Uuid {
        self.run
    }

    #[must_use]
    pub fn item(&self) -> Uuid {
        self.item
    }

    /// The digest's row for it.
    ///
    /// `awaiting` is `true` with no branch that could make it anything else. The
    /// field exists on [`ProposalRecord`] because a digest written later reports
    /// what a *human* has since decided; a harvest is the moment before that,
    /// and it has nothing to say about it.
    #[must_use]
    pub fn digest_record(&self) -> ProposalRecord {
        ProposalRecord {
            id: self.proposal.id.clone(),
            what: self.proposal.what.clone(),
            ring: self.proposal.ring.number(),
            awaiting: true,
        }
    }
}

/// Why a candidate did not reach the agenda.
///
/// Nothing a run produced disappears without a line: a question the harvest
/// quietly dropped is a question a human would have to open the dossier to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// §4.8: the run did not mark this next move worth pursuing.
    NotWorthPursuing,
    /// An empty question. Offered, it would key on nothing and merge with every
    /// other empty one.
    Blank,
}

impl SkipReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::NotWorthPursuing => "not_worth_pursuing",
            SkipReason::Blank => "blank",
        }
    }
}

/// A candidate the harvest did not offer, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub what: String,
    pub why: SkipReason,
}

/// What a finished run put back.
#[derive(Debug, Clone, PartialEq)]
pub struct Harvest {
    /// What layer one did with each candidate, in the order they were offered.
    /// [`Insert::Added`] carries the item the driver writes a row for;
    /// [`Insert::Merged`] names the item whose source list grew instead.
    pub inserted: Vec<Insert>,
    /// Every proposal, now a human's to decide. Never empty of meaning: a
    /// session with proposals and an empty *Needs you* is a session that decided
    /// them itself.
    pub needs_you: Vec<AwaitingHuman>,
    pub skipped: Vec<Skipped>,
    /// What the item this run was claimed for becomes (§6.5, §6.6). Applied to
    /// the agenda by [`harvest`] when the item is in it, and returned here as
    /// well because the `run_finished` event carries it.
    pub item: ItemStatus,
}

impl Harvest {
    /// The items layer one accepted as new.
    pub fn added(&self) -> impl Iterator<Item = &AgendaItem> {
        self.inserted.iter().filter_map(|i| match i {
            Insert::Added(item) => Some(&**item),
            Insert::Merged { .. } => None,
        })
    }

    /// The items layer one merged onto, with the source each gained.
    pub fn merged(&self) -> impl Iterator<Item = (Uuid, Source)> + '_ {
        self.inserted.iter().filter_map(|i| match i {
            Insert::Merged { into, source } => Some((*into, *source)),
            Insert::Added(_) => None,
        })
    }
}

/// What the agenda item becomes when its run ends (AGENT-014 §6.5, §6.6).
///
/// A verdict is `done`. A failure is a re-queue, and the second failure is an
/// expiry — [`MAX_ATTEMPTS`] is the same constant the item's own row counts
/// against, so "retried once" is one number rather than two that can drift.
///
/// This is deliberately *not* where the session-level breaker lives: two
/// children failing on the same `platform_defect` pauses the whole session
/// (§6.6, §7 row 10), and a rule about the session does not belong in a function
/// that can only see one run.
#[must_use]
pub fn outcome(verdict: &str, attempts_before: i32) -> ItemStatus {
    if !is_failure(verdict) {
        return ItemStatus::Done {
            verdict: verdict.to_string(),
        };
    }
    let attempts = attempts_before + 1;
    if attempts >= MAX_ATTEMPTS {
        return ItemStatus::Expired {
            why: format!("{attempts} attempts, the last ending `{verdict}`"),
        };
    }
    ItemStatus::Queued
}

/// Whether a verdict means the child failed rather than concluded.
#[must_use]
pub fn is_failure(verdict: &str) -> bool {
    FAILURE_VERDICTS.contains(&verdict)
}

/// Harvest a finished run into the agenda (AGENT-014 §6.5).
///
/// Offers every question the run opened (source 4.3) and every next move it
/// marked worth pursuing (source 4.8) through [`crate::agenda::offer`], returns
/// every proposal as something [`AwaitingHuman`], and settles the item the run
/// was claimed for.
///
/// The agenda is taken by value-holding reference because layer one needs to see
/// the items added earlier in this same harvest — two dossier entries asking the
/// same question are one item, and they would not be if each were offered
/// against the agenda as it stood before.
pub fn harvest(run: &RunHarvest, agenda: &mut Vec<AgendaItem>) -> Harvest {
    let mut inserted = Vec::new();
    let mut skipped = Vec::new();

    let mut give = |agenda: &mut Vec<AgendaItem>, item: AgendaItem| {
        let result = offer(agenda.as_mut_slice(), item);
        if let Insert::Added(added) = &result {
            agenda.push((**added).clone());
        }
        inserted.push(result);
    };

    for q in &run.open_questions {
        if q.question.trim().is_empty() {
            skipped.push(Skipped {
                what: q.id.clone(),
                why: SkipReason::Blank,
            });
            continue;
        }
        let source = if q.blocking {
            Source::OpenQuestionBlocking
        } else {
            Source::OpenQuestion
        };
        give(
            agenda,
            AgendaItem::new(run.session, &q.question, &q.subject, source).with_source_ref(&q.id),
        );
    }

    for q in &run.next_questions {
        if q.question.trim().is_empty() {
            skipped.push(Skipped {
                what: q.question.clone(),
                why: SkipReason::Blank,
            });
            continue;
        }
        if !q.worth_pursuing {
            skipped.push(Skipped {
                what: q.question.clone(),
                why: SkipReason::NotWorthPursuing,
            });
            continue;
        }
        give(
            agenda,
            AgendaItem::new(
                run.session,
                &q.question,
                &q.subject,
                Source::RefinementFollowup,
            )
            .with_source_ref(&format!("run:{}", run.run)),
        );
    }

    let needs_you = run
        .proposals
        .iter()
        .map(|p| AwaitingHuman {
            proposal: p.clone(),
            run: run.run,
            item: run.item,
        })
        .collect();

    // Last, so that every offer above was made while this item was still
    // non-terminal and layer one could merge onto it.
    let item = outcome(&run.verdict, run.attempts);
    if let Some(row) = agenda.iter_mut().find(|i| i.id == run.item) {
        if matches!(item, ItemStatus::Queued | ItemStatus::Expired { .. }) {
            row.attempts = run.attempts + 1;
        }
        row.status = item.clone();
    }

    Harvest {
        inserted,
        needs_you,
        skipped,
        item,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::dedupe_key;

    fn session() -> Uuid {
        Uuid::from_u128(1)
    }

    fn run_id() -> Uuid {
        Uuid::from_u128(2)
    }

    /// The agenda item a run was claimed for, `running`.
    fn claimed() -> AgendaItem {
        let mut item = AgendaItem::new(
            session(),
            "does momentum survive costs on BTC-USD?",
            "BTC-USD",
            Source::CharterSeed,
        );
        item.status = ItemStatus::Running { run_id: run_id() };
        item
    }

    fn finished(item: &AgendaItem, verdict: &str) -> RunHarvest {
        RunHarvest::new(session(), item.id, run_id(), verdict)
    }

    /// [`harvest`], with the assertion that must hold on every fixture.
    ///
    /// Every test here goes through this rather than through `harvest`, so P6 is
    /// checked on every output this module has ever produced rather than on the
    /// one somebody remembered to check. The same shape as
    /// `stall::tests::detect_checked`, for the same reason.
    fn harvest_checked(run: &RunHarvest, agenda: &mut Vec<AgendaItem>) -> Harvest {
        let h = harvest(run, agenda);
        assert_eq!(
            approval_violation(&h),
            None,
            "the harvest decided something a human was owed: a session never approves its own \
             proposals"
        );
        h
    }

    /// The approval this harvest claimed, if it claimed one.
    ///
    /// Split from the assertion so `the_guard_would_catch_a_session_that_
    /// approved_something` can watch it fire. Two nets, as in `stall`: the
    /// structural one is that every proposal comes back as an [`AwaitingHuman`]
    /// whose digest row is `awaiting`, and the key scan is what would catch a
    /// field added to that type later.
    fn approval_violation(h: &Harvest) -> Option<String> {
        for a in &h.needs_you {
            if !a.digest_record().awaiting {
                return Some("awaiting = false".to_string());
            }
            let json = serde_json::to_value(a).expect("a waiting proposal serialises");
            if let Some(key) = decision_key(&json) {
                return Some(key);
            }
        }
        // And the item the run was claimed for is never left approved: an
        // approval is `supervised` autonomy's human saying yes (§3), and a run
        // finishing is not that.
        if h.item == ItemStatus::Approved {
            return Some("item approved".to_string());
        }
        for item in h.added() {
            if item.status != ItemStatus::Proposed {
                return Some(format!("offered item is {}", item.status.word()));
            }
        }
        None
    }

    /// The first key anywhere in a JSON value that reads as a decision.
    fn decision_key(v: &serde_json::Value) -> Option<String> {
        const FORBIDDEN: &[&str] = &[
            "approved",
            "approval",
            "auto_approved",
            "accepted",
            "decided",
            "decision",
            "sign_off",
        ];
        match v {
            serde_json::Value::Object(map) => {
                for (k, value) in map {
                    if FORBIDDEN.contains(&k.as_str()) {
                        return Some(k.clone());
                    }
                    if let Some(found) = decision_key(value) {
                        return Some(found);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => items.iter().find_map(decision_key),
            _ => None,
        }
    }

    #[test]
    fn the_guard_would_catch_a_session_that_approved_something() {
        let planted = serde_json::json!({
            "proposal": {"id": "p_1", "ring": "paper", "approved": true},
            "run": "…"
        });
        assert_eq!(
            decision_key(&planted),
            Some("approved".to_string()),
            "the guard every fixture runs through does not actually catch anything"
        );
        assert_eq!(decision_key(&serde_json::json!({"awaiting": true})), None);
    }

    #[test]
    fn a_proposal_comes_back_as_something_a_human_still_owes_a_decision() {
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "supported")
            .proposing(RunProposal::new(
                "p_1",
                "promote_strategy",
                "promote mom_btc_4h to the registry",
                Ring::Artifacts,
            ))
            .proposing(RunProposal::new(
                "p_2",
                "deploy_paper",
                "run mom_btc_4h on paper for 30 days",
                Ring::Paper,
            ));
        let h = harvest_checked(&run, &mut agenda);

        assert_eq!(h.needs_you.len(), 2, "both rings go to Needs you");
        for a in &h.needs_you {
            assert_eq!(a.run(), run_id());
            assert_eq!(a.item(), item.id);
            let row = a.digest_record();
            assert!(row.awaiting, "{} was not left waiting", row.id);
            assert!(
                [1, 2].contains(&row.ring),
                "a proposal reached the digest in ring {}",
                row.ring
            );
        }
        assert_eq!(h.needs_you[0].proposal().kind, "promote_strategy");
        assert_eq!(h.needs_you[1].proposal().ring, Ring::Paper);
    }

    #[test]
    fn the_only_rings_a_proposal_can_carry_are_the_two_that_are_proposed_into() {
        assert_eq!(Ring::parse(1), Ok(Ring::Artifacts));
        assert_eq!(Ring::parse(2), Ok(Ring::Paper));
        for got in [-1_i16, 0, 3, 4] {
            assert_eq!(
                Ring::parse(got),
                Err(HarvestError::NotAProposableRing { got }),
                "ring {got} was accepted"
            );
        }
        assert_eq!(Ring::Artifacts.number(), 1);
        assert_eq!(Ring::Paper.number(), 2);
        // Ring 3 is live, and the refusal says why rather than just refusing.
        assert!(HarvestError::NotAProposableRing { got: 3 }
            .to_string()
            .contains("unreachable"));
    }

    #[test]
    fn an_open_question_becomes_an_item_that_opens_the_question() {
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "inconclusive")
            .opening(OpenQuestion::new(
                "q_41",
                "does the edge hold under 2× slippage?",
                "BTC-USD",
            ))
            .opening(
                OpenQuestion::new("q_42", "is the 4h bar series complete?", "BTC-USD").blocking(),
            );
        let h = harvest_checked(&run, &mut agenda);

        let added: Vec<&AgendaItem> = h.added().collect();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].sources, vec![Source::OpenQuestion]);
        assert_eq!(
            added[1].sources,
            vec![Source::OpenQuestionBlocking],
            "a blocking question comes in heavier, which is §5.1's table rather than a flag"
        );
        assert_eq!(
            added[0].source_refs,
            vec!["q_41".to_string()],
            "the card has to open the memory row"
        );
        assert_eq!(added[0].workflow, "investigate", "§4.3's default");
        assert_eq!(agenda.len(), 3, "both reached the agenda");
    }

    #[test]
    fn only_a_next_move_the_run_marked_worth_pursuing_becomes_an_item() {
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "refuted")
            .suggesting(NextQuestion::new("try it on ETH-USD", "ETH-USD").worth_pursuing())
            .suggesting(NextQuestion::new("try it on every altcoin", "alts"))
            .suggesting(NextQuestion::new("   ", "BTC-USD").worth_pursuing());
        let h = harvest_checked(&run, &mut agenda);

        let added: Vec<&AgendaItem> = h.added().collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].question, "try it on ETH-USD");
        assert_eq!(
            added[0].sources,
            vec![Source::RefinementFollowup],
            "everything a run suggests is follow-up, so the degenerating rule can see all of it"
        );
        assert_eq!(added[0].source_refs, vec![format!("run:{}", run_id())]);

        // Nothing vanished: both of the others are accounted for by name.
        assert_eq!(h.skipped.len(), 2);
        assert_eq!(h.skipped[0].what, "try it on every altcoin");
        assert_eq!(h.skipped[0].why, SkipReason::NotWorthPursuing);
        assert_eq!(h.skipped[1].why, SkipReason::Blank);
        assert_eq!(SkipReason::NotWorthPursuing.as_str(), "not_worth_pursuing");
    }

    #[test]
    fn a_harvested_question_goes_through_layer_one_rather_than_round_it() {
        // A contradiction already wants this question. The run suggesting it
        // must extend that item, not open a second one — and the merge is layer
        // one's, not a comparison written here.
        let item = claimed();
        let wanted = AgendaItem::new(
            session(),
            "Does the edge hold under 2× slippage?",
            "btc usd",
            Source::Contradiction,
        );
        let mut agenda = vec![item.clone(), wanted.clone()];
        let run = finished(&item, "supported").opening(OpenQuestion::new(
            "q_41",
            "does the edge hold under 2× slippage?",
            "BTC-USD",
        ));
        let h = harvest_checked(&run, &mut agenda);

        assert_eq!(
            wanted.dedupe_key,
            dedupe_key("does the edge hold under 2× slippage?", "BTC-USD"),
            "the two wordings have to actually key the same for this to be testing layer one"
        );
        assert_eq!(h.added().count(), 0);
        let merged: Vec<(Uuid, Source)> = h.merged().collect();
        assert_eq!(merged, vec![(wanted.id, Source::OpenQuestion)]);
        assert_eq!(agenda.len(), 2, "no second row");
        assert_eq!(
            agenda[1].sources,
            vec![Source::Contradiction, Source::OpenQuestion],
            "a question two sources want outranks one that one source wants"
        );
    }

    #[test]
    fn two_dossier_entries_asking_the_same_thing_are_one_item() {
        // The second offer has to see the first, which it only does if the
        // agenda grows as the harvest goes.
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "supported")
            .opening(OpenQuestion::new("q_41", "does it hold on ETH?", "ETH-USD"))
            .suggesting(NextQuestion::new("Does it hold on ETH", "eth usd").worth_pursuing());
        let h = harvest_checked(&run, &mut agenda);

        assert_eq!(h.added().count(), 1);
        assert_eq!(h.merged().count(), 1);
        assert_eq!(agenda.len(), 2);
        assert_eq!(
            agenda[1].sources,
            vec![Source::OpenQuestion, Source::RefinementFollowup]
        );
    }

    #[test]
    fn a_run_that_suggests_its_own_question_again_does_not_open_a_second_item() {
        // §6.5's order is the load-bearing part: the offers are made while the
        // item is still `running`, so layer one merges onto it. Marked `done`
        // first, the same question would have been inserted as a fresh item —
        // `offer` merges onto non-terminal items only — and the session would
        // have queued up the question it had just answered.
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "inconclusive").suggesting(
            NextQuestion::new("Does momentum survive costs on BTC-USD?", "btc usd")
                .worth_pursuing(),
        );
        let h = harvest_checked(&run, &mut agenda);

        assert_eq!(
            dedupe_key("Does momentum survive costs on BTC-USD?", "btc usd"),
            agenda[0].dedupe_key,
            "the fixture has to actually be the same question for this to be testing anything"
        );
        assert_eq!(h.added().count(), 0);
        assert_eq!(
            h.merged().collect::<Vec<_>>(),
            vec![(item.id, Source::RefinementFollowup)]
        );
        assert_eq!(agenda.len(), 1);
        assert_eq!(
            agenda[0].status,
            ItemStatus::Done {
                verdict: "inconclusive".into()
            },
            "and it is still marked done afterwards"
        );
    }

    #[test]
    fn a_verdict_settles_the_item_and_a_failure_re_queues_it() {
        for verdict in ["supported", "refuted", "inconclusive"] {
            assert_eq!(
                outcome(verdict, 0),
                ItemStatus::Done {
                    verdict: verdict.to_string()
                }
            );
        }
        for verdict in FAILURE_VERDICTS {
            assert!(is_failure(verdict));
            assert_eq!(outcome(verdict, 0), ItemStatus::Queued, "the first failure");
            match outcome(verdict, 1) {
                ItemStatus::Expired { why } => assert!(
                    why.contains(verdict) && why.contains(&MAX_ATTEMPTS.to_string()),
                    "the digest has to be able to say why: {why}"
                ),
                other => panic!("the second failure left the item {}", other.word()),
            }
        }
        assert!(!is_failure("supported"));
    }

    #[test]
    fn a_failed_run_puts_the_item_back_with_its_attempt_counted() {
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let run = finished(&item, "fenced");
        let h = harvest_checked(&run, &mut agenda);
        assert_eq!(h.item, ItemStatus::Queued);
        assert_eq!(agenda[0].status, ItemStatus::Queued);
        assert_eq!(agenda[0].attempts, 1);
        assert!(
            agenda[0].status.is_claimable(),
            "a re-queued item is one the driver may claim again"
        );

        // Second time round it expires rather than going back on the board.
        let mut agenda = vec![{
            let mut i = item.clone();
            i.attempts = 1;
            i.status = ItemStatus::Running { run_id: run_id() };
            i
        }];
        let run = finished(&item, "fenced").after_attempts(1);
        let h = harvest_checked(&run, &mut agenda);
        assert_eq!(h.item.word(), "expired");
        assert_eq!(agenda[0].attempts, MAX_ATTEMPTS);
        assert!(agenda[0].status.is_terminal());
    }

    #[test]
    fn a_harvest_of_a_run_whose_item_is_not_on_this_agenda_still_says_what_it_became() {
        // The driver may hold a filtered agenda. Nothing is applied, but the
        // status is still returned, because the `run_finished` event carries it
        // and an event with no verdict in it is a hole in the fold.
        let item = claimed();
        let mut agenda: Vec<AgendaItem> = Vec::new();
        let h = harvest_checked(&finished(&item, "supported"), &mut agenda);
        assert_eq!(
            h.item,
            ItemStatus::Done {
                verdict: "supported".into()
            }
        );
        assert!(agenda.is_empty());
    }

    #[test]
    fn a_run_that_brought_nothing_back_harvests_to_nothing() {
        let item = claimed();
        let mut agenda = vec![item.clone()];
        let h = harvest_checked(&finished(&item, "refuted"), &mut agenda);
        assert!(h.inserted.is_empty());
        assert!(
            h.needs_you.is_empty(),
            "a refuted run proposing nothing owes nobody a decision"
        );
        assert!(h.skipped.is_empty());
        assert_eq!(agenda.len(), 1);
    }
}
