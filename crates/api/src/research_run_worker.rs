//! The `ResearchRun` child worker — the thing HANDOFF §2 lists as missing
//! (AGENT-010 §9, AGENT-013 §8.5, FEAT-003 §8.2; Set S task S-10.11).
//!
//! # What was actually broken
//!
//! `campaign_driver::child_kind` has mapped two phases to `JobKind::ResearchRun`
//! since it was written, and nothing anywhere registered a worker for that kind.
//! A campaign reaching DIAGNOSE therefore submitted a child job that no pool
//! would ever claim, and `await_child` polled it every five seconds forever. The
//! driver was correct, the phase machine was correct, the idempotency was
//! correct, and no campaign could get past its second phase — the repository's
//! dominant defect shape, reached through a job kind rather than a function call.
//!
//! # The two phases
//!
//! **HYPOTHESIZE** is the one this task is about: a worker sub-run reading a
//! projection of the parent's context store, whose only output is one
//! [`Proposal`](research::propose::Proposal) — a move, a hypothesis, and ranges.
//! The platform owns the phases, the budget, the floor and the gates; the agent
//! owns what to try.
//!
//! **DIAGNOSE** is here because the driver already dispatches it and a campaign
//! cannot reach HYPOTHESIZE without passing through it. What it does is bounded
//! and says so: it reads the campaign's own log for the last phase that produced
//! a child job, and attaches that job's result as the generation's evidence. The
//! `DiagnosticBundle` proper — FEAT-003 §8.2 step 7, over the gate the candidate
//! reached — is **not** built here, and the output says which of the two a reader
//! is looking at rather than presenting a thin one as the real thing.
//!
//! # Why the proposal call pins temperature to zero
//!
//! The `propose` decision the driver logs needs a propensity, and a propensity is
//! `P(chosen | context)` under the policy that chose. For a grammar-constrained
//! call at temperature zero that probability is 1.0 and saying so is true. At any
//! other temperature it is unknown, and writing 1.0 anyway would put a fabricated
//! number into the one log every off-policy estimate on this platform reads. So
//! the call sets zero explicitly and the worker refuses a profile that would
//! raise it, rather than recording a propensity it cannot support.

// `JobError` is the job service's wire type — see `gate_worker` for why this is
// allowed rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use jobs::store::JobStore;
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use llm::{ChatRequest, LlmClient, Message, Provider};
use research::propose::Proposal;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent::subrun;
use crate::credentials::{CredentialCrypto, LlmCredentialStore};

/// The one retry FEAT-003 §8.2 step 3 allows: an invalid move comes back with
/// the validator's reason and the proposer tries again. Exactly one, because a
/// second failure under a grammar is a proposer that cannot answer, not a
/// proposer that slipped.
const PROPOSE_ATTEMPTS: usize = 2;

/// The campaign phases this worker performs.
///
/// Read by `campaign_driver`'s tests against `child_kind`, so the two lists
/// cannot drift: a phase mapped to `JobKind::ResearchRun` that this worker does
/// not handle is a campaign that stops there, and the failure would show up as a
/// stalled campaign rather than as a compile error.
pub const HANDLED_PHASES: &[&str] = &["diagnose", "hypothesize"];

/// Runs a research sub-run for one campaign phase.
pub struct ResearchRunWorker {
    store: Arc<JobStore>,
    pg: PgPool,
    crypto: Option<Arc<CredentialCrypto>>,
    profiles: Arc<harness::ProfileSet>,
    default_profile: String,
}

fn logic(code: &'static str, detail: impl Into<String>) -> JobError {
    JobError::logic(
        ledger::TerminalReason::IntegrityRejected,
        code,
        detail.into(),
    )
}

impl ResearchRunWorker {
    #[must_use]
    pub fn new(
        store: Arc<JobStore>,
        pg: PgPool,
        crypto: Option<Arc<CredentialCrypto>>,
        profiles: Arc<harness::ProfileSet>,
        default_profile: String,
    ) -> Self {
        Self {
            store,
            pg,
            crypto,
            profiles,
            default_profile,
        }
    }

