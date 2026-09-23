//! `ResearchSession` — the object behind Research's **Continuous** toggle
//! (AGENT-014, ADR-0037).
//!
//! The user sees two modes, Chat and Research, and under Research a toggle.
//! There is no third mode and nothing here is called a program. Internally there
//! is one object: a session with `continuous` set or not. A bounded Research run
//! **is** a session — `continuous = false`, one agenda item, one run — so every
//! Research conversation has the same shape and the desk, the digest and the
//! agenda have one thing to read rather than two.
//!
//! This crate holds the parts that are decisions rather than plumbing, so they
//! can be tested without a database, a job service or a model:
//!
//! * [`charter`] — what the user writes instead of a question, with the
//!   exploration floor as a type;
//! * [`agenda`] — the items, their sources, and layer one of the three-layer
//!   duplicate refusal;
//! * [`fold`] — the append-only log and the replay that is the session's only
//!   state;
//! * [`select`] — the score, the forced exploration share, and the candidate set
//!   a decision row records;
//! * [`stall`] — the four rules that notice a programme has stopped producing
//!   novel results, and the amendments it may suggest;
//! * [`digest`] — the one thing a human reads on coming back, every line of it
//!   carrying the id of what it names;
//! * [`schedule`] — whether the session may claim now, when it may claim next,
//!   and when the child running now has to drain;
//! * [`harvest`] — what a finished run puts back: the questions it becomes, and
//!   the proposals it hands to a human rather than to itself;
//! * [`execute`] — the other end of the same run: the brief it is handed, the
//!   lease it may spend, and what the session does this iteration;
//! * [`couplings`] — the three things outside the session that stop it;
//! * [`sources`] — the five subscriptions that put work on the agenda without
//!   anybody asking;
//! * [`replenish`] — what a drained agenda does when the endless setting is on.
//!
//! The driver that turns a fold into work lives in `api::session_driver`,
//! because it needs the job service and the conversation front door. What it may
//! do is decided here.

pub mod agenda;
pub mod charter;
pub mod couplings;
pub mod digest;
pub mod execute;
pub mod fold;
pub mod harvest;
pub mod replenish;
pub mod schedule;
pub mod select;
pub mod sources;
pub mod stall;

pub use agenda::{
    AgendaItem, DedupeLayer, DedupeScorecard, Insert, ItemStatus, LayerCount, Score, Source,
};
pub use charter::{
    admit, Admitted, Autonomy, Charter, CharterError, ExplorationShare, Judgement, Objective,
    Profile, ReviewEvery, Schedule, Scope, Seed, SessionBudget, StopWhen, TierRefusal, Window,
    EXPLORATION_FLOOR,
};
pub use couplings::{evaluate as evaluate_couplings, PauseReason, Signals, Verdict};
pub use digest::{Digest, DigestInput, Heading, Line, Period, Ref, RefKind, Section, SpendUnit};
pub use execute::{
    brief, day_bound, day_budget, lease, plan, redirect, stop, total_bound, Bound, Brief,
    BriefInput, CapKind, DayBudget, HoldReason, Lease, LeaseRefusal, LeaseRequest, Meter, Redirect,
    SiblingRun, Situation, Step, StopVerdict, Stopping, TrialCap,
};
pub use fold::{fold, Anomaly, EventKind, Folded, SessionEvent, SessionState};
pub use replenish::{absorb, on_drain, Drain, Replenished, SurveyReport};
pub use select::{
    novelty_from, precheck_factor, Declared, Inputs, PolicyField, PrecheckFactor, Provenance,
    Scored, Selection, SelectionPolicy, Signal,
};
pub use sources::{ingest, Fired, Ingest, World};
pub use stall::{detect, Amendment, Diagnosis, FinishedRun, Precheck, Rule, Tripped};

use uuid::Uuid;

/// A session, as the row holds it (AGENT-014 §2.1).
///
/// `state` is deliberately absent: it is [`fold`] of the log, and a column
/// holding a second copy of it is the thing ADR-P2-04 exists to prevent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResearchSession {
    pub id: Uuid,
    pub conversation_id: Uuid,
    /// The toggle. Everything else in this struct that is `Option` is optional
    /// *because of* this field, which is why [`Self::validate`] is the thing
    /// that checks the pairing rather than each field checking itself.
    pub continuous: bool,
    /// `continuous = false`: the one question this session is.
    pub question: Option<String>,
    /// `continuous = true`: what it is chartered to do.
    pub charter: Option<Charter>,
    pub budget: SessionBudget,
    pub schedule: Schedule,
    pub autonomy: Autonomy,
    /// "Keep generating questions" — the endless setting. Endless means
    /// replenish, not unbounded: the budget still ends it.
    pub replenish: bool,
    pub review_every: ReviewEvery,
    /// The profile the session runs on. Continuous requires a frontier profile
    /// (AGENT-014 §8); a bounded session may be any tier.
    pub tier: String,
}

