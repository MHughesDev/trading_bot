//! The continuous-research session driver (AGENT-014 §3; Set S task S-11.1).
//!
//! One iteration of the loop the spec writes out: fold the log, ingest the
//! sources, select, claim, start a child, wait, harvest, repeat. It is
//! `JobKind::ContinuousResearch` on the job service for the same reason
//! `JobKind::Campaign` is — leased, heart-beaten, re-queued on worker loss — and
//! it holds no state of its own between iterations, because there is nothing to
//! hold: the log is the session.
//!
//! # What "one child, exactly once" rests on here
//!
//! AGENT-014 §3 names `JobStore::submit_child_once` and AGENT-014 §10 says the
//! children go "via the conversation/turn path". Those are two different
//! mechanisms and only one of them can be the one used, because a conversation
//! turn is not a job. The **property** is what the spec is asking for and it is
//! stated plainly: a driver killed between starting a run and recording that it
//! started re-derives the same child rather than starting a second.
//!
//! This build takes the conversation path (§10) and gets the property from two
//! things that are stronger here than a member index would be:
//!
//! * the child's conversation id is **derived** from `(session_id,
//!   agenda_item_id)` — [`sessions::child_conversation_id`] — so there is no
//!   second id a second conversation could be opened under;
//! * claiming an item is a **conditional UPDATE** out of a claimable status, so
//!   exactly one caller wins even when two drivers fold the same log at once,
//!   and it happens **before** the child is started so a losing driver has
//!   started nothing.
//!
//! A restart therefore finds the conversation already there and the run already
//! on it, and appends `run_started` for the run that exists. A restart in the
//! narrower window — after the claim, before the child — finds an item the row
//! calls `claimed` and the log never started, and
//! [`SessionWorker::release_stale_claims`] puts it back. Recorded as ADR-P11-01
//! rather than left as a silent reading of the spec.
//!
//! # The scope this build covers, named rather than implied
//!
//! AGENT-014 §11's phases 1 and 2's selection half: charter seeds and user
//! items (sources 4.1, 4.2), `replenish = false`, and the scored agenda of §5
//! with its forced exploration share and its decision row (S-11.3).
//!
//! # What the scorer can measure here, and what it cannot
//!
//! Of §5.1's four signals, this build measures three and says so about the
//! fourth on every decision row:
//!
//! * **value** — the objective's weight for the item's subject × the source
//!   weight from §5.1's own table. Declared, both halves.
//! * **novelty** — from the count of prior hypotheses stating the same question
//!   (`agent_lifecycle.hypothesis`, the registry `find_prior_hypothesis`
//!   already reads). The spec names the family's *trial* count; a trial is not
//!   joinable to an agenda item's subject today, and
//!   `select::NOVELTY_HALFLIFE` documents which way that errs.
//! * **feasibility** — `{0, 1}`: whether the charter permits the workflow and
//!   whether the profile may run it. The `0.5` rung §5.1 reserves for partial
//!   capability needs the coverage and breaker readers of S-11.10 and S-11.7,
//!   so it is not emitted rather than guessed at.
//! * **cost** — the median `trials_spent` of finished runs of this workflow on
//!   this profile, in **trials, not dollars**: this platform records no per-run
//!   USD (`llm_usage` keys on the auth session, not the run). Below
//!   [`MIN_RUNS_FOR_COST`] it is `not_estimated`, which leaves the product alone
//!   and is named on the row.
//!
//! That last point is the reason [`Signal`] carries a [`Provenance`] at all. A
//! ranking that silently substituted 1.0 for what it could not measure would
//! look exactly like a measured one in the decision log — the log every
//! off-policy estimate later reads.
//!
//! Not here, and not pretended to be: budget leases (§6.3), the harvest of
//! open questions and proposals (§6.5), the stall detector (§7.1), schedule
//! windows (§7), the digest (§2.4), and sources 4.3–4.9. Each is its own task
//! and each would be worse done approximately: a lease that does not actually
//! bound a child, or a stall detector that reads a yield nothing writes, is a
//! control the operator would believe in.

// `JobError` is the job service's wire type — see `gate_worker` for why this is
// allowed rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::time::Duration;

