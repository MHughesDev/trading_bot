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
//! # Layer two, and where it is performed
//!
//! AGENT-014 P3 refuses duplicate work three times: at agenda insert (the
//! dedupe key), at intake (`hypothesis_precheck`), and at the ledger (the
//! family's trial count reducing `novelty`). One and three are in
//! `research_session::agenda::offer` and in the scorer above.
//!
//! §4's table says layer two is "**the child's** `hypothesis_precheck`". It is
//! performed here instead, by the driver, *before* the child is started — and
//! that is stricter rather than looser. The spec's own sentence about it is
//! "marks the item `superseded{by}` and **charges no trial**"; a child that had
//! to run the precheck itself would already have started a turn, spent tokens
//! and held a lease before it could find out. Doing it at the door costs
//! nothing and the item never becomes a run.
//!
//! What the driver reads is the same two facts the battery's `precheck` reads,
//! from the same columns, and it applies no rule of its own:
//!
//! * a `negative_result` on the item's subject whose `retry_condition` is
//!   **absent**. That field is derived by the platform from the failure class at
//!   the moment the negative is written, and `None` is the load-bearing half: a
//!   weak effect has no retry condition, so an idea that died of one does not
//!   come back because a venue changed. The item is `expired`, because "blocked"
//!   is not "duplicate" — there is nothing to go and read that would change the
//!   answer;
//! * a hypothesis in the registry with the same normalised statement **and a
//!   decided status**. `open` is not an answer and neither is `inconclusive` —
//!   the schema's comment for that column says "we could not tell is not an
//!   answer unless it says why not", and a question the platform could not
//!   settle is one worth asking again. The item is `superseded`.
//!
//! The subject half is inert for an item with no subject, and says so on the
//! log rather than passing quietly: the charter form's seeds are sentences, not
//! subjects (see `routes::desk_charter`), so most seeded items reach here
//! without one.
//!
//! # The four modules this driver calls, and what each call costs it
//!
//! `research_session` holds four decisions that had no caller until this
//! build. They are wired in the order §3's loop performs them, and each is a
//! decision made there and a *query* made here — which is the whole division:
//!
//! * **[`research_session::schedule`]** — before anything is claimed.
//!   [`OpenHours::parse`] is fallible and there is no path to UTC, so a session
//!   whose charter names a zone the platform does not know stops the driver by
//!   name rather than running in a fence that silently moved.
//!   [`Openness::Open::closes_at`] is the drain boundary and rides on
//!   `run_started`.
//! * **[`research_session::harvest`]** — on every finished run. The ordering is
//!   load-bearing and [`after_run`] is the function that holds it: every offer
//!   is planned and written *before* the item is settled, because
//!   `agenda::offer` merges onto non-terminal items only and a run that
//!   suggests re-asking its own question must extend the item it just ran
//!   rather than re-queue the question it just answered.
//! * **[`research_session::stall`]** — after every finished run.
//!   `detect(&[FinishedRun])` needs knowledge units, a failure class, a family
//!   and a cost, and `Folded::finished` is only `item → verdict`, so
//!   [`SessionWorker::finished_runs`] is a join of the fold to `agent_runs`,
//!   `agent_lifecycle.hypothesis`, `agent_memory.entry` and the agenda — not a
//!   replay.
//! * **[`research_session::digest`]** — every `review_every`, and again on
//!   stall and on complete. [`Folded::last_digest_seq`] and the log's own
//!   `seq` are the cadence; nothing consults a clock to decide whether one is
//!   due.
//!
//! # Three measurements this platform does not record, named rather than faked
//!
//! Each of these would have been one line to invent and each would have put a
//! number nobody measured into the one log every later analysis reads.
//!
//! * **Per-run USD.** There is none: `llm_usage.cost_usd` keys on the auth
//!   session rather than the run, which is why [`cost_signal`](SessionWorker::cost_signal)
//!   scores in trials. `digest::BudgetSummary.spent` is rendered beside a day
//!   budget written in dollars, so the digest is stored with [`SPEND_UNIT`]
//!   naming what it counted — `0139` makes that column NOT NULL with no
//!   default, so no writer can leave the question open. The stall detector is
//!   unaffected: cost drift is a ratio and the unit cancels.
//! * **A finished run's `trials_spent` may be NULL**, and 0062 says in as many
//!   words that an invented zero there is worse than the null. Such a run is
//!   **excluded** from the slice handed to `detect` — a zero cost would drag
//!   the session's own median down and trip cost drift on runs that did not
//!   drift — and the count of exclusions rides on the `session_stalled` event
//!   so the narrowing is visible rather than silent.
//! * **`worth_pursuing` (§4.8)** is recorded nowhere. Open secondary
//!   hypotheses are offered to [`harvest`] unmarked, which is what
//!   `NextQuestion::new` means, so they come back in `Harvest::skipped` with
//!   the reason on the log. Source 4.8 is wired and inert until the mark
//!   exists; it is not fed by pretending every secondary hypothesis was
//!   marked.
//!
//! # Still not here, and not pretended to be
//!
//! Budget leases (§6.3) and the per-run drain (§7 row 1's "suspend at the
//! boundary"): the driver refuses to *claim* outside a window and records the
//! boundary a live child is running past, but AGENT-010 §10.1's suspend is a
//! property of the run loop and does not exist. The **day budget** (§7 row 2)
//! is denominated in dollars and there is no per-run dollar to subtract, so
//! nothing here claims to enforce it — a budget bar that counted nothing would
//! be a control the operator would believe in. Sources 4.4–4.7 and 4.9, and
//! §6.6's session-level `platform_defect` breaker, are their own tasks.

// `JobError` is the job service's wire type — see `gate_worker` for why this is
// allowed rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use research_session::{
    agenda::{AgendaItem, DedupeLayer, Insert, ItemStatus, Source},
    charter::{ReviewEvery, Schedule},
    digest::{self, Entry, NegativeRecord, Pause, Period, RunRecord, Upcoming},
    fold::EventKind,
    harvest::{
        self, AwaitingHuman, Harvest, NextQuestion, OpenQuestion, Ring, RunHarvest, RunProposal,
        SkipReason,
    },
    schedule::{OpenHours, Openness},
    select::{self, Inputs, Scored, Signal},
    stall::{self, Diagnosis, FinishedRun, Precheck},
    DigestInput, Folded, ResearchSession, Score,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent::AgentManager;
use crate::credentials::LlmCredentialStore;
use crate::sessions::{self, LoggedEvent};

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

/// What the numbers in a stored digest are denominated in (`0139`).
///
/// **Trials.** `agent_runs.trials_spent` is the only per-run cost this platform
/// records; `llm_usage.cost_usd` keys on the auth session rather than the run,
/// so there is no per-run USD to sum. The column is NOT NULL with no default so
/// that a build which later measures dollars has to say so here rather than
/// inheriting a word somebody wrote in 2026.
const SPEND_UNIT: &str = "trials";

/// The battery verdict that means a run went ahead over the dedupe's objection.
///
/// AGENT-013 §7.1's vocabulary is `proceed_new | proceed_as_variant |
/// reject_duplicate` and **none of those three strings is written anywhere on
/// this platform** — `hypothesis_precheck` is a battery and it files a
/// `mlops.workflow_verdict` whose words are `blocked`, `duplicate` and `new`.
///
/// So `proceed_as_variant` is read off the verdict the platform does write.
/// §7.1's own sentence for circling is "the dedupe did its job and the policy
/// overrode it", and a run that finished after a `duplicate` verdict is exactly
/// that: the battery said this had been asked, and the item ran anyway. Every
/// other verdict, and the absence of one, reads as `proceed_new` — the reading
/// that cannot manufacture a stall.
const PRECHECK_OVERRIDDEN: &str = "duplicate";