/// What a session can be refused for at creation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error(
        "a continuous session needs a charter and a bounded one needs a question; this one has \
         {got}"
    )]
    WrongShape { got: String },
    #[error(transparent)]
    Charter(#[from] CharterError),
}

impl ResearchSession {
    /// A bounded Research run: one question, one item, one run.
    ///
    /// Built through the same constructor as a continuous session so that "every
    /// Research conversation gets a session row" is not a second code path that
    /// can be forgotten. The defaults it takes — `notify`, one run at a time, no
    /// windows — are inert for a bounded session and are still written, because
    /// a row whose shape depends on which constructor made it is a row two
    /// readers disagree about.
    #[must_use]
    pub fn bounded(id: Uuid, conversation_id: Uuid, question: &str, tier: &str) -> Self {
        Self {
            id,
            conversation_id,
            continuous: false,
            question: Some(question.to_string()),
            charter: None,
            budget: SessionBudget::default(),
            schedule: Schedule::default(),
            autonomy: Autonomy::default(),
            replenish: false,
            review_every: ReviewEvery::default(),
            tier: tier.to_string(),
        }
    }

    /// A continuous session under a charter.
    #[must_use]
    pub fn continuous(id: Uuid, conversation_id: Uuid, charter: Charter, tier: &str) -> Self {
        Self {
            id,
            conversation_id,
            continuous: true,
            question: None,
            charter: Some(charter),
            budget: SessionBudget::default(),
            schedule: Schedule::default(),
            autonomy: Autonomy::default(),
            replenish: false,
            review_every: ReviewEvery::default(),
            tier: tier.to_string(),
        }
    }

    /// The pairing rule, which the table also CHECKs.
    ///
    /// # Errors
    ///
    /// Refuses a continuous session with no charter, a bounded one with no
    /// question, and either one carrying both.
    pub fn validate(&self) -> Result<(), SessionError> {
        let has_q = self.question.is_some();
        let has_c = self.charter.is_some();
        let ok = if self.continuous { has_c } else { has_q };
        if ok && !(has_q && has_c) {
            return Ok(());
        }
        Err(SessionError::WrongShape {
            got: format!(
                "continuous = {}, question {}, charter {}",
                self.continuous,
                if has_q { "set" } else { "absent" },
                if has_c { "set" } else { "absent" }
            ),
        })
    }

    /// The exploration share this session selects under.
    ///
    /// A bounded session does not select — it has one item — so this is the
    /// charter's share or the floor, never zero. Returning zero for a bounded
    /// session would make "the share is never below the floor" false in a place
    /// nobody was looking.
    #[must_use]
    pub fn exploration_share(&self) -> f64 {
        self.charter
            .as_ref()
            .map_or(EXPLORATION_FLOOR, |c| c.exploration_share.get())
    }

    /// The first agenda item a bounded session is.
    #[must_use]
    pub fn sole_item(&self) -> Option<AgendaItem> {
        let q = self.question.as_ref()?;
        Some(AgendaItem::new(self.id, q, "", Source::User))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounded_session_is_one_item_from_the_user() {
        let s = ResearchSession::bounded(Uuid::new_v4(), Uuid::new_v4(), "does x work?", "opus");
        assert!(s.validate().is_ok());
        let item = s.sole_item().expect("a bounded session has its question");
        assert_eq!(item.sources, vec![Source::User]);
        assert!(item.bypasses_scoring());
        assert_eq!(item.status, ItemStatus::Proposed);
    }

    #[test]
    fn the_shape_rule_refuses_both_halves_and_neither() {
        let mut s = ResearchSession::bounded(Uuid::new_v4(), Uuid::new_v4(), "q", "opus");
        s.continuous = true;
        assert!(s.validate().is_err(), "continuous with only a question");

        let mut s = ResearchSession::continuous(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Charter {
                scope: Scope::default(),
                objective: Objective::BuildKnowledge { about: vec![] },
                seeds: vec![],
                permitted_workflows: vec![],
                exclusions: vec![],
                standing_constraints: vec![],
                exploration_share: ExplorationShare::default(),
                stop_when: vec![],
                delta_practical_default: None,
                version: 1,
            },
            "opus",
        );
        assert!(s.validate().is_ok());
        s.question = Some("and also this".into());
        assert!(s.validate().is_err(), "both a question and a charter");
    }

    #[test]
    fn a_bounded_session_still_reports_a_share_at_or_above_the_floor() {
        let s = ResearchSession::bounded(Uuid::new_v4(), Uuid::new_v4(), "q", "opus");
        assert!(s.exploration_share() >= EXPLORATION_FLOOR);
    }
}