    /// The parent Research run's model, and the profile it resolves to.
    ///
    /// The sub-run runs on the **parent's** tier, not on a default. A worker that
    /// quietly ran on a different model would make the campaign's proposals come
    /// from somewhere other than the run that is accountable for them, and the
    /// tier gate below would be checking the wrong profile.
    async fn profile_for(
        &self,
        run_id: Uuid,
    ) -> Result<(harness::profile::Profile, String), JobError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT provider, model FROM agent_runs WHERE run_id = $1")
                .bind(run_id)
                .fetch_optional(&self.pg)
                .await
                .map_err(|e| JobError::infrastructure(format!("reading the parent run: {e}")))?;
        let (provider, model) = row.ok_or_else(|| {
            logic(
                "unknown_parent_run",
                "this campaign names a parent run the platform has no record of",
            )
        })?;
        let profile = match self.profiles.resolve(&model) {
            Ok(p) => p.clone(),
            Err(_) => self
                .profiles
                .resolve(&self.default_profile)
                .map_err(|e| logic("no_profile", format!("no profile for {model}: {e}")))?
                .clone(),
        };
        Ok((profile, provider))
    }

    async fn client_for(&self, user_id: Uuid, provider: &str) -> Result<LlmClient, JobError> {
        let p = <Provider as std::str::FromStr>::from_str(provider)
            .map_err(|e| logic("unknown_provider", format!("{provider}: {e}")))?;
        let creds = LlmCredentialStore::new(self.pg.clone(), self.crypto.clone());
        let cred = creds
            .load(user_id, p.as_str())
            .await
            .map_err(|e| JobError::infrastructure(format!("reading credentials: {e}")))?;
        let (api_key, base_url) = match cred {
            Some(c) => ((!c.api_key.is_empty()).then_some(c.api_key), c.base_url),
            None if p.requires_api_key() => {
                return Err(logic(
                    "provider_not_configured",
                    format!("no credential for {provider}; the sub-run cannot call a model"),
                ))
            }
            None => (None, None),
        };
        Ok(LlmClient::new(p, api_key, base_url))
    }

    /// The campaign's log, most recent first.
    async fn events(
        &self,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<Vec<(String, Value)>, JobError> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant)
            .await
            .map_err(|e| JobError::infrastructure(format!("tenant transaction: {e}")))?;
        let rows: Vec<(String, Value)> = sqlx::query_as(
            "SELECT state, detail FROM mlops.campaign_event
              WHERE campaign_id = $1 ORDER BY occurred_at DESC",
        )
        .bind(campaign_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| JobError::infrastructure(format!("reading the campaign log: {e}")))?;
        let _ = tx.commit().await;
        Ok(rows)
    }

    /// DIAGNOSE: what the last phase that computed anything actually produced.
    ///
    /// Bounded on purpose, and the output says so. The `DiagnosticBundle` FEAT-003
    /// §8.2 step 7 describes is built over the gate a candidate reached, which
    /// needs the candidate's experiment and its gate ledger; this reads the
    /// campaign's own log, which is the evidence available to a job worker that
    /// holds nothing else. A campaign whose last compute produced nothing still
    /// advances — DIAGNOSE is a step, not a gate, and blocking here would stop a
    /// campaign for the absence of a bundle rather than for a result.
    async fn diagnose(
        &self,
        ctx: &JobContext,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<JobOutput, JobError> {
        ctx.progress(Progress {
            pct: None,
            stage: Some("diagnose".into()),
            message: Some("reading what the last phase produced".into()),
        })
        .await;

        let events = self.events(tenant, campaign_id).await?;
        let last_child = events.iter().find_map(|(state, detail)| {
            detail
                .get("job_id")
                .and_then(Value::as_str)
                .map(|j| (state.clone(), j.to_string()))
        });

        let Some((phase, job_id)) = last_child else {
            return Ok(JobOutput {
                summary: Some(
                    "no phase of this campaign has produced a child job yet; there is nothing to \
                     diagnose and the campaign advances"
                        .into(),
                ),
                result: json!({
                    "campaign_id": campaign_id,
                    "bundle": Value::Null,
                    "bundle_kind": "none",
                    "why": "no completed child job",
                }),
                ..Default::default()
            });
        };

        let child = self
            .store
            .get(&job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading {job_id}: {e}")))?;
        let evidence = child.result.unwrap_or(Value::Null);

        Ok(JobOutput {
            summary: Some(format!(
                "diagnosis from the `{phase}` phase's job {job_id}; this is the phase result, \
                 not a DiagnosticBundle"
            )),
            result: json!({
                "campaign_id": campaign_id,
                "from_phase": phase,
                "from_job": job_id,
                "bundle": evidence,
                // Named so the HYPOTHESIZE brief, and anybody reading the log,
                // can tell a phase result from the §8.2 bundle rather than
                // assuming the richer one.
                "bundle_kind": "phase_result",
            }),
            ..Default::default()
        })
    }

    /// HYPOTHESIZE: one worker sub-run, one proposal.
    async fn propose(
        &self,
        ctx: &JobContext,
        tenant: &str,
        campaign_id: Uuid,
        sub_run_id: Uuid,
    ) -> Result<JobOutput, JobError> {
        let brief = subrun::campaign_brief(&self.pg, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the campaign brief: {e}")))?
            .ok_or_else(|| {
                logic(
                    "no_campaign_brief",
                    "this campaign has no pinned projection, so it was not defined by a Research \
                     run through `define_campaign`; §8.5's worker sub-run has nothing to read",
                )
            })?;

        let (profile, provider) = self.profile_for(brief.run_id).await?;
        // The tier gate, read a second time and on the other side of the job
        // boundary. The parent was admitted under it at run start; a campaign
        // outlives that admission, and a profile edited mid-campaign would
        // otherwise keep proposing on a tier that no longer claims it can.
        if !profile.may_run(harness::state::Workflow::Optimise) {
            return Err(logic(
                "tier_does_not_drive_campaigns",
                format!(
                    "the `{}` profile does not offer the `optimise` workflow (AGENT-013 §11)",
                    profile.model_id
                ),
            ));
        }
        if profile.output.temperature_tool_calls != 0.0 {
            return Err(logic(
                "propensity_not_supportable",
                "this profile samples above temperature zero, so the propensity of the move it \
                 proposes is unknown; the `propose` decision would have to carry a number nobody \
                 computed",
            ));
        }

        let job = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let client = self.client_for(job.user_id, &provider).await?;

        let generation = self.last_diagnosis(tenant, campaign_id).await?;
        let mut correction: Option<String> = None;
        let mut last_error = String::new();

        for attempt in 1..=PROPOSE_ATTEMPTS {
            ctx.progress(Progress {
                pct: None,
                stage: Some("hypothesize".into()),
                message: Some(format!("proposing (attempt {attempt})")),
            })
            .await;

            let reply = client
                .chat(&ChatRequest {
                    model: profile.model_id.clone(),
                    system: Some(subrun::WORKER_CHARTER.to_string()),
                    messages: vec![Message::User {
                        content: user_message(&brief.brief, &generation, correction.as_deref()),
                    }],
                    tools: vec![],
                    max_tokens: profile.context.reserve_for_output,
                    // Pinned, not read from the profile: the propensity below is
                    // only 1.0 because this is zero.
                    temperature: Some(0.0),
                    schema: Some(Proposal::schema()),
                    num_ctx: Some(profile.context.effective_budget_tokens),
                    keep_alive: None,
                    ..Default::default()
                })
                .await
                .map_err(|e| JobError::infrastructure(format!("the proposer failed: {e}")))?;

            let content = reply.content.unwrap_or_default();
            match parse(&content) {
                Ok((proposal, unchecked)) => {
                    let outcome = json!({
                        "proposal": proposal,
                        "unchecked": unchecked.pending,
                        "attempts": attempt,
                        "brief_digest": brief.digest,
                    });
                    subrun::settle(&self.pg, sub_run_id, &outcome)
                        .await
                        .map_err(|e| {
                            JobError::infrastructure(format!("recording the sub-run: {e}"))
                        })?;
                    return Ok(JobOutput {
                        summary: Some(format!(
                            "proposed `{}`: {}",
                            proposal.r#move.kind(),
                            proposal.hypothesis
                        )),
                        result: json!({
                            "campaign_id": campaign_id,
                            "sub_run_id": sub_run_id,
                            "role": "worker",
                            "projection": brief.projection,
                            "withheld": brief.withheld,
                            "brief_digest": brief.digest,
                            "proposal": proposal,
                            "unchecked": unchecked.pending,
                            // Every move the vocabulary offered. The decision log
                            // wants "all options considered", and under a grammar
                            // that is exactly the enum: the proposer could have
                            // answered with any of them and answered with one.
                            "candidate_set": research::propose::Move::KINDS,
                            // True because the call above pins temperature to
                            // zero under a grammar, and checked above rather
                            // than assumed.
                            "propensity": 1.0,
                        }),
                        actual: Cost {
                            compute_s: None,
                            gpu_s: None,
                            cost_usd: Some(0.0),
                        },
                        ..Default::default()
                    }
                    .clamped());
                }
                Err(why) => {
                    last_error = why.clone();
                    correction = Some(why);
                }
            }
        }

        Err(logic(
            "proposal_invalid",
            format!(
                "the proposer could not produce a valid move in {PROPOSE_ATTEMPTS} attempts \
                 under the grammar; last refusal: {last_error}"
            ),
        ))
    }

    /// The most recent DIAGNOSE result, which is this generation's evidence.
    async fn last_diagnosis(&self, tenant: &str, campaign_id: Uuid) -> Result<Value, JobError> {
        let events = self.events(tenant, campaign_id).await?;
        let Some(job_id) = events.iter().find_map(|(state, detail)| {
            (state == "diagnose")
                .then(|| detail.get("job_id").and_then(Value::as_str))
                .flatten()
                .map(str::to_string)
        }) else {
            return Ok(Value::Null);
        };
        let child = self
            .store
            .get(&job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading {job_id}: {e}")))?;
        Ok(child.result.unwrap_or(Value::Null))
    }
}

/// What the worker is asked, on top of the brief.
///
/// The brief is the *pinned* half and is byte-identical for every worker of this
/// campaign; the generation's own evidence and any correction follow it, so the
/// shared prefix stays a prefix. Putting the correction first would move the
/// cache boundary to the top of the message and cost the whole brief on a retry.
fn user_message(brief: &str, generation: &Value, correction: Option<&str>) -> String {
    let mut out = String::with_capacity(brief.len() + 512);
    out.push_str(brief);
    if !generation.is_null() {
        out.push_str("\n\n── THIS GENERATION ──\n\n");
        out.push_str(&serde_json::to_string_pretty(generation).unwrap_or_default());
    }
    if let Some(why) = correction {
        out.push_str("\n\n── YOUR LAST ANSWER WAS REFUSED ──\n\n");
        out.push_str(why);
        out.push_str("\n\nAnswer again, fixing exactly that.");
    }
    out
}

/// Parse and validate a reply, returning the validator's reason on failure so the
/// single retry has something to act on.
fn parse(content: &str) -> Result<(Proposal, research::propose::Unchecked), String> {
    let text = content.trim();
    if text.is_empty() {
        return Err("the reply was empty".to_string());
    }
    let proposal: Proposal = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let unchecked = proposal.validate().map_err(|e| e.to_string())?;
    Ok((proposal, unchecked))
}

#[async_trait::async_trait]
impl Worker for ResearchRunWorker {
    fn kind(&self) -> JobKind {
        JobKind::ResearchRun
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        // One model call, or two. No compute of its own.
        Cost {
            compute_s: Some(30.0),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let campaign_id: Uuid = manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| logic("invalid_manifest", "a research sub-run needs a campaign_id"))?;
        let phase = manifest
            .get("phase")
            .and_then(Value::as_str)
            .ok_or_else(|| logic("invalid_manifest", "a research sub-run needs its phase"))?;

        let job = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let tenant = job.user_id.to_string();

        match phase {
            "diagnose" => self.diagnose(ctx, &tenant, campaign_id).await,
            "hypothesize" => {
                let sub_run_id: Uuid = manifest
                    .get("sub_run_id")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| {
                        logic(
                            "invalid_manifest",
                            "a HYPOTHESIZE sub-run is dispatched against a claimed position; \
                             without its sub_run_id there is nothing to settle and a second \
                             worker could take the same generation",
                        )
                    })?;
                self.propose(ctx, &tenant, campaign_id, sub_run_id).await
            }
            other => Err(logic(
                "phase_has_no_research_run",
                format!(
                    "the campaign driver dispatched a research sub-run for `{other}`, which this \
                     worker does not perform; the phases it does are `diagnose` and `hypothesize`"
                ),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research::propose::{ExitMode, Move};

    #[test]
    fn a_conformant_reply_parses_and_validates() {
        let (p, unchecked) = parse(
            r#"{"move":"change_exit","mode":"trail","hypothesis":"trailing keeps the tail","ranges":[]}"#,
        )
        .expect("a conformant reply");
        assert_eq!(
            p.r#move,
            Move::ChangeExit {
                mode: ExitMode::Trail
            }
        );
        assert!(!unchecked.pending.is_empty());
    }

    /// The retry gets the validator's reason, not "invalid". A retry with no
    /// location is a second attempt at the same mistake.
    #[test]
    fn an_invalid_move_comes_back_with_the_reason() {
        let why = parse(r#"{"move":"simplify","hypothesis":"","ranges":[]}"#)
            .expect_err("an empty hypothesis is refused");
        assert!(why.contains("hypothesis"), "{why}");

        let why = parse(
            r#"{"move":"simplify","hypothesis":"tidy","ranges":[{"name":"n","kind":{"kind":"int","min":1,"max":5,"step":1}}]}"#,
        )
            .expect_err("a removal may not carry ranges");
        assert!(why.contains("ranges"), "{why}");
    }

    #[test]
    fn an_empty_reply_is_a_refusal_rather_than_a_parse_panic() {
        assert!(parse("   ").is_err());
    }

    /// The pinned brief stays a prefix: the generation's evidence and any
    /// correction go after it, so a retry does not move the cache boundary to
    /// the top of the message and pay for the whole brief again.
    #[test]
    fn the_brief_is_a_prefix_of_every_message_built_from_it() {
        let brief = "── EVIDENCE ──\nsharpe 1.4";
        let plain = user_message(brief, &Value::Null, None);
        let with_generation = user_message(brief, &json!({"gate": 2}), None);
        let retried = user_message(brief, &json!({"gate": 2}), Some("that node does not exist"));
        assert!(plain.starts_with(brief));
        assert!(with_generation.starts_with(&plain));
        assert!(retried.starts_with(&with_generation));
        assert!(retried.contains("that node does not exist"));
    }
}