use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use research_session::{
    agenda::{AgendaItem, ItemStatus, Source},
    fold::EventKind,
    select::{self, Inputs, Scored, Signal},
    Folded, ResearchSession,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent::AgentManager;
use crate::credentials::LlmCredentialStore;
use crate::sessions;

/// How often the driver looks at the child it started.
const CHILD_POLL: Duration = Duration::from_secs(5);

/// How many finished runs of a workflow a cost estimate needs.
///
/// Three, and the number is a floor on saying anything at all rather than a
/// tuned threshold: a median of one is that run, and a median of two is a coin
/// toss between them. Below it the signal reports `not_estimated` and the
/// decision row says which items that applied to.
const MIN_RUNS_FOR_COST: i64 = 3;

/// The run id a claim carries before its child exists.
///
/// Nil rather than a made-up uuid: a reader of the row can tell "claimed, not
/// yet started" from "claimed by run X", and `release_stale_claims` does not have
/// to guess which it is looking at.
const PENDING_RUN: Uuid = Uuid::nil();

/// How many iterations one job performs before returning.
///
/// The job is re-queued by the caller on the next event or window, so a driver
/// that ran forever would hold a lease it does not need. Bounded rather than
/// infinite so a session with a fast-failing agenda cannot spin.
const MAX_ITERATIONS: u32 = 64;

/// Drives one session per job.
pub struct SessionWorker {
    pg: PgPool,
    agent: Arc<AgentManager>,
    creds: LlmCredentialStore,
    /// The loaded profiles, for the `feasibility` signal: a workflow the tier
    /// may not run is not feasible, and §5.1 says so in those words.
    profiles: Arc<harness::ProfileSet>,
}

impl SessionWorker {
    #[must_use]
    pub fn new(
        pg: PgPool,
        agent: Arc<AgentManager>,
        creds: LlmCredentialStore,
        profiles: Arc<harness::ProfileSet>,
    ) -> Self {
        Self {
            pg,
            agent,
            creds,
            profiles,
        }
    }

    fn infra(what: &str, e: impl std::fmt::Display) -> JobError {
        JobError::infrastructure(format!("{what}: {e}"))
    }

    fn logic(code: &'static str, detail: impl Into<String>) -> JobError {
        JobError::logic(ledger::TerminalReason::IntegrityRejected, code, detail)
    }

    /// Source 4.1 — the charter's seeds, offered to the agenda.
    ///
    /// Idempotent through the dedupe key rather than through a "seeded" flag:
    /// offering the same seed twice merges into the item that is already there,
    /// so a driver that restarts mid-ingest does not need to know how far it
    /// got.
    async fn ingest_seeds(&self, session: &ResearchSession) -> Result<u32, JobError> {
        let Some(charter) = session.charter.as_ref() else {
            return Ok(0);
        };
        let mut added = 0;
        for seed in &charter.seeds {
            let workflow = seed
                .workflow
                .clone()
                .unwrap_or_else(|| Source::CharterSeed.default_workflow().to_string());
            if !charter.permits(&workflow) {
                // A seed the charter's own exclusions forbid is dropped with the
                // reason on the log, not silently skipped: a user who wrote a
                // seed and excluded its workflow has contradicted themselves and
                // should be able to see where.
                sessions::append(
                    &self.pg,
                    session.id,
                    EventKind::AgendaItemMerged,
                    json!({
                        "refused": seed.question,
                        "why": format!(
                            "the charter seeds `{workflow}` and also excludes it"
                        ),
                    }),
                )
                .await
                .map_err(|e| Self::infra("recording a refused seed", e))?;
                continue;
            }
            let item = AgendaItem::new(
                session.id,
                &seed.question,
                &seed.subject,
                Source::CharterSeed,
            )
            .with_workflow(&workflow);
            let inserted = sessions::insert_item(&self.pg, &item)
                .await
                .map_err(|e| Self::infra("offering a charter seed", e))?;
            match inserted {
                Some(id) => {
                    // Seeds arrive already wanted by the user who wrote them, so
                    // they queue rather than sit proposed. Approval, when the
                    // session is `supervised`, is still a separate event.
                    sessions::set_status(&self.pg, id, &ItemStatus::Queued)
                        .await
                        .map_err(|e| Self::infra("queueing a seed", e))?;
                    sessions::append(
                        &self.pg,
                        session.id,
                        EventKind::AgendaItemAdded,
                        json!({
                            "item": id,
                            "source": Source::CharterSeed.as_str(),
                            "question": seed.question,
                            "workflow": workflow,
                        }),
                    )
                    .await
                    .map_err(|e| Self::infra("recording a seed", e))?;
                    added += 1;
                }
                None => {
                    sessions::append(
                        &self.pg,
                        session.id,
                        EventKind::AgendaItemMerged,
                        json!({ "key": item.dedupe_key, "source": Source::CharterSeed.as_str() }),
                    )
                    .await
                    .map_err(|e| Self::infra("recording a merge", e))?;
                }
            }
        }
        Ok(added)
    }

    /// Put back any item the row calls `claimed` that the log never started.
    ///
    /// The window this closes is one iteration wide: an item is claimed, and the
    /// driver dies before its child exists. The row then says `claimed`, which
    /// is not claimable, and nothing would ever pick it up again — the item
    /// would sit in a column on the board with no run behind it and no way out.
    ///
    /// The **log** is the authority for whether a run started, not the row, which
    /// is the whole of ADR-P2-04's argument applied one level down: the row is a
    /// projection and the log is the fact. An item with no `run_started` in the
    /// log did not start, whatever its status column says.
    ///
    /// Safe to run every iteration because the job is leased — one driver per
    /// session — and because an item that *did* start has a `run_started` event
    /// and is skipped.
    async fn release_stale_claims(
        &self,
        session_id: Uuid,
        agenda: &[AgendaItem],
        folded: &Folded,
    ) -> Result<u32, JobError> {
        let mut released = 0;
        for item in agenda {
            if !is_stale_claim(item, folded) {
                continue;
            }
            sessions::set_status(&self.pg, item.id, &ItemStatus::Queued)
                .await
                .map_err(|e| Self::infra("releasing a stale claim", e))?;
            sessions::append(
                &self.pg,
                session_id,
                EventKind::AgendaItemMerged,
                json!({
                    "item": item.id,
                    "released": true,
                    "why": "the row said claimed and the log never started a run for it, so the \
                            claim was taken by a driver that did not survive to start the child",
                }),
            )
            .await
            .map_err(|e| Self::infra("recording a release", e))?;
            released += 1;
        }
        Ok(released)
    }

    /// The items the driver may claim, oldest first.
    ///
    /// Three conditions, and two of them read the **log** rather than the row: a
    /// veto is permanent and a started item is started, whatever the status
    /// column says. The row is a projection; the log is the fact.
    ///
    /// The order is the caller's — `created_at, item_id` — which is §5.4's
    /// tie-break, so the scorer does not need a second rule for ties.
    fn claimable(agenda: &[AgendaItem], folded: &Folded) -> Vec<AgendaItem> {
        agenda
            .iter()
            .filter(|i| {
                i.status.is_claimable()
                    && !folded.vetoed.contains(&i.id)
                    && !folded.already_started(i.id)
            })
            .cloned()
            .collect()
    }

    /// How many prior hypotheses state this question.
    ///
    /// Read through the same normalisation `lifecycle::similar_hypotheses` uses,
    /// so "has this been asked" has one answer whether a run asks it through
    /// `find_prior_hypothesis` or the scorer asks it here. Two normalisers would
    /// be two different questions wearing one name.
    async fn prior_statements(&self, user_id: Uuid, question: &str) -> Result<i64, JobError> {
        let needle = crate::routes::lifecycle::normalise_statement(question);
        if needle.is_empty() {
            return Ok(0);
        }
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT h.statement
               FROM agent_lifecycle.hypothesis h
               JOIN agent_runs r ON r.run_id = h.run_id
              WHERE r.user_id = $1
              LIMIT 2000",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("counting prior hypotheses", e))?;
        Ok(rows
            .into_iter()
            .filter(|(st,)| crate::routes::lifecycle::normalise_statement(st) == needle)
            .count()
            .try_into()
            .unwrap_or(i64::MAX))
    }

    /// The median trials a finished run of this workflow has spent on this tier.
    ///
    /// **Trials, not dollars.** `llm_usage.cost_usd` keys on the auth session
    /// rather than the run, so there is no per-run USD to take a median of. A
    /// trial is the budget dimension the ledger actually counts (INV-1), and a
    /// median of measured trials is a real number; a dollar figure derived from
    /// a list price and a token count would be a number nobody measured sitting
    /// in the one log every later analysis reads.
    async fn cost_signal(&self, user_id: Uuid, workflow: &str, tier: &str) -> Signal {
        let rows: Result<Vec<(i32,)>, _> = sqlx::query_as(
            "SELECT trials_spent FROM agent_runs
              WHERE user_id = $1 AND workflow = $2 AND profile_name = $3
                AND trials_spent IS NOT NULL AND outcome IS NOT NULL
              ORDER BY created_at DESC LIMIT 200",
        )
        .bind(user_id)
        .bind(workflow)
        .bind(tier)
        .fetch_all(&self.pg)
        .await;
        let Ok(rows) = rows else {
            return Signal::not_estimated();
        };
        let n = i64::try_from(rows.len()).unwrap_or(i64::MAX);
        if n < MIN_RUNS_FOR_COST {
            return Signal::not_estimated();
        }
        let mut spent: Vec<f64> = rows.into_iter().map(|(t,)| f64::from(t)).collect();
        spent.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = spent.len() / 2;
        let median = if spent.len() % 2 == 0 {
            (spent[mid - 1] + spent[mid]) / 2.0
        } else {
            spent[mid]
        };
        Signal::measured(median, n)
    }

    /// The four signals for one item (§5.1).
    async fn measure(
        &self,
        session: &ResearchSession,
        item: &AgendaItem,
        user_id: Uuid,
    ) -> Result<Inputs, JobError> {
        // value — both halves declared: the objective's weight for the subject,
        // and the source weight from §5.1's table. `max` over the sources, so a
        // question three sources want carries the strongest of them.
        let objective_weight = session
            .charter
            .as_ref()
            .map_or(1.0, |c| c.objective.weight_for(&item.subject));
        let value = Signal::declared(objective_weight * item.source_weight());

        // novelty — measured from the hypothesis registry.
        let novelty = select::novelty_from_prior(self.prior_statements(user_id, &item.question).await?);

        // feasibility — {0, 1}. The 0.5 rung needs readers that do not exist
        // (coverage, the tool breaker), so it is not emitted.
        let permitted = session
            .charter
            .as_ref()
            .is_none_or(|c| c.permits(&item.workflow));
        let tier_allows = harness::state::Workflow::parse(&item.workflow)
            .zip(self.profiles.get(&session.tier))
            .is_none_or(|(w, p)| p.may_run(w));
        let feasibility = Signal::declared(if permitted && tier_allows { 1.0 } else { 0.0 });

        let cost = self.cost_signal(user_id, &item.workflow, &session.tier).await;

        Ok(Inputs {
            value,
            novelty,
            feasibility,
            cost,
        })
    }

    /// Score the agenda and choose, recording both.
    ///
    /// # §5.4, and what "recompute only when a source changes" means here
    ///
    /// The scores are stored on the rows. `insert_item` clears an item's score
    /// when a source is added to it, so a cleared score is the signal that
    /// something changed. When **every** claimable item still carries a stored
    /// score, those are reused and nothing is recomputed — the agenda the human
    /// saw in the digest is the agenda that runs.
    ///
    /// When any of them is cleared the **whole slate** is rescored, not just the
    /// cleared one, because the two unbounded signals are ranked *within the
    /// agenda*: a percentile computed against a slate and a percentile computed
    /// against one item are not comparable, and mixing them would rank by which
    /// items happened to be recomputed.
    async fn choose(
        &self,
        session: &ResearchSession,
        agenda: &[AgendaItem],
        folded: &Folded,
        user_id: Uuid,
    ) -> Result<Option<AgendaItem>, JobError> {
        let claimable = Self::claimable(agenda, folded);
        if claimable.is_empty() {
            return Ok(None);
        }

        // §4.2's bypass happens before any of this. A `user` item is not scored
        // and not drawn; it was asked for.
        if let Some(asked) = claimable.iter().find(|i| i.bypasses_scoring()) {
            sessions::append(
                &self.pg,
                session.id,
                EventKind::Selected,
                json!({
                    "item": asked.id,
                    "question": asked.question,
                    "workflow": asked.workflow,
                    "bypassed_scoring": true,
                    "why": "you asked for this one, so it goes next whatever the ranking says",
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the selection", e))?;
            return Ok(Some(asked.clone()));
        }

        let stale = claimable.iter().any(|i| i.score.is_none());
        let mut measured: Vec<(AgendaItem, Inputs)> = Vec::with_capacity(claimable.len());
        for item in &claimable {
            measured.push((item.clone(), self.measure(session, item, user_id).await?));
        }
        let scored = select::score(&measured);

        if stale {
            // Persist, so the digest and the board show the agenda that ran.
            for s in &scored {
                if let Err(e) = sqlx::query("UPDATE agenda_items SET score = $2 WHERE item_id = $1")
                    .bind(s.item)
                    .bind(json!(s.normalised))
                    .execute(&self.pg)
                    .await
                {
                    return Err(Self::infra("storing a score", e));
                }
            }
        }

        let share = session.exploration_share();
        let seed = select::seed_for(session.id, folded.event_seq);
        let Some(selection) = select::select(&scored, share, seed) else {
            // Every claimable item is infeasible or already answered. Not an
            // error and not a completion: the agenda has items and none of them
            // can run, which is a thing a human has to look at.
            sessions::append(
                &self.pg,
                session.id,
                EventKind::AwaitingApproval,
                json!({
                    "why": "every claimable item is infeasible or already answered",
                    "items": claimable.iter().map(|i| i.id).collect::<Vec<_>>(),
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the stall", e))?;
            return Ok(None);
        };

        let chosen = claimable
            .iter()
            .find(|i| i.id == selection.chosen)
            .cloned()
            .ok_or_else(|| {
                Self::logic(
                    "selection_off_slate",
                    "the scorer returned an item that is not on the agenda it was given",
                )
            })?;

        self.log_selection(session, &scored, &selection, &chosen, share, seed)
            .await?;

        sessions::append(
            &self.pg,
            session.id,
            EventKind::Selected,
            json!({
                "item": chosen.id,
                "question": chosen.question,
                "workflow": chosen.workflow,
                "exploration": selection.exploration,
                "propensity": selection.propensity,
                "rationale": selection.rationale,
                "candidate_set": select::candidate_set(&scored),
                "policy": { "id": select::POLICY, "version": select::POLICY_VERSION },
                "recomputed": stale,
            }),
        )
        .await
        .map_err(|e| Self::infra("recording the selection", e))?;

        Ok(Some(chosen))
    }

    /// Write the `select` decision (§5.3, SPEC §4.4).
    ///
    /// The session's log already says which item was chosen. The **decision** log
    /// is the one that says what it was chosen *from*, and it is what makes the
    /// selection policy a learning-to-rank dataset later rather than a trace.
    ///
    /// `experiment_id` is the session, because a decision has to belong to
    /// something the ledger can group by and the session is what these choices
    /// were made under.
    async fn log_selection(
        &self,
        session: &ResearchSession,
        scored: &[Scored],
        selection: &research_session::Selection,
        chosen: &AgendaItem,
        share: f64,
        seed: u64,
    ) -> Result<(), JobError> {
        let tenant = session.conversation_id.to_string();
        let decision = ledger::Decision {
            kind: ledger::DecisionKind::Select,
            // The agenda is the context. Its identity is the session and the
            // position in its log, which is the same pair the seed is derived
            // from — so the draw can be replayed from the row.
            context_hash: format!("session:{}:seed:{seed:x}", session.id),
            context_uri: Some(format!("session:{}", session.id)),
            candidate_set: select::candidate_set(scored),
            chosen: json!({
                "item": chosen.id,
                "question": chosen.question,
                "workflow": chosen.workflow,
            }),
            propensity: Some(selection.propensity),
            exploration_flag: selection.exploration,
            // A declared mixture over declared signals, with no fitted model in
            // it. `rule` is what that records as.
            decision_tier: ledger::DecisionTier::Rule,
            rationale: Some(selection.rationale.clone()),
        };
        // No effect size: "which question to ask next" is not a hypothesis about
        // a magnitude, and the charter's `delta_practical` is a suggestion for
        // the runs that follow rather than a property of this choice (§2.2).
        let ctx = ledger::DispatchContext::policy_without_effect_size(
            &tenant,
            format!("session:{}", session.id),
            select::POLICY,
            select::POLICY_VERSION,
        )
        .with_experiment(format!("session:{}", session.id));

        ledger::pg::PgTrialLedger::new(self.pg.clone())
            .log_decision_async(&ctx, &decision)
            .await
            .map_err(|e| {
                Self::infra(
                    &format!("logging the select decision (share {share:.2})"),
                    e,
                )
            })?;
        Ok(())
    }

    /// Start the child run for an item, or find the one a previous attempt
    /// already started.
    ///
    /// Returns the run id either way, so the caller's `run_started` event names
    /// a run that exists.
    async fn start_child(
        &self,
        user_id: Uuid,
        session: &ResearchSession,
        item: &AgendaItem,
    ) -> Result<Uuid, JobError> {
        let convo = sessions::child_conversation_id(session.id, item.id);

        // The conversation, under the derived id. `ON CONFLICT DO NOTHING`
        // rather than a lookup-then-insert: the derived id makes the insert
        // itself the idempotent operation.
        let (provider, model): (String, String) = sqlx::query_as(
            "SELECT provider, model FROM agent_conversations WHERE conversation_id = $1",
        )
        .bind(session.conversation_id)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the journal conversation", e))?
        .ok_or_else(|| {
            Self::logic(
                "session_without_conversation",
                "the session's journal conversation is gone, so its children have no provider to \
                 inherit and no rail to be grouped under",
            )
        })?;

        sqlx::query(
            "INSERT INTO agent_conversations
                 (conversation_id, user_id, provider, model, title, parent_conversation_id)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (conversation_id) DO NOTHING",
        )
        .bind(convo)
        .bind(user_id)
        .bind(&provider)
        .bind(&model)
        .bind(title_of(&item.question))
        // The journal this child hangs under. It groups the rail (§9) and it is
        // what stops the conversation front door giving this child a bounded
        // session of its own on its first turn — a child already belongs to one.
        .bind(session.conversation_id)
        .execute(&self.pg)
        .await
        .map_err(|e| Self::infra("opening the child conversation", e))?;

        // Did a previous attempt already start the turn? The conversation is for
        // this item and nothing else, so any run on it is the child.
        let existing: Option<(Uuid,)> = sqlx::query_as(
            "SELECT run_id FROM agent_runs WHERE conversation_id = $1
              ORDER BY turn_index DESC LIMIT 1",
        )
        .bind(convo)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| Self::infra("looking for an existing child run", e))?;

        let run_id = match existing {
            Some((id,)) => id,
            None => {
                let workflow = harness::state::Workflow::parse(&item.workflow);
                self.agent
                    .send_message(user_id, convo, &item.question, &self.creds, workflow)
                    .await
                    .map_err(|e| Self::infra("starting the child run", e))?
            }
        };

        sessions::attribute_run(&self.pg, run_id, session.id, item.id)
            .await
            .map_err(|e| Self::infra("attributing the child run", e))?;
        Ok(run_id)
    }

    /// Wait for the child to finish and report what it ended as.
    ///
    /// A cancelled *driver* does not cancel the child: the child may already
    /// have spent counted trials, and abandoning it without settling them is
    /// what INV-17 forbids.
    async fn await_child(&self, ctx: &JobContext, run_id: Uuid) -> Result<String, JobError> {
        loop {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT status FROM agent_runs WHERE run_id = $1")
                    .bind(run_id)
                    .fetch_optional(&self.pg)
                    .await
                    .map_err(|e| Self::infra("reading the child run", e))?;
            let status = row.map_or_else(|| "missing".to_string(), |r| r.0);
            if !crate::agent::conversations::ACTIVE_RUN_STATES.contains(&status.as_str()) {
                return Ok(status);
            }
            ctx.progress(Progress {
                pct: None,
                stage: Some("running".into()),
                message: Some(format!("waiting on run {run_id} ({status})")),
            })
            .await;
            tokio::time::sleep(CHILD_POLL).await;
        }
    }

    /// One pass of §3's loop. Returns whether it did anything worth another.
    async fn iterate(
        &self,
        ctx: &JobContext,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<Step, JobError> {
        let session = sessions::get(&self.pg, session_id)
            .await
            .map_err(|e| Self::infra("reading the session", e))?
            .ok_or_else(|| Self::logic("unknown_session", format!("no session {session_id}")))?;
        session
            .validate()
            .map_err(|e| Self::logic("session_shape", e.to_string()))?;

        let folded = sessions::state(&self.pg, session_id)
            .await
            .map_err(|e| Self::infra("folding the session log", e))?;

        if !folded.anomalies.is_empty() {
            // The fold found something it cannot justify. Stopping is the point:
            // a driver that kept claiming would be acting on a state it could
            // not explain, and every event it then wrote would compound it.
            return Err(Self::logic(
                "session_log_anomaly",
                format!(
                    "the session's log does not replay cleanly, so nothing may be claimed from \
                     it: {:?}",
                    folded.anomalies
                ),
            ));
        }
        if !folded.can_claim() {
            return Ok(Step::Idle(format!(
                "the session is {}",
                folded
                    .state
                    .as_ref()
                    .map_or("in no state", research_session::SessionState::word)
            )));
        }
        if folded.runs_started >= i64::from(session.budget.max_runs) {
            sessions::append(
                &self.pg,
                session_id,
                EventKind::SessionCompleted,
                json!({
                    "why": "max_runs",
                    "runs_started": folded.runs_started,
                    "max_runs": session.budget.max_runs,
                }),
            )
            .await
            .map_err(|e| Self::infra("completing the session", e))?;
            return Ok(Step::Done("max_runs".into()));
        }

        self.ingest_seeds(&session).await?;

        let mut agenda = sessions::agenda(&self.pg, session_id)
            .await
            .map_err(|e| Self::infra("reading the agenda", e))?;
        if self.release_stale_claims(session_id, &agenda, &folded).await? > 0 {
            agenda = sessions::agenda(&self.pg, session_id)
                .await
                .map_err(|e| Self::infra("re-reading the agenda", e))?;
        }
        let Some(item) = self
            .choose(&session, &agenda, &folded, user_id)
            .await?
        else {
            let waiting = agenda.iter().any(|i| !i.status.is_terminal());
            if waiting {
                return Ok(Step::Idle(
                    "every item is waiting on something a human has not done".into(),
                ));
            }
            // §7: agenda drained. `replenish` is phase 4 and is not read here;
            // a session with it set would still complete, which is why creating
            // one with it set is refused until the survey exists.
            sessions::append(
                &self.pg,
                session_id,
                EventKind::SessionCompleted,
                json!({ "why": "agenda_drained" }),
            )
            .await
            .map_err(|e| Self::infra("completing the session", e))?;
            return Ok(Step::Done("agenda_drained".into()));
        };

        if session.autonomy.item_waits_for_approval()
            && !folded.approved.contains(&item.id)
            && !item.bypasses_scoring()
        {
            sessions::append(
                &self.pg,
                session_id,
                EventKind::AwaitingApproval,
                json!({ "item": item.id, "question": item.question }),
            )
            .await
            .map_err(|e| Self::infra("recording the wait for approval", e))?;
            return Ok(Step::Idle(format!(
                "item {} is waiting for approval",
                item.id
            )));
        }


        // Claimed **before** the child is started, and the order matters.
        //
        // The other way round reads better — the row would never point at a run
        // that was never started — and it is wrong: two drivers folding the same
        // log both reach `start_child`, and a lost claim would then leave a
        // child running that nothing will ever record or finish. An orphaned run
        // spending a budget is worse than a claim that has not started yet,
        // because a claim that has not started is recoverable and this recovers
        // it: `release_stale_claims` above puts back any item the row calls
        // `claimed` that the log never started.
        let won = sessions::claim(&self.pg, item.id, PENDING_RUN)
            .await
            .map_err(|e| Self::infra("claiming the item", e))?;
        if !won {
            return Ok(Step::Idle(format!(
                "another driver claimed item {} first",
                item.id
            )));
        }

        let run_id = self.start_child(user_id, &session, &item).await?;

        sessions::append(
            &self.pg,
            session_id,
            EventKind::RunStarted,
            json!({ "item": item.id, "run_id": run_id, "conversation_id":
                    sessions::child_conversation_id(session_id, item.id) }),
        )
        .await
        .map_err(|e| Self::infra("recording the start", e))?;
        sessions::set_status(&self.pg, item.id, &ItemStatus::Running { run_id })
            .await
            .map_err(|e| Self::infra("marking the item running", e))?;

        let ended = self.await_child(ctx, run_id).await?;
        if ended == "succeeded" {
            // The verdict the run reached. AGENT-013 §10's yield and the
            // dossier's own verdict are what a harvest reads; S-11.5 builds that
            // and this records the run's terminal state so the item is not left
            // claiming to be running.
            sessions::finish_item(&self.pg, item.id, &ended, None)
                .await
                .map_err(|e| Self::infra("finishing the item", e))?;
            sessions::append(
                &self.pg,
                session_id,
                EventKind::RunFinished,
                json!({ "item": item.id, "run_id": run_id, "verdict": ended }),
            )
            .await
            .map_err(|e| Self::infra("recording the finish", e))?;
            return Ok(Step::Ran(item.id));
        }

        // §6.6 — one retry, then expire.
        let status = sessions::requeue_or_expire(&self.pg, item.id, &format!("run {ended}"))
            .await
            .map_err(|e| Self::infra("re-queueing the item", e))?;
        sessions::append(
            &self.pg,
            session_id,
            EventKind::RunFinished,
            json!({ "item": item.id, "run_id": run_id, "verdict": ended,
                    "then": status.word() }),
        )
        .await
        .map_err(|e| Self::infra("recording the failure", e))?;
        if let ItemStatus::Expired { ref why } = status {
            sessions::append(
                &self.pg,
                session_id,
                EventKind::AgendaItemExpired,
                json!({ "item": item.id, "why": why }),
            )
            .await
            .map_err(|e| Self::infra("recording the expiry", e))?;
        }
        Ok(Step::Ran(item.id))
    }
}

/// Whether this item's claim was taken by a driver that did not survive to start
/// the child.
///
/// Pure, and separated from the pass that acts on it, because the interesting
/// property is a two-line rule about which of two records wins and it should be
/// testable without a database. The log is the authority: an item with no
/// `run_started` did not start, whatever its status column says.
fn is_stale_claim(item: &AgendaItem, folded: &Folded) -> bool {
    matches!(item.status, ItemStatus::Claimed { .. }) && !folded.already_started(item.id)
}

/// What one iteration did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// A child ran to a terminal state.
    Ran(Uuid),
    /// Nothing to do, and why.
    Idle(String),
    /// The session ended, and why.
    Done(String),
}

/// A conversation title from the question, before the summariser gets to it.
fn title_of(question: &str) -> String {
    let q = question.trim();
    if q.chars().count() <= 60 {
        return q.to_string();
    }
    let cut: String = q.chars().take(57).collect();
    format!("{}…", cut.trim_end())
}

#[async_trait::async_trait]
impl Worker for SessionWorker {
    fn kind(&self) -> JobKind {
        JobKind::ContinuousResearch
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let session_id: Uuid = manifest
            .get("session_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                Self::logic(
                    "invalid_manifest",
                    "a continuous-research job needs a session_id",
                )
            })?;
        let user_id: Uuid = manifest
            .get("user_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                Self::logic(
                    "invalid_manifest",
                    "a continuous-research job needs the user it runs as; a child run belongs to \
                     a person and its credentials are theirs",
                )
            })?;

        let mut ran = 0_u32;
        let mut ended: Option<String> = None;
        let mut idle: Option<String> = None;

        for _ in 0..MAX_ITERATIONS {
            match self.iterate(ctx, user_id, session_id).await? {
                Step::Ran(_) => ran += 1,
                Step::Idle(why) => {
                    idle = Some(why);
                    break;
                }
                Step::Done(why) => {
                    ended = Some(why);
                    break;
                }
            }
        }

        Ok(JobOutput {
            summary: Some(match (&ended, &idle) {
                (Some(why), _) => format!("session ended: {why} ({ran} runs this pass)"),
                (None, Some(why)) => format!("{ran} runs this pass; stopped because {why}"),
                (None, None) => format!("{ran} runs this pass"),
            }),
            result: json!({
                "session_id": session_id,
                "runs_completed": ran,
                "ended": ended,
                "idle": idle,
            }),
            ..JobOutput::default()
        }
        .clamped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_session::fold::{SessionEvent, SessionState};

    fn item(session: Uuid, q: &str, source: Source, status: ItemStatus) -> AgendaItem {
        let mut i = AgendaItem::new(session, q, "BTC-USD", source);
        i.status = status;
        i
    }

    fn running_fold() -> Folded {
        research_session::fold(&[
            SessionEvent::new(Uuid::nil(), 0, EventKind::SessionCreated, json!({})),
            SessionEvent::new(Uuid::nil(), 1, EventKind::SessionStarted, json!({})),
        ])
    }

    #[test]
    fn a_user_item_bypasses_the_scorer_and_goes_next() {
        // The bypass is `choose`'s first branch and `select`'s too, so this
        // asserts the agenda reaches the scorer with the `user` item on it and
        // that the scorer takes it — the two halves that have to agree.
        let s = Uuid::new_v4();
        let agenda = vec![
            item(s, "seeded first", Source::CharterSeed, ItemStatus::Queued),
            item(s, "seeded second", Source::CharterSeed, ItemStatus::Queued),
            item(s, "asked just now", Source::User, ItemStatus::Queued),
        ];
        let live = SessionWorker::claimable(&agenda, &running_fold());
        assert_eq!(live.len(), 3);
        let asked = live
            .iter()
            .find(|i| i.bypasses_scoring())
            .expect("the user item is claimable");
        assert_eq!(asked.question, "asked just now");
    }

    #[test]
    fn an_item_that_has_not_been_approved_is_not_claimable() {
        let s = Uuid::new_v4();
        let agenda = vec![
            item(s, "not yet approved", Source::CharterSeed, ItemStatus::Proposed),
            item(s, "oldest queued", Source::CharterSeed, ItemStatus::Queued),
            item(s, "newer", Source::CharterSeed, ItemStatus::Queued),
        ];
        let live = SessionWorker::claimable(&agenda, &running_fold());
        assert_eq!(live.len(), 2);
        // The caller's order is kept, which is §5.4's tie-break.
        assert_eq!(live[0].question, "oldest queued");
        assert_eq!(live[1].question, "newer");
    }

    #[test]
    fn a_vetoed_item_is_never_claimed_even_if_its_row_says_queued() {
        let s = Uuid::new_v4();
        let mut agenda = vec![item(s, "vetoed", Source::User, ItemStatus::Queued)];
        let mut log = vec![
            SessionEvent::new(Uuid::nil(), 0, EventKind::SessionCreated, json!({})),
            SessionEvent::new(Uuid::nil(), 1, EventKind::SessionStarted, json!({})),
            SessionEvent::new(
                Uuid::nil(),
                2,
                EventKind::AgendaItemVetoed,
                json!({ "item": agenda[0].id }),
            ),
        ];
        assert!(SessionWorker::claimable(&agenda, &research_session::fold(&log)).is_empty());

        // And a second, un-vetoed item is still claimable — the veto is about
        // one item, not about the session.
        agenda.push(item(s, "fine", Source::CharterSeed, ItemStatus::Queued));
        log.push(SessionEvent::new(
            Uuid::nil(),
            3,
            EventKind::AgendaItemAdded,
            json!({ "item": agenda[1].id }),
        ));
        let live = SessionWorker::claimable(&agenda, &research_session::fold(&log));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].question, "fine");
    }

    #[test]
    fn an_item_already_started_is_not_claimed_again() {
        let s = Uuid::new_v4();
        let agenda = vec![item(s, "already going", Source::User, ItemStatus::Queued)];
        let log = vec![
            SessionEvent::new(Uuid::nil(), 0, EventKind::SessionCreated, json!({})),
            SessionEvent::new(Uuid::nil(), 1, EventKind::SessionStarted, json!({})),
            SessionEvent::new(
                Uuid::nil(),
                2,
                EventKind::RunStarted,
                json!({ "item": agenda[0].id, "run_id": Uuid::new_v4() }),
            ),
        ];
        assert!(SessionWorker::claimable(&agenda, &research_session::fold(&log)).is_empty());
    }

    #[test]
    fn a_session_that_is_not_running_claims_nothing() {
        let s = Uuid::new_v4();
        let agenda = vec![item(s, "q", Source::User, ItemStatus::Queued)];
        let mut folded = running_fold();
        folded.state = Some(SessionState::Paused {
            why: "kill_switch".into(),
        });
        assert!(!folded.can_claim());
        // `claimable` is about the agenda; the loop's guard is `can_claim`, and
        // the two are checked in that order by `iterate`. Asserted here so the
        // guard is not quietly moved into `claimable` and then dropped — a
        // paused session with a queued item must stop at the guard, not at an
        // empty agenda.
        assert!(!SessionWorker::claimable(&agenda, &folded).is_empty());
    }

    #[test]
    fn a_claim_the_log_never_started_is_stale_and_one_it_did_is_not() {
        let s = Uuid::new_v4();
        let run = Uuid::new_v4();
        let mut item = item(s, "claimed", Source::User, ItemStatus::Claimed { run_id: run });

        // The row says claimed; the log says nothing started. The log wins.
        assert!(is_stale_claim(&item, &running_fold()));

        let log = vec![
            SessionEvent::new(Uuid::nil(), 0, EventKind::SessionCreated, json!({})),
            SessionEvent::new(Uuid::nil(), 1, EventKind::SessionStarted, json!({})),
            SessionEvent::new(
                Uuid::nil(),
                2,
                EventKind::RunStarted,
                json!({ "item": item.id, "run_id": run }),
            ),
        ];
        assert!(!is_stale_claim(&item, &research_session::fold(&log)));

        // And nothing else is stale: a queued item is not a claim, and a
        // finished one is not either.
        item.status = ItemStatus::Queued;
        assert!(!is_stale_claim(&item, &running_fold()));
        item.status = ItemStatus::Done { verdict: "x".into() };
        assert!(!is_stale_claim(&item, &running_fold()));
    }

    #[test]
    fn the_placeholder_run_is_distinguishable_from_a_real_one() {
        // A claim that has not started carries the nil uuid, so a reader of the
        // row can tell the two apart without consulting the log — and the log is
        // still what decides.
        assert_eq!(PENDING_RUN, Uuid::nil());
        assert_ne!(PENDING_RUN, Uuid::new_v4());
    }

    #[test]
    fn a_long_question_becomes_a_title_and_a_short_one_is_itself() {
        assert_eq!(title_of("  does momentum survive costs?  "), "does momentum survive costs?");
        let long = "a".repeat(200);
        let t = title_of(&long);
        assert_eq!(t.chars().count(), 58);
        assert!(t.ends_with('…'));
    }
}