/// How many finished runs a stall check looks back over.
///
/// `stall::window_size` takes `max(6, 20%)` of whatever it is given, so the
/// slice has to hold the whole session for the share to mean what §7.1 says.
/// This is a ceiling on the read, not on the rule: a session that finished more
/// runs than this has a window of 20% of this number, and the number is far
/// above `SessionBudget::max_runs`.
const STALL_LOOKBACK: i64 = 2000;

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

    /// Layer two: has this already been answered, or closed for good?
    ///
    /// Returns the status the item should take, and `None` when it may proceed.
    /// No verdict logic of its own — both branches read a field the platform
    /// derived and persisted.
    async fn intake_check(
        &self,
        user_id: Uuid,
        item: &AgendaItem,
    ) -> Result<Option<(ItemStatus, DedupeLayer, String)>, JobError> {
        // A door the platform closed. Checked first because it is the stronger
        // answer: a duplicate says "go and read it", a closed negative says
        // there is nothing to read that would change it.
        if !item.subject.trim().is_empty() {
            let negatives: Vec<(String, Value)> = sqlx::query_as(
                "SELECT claim, extra FROM agent_memory.entry
                  WHERE user_id = $1 AND kind = 'negative_result' AND subject = $2",
            )
            .bind(user_id)
            .bind(&item.subject)
            .fetch_all(&self.pg)
            .await
            .map_err(|e| Self::infra("reading negative results", e))?;

            let closed: Vec<String> = negatives
                .iter()
                .filter(|(_, extra)| {
                    extra
                        .get("retry_condition")
                        .and_then(Value::as_str)
                        .is_none_or(|c| c.trim().is_empty())
                })
                .map(|(claim, _)| claim.clone())
                .collect();
            if !closed.is_empty() {
                let why = format!(
                    "`{}` has {} negative result(s) with no retry condition ({}). The platform \
                     derived those classes; nothing reopens them, and a run would spend the \
                     budget to learn what is already written down",
                    item.subject,
                    closed.len(),
                    closed.join("; ")
                );
                return Ok(Some((
                    ItemStatus::Expired { why: why.clone() },
                    DedupeLayer::Intake,
                    why,
                )));
            }
        }

        // Asked and answered. `open` and `inconclusive` are not answers — the
        // schema says so about the second in as many words — so a question the
        // platform could not settle is one worth asking again.
        let needle = crate::routes::lifecycle::normalise_statement(&item.question);
        if needle.is_empty() {
            return Ok(None);
        }
        let decided: Vec<(Uuid, String, String)> = sqlx::query_as(
            "SELECT h.hypothesis_id, h.statement, h.status
               FROM agent_lifecycle.hypothesis h
               JOIN agent_runs r ON r.run_id = h.run_id
              WHERE r.user_id = $1 AND h.status IN ('supported', 'refuted')
              LIMIT 2000",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the hypothesis registry", e))?;

        let Some((id, _, status)) = decided
            .into_iter()
            .find(|(_, st, _)| crate::routes::lifecycle::normalise_statement(st) == needle)
        else {
            return Ok(None);
        };
        let why = format!(
            "this exact question is already in the registry as {id}, {status}. Read it rather \
             than spending a trial on it"
        );
        Ok(Some((
            // `superseded{by_item}` wants an agenda item and this was superseded
            // by a *hypothesis*, so the id goes in the reason where a reader
            // will find it and the status carries the item's own id — which is
            // honest about what the column can hold.
            ItemStatus::Superseded { by_item: item.id },
            DedupeLayer::Intake,
            why,
        )))
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

        // Layer two, at the door. A duplicate caught here costs nothing: no
        // conversation, no turn, no lease, no trial. The selection is still
        // logged below, because the decision log's job is to record what the
        // policy chose — including a choice the intake then refused, which is
        // exactly the case an off-policy estimate needs to see.
        if let Some((status, layer, why)) = self.intake_check(user_id, &chosen).await? {
            self.log_selection(session, &scored, &selection, &chosen, share, seed)
                .await?;
            sessions::set_status(&self.pg, chosen.id, &status)
                .await
                .map_err(|e| Self::infra("recording the intake refusal", e))?;
            let kind = if matches!(status, ItemStatus::Expired { .. }) {
                EventKind::AgendaItemExpired
            } else {
                EventKind::AgendaItemSuperseded
            };
            sessions::append(
                &self.pg,
                session.id,
                kind,
                json!({
                    "item": chosen.id,
                    "why": why,
                    // P3: the scorecard reports which of the three layers caught
                    // each duplicate, which only works if the layer is recorded
                    // at the moment it catches one.
                    "dedupe_layer": layer.as_str(),
                    "trials_charged": 0,
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the intake refusal", e))?;
            // Nothing is claimed this iteration. The next one folds a log that
            // now has one fewer claimable item and picks again.
            return Ok(None);
        }

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

    // ── §7 rows 1–2: the calendar (S-11.6) ──────────────────────────────────

    /// Read the session's windows, or stop.
    ///
    /// The refusal is the point. [`OpenHours::parse`] has no path to UTC, so a
    /// charter naming `Amercia/New_York` cannot become a session that ran all
    /// night in a fence the operator believed in. `routes::desk_charter` already
    /// calls the same function at the form, which means a schedule that reaches
    /// here unreadable was written round the door — exactly the case that should
    /// be loud rather than repaired.
    fn open_hours(schedule: &Schedule) -> Result<OpenHours, JobError> {
        OpenHours::parse(schedule).map_err(|e| {
            Self::logic(
                "schedule_unreadable",
                format!(
                    "this session's schedule cannot be read, so there is no window to claim \
                     inside: {e}"
                ),
            )
        })
    }

    /// The gate `iterate` puts in front of everything else.
    ///
    /// Separated from the pass that acts on it for the reason
    /// [`is_stale_claim`] is: the interesting property is "may this session
    /// claim at this instant", it is a two-line rule, and it should be testable
    /// without a database, a job service or a wait until Tuesday.
    fn window_gate(
        schedule: &Schedule,
        at: chrono::DateTime<Utc>,
    ) -> Result<Openness, JobError> {
        Ok(Self::open_hours(schedule)?.openness(at))
    }

    // ── §6.5: the harvest (S-11.5) ──────────────────────────────────────────

    /// What the finished run put back, read from the rows the platform wrote.
    ///
    /// Not the dossier: `mlops.dossier.body` carries claims and rendered
    /// sections and has no `next_moves` field, so §6.5's two feeds are read from
    /// the two places the platform does persist them — `agent_memory.entry` for
    /// the questions this run opened (source 4.3) and `agent_lifecycle.hypothesis`
    /// for the secondary hypotheses it registered and did not decide (source
    /// 4.8).
    ///
    /// **Nothing here marks a next question `worth_pursuing`,** because nothing
    /// on this platform records that a run marked one. `NextQuestion::new`
    /// defaults to unmarked for the reason `FinishedRun::new` defaults to
    /// barren, and the harvest reports every unmarked one in `Harvest::skipped`
    /// — so the questions are on the log and none of them reached the agenda by
    /// somebody assuming an answer.
    async fn read_harvest(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        item: &AgendaItem,
        run_id: Uuid,
        verdict: &str,
    ) -> Result<RunHarvest, JobError> {
        let mut run = RunHarvest::new(session_id, item.id, run_id, verdict)
            .after_attempts(item.attempts);

        // Source 4.3 — the questions this run opened. `blocking` first is the
        // scorer's business (§5.1 gives it a heavier source), so it arrives as
        // a source rather than as a flag.
        let opened: Vec<(String, String, String, Value)> = sqlx::query_as(
            "SELECT entry_id, claim, subject, extra FROM agent_memory.entry
              WHERE user_id = $1 AND kind = 'open_question' AND prov_run_id = $2
                AND NOT archived
              ORDER BY created_at",
        )
        .bind(user_id)
        .bind(run_id.to_string())
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's open questions", e))?;
        for (id, claim, subject, extra) in opened {
            let q = OpenQuestion::new(&id, &claim, &subject);
            let blocking = extra
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            run = run.opening(if blocking { q.blocking() } else { q });
        }

        // Source 4.8 — the secondary and exploratory hypotheses this run left
        // open. Offered unmarked; see the doc comment.
        let secondary: Vec<(String,)> = sqlx::query_as(
            "SELECT statement FROM agent_lifecycle.hypothesis
              WHERE run_id = $1 AND role IN ('secondary', 'exploratory') AND status = 'open'
              ORDER BY created_at",
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's secondary hypotheses", e))?;
        for (statement,) in secondary {
            run = run.suggesting(NextQuestion::new(&statement, &item.subject));
        }

        // Proposals. Read `pending` only, and **read** is all this does: a
        // decided proposal was decided by a human and a session that touched
        // the column would be a session deciding its own (P6).
        let proposals: Vec<(Uuid, String, i16, String, String)> = sqlx::query_as(
            "SELECT proposal_id, kind, ring, subject, rationale
               FROM agent_lifecycle.proposal
              WHERE run_id = $1 AND decision = 'pending'
              ORDER BY created_at",
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's proposals", e))?;
        for (id, kind, ring, subject, rationale) in proposals {
            // A ring outside {1, 2} is refused by `Ring::parse` and the row is
            // dropped with its number on the log rather than coerced: a
            // proposal into a ring nothing can propose into is a row somebody
            // has to look at.
            match Ring::parse(ring) {
                Ok(ring) => {
                    let what = if rationale.trim().is_empty() {
                        subject
                    } else {
                        format!("{subject} — {rationale}")
                    };
                    run = run.proposing(RunProposal::new(&id.to_string(), &kind, &what, ring));
                }
                Err(e) => {
                    sessions::append(
                        &self.pg,
                        session_id,
                        EventKind::AgendaItemMerged,
                        json!({ "refused_proposal": id, "why": e.to_string() }),
                    )
                    .await
                    .map_err(|e| Self::infra("recording a refused proposal", e))?;
                }
            }
        }
        Ok(run)
    }

    /// Write out the plan [`after_run`] made, in the order it made it.
    ///
    /// The order is the whole reason there is a plan. Every offer is written
    /// while the item the run was claimed for is **still non-terminal in the
    /// row**, so layer one merges a re-asked question onto it; the settle comes
    /// last and is the step that makes the row terminal.
    async fn apply_after_run(
        &self,
        session_id: Uuid,
        item: &AgendaItem,
        run_id: Uuid,
        verdict: &str,
        plan: &[AfterRun],
    ) -> Result<(), JobError> {
        for step in plan {
            match step {
                AfterRun::Offer(offered) => {
                    // The database decides again, under the row lock, and its
                    // answer is the one that counts: `insert_item` runs the same
                    // `agenda::offer` against the live agenda rather than a copy
                    // of it that another driver may have moved on from.
                    let inserted = sessions::insert_item(&self.pg, offered)
                        .await
                        .map_err(|e| Self::infra("offering a harvested question", e))?;
                    match inserted {
                        Some(id) => {
                            // Queued rather than proposed, for the reason a
                            // charter seed is: `proposed` is not claimable at
                            // all, so an item left there would never run in any
                            // autonomy setting. A `supervised` session still
                            // stops at the approval gate before it claims one.
                            sessions::set_status(&self.pg, id, &ItemStatus::Queued)
                                .await
                                .map_err(|e| Self::infra("queueing a harvested question", e))?;
                            sessions::append(
                                &self.pg,
                                session_id,
                                EventKind::AgendaItemAdded,
                                json!({
                                    "item": id,
                                    "source": offered.sources.first().map(|s| s.as_str()),
                                    "question": offered.question,
                                    "workflow": offered.workflow,
                                    "harvested_from": run_id,
                                }),
                            )
                            .await
                            .map_err(|e| Self::infra("recording a harvested question", e))?;
                        }
                        None => {
                            sessions::append(
                                &self.pg,
                                session_id,
                                EventKind::AgendaItemMerged,
                                json!({
                                    "key": offered.dedupe_key,
                                    "source": offered.sources.first().map(|s| s.as_str()),
                                    "harvested_from": run_id,
                                }),
                            )
                            .await
                            .map_err(|e| Self::infra("recording a harvested merge", e))?;
                        }
                    }
                }
                AfterRun::Merged {
                    into,
                    source,
                    source_refs,
                } => {
                    sessions::merge_source(&self.pg, *into, *source, source_refs)
                        .await
                        .map_err(|e| Self::infra("extending a merged item's sources", e))?;
                    sessions::append(
                        &self.pg,
                        session_id,
                        EventKind::AgendaItemMerged,
                        json!({
                            "item": into,
                            "source": source.as_str(),
                            "harvested_from": run_id,
                        }),
                    )
                    .await
                    .map_err(|e| Self::infra("recording a harvested merge", e))?;
                }
                AfterRun::Skipped { what, why } => {
                    sessions::append(
                        &self.pg,
                        session_id,
                        EventKind::AgendaItemMerged,
                        json!({
                            "skipped": what,
                            "why": why.as_str(),
                            "harvested_from": run_id,
                        }),
                    )
                    .await
                    .map_err(|e| Self::infra("recording a skipped candidate", e))?;
                }
                AfterRun::NeedsYou(awaiting) => {
                    // P6. The only thing written is that a human owes this a
                    // decision: `AwaitingHuman::digest_record` has no branch
                    // that can say otherwise, and nothing here touches
                    // `agent_lifecycle.proposal.decision`.
                    let record = awaiting.digest_record();
                    sessions::append(
                        &self.pg,
                        session_id,
                        EventKind::AwaitingApproval,
                        json!({
                            "proposal": record.id,
                            "what": record.what,
                            "ring": record.ring,
                            "awaiting": record.awaiting,
                            "run_id": awaiting.run(),
                            "item": awaiting.item(),
                        }),
                    )
                    .await
                    .map_err(|e| Self::infra("routing a proposal to Needs you", e))?;
                }
                AfterRun::Settle(status) => match status {
                    ItemStatus::Done { verdict } => {
                        sessions::finish_item(&self.pg, item.id, verdict, None)
                            .await
                            .map_err(|e| Self::infra("finishing the item", e))?;
                    }
                    _ => {
                        // §6.6's counter lives on the row, so the row is what
                        // raises it. `harvest::outcome` and `requeue_or_expire`
                        // reach the same answer because both count against
                        // `agenda::MAX_ATTEMPTS`, which is one constant.
                        let settled = sessions::requeue_or_expire(
                            &self.pg,
                            item.id,
                            &format!("the child ended `{verdict}`"),
                        )
                        .await
                        .map_err(|e| Self::infra("re-queueing the item", e))?;
                        if let ItemStatus::Expired { ref why } = settled {
                            sessions::append(
                                &self.pg,
                                session_id,
                                EventKind::AgendaItemExpired,
                                json!({ "item": item.id, "why": why }),
                            )
                            .await
                            .map_err(|e| Self::infra("recording the expiry", e))?;
                        }
                    }
                },
            }
        }
        Ok(())
    }

    // ── §7.1: the stall detector (S-11.8) ───────────────────────────────────

    /// Every finished run of this session, as [`stall::detect`] needs it.
    ///
    /// `Folded::finished` is `item → verdict` and four rules need eleven facts,
    /// so this is a join and not a replay: `agent_runs` for the yield counts and
    /// the trials, `agent_lifecycle.hypothesis` for the **family** (an
    /// `AgendaItem` has none), `agent_memory.entry` for the failure class the
    /// run recorded, `mlops.workflow_verdict` for what the precheck said, and
    /// the agenda for the sources that wanted the item.
    ///
    /// Returns the runs it could represent and a note of what it left out, so
    /// the caller can put the exclusions on the log rather than let the window
    /// quietly narrow.
    async fn finished_runs(
        &self,
        session_id: Uuid,
        agenda: &[AgendaItem],
    ) -> Result<(Vec<FinishedRun>, StallGaps), JobError> {
        let rows: Vec<FinishedRunRow> = sqlx::query_as(
            "SELECT r.run_id, r.agenda_item_id, r.workflow, r.trials_spent,
                    r.findings_established, r.findings_refuted, r.negatives,
                    r.negatives_new_class, r.questions_closed, r.contradictions_surfaced,
                    h.family_id,
                    n.failure_class,
                    v.verdict AS precheck_verdict
               FROM agent_runs r
               LEFT JOIN LATERAL (
                    SELECT family_id FROM agent_lifecycle.hypothesis
                     WHERE run_id = r.run_id AND family_id IS NOT NULL
                     ORDER BY created_at LIMIT 1
               ) h ON TRUE
               LEFT JOIN LATERAL (
                    SELECT extra->>'failure_class' AS failure_class
                      FROM agent_memory.entry
                     WHERE prov_run_id = r.run_id::text AND kind = 'negative_result'
                       AND extra ? 'failure_class'
                     ORDER BY created_at LIMIT 1
               ) n ON TRUE
               LEFT JOIN LATERAL (
                    SELECT verdict FROM mlops.workflow_verdict
                     WHERE run_id = r.run_id AND workflow = 'hypothesis_precheck'
                     ORDER BY created_at DESC LIMIT 1
               ) v ON TRUE
              WHERE r.session_id = $1 AND r.agenda_item_id IS NOT NULL
                AND r.finished_at IS NOT NULL
              ORDER BY r.finished_at, r.run_id
              LIMIT $2",
        )
        .bind(session_id)
        .bind(STALL_LOOKBACK)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the session's finished runs", e))?;

        let mut runs = Vec::with_capacity(rows.len());
        let mut gaps = StallGaps::default();
        for row in &rows {
            let Some(item) = row
                .agenda_item_id
                .and_then(|id| agenda.iter().find(|i| i.id == id))
            else {
                gaps.off_agenda += 1;
                continue;
            };
            match finished_run(row, item) {
                Some(run) => runs.push(run),
                None => gaps.no_recorded_cost += 1,
            }
            if row.family_id.is_none() {
                gaps.no_family += 1;
            }
            if row.precheck_verdict.is_none() {
                gaps.no_precheck += 1;
            }
        }
        Ok((runs, gaps))
    }

    /// §7.1, after every finished run. Stalls the session when a rule trips.
    ///
    /// Appending `session_stalled` is what stops the loop: the next fold reads
    /// `stalled`, `can_claim` is false, and nothing runs until a human resumes,
    /// amends or stops. The diagnosis rides in the payload written by
    /// [`Diagnosis::event_payload`] so the state word and the detail cannot name
    /// different rules.
    async fn check_stall(
        &self,
        session_id: Uuid,
        agenda: &[AgendaItem],
    ) -> Result<Option<Diagnosis>, JobError> {
        let (runs, gaps) = self.finished_runs(session_id, agenda).await?;
        let Some(diagnosis) = stall::detect(&runs) else {
            return Ok(None);
        };
        let mut payload = diagnosis.event_payload();
        if let Some(obj) = payload.as_object_mut() {
            // What the detector could not see. On the event rather than in a
            // log line because a human reading a diagnosis is entitled to know
            // how much of the session it was computed over.
            obj.insert("not_measured".into(), gaps.as_json(runs.len()));
        }
        sessions::append(&self.pg, session_id, EventKind::SessionStalled, payload)
            .await
            .map_err(|e| Self::infra("recording the stall", e))?;
        Ok(Some(diagnosis))
    }

    // ── §2.4: the digest (S-11.12) ──────────────────────────────────────────

    /// Write a digest for everything since the last one, if one is due.
    ///
    /// `force` is the stall/complete/stop trigger — S-11.12 says a stall writes
    /// one immediately, and "immediately" cannot wait for a run count. Returns
    /// whether one was written.
    async fn maybe_digest(
        &self,
        session: &ResearchSession,
        user_id: Uuid,
        agenda: &[AgendaItem],
        openness: Openness,
        force: Option<&Diagnosis>,
    ) -> Result<bool, JobError> {
        let log = sessions::log(&self.pg, session.id)
            .await
            .map_err(|e| Self::infra("reading the session log for the digest", e))?;
        let folded = research_session::fold(
            &log.iter().map(|e| e.event.clone()).collect::<Vec<_>>(),
        );
        let period = period_since(&log, folded.last_digest_seq);
        let Some(period) = period else {
            // Nothing has happened since the last digest. A second digest of an
            // empty stretch is a card telling the human nothing twice.
            return Ok(false);
        };
        if force.is_none() && !digest_is_due(&log, &period, session.review_every, openness) {
            return Ok(false);
        }

        let input = self
            .digest_input(session, user_id, agenda, &log, period, force)
            .await?;
        let digest = digest::write(&input);
        let written = sessions::write_digest(&self.pg, &digest, SPEND_UNIT)
            .await
            .map_err(|e| Self::infra("storing the digest", e))?;
        if written {
            sessions::append(
                &self.pg,
                session.id,
                EventKind::DigestWritten,
                json!({
                    "digest_id": sessions::digest_id(session.id, period.to_seq),
                    "from_seq": period.from_seq,
                    "to_seq": period.to_seq,
                    "lines": digest.line_count(),
                    // The unit, on the log as well as on the row. A reader of
                    // the event should not have to open the table to find out
                    // whether `spent` is money.
                    "spend_unit": SPEND_UNIT,
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the digest", e))?;
        }
        Ok(written)
    }

    /// Everything `digest::write` reads, for one period.
    async fn digest_input(
        &self,
        session: &ResearchSession,
        user_id: Uuid,
        agenda: &[AgendaItem],
        log: &[LoggedEvent],
        period: Period,
        stalled: Option<&Diagnosis>,
    ) -> Result<DigestInput, JobError> {
        let in_period = |e: &&LoggedEvent| {
            e.event.seq >= period.from_seq && e.event.seq <= period.to_seq
        };

        let mut runs = Vec::new();
        for e in log.iter().filter(in_period) {
            if e.event.kind != EventKind::RunFinished {
                continue;
            }
            let (Some(item), Some(run)) = (
                uuid_at(&e.event.payload, "item"),
                uuid_at(&e.event.payload, "run_id"),
            ) else {
                continue;
            };
            let verdict = e
                .event
                .payload
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            runs.push(self.run_record(user_id, item, run, verdict).await?);
        }

        // §2.4's next five, best first. The agenda's stored scores are the ones
        // the selection used (§5.4), so the digest shows the agenda that will
        // run rather than one recomputed for the card.
        let mut upcoming: Vec<&AgendaItem> = agenda
            .iter()
            .filter(|i| i.status.is_claimable())
            .collect();
        upcoming.sort_by(|a, b| {
            let key = |i: &AgendaItem| i.score.as_ref().map_or(f64::MIN, |s: &Score| s.total);
            key(b)
                .partial_cmp(&key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let upcoming: Vec<Upcoming> = upcoming
            .into_iter()
            .take(digest::NEXT_UP)
            .map(|i| Upcoming {
                item: i.id,
                question: i.question.clone(),
                score: i.score,
                why: why_next(i),
            })
            .collect();

        let mut pauses: Vec<Pause> = log
            .iter()
            .filter(in_period)
            .filter_map(pause_line)
            .collect();
        if let Some(diagnosis) = stalled {
            // The stall event was appended before this digest was written, so
            // its seq is the one after the period's end. Named from the
            // diagnosis rather than paraphrased — S-11.12's acceptance is that a
            // stall writes a digest carrying the rule and what to do about it.
            pauses.push(Pause::stalled(
                period.to_seq + 1,
                period.to,
                diagnosis,
            ));
        }

        Ok(DigestInput {
            session: session.id,
            period,
            runs,
            upcoming,
            pauses,
        })
    }

    /// One finished run, as §2.4 renders it: verdicts, not titles.
    async fn run_record(
        &self,
        user_id: Uuid,
        item: Uuid,
        run: Uuid,
        verdict: &str,
    ) -> Result<RunRecord, JobError> {
        let mut record = RunRecord::new(item, run, verdict);

        // The memory this run wrote, by kind. `prov_run_id` is the join: every
        // entry names the run that produced it and the column is NOT NULL.
        let entries: Vec<(String, String, String, String, Value)> = sqlx::query_as(
            "SELECT entry_id, kind, claim, confidence, extra FROM agent_memory.entry
              WHERE user_id = $1 AND prov_run_id = $2 AND NOT archived
              ORDER BY created_at",
        )
        .bind(user_id)
        .bind(run.to_string())
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's memory entries", e))?;

        for (id, kind, claim, confidence, extra) in entries {
            match (kind.as_str(), confidence.as_str()) {
                ("finding", "established") => {
                    record.findings_established.push(Entry::new(&id, &claim));
                }
                ("finding", "refuted") => record.findings_refuted.push(Entry::new(&id, &claim)),
                ("negative_result", _) => record.negatives.push(NegativeRecord {
                    id,
                    subject: extra
                        .get("subject")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    class: extra
                        .get("failure_class")
                        .and_then(Value::as_str)
                        .unwrap_or("unclassified")
                        .to_string(),
                    // Whether the class was new for the subject is the platform's
                    // answer, counted on the run row; an entry does not carry it.
                    class_is_new: false,
                }),
                ("open_question", _) => record.questions_opened.push(Entry::new(&id, &claim)),
                _ => {}
            }
        }

        // The counts the platform computed, which are what the yield is made of.
        let counts: Option<YieldRow> = sqlx::query_as(
            "SELECT findings_established, findings_refuted, negatives, negatives_new_class,
                    questions_closed, contradictions_surfaced, trials_spent
               FROM agent_runs WHERE run_id = $1",
        )
        .bind(run)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's yield counts", e))?;
        if let Some(counts) = counts {
            record.knowledge_units = counts.knowledge_units();
            // `spent` is in trials; `SPEND_UNIT` is what says so, and `0139`
            // makes the column that carries it NOT NULL. A run that recorded no
            // trials contributes nothing rather than a zero it did not measure —
            // the sum is then an understatement a reader can see on the log
            // beside `not_measured`, not a figure dressed as a measurement.
            record.spent = f64::from(counts.trials_spent.unwrap_or(0));
            // A new class is a count on the run, so it marks the negatives the
            // run wrote rather than being asserted per entry.
            let mut remaining = counts.negatives_new_class;
            for negative in &mut record.negatives {
                if remaining <= 0 {
                    break;
                }
                negative.class_is_new = true;
                remaining -= 1;
            }
        }

        let proposals: Vec<(Uuid, String, i16, String)> = sqlx::query_as(
            "SELECT proposal_id, subject, ring, decision FROM agent_lifecycle.proposal
              WHERE run_id = $1 ORDER BY created_at",
        )
        .bind(run)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the run's proposals for the digest", e))?;
        for (id, subject, ring, decision) in proposals {
            record.proposals.push(digest::ProposalRecord {
                id: id.to_string(),
                what: subject,
                ring,
                // What a **human** has since decided. The digest renders only
                // the ones still awaiting, and this is the one place the
                // platform is allowed to say a proposal is not awaiting —
                // because it is reading a human's decision, not making one.
                awaiting: decision == "pending",
            });
        }
        Ok(record)
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

        // §7 rows 1–2. Before the agenda is even read: an item scored and
        // selected outside a window is work the driver was not allowed to plan,
        // and the events it would write are events the human would have to read
        // past.
        let openness = Self::window_gate(&session.schedule, Utc::now())?;
        if !openness.may_claim() {
            let agenda = sessions::agenda(&self.pg, session_id)
                .await
                .map_err(|e| Self::infra("reading the agenda", e))?;
            sessions::append(
                &self.pg,
                session_id,
                EventKind::WindowClosed,
                json!({
                    "openness": openness.as_str(),
                    "re_arms_at": openness.re_arms_at(),
                    "why": "the local clock in this schedule's zone is outside every window it \
                            names, so nothing is claimed",
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the closed window", e))?;
            // §2.1's `review_every: window` means once per window, and this is
            // the boundary. `force` is `None`: a window that closed on a session
            // that did nothing writes no digest.
            self.maybe_digest(&session, user_id, &agenda, openness, None)
                .await?;
            return Ok(Step::Idle(format!(
                "the schedule is {}{}",
                openness.as_str(),
                openness
                    .re_arms_at()
                    .map_or(String::new(), |t| format!("; it re-arms at {t}"))
            )));
        }

        if folded.runs_started >= i64::from(session.budget.max_runs) {
            let agenda = sessions::agenda(&self.pg, session_id)
                .await
                .map_err(|e| Self::infra("reading the agenda", e))?;
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
            // §7 row 3: `complete` *after* the final digest, which is why this
            // is not left to the cadence.
            self.force_digest(&session, user_id, &agenda).await?;
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
            self.force_digest(&session, user_id, &agenda).await?;
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
                    sessions::child_conversation_id(session_id, item.id),
                    // §7 row 1's drain boundary, from `OpenHours::closes_after`.
                    // Recorded rather than acted on: suspending a live child at
                    // it is AGENT-010 §10.1's per-run drain and does not exist,
                    // so what this driver can honestly promise is that it
                    // claims nothing new past the boundary — and a reader of
                    // the log can see which children ran through one.
                    "drains_at": openness.re_arms_at() }),
        )
        .await
        .map_err(|e| Self::infra("recording the start", e))?;
        sessions::set_status(&self.pg, item.id, &ItemStatus::Running { run_id })
            .await
            .map_err(|e| Self::infra("marking the item running", e))?;

        let ended = self.await_child(ctx, run_id).await?;

        // The verdict, in the vocabulary a harvest speaks. `agent_runs.status`
        // is the run drivers' word and `harvest::FAILURE_VERDICTS` is
        // AGENT-014's; neither contains the other's, so the two meet in
        // `verdict_of` where the mapping is one function with a test.
        let primary: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM agent_lifecycle.hypothesis
              WHERE run_id = $1 AND role = 'primary'
              ORDER BY created_at DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| Self::infra("reading the child's primary hypothesis", e))?;
        let verdict = verdict_of(&ended, primary.as_ref().map(|r| r.0.as_str()));

        // §6.5. Read what the run put back, plan it, record the finish, then
        // write the plan — in that order, because `run_finished` is §3's next
        // arrow and because every offer must be written while the item's row is
        // still non-terminal.
        let run = self
            .read_harvest(user_id, session_id, &item, run_id, &verdict)
            .await?;
        let mut live: Vec<AgendaItem> = agenda
            .iter()
            .filter(|i| !i.status.is_terminal())
            .cloned()
            .collect();
        let plan = after_run(&run, &mut live);
        let settled = plan
            .iter()
            .find_map(|s| match s {
                AfterRun::Settle(status) => Some(status.clone()),
                _ => None,
            })
            .unwrap_or(ItemStatus::Queued);

        sessions::append(
            &self.pg,
            session_id,
            EventKind::RunFinished,
            json!({
                "item": item.id,
                "run_id": run_id,
                "verdict": verdict,
                "run_status": ended,
                "then": settled.word(),
            }),
        )
        .await
        .map_err(|e| Self::infra("recording the finish", e))?;

        self.apply_after_run(session_id, &item, run_id, &verdict, &plan)
            .await?;

        // §7.1, after every finished run, and §2.4's "also written on stall".
        let agenda = sessions::agenda(&self.pg, session_id)
            .await
            .map_err(|e| Self::infra("re-reading the agenda after the harvest", e))?;
        let diagnosis = self.check_stall(session_id, &agenda).await?;
        if let Some(diagnosis) = diagnosis {
            self.maybe_digest(&session, user_id, &agenda, openness, Some(&diagnosis))
                .await?;
            return Ok(Step::Done(format!("stalled: {}", diagnosis.headline())));
        }

        self.maybe_digest(&session, user_id, &agenda, openness, None)
            .await?;
        Ok(Step::Ran(item.id))
    }

    /// Write a digest whatever the cadence says (§7 rows 3–6: complete, stop).
    ///
    /// The cadence is a rule about *reviews*; an ending is not a review, and a
    /// session that completed one run short of its `review_every` must not end
    /// with its last stretch unreported. There is no diagnosis, so the period's
    /// own pauses are the whole of what it says about stopping.
    async fn force_digest(
        &self,
        session: &ResearchSession,
        user_id: Uuid,
        agenda: &[AgendaItem],
    ) -> Result<bool, JobError> {
        let log = sessions::log(&self.pg, session.id)
            .await
            .map_err(|e| Self::infra("reading the session log for the final digest", e))?;
        let folded = research_session::fold(
            &log.iter().map(|e| e.event.clone()).collect::<Vec<_>>(),
        );
        let Some(period) = period_since(&log, folded.last_digest_seq) else {
            return Ok(false);
        };
        let input = self
            .digest_input(session, user_id, agenda, &log, period, None)
            .await?;
        let digest = digest::write(&input);
        let written = sessions::write_digest(&self.pg, &digest, SPEND_UNIT)
            .await
            .map_err(|e| Self::infra("storing the final digest", e))?;
        if written {
            sessions::append(
                &self.pg,
                session.id,
                EventKind::DigestWritten,
                json!({
                    "digest_id": sessions::digest_id(session.id, period.to_seq),
                    "from_seq": period.from_seq,
                    "to_seq": period.to_seq,
                    "lines": digest.line_count(),
                    "spend_unit": SPEND_UNIT,
                    "final": true,
                }),
            )
            .await
            .map_err(|e| Self::infra("recording the final digest", e))?;
        }
        Ok(written)
    }
}

// ── the harvest's plan, and the order that is the point of it ───────────────

/// One thing the driver does after a child finishes (AGENT-014 §6.5, §6.6).
///
/// A plan rather than a run of awaits, because the **order** is the property
/// that matters and an order buried inside a two-hundred-line async function is
/// an order nothing can test. `harvest` decides *what*; this records *when*, and
/// `every_offer_is_planned_before_the_item_is_settled` is a test about this
/// driver's call site rather than about the module it calls.
#[derive(Debug, Clone, PartialEq)]
enum AfterRun {
    /// A question the run produced, offered to the agenda (source 4.3 or 4.8).
    Offer(Box<AgendaItem>),
    /// Layer one merged the question onto an item that was already there.
    ///
    /// `source_refs` is what the merge added, so the agenda card can open the
    /// memory row that wanted the question. It used to be lost here: the
    /// candidate carried a ref, `Insert::Merged` did not return one, and the
    /// card showed a question three sources wanted with no link to any of them.
    Merged {
        into: Uuid,
        source: Source,
        source_refs: Vec<String>,
    },
    /// A candidate the harvest did not offer, and why.
    Skipped { what: String, why: SkipReason },
    /// A proposal on its way to *Needs you*. Carries no decision and there is
    /// no shape in which it could (P6).
    NeedsYou(Box<AwaitingHuman>),
    /// What the item the run was claimed for becomes. **Last, always.**
    Settle(ItemStatus),
}

/// Plan what a finished run puts back.
///
/// The offers come before the settle because `agenda::offer` merges onto
/// non-terminal items only: settle first and a run suggesting its own question
/// re-queues the question it has just answered. `harvest` holds that order for
/// the in-memory agenda and this holds it for the rows.
fn after_run(run: &RunHarvest, agenda: &mut Vec<AgendaItem>) -> Vec<AfterRun> {
    let Harvest {
        inserted,
        needs_you,
        skipped,
        item,
    } = harvest::harvest(run, agenda);

    let mut plan: Vec<AfterRun> = Vec::with_capacity(inserted.len() + needs_you.len() + 2);
    for i in inserted {
        plan.push(match i {
            Insert::Added(added) => AfterRun::Offer(added),
            Insert::Merged {
                into,
                source,
                source_refs,
            } => AfterRun::Merged {
                into,
                source,
                source_refs,
            },
        });
    }
    for s in skipped {
        plan.push(AfterRun::Skipped {
            what: s.what,
            why: s.why,
        });
    }
    for p in needs_you {
        plan.push(AfterRun::NeedsYou(Box::new(p)));
    }
    plan.push(AfterRun::Settle(item));
    plan
}

/// What the child's terminal row means, in the vocabulary a harvest speaks.
///
/// Two vocabularies meet here and neither is this driver's. `agent_runs.status`
/// is written by the run drivers and its terminal words are `completed`,
/// `failed`, `fenced`, `refused`, `suspended`, `cancelled` and `offered`.
/// `harvest::FAILURE_VERDICTS` is `failed | fenced | interrupted`, and a
/// verdict that is not one of those marks the item `done`.
///
/// **`succeeded` is not a word either vocabulary contains**, which is why this
/// function exists rather than a comparison against it: an earlier build tested
/// `ended == "succeeded"` and therefore took the failure branch for every run
/// the platform has ever finished.
///
/// A completed run's verdict is the one its **primary hypothesis** reached,
/// because that is the answer the question got. A completed run that registered
/// none says `completed`, which is honest about there being no hypothesis to
/// report rather than claiming one was supported.
fn verdict_of(run_status: &str, primary_hypothesis: Option<&str>) -> String {
    match run_status {
        "completed" => primary_hypothesis
            .filter(|s| !s.trim().is_empty() && *s != "open")
            .unwrap_or("completed")
            .to_string(),
        "failed" | "fenced" => run_status.to_string(),
        // Cancelled, refused, suspended, offered, and a row that has gone: the
        // child did not conclude, and §6.6's re-queue is what that earns. The
        // alternative reading — `done{cancelled}` — would show the board a
        // question as settled because a human stopped the run.
        _ => "interrupted".to_string(),
    }
}

// ── the stall detector's inputs ─────────────────────────────────────────────

/// One finished run's rows, before they become a [`FinishedRun`].
#[derive(sqlx::FromRow)]
struct FinishedRunRow {
    run_id: Uuid,
    agenda_item_id: Option<Uuid>,
    workflow: Option<String>,
    trials_spent: Option<i32>,
    findings_established: i32,
    findings_refuted: i32,
    negatives: i32,
    negatives_new_class: i32,
    questions_closed: i32,
    contradictions_surfaced: i32,
    family_id: Option<String>,
    failure_class: Option<String>,
    precheck_verdict: Option<String>,
}

/// The yield counts, so `knowledge_units` has one definition.
#[derive(sqlx::FromRow)]
struct YieldRow {
    findings_established: i32,
    findings_refuted: i32,
    negatives: i32,
    negatives_new_class: i32,
    questions_closed: i32,
    contradictions_surfaced: i32,
    trials_spent: Option<i32>,
}

impl YieldRow {
    /// Through `harness::lifecycle::r#yield::Yield`, not re-derived here.
    ///
    /// The weighting is the whole design of that metric and it is going to
    /// change; a second copy of the arithmetic would drift the first time it
    /// did, and the digest and the stall detector would then disagree about how
    /// much a session had learned.
    fn knowledge_units(&self) -> u32 {
        let counts = harness::lifecycle::r#yield::Yield {
            findings_established: u32::try_from(self.findings_established).unwrap_or(0),
            findings_refuted: u32::try_from(self.findings_refuted).unwrap_or(0),
            negatives: u32::try_from(self.negatives).unwrap_or(0),
            negatives_new_class: u32::try_from(self.negatives_new_class).unwrap_or(0),
            questions_closed: u32::try_from(self.questions_closed).unwrap_or(0),
            contradictions_surfaced: u32::try_from(self.contradictions_surfaced).unwrap_or(0),
            ..harness::lifecycle::r#yield::Yield::default()
        };
        counts.knowledge_units()
    }
}

/// What the stall detector could not be told, and how often.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StallGaps {
    /// Runs left out because they recorded no trials. See [`finished_run`].
    no_recorded_cost: u32,
    /// Runs whose item is no longer on the agenda this driver read.
    off_agenda: u32,
    /// Runs with no family on any hypothesis, which cannot be grouped.
    no_family: u32,
    /// Runs with no `hypothesis_precheck` verdict, which cannot be circling.
    no_precheck: u32,
}

impl StallGaps {
    /// The `not_measured` block on the `session_stalled` event.
    fn as_json(self, considered: usize) -> Value {
        json!({
            "runs_considered": considered,
            "excluded_no_recorded_cost": self.no_recorded_cost,
            "excluded_off_agenda": self.off_agenda,
            "family_not_recorded": self.no_family,
            "precheck_not_recorded": self.no_precheck,
            "why": "a run with no `trials_spent` is left out rather than costed at zero (0062: \
                    an invented zero is worse than a null), a run with no family is its own \
                    family so it cannot group with another, and a run with no \
                    `hypothesis_precheck` verdict reads as `proceed_new` — the reading that \
                    cannot manufacture a stall",
        })
    }
}

/// One row as [`stall::detect`] needs it, or `None` when it cannot be costed.
///
/// **A run that recorded no `trials_spent` is left out.** 0062 says in as many
/// words that an invented zero there is worse than a null, and cost drift is
/// where that bites: `cost_drift` sums the window's costs and takes a median
/// over the history's cost per knowledge unit, so a zero would pull the
/// session's own median down and trip the rule on runs that did not drift. The
/// other three rules lose a run they could have read, which is the
/// under-detection this crate prefers to a false alarm — and the count is on
/// the event either way.
fn finished_run(row: &FinishedRunRow, item: &AgendaItem) -> Option<FinishedRun> {
    let trials = row.trials_spent?;
    let counts = YieldRow {
        findings_established: row.findings_established,
        findings_refuted: row.findings_refuted,
        negatives: row.negatives,
        negatives_new_class: row.negatives_new_class,
        questions_closed: row.questions_closed,
        contradictions_surfaced: row.contradictions_surfaced,
        trials_spent: row.trials_spent,
    };
    let mut run = FinishedRun::new(item.id, row.run_id, &family_of(row));
    run.workflow = row
        .workflow
        .clone()
        .unwrap_or_else(|| item.workflow.clone());
    run.sources = item.sources.clone();
    run.precheck = precheck_from(row.precheck_verdict.as_deref());
    run.knowledge_units = counts.knowledge_units();
    run.findings_established = u32::try_from(row.findings_established).unwrap_or(0);
    run.failure_class = row.failure_class.clone();
    run.new_failure_class = row.negatives_new_class > 0;
    run.cost = f64::from(trials);
    Some(run)
}

/// The hypothesis family this run tested.
///
/// `AgendaItem` has no family — two tracks have now reported that — so it comes
/// from the run's own hypothesis. When no hypothesis recorded one, the run is
/// **its own family**: a name derived from the run id, which two rules can then
/// never group with anything. That is the honest shape of an unrecorded family,
/// and it is the one that cannot invent evidence — `circling` needs two runs of
/// one family and a per-run name can never reach two, so an unrecorded family
/// cannot trip a rule and cannot be suggested as an `ExcludeFamily` amendment
/// either, because `sole_value` over a window of distinct names returns `None`.
fn family_of(row: &FinishedRunRow) -> String {
    row.family_id
        .as_ref()
        .filter(|f| !f.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| format!("unrecorded-family:run:{}", row.run_id))
}

/// What the precheck said, in §7.1's vocabulary.
///
/// See [`PRECHECK_OVERRIDDEN`]: AGENT-013 §7.1's three words are written
/// nowhere on this platform, and `hypothesis_precheck` files a battery verdict
/// instead. `duplicate` is the one that means what `proceed_as_variant` means to
/// §7.1's circling rule; everything else, and no verdict at all, reads as
/// `proceed_new`.
fn precheck_from(verdict: Option<&str>) -> Precheck {
    if verdict == Some(PRECHECK_OVERRIDDEN) {
        Precheck::ProceedAsVariant
    } else {
        Precheck::ProceedNew
    }
}

// ── the digest's cadence ────────────────────────────────────────────────────

/// The stretch of the log no digest has covered, or `None` when there is none.
///
/// The period is over the log's own positions rather than over the clock, so
/// two digests cannot overlap or leave a gap — which is what makes
/// `(session_id, to_seq)` an identity `0139` can make unique.
fn period_since(log: &[LoggedEvent], last_digest_seq: Option<i64>) -> Option<Period> {
    let from_seq = last_digest_seq.map_or(0, |s| s + 1);
    let first = log.iter().find(|e| e.event.seq >= from_seq)?;
    let last = log.last()?;
    Some(Period {
        from: first.at,
        to: last.at,
        from_seq,
        to_seq: last.event.seq,
    })
}

/// Whether `review_every` says one is due (AGENT-014 §2.1).
///
/// Counted off the log rather than off `Folded::runs_started`, which is a total
/// over the whole session and cannot say how many happened since the last
/// digest. A closed window is the other trigger: `ReviewEvery::Window` means
/// "once per window", and the boundary is when a window's work is finished.
fn digest_is_due(
    log: &[LoggedEvent],
    period: &Period,
    review_every: ReviewEvery,
    openness: Openness,
) -> bool {
    let finished = log
        .iter()
        .filter(|e| e.event.seq >= period.from_seq && e.event.seq <= period.to_seq)
        .filter(|e| e.event.kind == EventKind::RunFinished)
        .count();
    match review_every {
        // A cadence of zero runs would write a digest after every event, which
        // is the log with extra steps. Treated as one.
        ReviewEvery::Runs { runs } => finished >= usize::try_from(runs.max(1)).unwrap_or(1),
        // Nothing to report is not a review. An always-open session never
        // reaches a boundary, so it never triggers this way — which is why the
        // stall, complete and stop triggers are not optional.
        ReviewEvery::Window => finished > 0 && !openness.may_claim(),
    }
}

/// Why this item is next, for §2.4's "the next five with their scores and why".
///
/// **Not the `decision` row's rationale, which `Upcoming::why` is documented
/// as.** An item that has not been chosen has no `decision` row — it appears in
/// other items' `candidate_set` and nowhere else — so the rationale §5.3 writes
/// does not exist for any of the five this list shows. What does exist is the
/// score the selection will use and the sources that wanted the question, so
/// that is what the line says.
fn why_next(item: &AgendaItem) -> String {
    let sources: Vec<&str> = item.sources.iter().map(|s| s.as_str()).collect();
    match &item.score {
        Some(s) => format!(
            "wanted by {}; value {:.2} × novelty {:.2} × feasibility {:.2} ÷ cost {:.2}",
            sources.join(", "),
            s.value,
            s.novelty,
            s.feasibility,
            s.cost
        ),
        None => format!(
            "wanted by {}; a source changed, so it is rescored before it runs",
            sources.join(", ")
        ),
    }
}

/// The digest line for an event that stopped the session, if it is one.
fn pause_line(e: &LoggedEvent) -> Option<Pause> {
    let why = |key: &str, fallback: &str| {
        e.event
            .payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_string()
    };
    let what = match e.event.kind {
        EventKind::SessionPaused => format!("paused: {}", why("why", "no reason recorded")),
        EventKind::SessionStopped => "stopped by you".to_string(),
        EventKind::SessionCompleted => format!("completed: {}", why("why", "no reason recorded")),
        EventKind::WindowClosed => format!(
            "outside the schedule window ({}); nothing was claimed",
            why("openness", "closed")
        ),
        EventKind::DayBudgetExhausted => "the day budget was spent".to_string(),
        // A stall writes its own line from the diagnosis, which carries the
        // rule and the amendment; the bare event would say only "stalled".
        _ => return None,
    };
    Some(Pause::new(e.event.seq, e.at, &what))
}

/// A uuid out of an event payload.
fn uuid_at(payload: &Value, key: &str) -> Option<Uuid> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
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
    use chrono::TimeZone;
    use research_session::charter::Window;
    use research_session::fold::{SessionEvent, SessionState};

    /// A schedule of one window, for the gate tests.
    fn schedule(days: &[u8], start: &str, end: &str, tz: &str) -> Schedule {
        Schedule {
            windows: vec![Window {
                days: days.to_vec(),
                start: start.to_string(),
                end: end.to_string(),
                tz: tz.to_string(),
            }],
            max_concurrent_runs: 1,
        }
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("a real UTC instant")
    }

    // ── §7 rows 1–2: the driver consults the calendar ───────────────────────

    #[test]
    fn the_driver_refuses_a_zone_the_platform_does_not_know_and_does_not_reach_for_utc() {
        // The typo that motivated `OpenHours::parse` being fallible. The
        // assertion that matters is the second one: at this instant the same
        // window read as UTC *is* open, so a fallback would not have failed
        // here — it would have run.
        let typo = schedule(&[1, 2, 3, 4, 5], "09:00", "17:00", "Amercia/New_York");
        let at = utc(2026, 1, 5, 14, 30);
        let err = SessionWorker::window_gate(&typo, at)
            .expect_err("an unknown zone is refused rather than repaired");
        assert!(
            format!("{err:?}").contains("Amercia/New_York"),
            "the refusal names the zone it could not read: {err:?}"
        );

        let as_utc = schedule(&[1, 2, 3, 4, 5], "09:00", "17:00", "UTC");
        assert!(
            SessionWorker::window_gate(&as_utc, at)
                .expect("UTC is a zone")
                .may_claim(),
            "a silent fallback to UTC would have claimed at this instant"
        );
    }

    #[test]
    fn the_gate_closes_outside_the_window_and_names_when_it_re_arms() {
        // Weekdays 09:00–17:00 New York. 2026-01-05 is a Monday on EST.
        let s = schedule(&[1, 2, 3, 4, 5], "09:00", "17:00", "America/New_York");

        // Monday 14:30 UTC is 09:30 in New York: inside.
        let open = SessionWorker::window_gate(&s, utc(2026, 1, 5, 14, 30)).expect("parses");
        assert!(open.may_claim(), "inside the window the driver may claim");
        assert!(
            matches!(open, Openness::Open { .. }),
            "and the drain boundary is what it carries: {open:?}"
        );
        assert!(
            open.re_arms_at().is_some(),
            "`closes_after` is the boundary the run_started event records"
        );

        // Sunday: no window at all.
        let shut = SessionWorker::window_gate(&s, utc(2026, 1, 4, 14, 30)).expect("parses");
        assert!(
            !shut.may_claim(),
            "outside every window nothing may be claimed"
        );
        let opens_at = shut
            .re_arms_at()
            .expect("a closed schedule says when it opens");
        assert!(
            opens_at > utc(2026, 1, 4, 14, 30),
            "the driver re-arms forwards, not onto an instant already past"
        );
        assert!(
            SessionWorker::window_gate(&s, opens_at)
                .expect("parses")
                .may_claim(),
            "the instant it re-arms on is one the gate itself calls open"
        );
    }

    #[test]
    fn a_session_with_no_windows_is_always_open_and_has_no_boundary() {
        let gate = SessionWorker::window_gate(&Schedule::default(), utc(2026, 1, 4, 3, 0))
            .expect("an empty window list parses");
        assert!(gate.may_claim(), "no windows means always open");
        assert_eq!(gate.re_arms_at(), None, "there is no boundary to wait for");
    }

    // ── §6.5: the harvest, and the order that is the point of it ────────────

    fn harvested(item: &AgendaItem) -> RunHarvest {
        RunHarvest::new(item.session_id, item.id, Uuid::new_v4(), "supported")
            .opening(OpenQuestion::new(
                "q_1",
                "does the same edge survive on ETH-USD?",
                "ETH-USD",
            ))
            // The run suggesting its own question again. This is the case the
            // ordering exists for.
            .opening(OpenQuestion::new("q_2", &item.question, &item.subject))
            .suggesting(NextQuestion::new("try a longer window", "BTC-USD"))
            .proposing(RunProposal::new(
                "p_1",
                "promote_strategy",
                "momentum on BTC-USD",
                Ring::Artifacts,
            ))
    }

    #[test]
    fn every_offer_is_planned_before_the_item_is_settled() {
        // The load-bearing ordering, asserted at the *call site*: `offer` merges
        // onto non-terminal items only, so settling first would re-queue the
        // question the run had just answered.
        let session = Uuid::new_v4();
        let mut running = item(
            session,
            "does momentum survive costs on BTC-USD?",
            Source::CharterSeed,
            ItemStatus::Running {
                run_id: Uuid::new_v4(),
            },
        );
        running.subject = "BTC-USD".into();
        let mut agenda = vec![running.clone()];

        let plan = after_run(&harvested(&running), &mut agenda);

        let settle = plan
            .iter()
            .position(|s| matches!(s, AfterRun::Settle(_)))
            .expect("the plan settles the item");
        let offers: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, AfterRun::Offer(_) | AfterRun::Merged { .. }))
            .map(|(i, _)| i)
            .collect();
        assert!(
            !offers.is_empty(),
            "a plan with no offers could pass this test by matching nothing"
        );
        assert!(
            offers.iter().all(|i| *i < settle),
            "every offer must be written while the item is still non-terminal; plan: {plan:?}"
        );
        assert_eq!(
            settle,
            plan.len() - 1,
            "the settle is the last step, always"
        );

        // And the specific case: the run re-asked its own question, so layer one
        // merged onto the item it ran rather than opening a second one.
        assert!(
            plan.iter().any(
                |s| matches!(s, AfterRun::Merged { into, .. } if *into == running.id)
            ),
            "the run's own question merged onto the item it ran: {plan:?}"
        );
    }

    #[test]
    fn a_proposal_leaves_the_plan_owing_a_human_a_decision_and_nothing_else() {
        // P6. `AwaitingHuman` has no decision field; this asserts the *driver*
        // does not add one on its way to the log.
        let session = Uuid::new_v4();
        let running = item(
            session,
            "does momentum survive costs?",
            Source::CharterSeed,
            ItemStatus::Running {
                run_id: Uuid::new_v4(),
            },
        );
        let mut agenda = vec![running.clone()];
        let plan = after_run(&harvested(&running), &mut agenda);

        let awaiting: Vec<&AwaitingHuman> = plan
            .iter()
            .filter_map(|s| match s {
                AfterRun::NeedsYou(a) => Some(&**a),
                _ => None,
            })
            .collect();
        assert_eq!(awaiting.len(), 1, "the run's one proposal is routed");
        let record = awaiting[0].digest_record();
        assert!(record.awaiting, "it is still a human's to decide");
        assert_eq!(record.ring, 1);

        // The payload the driver writes, built the same way `apply_after_run`
        // builds it. Nothing in it can be read as a decision.
        let payload = json!({
            "proposal": record.id,
            "what": record.what,
            "ring": record.ring,
            "awaiting": record.awaiting,
            "run_id": awaiting[0].run(),
            "item": awaiting[0].item(),
        });
        assert_eq!(payload.get("awaiting"), Some(&json!(true)));
        assert!(
            payload.get("decision").is_none() && payload.get("approved").is_none(),
            "a session never approves its own proposals: {payload}"
        );
    }

    #[test]
    fn a_next_question_nothing_marked_worth_pursuing_is_skipped_on_the_log_not_dropped() {
        // §4.8's mark is recorded nowhere on this platform, so the driver offers
        // secondary hypotheses unmarked. The point of this test is that they
        // still reach the log with a reason.
        let session = Uuid::new_v4();
        let running = item(
            session,
            "does momentum survive costs?",
            Source::User,
            ItemStatus::Running {
                run_id: Uuid::new_v4(),
            },
        );
        let mut agenda = vec![running.clone()];
        let plan = after_run(&harvested(&running), &mut agenda);
        let skipped: Vec<&AfterRun> = plan
            .iter()
            .filter(|s| matches!(s, AfterRun::Skipped { .. }))
            .collect();
        assert_eq!(skipped.len(), 1, "the unmarked next question: {plan:?}");
        assert!(matches!(
            skipped[0],
            AfterRun::Skipped {
                why: SkipReason::NotWorthPursuing,
                ..
            }
        ));
    }

    #[test]
    fn a_child_that_did_not_conclude_re_queues_rather_than_marking_the_question_settled() {
        // `agent_runs.status` never takes the value `succeeded`, which an
        // earlier build compared against — so every run took the failure branch.
        // These are the words the run drivers actually write.
        assert_eq!(verdict_of("completed", Some("supported")), "supported");
        assert_eq!(verdict_of("completed", Some("refuted")), "refuted");
        // `open` is not an answer, so it is not reported as one.
        assert_eq!(verdict_of("completed", Some("open")), "completed");
        assert_eq!(verdict_of("completed", None), "completed");
        assert_eq!(verdict_of("failed", None), "failed");
        assert_eq!(verdict_of("fenced", None), "fenced");
        for word in ["cancelled", "refused", "suspended", "offered", "missing"] {
            assert_eq!(
                verdict_of(word, None),
                "interrupted",
                "`{word}` did not conclude, so the item goes back on the agenda"
            );
        }
        // And the two halves agree about which of those settle an item.
        assert!(harvest::is_failure(&verdict_of("cancelled", None)));
        assert!(!harvest::is_failure(&verdict_of(
            "completed",
            Some("refuted")
        )));
    }

    // ── §7.1: the stall detector gets facts it can read ─────────────────────

    fn run_row(run: Uuid, item: Uuid, units: i32) -> FinishedRunRow {
        FinishedRunRow {
            run_id: run,
            agenda_item_id: Some(item),
            workflow: Some("investigate".into()),
            trials_spent: Some(2),
            // `questions_closed` is a knowledge unit, so this is the cheapest
            // way to make a row that produced something.
            findings_established: 0,
            findings_refuted: 0,
            negatives: 0,
            negatives_new_class: 0,
            questions_closed: units,
            contradictions_surfaced: 0,
            family_id: Some("momentum".into()),
            failure_class: None,
            precheck_verdict: None,
        }
    }

    #[test]
    fn a_barren_stretch_of_rows_reaches_the_detector_and_trips_it() {
        // The join is the thing under test: `Folded::finished` is
        // `item -> verdict`, and this asserts the rows the driver reads become
        // `FinishedRun`s the rule can actually count.
        let session = Uuid::new_v4();
        let items: Vec<AgendaItem> = (0..6)
            .map(|n| {
                item(
                    session,
                    &format!("question {n}"),
                    Source::CharterSeed,
                    ItemStatus::Done {
                        verdict: "supported".into(),
                    },
                )
            })
            .collect();
        let runs: Vec<FinishedRun> = items
            .iter()
            .map(|i| {
                let row = run_row(Uuid::new_v4(), i.id, 0);
                finished_run(&row, i).expect("a costed row converts")
            })
            .collect();
        assert_eq!(runs.len(), 6, "six rows became six records");
        let diagnosis = stall::detect(&runs).expect("six barren runs trip the streak");
        assert_eq!(diagnosis.headline(), "barren_streak");
        assert_eq!(
            diagnosis.items().len(),
            6,
            "and it names every item the driver handed it"
        );
        assert!(
            diagnosis.event_payload().get("rules").is_some(),
            "the payload the driver appends carries the rules"
        );

        // The same six, each having closed a question, trip nothing.
        let productive: Vec<FinishedRun> = items
            .iter()
            .map(|i| {
                let row = run_row(Uuid::new_v4(), i.id, 1);
                finished_run(&row, i).expect("a costed row converts")
            })
            .collect();
        assert!(
            stall::detect(&productive).is_none(),
            "the converter is not simply producing barren records"
        );
    }

    #[test]
    fn a_run_that_recorded_no_trials_is_left_out_rather_than_costed_at_zero() {
        let session = Uuid::new_v4();
        let i = item(session, "q", Source::CharterSeed, ItemStatus::Queued);
        let mut row = run_row(Uuid::new_v4(), i.id, 3);
        assert!(finished_run(&row, &i).is_some(), "a costed row converts");
        row.trials_spent = None;
        assert!(
            finished_run(&row, &i).is_none(),
            "an absent measurement is not a zero, and a zero here drags the session's own \
             median down until cost drift trips on runs that did not drift"
        );
    }

    #[test]
    fn an_unrecorded_family_is_its_own_family_and_can_never_group() {
        let a = run_row(Uuid::new_v4(), Uuid::new_v4(), 0);
        let mut b = run_row(Uuid::new_v4(), Uuid::new_v4(), 0);
        b.run_id = Uuid::new_v4();
        assert_eq!(family_of(&a), "momentum", "a recorded family is used");

        let mut a = a;
        a.family_id = None;
        b.family_id = Some("   ".into());
        assert_ne!(
            family_of(&a),
            family_of(&b),
            "two runs with no family are two families, so `circling` — which needs two of one \
             — can never reach its threshold on them"
        );
        assert!(family_of(&a).contains(&a.run_id.to_string()));
    }

    #[test]
    fn only_the_verdict_that_means_the_dedupe_was_overridden_reads_as_a_variant() {
        assert_eq!(precheck_from(Some("duplicate")), Precheck::ProceedAsVariant);
        // Everything else, and the absence of a verdict, takes the reading that
        // cannot manufacture a stall.
        for other in [Some("new"), Some("blocked"), Some("inconclusive"), None] {
            assert_eq!(
                precheck_from(other),
                Precheck::ProceedNew,
                "{other:?} is not evidence that the policy overrode the dedupe"
            );
        }
    }

    #[test]
    fn the_stall_gaps_block_says_what_the_detector_could_not_see() {
        let gaps = StallGaps {
            no_recorded_cost: 2,
            off_agenda: 0,
            no_family: 3,
            no_precheck: 4,
        };
        let json = gaps.as_json(7);
        assert_eq!(json.get("runs_considered"), Some(&json!(7)));
        assert_eq!(json.get("excluded_no_recorded_cost"), Some(&json!(2)));
        assert_eq!(json.get("precheck_not_recorded"), Some(&json!(4)));
    }

    // ── §2.4: the digest's cadence is read off the log ──────────────────────

    fn logged(seq: i64, kind: EventKind, minute: u32) -> LoggedEvent {
        LoggedEvent {
            event: SessionEvent::new(Uuid::nil(), seq, kind, json!({})),
            at: utc(2026, 1, 5, 12, minute),
        }
    }

    #[test]
    fn the_period_is_everything_since_the_last_digest_and_nothing_before_it() {
        let log = vec![
            logged(0, EventKind::SessionCreated, 0),
            logged(1, EventKind::SessionStarted, 1),
            logged(2, EventKind::DigestWritten, 2),
            logged(3, EventKind::RunStarted, 3),
            logged(4, EventKind::RunFinished, 4),
        ];
        let period = period_since(&log, Some(2)).expect("there is a stretch after the digest");
        assert_eq!(period.from_seq, 3, "the digest's own seq is behind us");
        assert_eq!(period.to_seq, 4);
        assert_eq!(period.from, utc(2026, 1, 5, 12, 3));
        assert_eq!(period.to, utc(2026, 1, 5, 12, 4));

        // A session with no digest yet covers its whole log.
        let first = period_since(&log, None).expect("a first digest covers everything");
        assert_eq!((first.from_seq, first.to_seq), (0, 4));

        // And nothing new since the last one is not a period. A second digest of
        // an empty stretch is a card telling the human nothing twice.
        assert!(period_since(&log, Some(4)).is_none());
    }

    #[test]
    fn the_run_cadence_is_counted_over_the_period_and_not_over_the_session() {
        let log = vec![
            logged(0, EventKind::SessionCreated, 0),
            logged(1, EventKind::SessionStarted, 1),
            logged(2, EventKind::RunFinished, 2),
            logged(3, EventKind::DigestWritten, 3),
            logged(4, EventKind::RunFinished, 4),
            logged(5, EventKind::RunFinished, 5),
        ];
        let period = period_since(&log, Some(3)).expect("a stretch after the digest");
        let every_two = ReviewEvery::Runs { runs: 2 };
        let every_three = ReviewEvery::Runs { runs: 3 };
        assert!(
            digest_is_due(&log, &period, every_two, Openness::AlwaysOpen),
            "two runs finished since the last digest"
        );
        assert!(
            !digest_is_due(&log, &period, every_three, Openness::AlwaysOpen),
            "the session has finished three, but only two are in this period"
        );
    }

    #[test]
    fn a_window_cadence_waits_for_the_boundary_and_says_nothing_about_a_quiet_one() {
        let log = vec![
            logged(0, EventKind::SessionCreated, 0),
            logged(1, EventKind::SessionStarted, 1),
            logged(2, EventKind::RunFinished, 2),
        ];
        let period = period_since(&log, None).expect("a stretch");
        let closed = Openness::Closed {
            opens_at: utc(2026, 1, 6, 14, 0),
        };
        let open = Openness::Open {
            closes_at: utc(2026, 1, 5, 22, 0),
        };
        assert!(
            digest_is_due(&log, &period, ReviewEvery::Window, closed),
            "the window closed on a period with work in it"
        );
        assert!(
            !digest_is_due(&log, &period, ReviewEvery::Window, open),
            "a window still open has not reached its boundary"
        );
        assert!(
            !digest_is_due(&log, &period, ReviewEvery::Window, Openness::AlwaysOpen),
            "an always-open session never reaches a boundary, which is why the stall, \
             complete and stop triggers are not optional"
        );

        // A boundary on a period in which nothing finished is not a review.
        let quiet = vec![
            logged(0, EventKind::SessionCreated, 0),
            logged(1, EventKind::SessionStarted, 1),
        ];
        let quiet_period = period_since(&quiet, None).expect("a stretch");
        assert!(!digest_is_due(
            &quiet,
            &quiet_period,
            ReviewEvery::Window,
            closed
        ));
    }

    #[test]
    fn a_pause_line_is_written_for_every_kind_that_stops_the_session() {
        let mut stopped = logged(7, EventKind::WindowClosed, 7);
        stopped.event.payload = json!({ "openness": "closed" });
        let line = pause_line(&stopped).expect("a closed window stops the session");
        assert_eq!(line.seq, 7, "the line opens the event that did it");
        assert!(line.what.contains("closed"), "{}", line.what);

        // And an ordinary event is not a pause, or the digest's *Paused*
        // section would be the log.
        assert!(pause_line(&logged(8, EventKind::Selected, 8)).is_none());
        assert!(pause_line(&logged(9, EventKind::RunFinished, 9)).is_none());
        // The stall writes its own line from the diagnosis, which carries the
        // rule and the amendment; the bare event would say only "stalled".
        assert!(pause_line(&logged(10, EventKind::SessionStalled, 10)).is_none());
    }

    #[test]
    fn the_next_five_say_what_wanted_them_and_never_read_an_absent_score_as_zero() {
        let session = Uuid::new_v4();
        let mut i = item(session, "q", Source::OpenQuestionBlocking, ItemStatus::Queued);
        let unscored = why_next(&i);
        assert!(
            unscored.contains("open_question_blocking"),
            "{unscored}"
        );
        assert!(
            !unscored.contains("0.00"),
            "an item nothing has scored is not an item worth nothing: {unscored}"
        );

        i.score = Some(Score::combine(1.3, 0.8, 1.0, 2.0));
        let scored = why_next(&i);
        assert!(scored.contains("value 1.30"), "{scored}");
        assert!(scored.contains("novelty 0.80"), "{scored}");
    }

    #[test]
    fn the_digest_is_stored_saying_which_unit_its_numbers_are_in() {
        // The trap S-11.12 left: `budget.spent` is rendered beside a day budget
        // written in dollars and the only per-run cost this platform records is
        // trials. `0139` makes the column NOT NULL with no default; this is the
        // constant that fills it.
        assert_eq!(SPEND_UNIT, "trials");
        assert_ne!(
            SPEND_UNIT, "usd",
            "there is no per-run USD: llm_usage.cost_usd keys on the auth session, not the run"
        );
    }

    #[test]
    fn knowledge_units_are_the_harness_yields_arithmetic_and_not_a_second_copy() {
        let row = YieldRow {
            findings_established: 1,
            findings_refuted: 1,
            negatives: 1,
            negatives_new_class: 1,
            questions_closed: 1,
            contradictions_surfaced: 1,
            trials_spent: Some(3),
        };
        let expected = harness::lifecycle::r#yield::Yield {
            findings_established: 1,
            findings_refuted: 1,
            negatives: 1,
            negatives_new_class: 1,
            questions_closed: 1,
            contradictions_surfaced: 1,
            ..harness::lifecycle::r#yield::Yield::default()
        }
        .knowledge_units();
        assert_eq!(row.knowledge_units(), expected);
        assert!(expected > 0, "a fixture that counted nothing proves nothing");
    }

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
