//! The `GateAdvance` worker: the production caller of the sixteen-gate stack
//! (SPEC §12.3, checklist 2.14, plan 5.5).
//!
//! `backtest::gates::stack::evaluate_stack` is arithmetic over evidence. This is
//! the thing that *gathers* the evidence — from the ledger, from the leakage
//! suite's runs, from the stored return series — and records every verdict
//! through `GateLog`. Without it the sixteen evaluators are correct and unused,
//! and `gate_pass_rate` in the self-monitor has nothing to read.
//!
//! ## The rule the assembly follows
//!
//! **Absent evidence is left absent.** Every field of `StackEvidence` this
//! worker cannot fill is left `None`, which makes its gate `inconclusive`, which
//! is never a pass. There is no default, no "assume it's fine if we can't
//! check", and no silent omission — a gate that could not run is recorded as a
//! failure carrying its reason, so the stack's length stays sixteen and the pass
//! rate counts the hole.
//!
//! That is the opposite of the obvious design, in which the worker fills what it
//! can and reports on what it filled. The obvious design has the property that
//! the platform's measured gate pass rate goes *up* as its evidence pipeline
//! degrades, which is the single most misleading number this system could
//! produce.

// `JobError` is the job service's wire type: a code, three optional strings and
// a terminal reason, capped at 400 bytes on purpose (JB-07). Boxing it to satisfy
// the large-Err lint would add an allocation to every refusal in exchange for a
// few bytes on a path that is already an error, and would put a `Box` in the
// signature of every fallible worker helper in the tree.
#![allow(clippy::result_large_err)]

use backtest::gates::evaluators::LeakageRun;
use backtest::gates::profile::GateProfile;
use backtest::gates::stack::{evaluate_stack, record_stack, StackEvidence, GATE_COUNT};
use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// The trailing window the regime terciles are measured over, matching the
/// labeller's (`apps/platform::regime_jobs`). The two must agree or Gate 6 and
/// Gate 11 would count different regimes over the same period.
pub const VOL_WINDOW: usize = 21;

/// The shortest overlap Gate 10's attribution regression is estimable on.
///
/// `newey_west_fit`'s own floor is 30 observations and `3 x (factors + 1)`;
/// below it the regression is not identified and it returns `None`. Stating the
/// floor here means the refusal names the overlap that was too short rather than
/// the regression that could not be run.
pub const MIN_ATTRIBUTION_PERIODS: usize = 30;

/// The checks the leakage suite runs. Gate 2 refuses a run that is missing any
/// of them, so this list is the contract between the suite and the gate.
pub const LEAKAGE_CHECKS: &[&str] = &[
    "causal_access",
    "random_label",
    "snapshot_reproducibility",
    "cv_wf_gap",
    "synthetic_injection",
];

/// Evaluates the sixteen gates for one subject and records the verdicts.
pub struct GateAdvanceWorker {
    pg: PgPool,
    /// Runs the evidence funnel for a campaign's candidate when the GATE child
    /// is told to (`run_funnel` in its manifest). Absent in tests that judge
    /// evidence the ledger already holds.
    funnel: Option<std::sync::Arc<dyn crate::campaign_driver::funnel_worker::CampaignFunnel>>,
}

impl GateAdvanceWorker {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg, funnel: None }
    }

    /// Give the worker the funnel a campaign GATE runs before it judges.
    #[must_use]
    pub fn with_funnel(
        mut self,
        funnel: std::sync::Arc<dyn crate::campaign_driver::funnel_worker::CampaignFunnel>,
    ) -> Self {
        self.funnel = Some(funnel);
        self
    }

    /// A campaign GATE with no funnel evidence yet: run the funnel on the
    /// campaign's incumbent, then point the judgement at the trial it wrote.
    ///
    /// The funnel is what records gates 3-8, 10 and 12's inputs, and nothing in a
    /// campaign ran it, so those gates read inconclusive for every campaign
    /// candidate. The incumbent is the one the COMPARE worker promoted (or the
    /// declared start), so the evidence is about the configuration the campaign
    /// would deploy and not about the strategy's defaults.
    async fn run_campaign_funnel(
        &self,
        ctx: &JobContext,
        manifest: &Value,
    ) -> Result<Value, JobError> {
        use crate::campaign_driver::{compare_worker, funnel_worker, study_worker};
        let logic = |code: &'static str, why: String| {
            JobError::logic(ledger::TerminalReason::IntegrityRejected, code, why)
        };
        let Some(funnel) = &self.funnel else {
            return Err(logic(
                "no_funnel_runner",
                "this GATE needs the evidence funnel run and this worker was built without one"
                    .into(),
            ));
        };
        let tenant = ctx
            .trial
            .as_ref()
            .map(|t| t.tenant_id().to_string())
            .ok_or_else(|| {
                logic(
                    "unregistered_trial",
                    "gate_advance needs a registered trial".into(),
                )
            })?;
        let user: Uuid = tenant.parse().map_err(|_| {
            logic(
                "tenant_is_not_a_user",
                format!("tenant `{tenant}` is not a user id"),
            )
        })?;
        let campaign_id: Uuid = manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| logic("invalid_manifest", "run_funnel needs a campaign_id".into()))?;
        let (delta, _profile, search_space) =
            study_worker::campaign_delta_and_profile(&self.pg, &tenant, campaign_id).await?;
        let spec = study_worker::evaluation_spec(&search_space)?;
        let incumbent = compare_worker::standing_incumbent(&self.pg, &tenant, campaign_id, &spec)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the incumbent: {e}")))?;

        ctx.progress(Progress {
            pct: Some(0.05),
            stage: Some("funnel".into()),
            message: Some("running the evidence funnel on the incumbent".into()),
        })
        .await;
        let ran = funnel
            .run(user, campaign_id, &spec, delta, &incumbent)
            .await
            .map_err(|e| JobError::infrastructure(format!("the funnel: {e}")))?;

        let experiment = study_worker::experiment_name(campaign_id);
        let (trial_id, dataset_id) = funnel_worker::funnel_trial(&self.pg, &tenant, &experiment)
            .await
            .map_err(|e| JobError::infrastructure(format!("finding the funnel trial: {e}")))?
            .ok_or_else(|| {
                logic(
                    "funnel_recorded_nothing",
                    format!(
                        "the funnel finished on `{experiment}` but no trial carries a                          `deflated_sharpe`; the evidence was not recorded (axis `{}`)",
                        ran.axis.param
                    ),
                )
            })?;
        let mut resolved = manifest.clone();
        resolved["trial_id"] = json!(trial_id);
        resolved["dataset_id"] = json!(dataset_id);
        resolved["sweep_axis"] = json!({
            "param": ran.axis.param, "center": ran.axis.center, "step": ran.axis.step
        });
        resolved["run_funnel"] = json!(false);
        Ok(resolved)
    }

    fn ledger(&self) -> ledger::pg::PgTrialLedger {
        ledger::pg::PgTrialLedger::new(self.pg.clone())
    }

    /// The market's filtered labels, one per observation, when a scope covers
    /// this strategy and its labels cover the whole series.
    ///
    /// Two refusals, both deliberate:
    ///
    /// * **No scope, or an ambiguous one** — the market is unconfigured, and
    ///   choosing a series for the candidate is the guess ADR-P3-07 refused to
    ///   make once and should not make here either.
    /// * **Partial coverage** — every observation gets a label or none are used.
    ///   Dropping the unlabelled days would silently change the denominator
    ///   Gate 11 attributes P&L over, and a regime share computed over a subset
    ///   of the sample is not the share it is reported as.
    ///
    /// The reason is returned either way so the job output can say which reading
    /// the gates got.
    async fn scope_labels(
        &self,
        req: &Request,
        timestamps: &[chrono::DateTime<chrono::Utc>],
    ) -> Result<LabelSource, sqlx::Error> {
        use crate::knowledge::{
            labels_as_of, RegimeScopeStore, RegimeStore, ScopeResolution, RULE_MODEL_VERSION,
        };

        if timestamps.is_empty() {
            return Ok(LabelSource::none("the trial has no return series"));
        }
        let scope = match RegimeScopeStore::new(self.pg.clone())
            .resolve(&req.asset_class, req.instrument.as_deref())
            .await?
        {
            ScopeResolution::Scope(s) => s,
            ScopeResolution::Ambiguous(names) => {
                return Ok(LabelSource::none(format!(
                    "{} market scopes claim asset class '{}' and the job named no instrument: {}",
                    names.len(),
                    req.asset_class,
                    names.join(", ")
                )));
            }
            ScopeResolution::Unconfigured => {
                return Ok(LabelSource::none(format!(
                    "no market scope is configured for '{}'{}",
                    req.asset_class,
                    req.instrument
                        .as_deref()
                        .map_or(String::new(), |s| format!(" covering {s}"))
                )));
            }
        };

        let (from, to) = (
            *timestamps.first().expect("non-empty"),
            *timestamps.last().expect("non-empty"),
        );
        let history = RegimeStore::new(self.pg.clone())
            .history(
                &scope.market_scope,
                RULE_MODEL_VERSION,
                // The labeller stamps a label at its bar close; a label in force
                // at the first observation may have been written well before it.
                from - chrono::Duration::days(LABEL_LOOKBEHIND_DAYS),
                to,
            )
            .await?;
        let resolved = labels_as_of(&history, timestamps);
        let labelled = resolved.iter().filter(|l| l.is_some()).count();
        if labelled < resolved.len() {
            return Ok(LabelSource::none(format!(
                "scope {} labels {labelled} of {} observations; a regime share over part of the \
                 sample is not the share it would be reported as",
                scope.market_scope,
                resolved.len()
            )));
        }
        Ok(LabelSource {
            labels: Some(
                resolved
                    .into_iter()
                    .map(|l| match l.expect("all labelled") {
                        crate::knowledge::RegimeLabel::LowVol => {
                            backtest::gates::evaluators::VolRegime::Low
                        }
                        crate::knowledge::RegimeLabel::MidVol => {
                            backtest::gates::evaluators::VolRegime::Mid
                        }
                        crate::knowledge::RegimeLabel::HighVol => {
                            backtest::gates::evaluators::VolRegime::High
                        }
                    })
                    .collect(),
            ),
            reason: format!("scope {} ({})", scope.market_scope, scope.series_symbol),
        })
    }
}

impl GateAdvanceWorker {
    /// Gate 15's inputs, built from the forward record of the paper deployment
    /// that traded this trial.
    ///
    /// `Ok(None)` is an answer, not a gap in this function: the trial has no
    /// deployment, or the deployment has no observed-knowledge rows yet (backfilled
    /// rows are not forward evidence, D-T1). The gate then says so in its own
    /// words. The kill criteria are a fact about the *deployment*, written before
    /// the first fill, so they come from its row and not from the observations.
    async fn paper_inputs(
        &self,
        tenant: &str,
        trial_id: Uuid,
    ) -> Result<Option<backtest::gates::evaluators::PaperInputs>, sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let deployment: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT deployment_id, kill_criteria FROM mlops.paper_deployment
              WHERE tenant_id = $1 AND trial_id = $2
              ORDER BY deployed_at DESC LIMIT 1",
        )
        .bind(tenant)
        .bind(trial_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some((deployment_id, kill_criteria)) = deployment else {
            return Ok(None);
        };
        let rows = reconciliation::store::load_observations(
            &self.pg,
            tenant,
            deployment_id,
            chrono::DateTime::<chrono::Utc>::MIN_UTC,
            chrono::Utc::now(),
        )
        .await?;
        Ok(paper_from_rows(&rows, &kill_criteria))
    }
}

/// Gate 15's inputs from a deployment's observations, without a database.
///
/// `None` for a window with no observed-knowledge row and for one whose rows are
/// not in a single numeraire — both are "no evidence", and the gate's own
/// inconclusive message is the right thing to say about each.
#[must_use]
pub fn paper_from_rows(
    rows: &[domain::forward::ForwardObservation],
    kill_criteria: &str,
) -> Option<backtest::gates::evaluators::PaperInputs> {
    let evidence = domain::forward::Gate15Evidence::from_observations(rows).ok()?;
    Some(
        backtest::gates::evaluators::PaperInputs::from_forward_evidence(
            &evidence,
            !kill_criteria.trim().is_empty(),
        ),
    )
}

/// How far before a series' first observation the labeller's history is read.
use crate::knowledge::LABEL_LOOKBEHIND_DAYS;

/// Where Gate 11's regimes came from, and why.
struct LabelSource {
    labels: Option<Vec<backtest::gates::evaluators::VolRegime>>,
    reason: String,
}

impl LabelSource {
    fn none(reason: impl Into<String>) -> Self {
        Self {
            labels: None,
            reason: reason.into(),
        }
    }
}

/// What the manifest has to say.
struct Request {
    experiment_id: String,
    trial_id: Uuid,
    profile_id: String,
    asset_class: String,
    /// The dataset the leakage suite audited. Gate 2 reads the newest run for
    /// exactly this subject — a clean run on a *different* dataset says nothing
    /// about this one.
    dataset_id: String,
    /// What the candidate trades, when the job says. It is how the market scope
    /// is resolved: a regime is a property of a market, and the market is the
    /// one the strategy is in (SPEC §5.4). Optional, because a job that does not
    /// say gets the asset class's scope when exactly one is configured and no
    /// labels at all when several are.
    instrument: Option<String>,
    /// The campaign this candidate belongs to, when it belongs to one. Naming it
    /// is not supplying a statistic: the worker then reads the campaign's own
    /// immutable DEFINE hash from the ledger, which is Gate 1's evidence.
    campaign_id: Option<Uuid>,
    /// The model versions the candidate binds, each with what it is used for (Set N N-3.6).
    /// A binding whose usage rules fail refuses the job before any evidence is gathered.
    model_bindings: Vec<backtest::gates::model_binding::ModelBinding>,
}

fn parse(manifest: &Value) -> Result<Request, JobError> {
    let field = |k: &str| -> Result<String, JobError> {
        manifest
            .get(k)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    format!("a gate_advance job needs `{k}`"),
                )
            })
    };
    let trial_id = field("trial_id")?.parse::<Uuid>().map_err(|e| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_manifest",
            format!("trial_id is not a uuid: {e}"),
        )
    })?;
    Ok(Request {
        experiment_id: field("experiment_id")?,
        trial_id,
        // The profile is named by the caller and then *looked up*, never
        // constructed from the manifest: thresholds are not something a
        // submission gets to supply (INV-23).
        profile_id: field("profile_id").unwrap_or_else(|_| "strict_v1".to_string()),
        asset_class: field("asset_class").unwrap_or_else(|_| "crypto".to_string()),
        dataset_id: field("dataset_id")?,
        instrument: manifest
            .get("instrument")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        campaign_id: manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
        model_bindings: match manifest.get("model_bindings") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    format!("model_bindings must be [{{model_id, version, uses}}]: {e}"),
                )
            })?,
        },
    })
}

/// The immutable profile for an id.
///
/// Two profiles exist in code and both are hard-coded threshold sets that match
/// the immutable rows in `mlops.gate_profile` (migration 0050). An unknown id is
/// refused rather than defaulted: judging a candidate under a profile nobody
/// named is judging it under thresholds nobody chose.
fn profile_for(id: &str) -> Result<GateProfile, JobError> {
    match id {
        "strict_v1" => Ok(GateProfile::strict_v1()),
        "paper_v1" => Ok(GateProfile::paper_v1()),
        other => Err(JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "unknown_gate_profile",
            format!("`{other}` is not a gate profile this build knows (INV-23)"),
        )),
    }
}

/// Attach the per-asset-class half of the profile — Gate 10's declared factor
/// battery and Gate 11's crisis windows — from
/// `mlops.gate_profile_asset_class` (migration 0050, ADR-P2-15/16).
///
/// The code-side constructors leave `asset_classes` empty, so before this
/// existed **Gate 10 was inconclusive for every candidate under every profile**,
/// reporting "profile strict_v1 declares no factor battery for asset class
/// `crypto`" — while the rows declaring exactly that battery had been in the
/// database since 0050. The factor series were the visible half of the gap; this
/// was the other half.
///
/// This reads the immutable table; it does not construct thresholds. A profile
/// whose per-asset-class rows are missing stays empty, and Gate 10 reports the
/// asset class as undeclared, which is then true.
async fn load_asset_classes(
    pg: &PgPool,
    tenant_id: &str,
    mut profile: GateProfile,
) -> Result<GateProfile, JobError> {
    let rows: Vec<(String, Vec<String>, Value)> = sqlx::query_as(
        "SELECT asset_class, factor_battery, crisis_windows
         FROM mlops.gate_profile_asset_class WHERE profile_id = $1",
    )
    .bind(profile.profile_id())
    .fetch_all(pg)
    .await
    .map_err(|e| JobError::infrastructure(format!("reading the profile's asset classes: {e}")))?;
    let _ = tenant_id;
    for (asset_class, factor_battery, crisis) in rows {
        // A malformed crisis-window blob is refused rather than silently read as
        // an empty list: "no crisis windows declared" and "the declaration did
        // not parse" are different facts, and only one of them is about the
        // world.
        let crisis_windows: Vec<backtest::gates::CrisisWindow> = serde_json::from_value(crisis)
            .map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "malformed_gate_profile",
                    format!("{asset_class}'s crisis windows do not parse: {e}"),
                )
            })?;
        profile = profile.with_asset_class(
            asset_class,
            backtest::gates::AssetClassGates {
                factor_battery,
                crisis_windows,
            },
        );
    }
    Ok(profile)
}

#[async_trait::async_trait]
impl Worker for GateAdvanceWorker {
    fn kind(&self) -> JobKind {
        JobKind::GateAdvance
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let resolved;
        let manifest = if manifest.get("run_funnel").and_then(Value::as_bool) == Some(true) {
            resolved = self.run_campaign_funnel(ctx, manifest).await?;
            &resolved
        } else {
            manifest
        };
        let req = parse(manifest)?;
        let profile = profile_for(&req.profile_id)?;
        let led = self.ledger();

        // The job service authorises a counted kind with a registered trial
        // (INV-16); its tenant is the one everything below reads under.
        let tenant = ctx
            .trial
            .as_ref()
            .map(|t| t.tenant_id().to_string())
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unregistered_trial",
                    "gate_advance counts as a trial and cannot run without one (INV-16)",
                )
            })?;

        // Gate 10's declared battery and Gate 11's crisis windows live in the
        // immutable per-asset-class table, not in the code-side constructor.
        // Without this the profile arrives with none and Gate 10 reports the
        // asset class as undeclared, whatever evidence exists.
        let profile = load_asset_classes(&self.pg, &tenant, profile).await?;

        // Set N N-3.6: a bound model that fails its usage rules (or has none measured) may not
        // size a position, so the candidate does not enter the stack.
        if let Err(why) =
            crate::models_usage_policy::check_bindings(&self.pg, &req.model_bindings).await
        {
            return Err(JobError::logic(
                ledger::TerminalReason::GateFailed,
                "model_binding_refused",
                why,
            ));
        }

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("gathering".into()),
            message: None,
        })
        .await;

        // ── the evidence, each piece absent rather than defaulted ───────────
        fn infra(what: &'static str) -> impl Fn(ledger::LedgerError) -> JobError {
            move |e| JobError::infrastructure(format!("reading {what}: {e}"))
        }

        let n_eff = led
            .n_eff_async(&tenant)
            .await
            .map_err(infra("N_eff"))?
            .value();
        let trial_count = led
            .experiment_trial_count_async(&tenant, &req.experiment_id)
            .await
            .map_err(infra("the trial count"))?;

        // The numbers the funnel computed, read from the ledger rather than
        // taken from the manifest (ADR-P2-31). A statistic that was never
        // recorded stays `None` and its gate stays inconclusive — which is the
        // correct answer to "did this pass Gate 7" when nobody measured PBO.
        let stats = led
            .statistics_async(&tenant, req.trial_id)
            .await
            .map_err(infra("the trial statistics"))?;
        let stat = |k: &str| stats.get(k).copied().filter(|v| v.is_finite());

        let series = led
            .return_series_async(&tenant, req.trial_id)
            .await
            .map_err(infra("the return series"))?;
        let (timestamps, returns) = series
            .map(|s| (s.timestamps, s.returns))
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        // Gates 9 and 11 are *derived from the series the ledger already holds*,
        // not supplied. That distinction is the point: a manifest field carrying
        // a Sharpe or a regime share would be a gate statistic the submitter
        // chose, which is §12.7's gate-hacking in its purest form. Everything
        // this worker evaluates, it computes (ADR-P2-31).
        let length = length_inputs(&timestamps, &returns, n_eff);
        let strategy_series: Vec<(chrono::DateTime<chrono::Utc>, f64)> = timestamps
            .iter()
            .copied()
            .zip(returns.iter().copied())
            .collect();

        let family_rows = led
            .experiment_family_async(&tenant, &req.experiment_id)
            .await
            .map_err(infra("the candidate family"))?;
        let candidate_index = family_rows
            .iter()
            .position(|(id, _)| *id == req.trial_id)
            .unwrap_or(0);
        let family: Vec<Vec<f64>> = family_rows.into_iter().map(|(_, r)| r).collect();

        let leakage = led
            .latest_leakage_run_async(&tenant, &req.dataset_id)
            .await
            .map_err(infra("the leakage run"))?
            .map(
                |(subject, checks_run, blocking_count, flag_count, finished_at)| LeakageRun {
                    subject,
                    checks_run,
                    blocking_count,
                    flag_count,
                    finished_at,
                },
            );

        // The seed the bootstrap draws from is the platform's, not the
        // candidate's (§12.7). Falling back to the trial's own id when the
        // subject has no campaign keeps it out of the agent's reach either way:
        // the agent cannot choose its trial id any more than it can read the
        // campaign seed.
        let bootstrap_seed = trial_seed(req.trial_id);

        // Gate 1's evidence, read from the campaign rather than supplied.
        let prereg_hash = match req.campaign_id {
            Some(id) => led
                .campaign_define_hash_async(&tenant, id)
                .await
                .map_err(infra("the campaign DEFINE hash"))?,
            None => None,
        };

        // The platform's own filtered labels for this strategy's market, when a
        // scope is configured (SPEC §5.4, ADR-P3-08). One per observation,
        // aligned causally: the label the platform could have known at that
        // instant, never the one it computed afterwards.
        //
        // This is what takes Gates 6 and 11 from "measured off the candidate's
        // own returns, which is the weak reading ADR-P2-33 settled for" to a
        // market reading. When no scope covers the strategy the labels stay
        // `None` and both gates fall back to that weak reading, which the
        // verdict still says out loud.
        let market_labels = self
            .scope_labels(&req, &timestamps)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the regime scope: {e}")))?;

        // Gate 6 pairs the walk-forward Sharpe with how many regimes it spanned.
        // Counted from the market's labels when they exist, and otherwise from
        // the candidate's own terciles — a weaker reading than a market one and
        // a much stronger reading than none (ADR-P2-33). An absent series leaves
        // it `None`, which keeps Gate 6 inconclusive rather than passing on one
        // regime.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let walk_forward_regimes = stat("walk_forward_regimes")
            .map(|v| v as u32)
            .or_else(|| {
                market_labels
                    .labels
                    .as_ref()
                    .map(|l| backtest::gates::stack::distinct_regimes(l))
            })
            .or_else(|| {
                (strategy_series.len() > VOL_WINDOW).then(|| {
                    backtest::gates::stack::distinct_regimes(
                        &backtest::gates::evaluators::vol_tercile_regimes(
                            &strategy_series,
                            VOL_WINDOW,
                        ),
                    )
                })
            });

        // The shaped evidence gates 3, 4, 10 and 12 read (migration 0119). Like
        // the scalars, it is read from the ledger and never taken from the
        // manifest.
        let shaped = led
            .gate_evidence_async(&tenant, req.trial_id)
            .await
            .map_err(infra("the gate evidence"))?;
        let shaped = shaped.unwrap_or_default();

        // Gate 3's ladder is a shape, not a scalar. The stored rungs are used
        // when the cost sweep recorded them; the recorded breakeven multiple is
        // the fallback, turned back into the two rungs that bracket it — alive
        // at 1×, dead at the recorded multiple — which is exactly what
        // `breakeven_multiple` would interpolate from.
        //
        // The fallback is deliberately narrow. `b > 1.0` only: a breakeven at or
        // below 1× means the edge does not survive its *own* cost model, and
        // reconstructing "alive at 1×" for it would assert the opposite of what
        // was measured. Such a candidate reaches Gate 3 through the stored rungs
        // or not at all, and without them the gate says it cannot judge — which
        // is worse for nobody, because a candidate that dies at 1× fails Gate 1
        // on its median long before this.
        let cost_ladder = cost_ladder(&shaped, stat("breakeven_cost_multiple"));

        // Gate 4. Assembled only when both halves are present; a half-filled
        // `CapacityInputs` with zeros in it is the exact shape of a gate given an
        // input instead of evidence.
        let (mut adv, mut vol, mut peak, mut sharpe_by_aum) = (
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
            Vec::new(),
        );
        let capacity = shaped.capacity_base.and_then(|base| {
            backtest::capacity::capacity_inputs(
                &shaped.instrument_pnl,
                base,
                &shaped.capacity_curve,
                &mut adv,
                &mut vol,
                &mut peak,
                &mut sharpe_by_aum,
            )
        });

        // Gate 12. The concentration half refuses to cross a numeraire; when it
        // does, the pairs are empty and the numeraire is `None`, and the gate
        // reports that rather than summing two units together.
        let (numeraire, per_instrument_pnl) = match ledger::evidence::per_instrument_pnl(
            &shaped.instrument_pnl,
        ) {
            Ok((unit, pairs)) => (Some(unit), pairs),
            Err(why) => {
                if !shaped.instrument_pnl.is_empty() {
                    tracing::info!(trial = %req.trial_id, reason = %why, "gate 12 concentration");
                }
                (None, Vec::new())
            }
        };
        let neighbourhood: Vec<(f64, f64)> = shaped
            .neighbourhood
            .iter()
            .map(|p| (p.param_value, p.metric))
            .collect();
        let perturbation = shaped.neighbourhood.iter().find(|p| p.is_center).map(|c| {
            backtest::gates::PerturbationInputs {
                center: c.metric,
                neighbourhood: &neighbourhood,
                per_instrument_pnl: &per_instrument_pnl,
                numeraire: numeraire.as_deref(),
            }
        });

        // Gate 10. The stored factor series, aligned period-for-period to the
        // strategy's **own stored net return series** — the same series N_eff and
        // Gates 13/14 read, not a series the manifest supplied.
        //
        // `MIN_ATTRIBUTION_PERIODS` is `newey_west_fit`'s own floor. Passing it a
        // shorter overlap would make it return `None` anyway; refusing here means
        // the reason names the overlap rather than the regression.
        let aligned = backtest::factors::align_to_strategy(
            &timestamps,
            &returns,
            &shaped.factors,
            MIN_ATTRIBUTION_PERIODS,
        );
        let factors =
            aligned
                .as_ref()
                .map(|(net, battery)| backtest::gates::evaluators::FactorInputs {
                    strategy_net: net,
                    factors: battery,
                    // Which factor is "the market" is a property of the declared
                    // battery, not of this worker. The profile names it first; falling
                    // back to the crypto market factor only when the profile declared
                    // nothing, in which case Gate 10 is already inconclusive.
                    market_factor: profile
                        .asset_class(&req.asset_class)
                        .and_then(|d| d.factor_battery.first().cloned())
                        .unwrap_or_else(|| backtest::factors::CRYPTO_MARKET.to_string()),
                    // Neutrality is a claim a strategy makes, and nothing in this
                    // platform records one yet. An unmade claim is not checked — which
                    // is the spec's rule, not a leniency (§12.3 Gate 10).
                    neutral_claim: false,
                });

        ctx.progress(Progress {
            pct: Some(0.5),
            stage: Some("evaluating".into()),
            message: None,
        })
        .await;

        // Gate 15's forward evidence, from the deployment's own observations.
        //
        // This was `paper: None` for every candidate, so Gate 15 reported "this
        // candidate has not been run in paper" about candidates that had been,
        // while `reconciliation` recorded their observations into a table
        // nothing here read (OPEN-TASKS C1.6). Read like every other input: from
        // the ledger's own tables, never from the manifest, and absent when
        // absent.
        let paper = self
            .paper_inputs(&tenant, req.trial_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the forward record: {e}")))?;

        let evidence = StackEvidence {
            // Gate 1 is structural: the claim was hash-locked before anything ran
            // or it was not. The hash comes from the campaign's own immutable
            // DEFINE event, never from the manifest — a hash the submitter
            // supplies is a claim the submitter could have written afterwards.
            prereg_hash: prereg_hash.clone(),
            leakage_run: leakage.as_ref(),
            leakage_checks: LEAKAGE_CHECKS,
            // Gate 3's ladder is a shape, not a scalar, so the recorded
            // breakeven multiple is turned back into the two rungs that bracket
            // it: alive at 1×, dead at the recorded multiple. That is exactly
            // what `breakeven_multiple` would interpolate from, and nothing is
            // claimed that the cost sweep did not measure.
            cost_ladder: &cost_ladder,
            capacity: capacity.as_ref(),
            cpcv_p05_sharpe: stat("cpcv_p05_sharpe"),
            walk_forward_sharpe: stat("walk_forward_sharpe"),
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            walk_forward_regimes,
            pbo: stat("pbo"),
            dsr: stat("deflated_sharpe"),
            length,
            factors: factors.as_ref(),
            // Gate 11 reads the platform's filtered labels when a market scope
            // is configured for what this strategy trades. When one is not, it
            // is handed the strategy's own series as the market and the
            // evaluator flags that case (`labelled_from_strategy`) rather than
            // pretending it is a market reading — a flagged verdict is more
            // useful than a missing one, because a strategy whose entire P&L
            // came from one volatility regime is worth catching either way
            // (ADR-P2-29, ADR-P3-08).
            market_returns: &strategy_series,
            strategy_returns: &strategy_series,
            regime_labels: market_labels.labels.as_deref(),
            perturbation: perturbation.as_ref(),
            return_series: &returns,
            family: &family,
            candidate_index,
            bootstrap_seed,
            paper,
            capital_fraction: ledger::AllowedFraction::default(),
            n_eff,
            trial_count,
            asset_class: &req.asset_class,
            experiment_id: Some(req.experiment_id.clone()),
            trial_id: Some(req.trial_id),
            campaign_id: req.campaign_id,
        };

        let verdict = evaluate_stack(&profile, &evidence, chrono::Utc::now());
        // EV-09 / RB-3.3: a campaign candidate cannot pass Gate 3 or anything after it unless
        // the campaign's matched random arm has completed. The rule is
        // `research::random_arm::gate_check` (through `arm_gate`); the gate stack only applies
        // the answer, because `backtest` cannot depend on the crates that own the arm.
        let arm_requirement = match req.campaign_id {
            Some(campaign) => Some(
                match crate::knowledge_routes::arm_gate(&self.pg, campaign).await {
                    Ok(()) => {
                        backtest::gates::random_arm_gate::ArmRequirement::met(campaign.to_string())
                    }
                    Err(why) => backtest::gates::random_arm_gate::ArmRequirement::refused(
                        campaign.to_string(),
                        why,
                    ),
                },
            ),
            None => None,
        };
        let verdict =
            backtest::gates::random_arm_gate::apply_to_stack(verdict, arm_requirement.as_ref());
        let ids = record_stack(&led, &tenant, &profile, &evidence, &verdict)
            .map_err(|e| JobError::infrastructure(format!("recording the gate stack: {e}")))?;

        let inconclusive: Vec<i32> = verdict.inconclusive().iter().map(|o| o.gate_no).collect();
        let passed = verdict.outcomes.iter().filter(|o| o.passed).count();

        Ok(JobOutput {
            summary: Some(format!(
                "{passed} of {GATE_COUNT} gates passed under {} ({} could not be evaluated){}",
                profile.profile_id(),
                inconclusive.len(),
                verdict
                    .blocked_at
                    .map_or(String::new(), |g| format!("; blocked at gate {g}")),
            )),
            result: json!({
                "experiment_id": req.experiment_id,
                "trial_id": req.trial_id,
                "profile_id": profile.profile_id(),
                "passed": passed,
                "of": GATE_COUNT,
                "blocked_at": verdict.blocked_at,
                "inconclusive": inconclusive,
                "n_eff": n_eff,
                "trial_count": trial_count,
                "verdicts_recorded": ids.len(),
                // Which reading Gates 6 and 11 got. A market reading and a
                // fallback to the candidate's own volatility are different
                // claims, and the record says which one was made.
                "regime_labels": if market_labels.labels.is_some() {
                    "market"
                } else {
                    "strategy_fallback"
                },
                "regime_label_source": market_labels.reason,
            }),
            ..Default::default()
        })
    }
}

/// Gate 3's ladder, from the stored rungs when the cost sweep recorded them and
/// from the recorded breakeven multiple when it did not.
///
/// The fallback reconstructs the two rungs that bracket zero — alive at 1x, dead
/// at the recorded multiple — which is exactly what `breakeven_multiple` would
/// interpolate from, so nothing is claimed that the sweep did not measure.
///
/// It is deliberately narrow in two ways.
///
/// * **`b > 1.0` only.** A breakeven at or below 1x means the edge does not
///   survive its own cost model. Reconstructing "alive at 1x" for such a
///   candidate would assert the opposite of what was measured, and a gate fed an
///   input that contradicts its evidence is worse than a gate that says it
///   cannot judge.
/// * **Stored rungs win.** They are the measurement; the scalar is its summary.
fn cost_ladder(
    shaped: &ledger::evidence::TrialEvidence,
    recorded_breakeven: Option<f64>,
) -> Vec<backtest::gates::evaluators::CostRung> {
    let rung = |multiple: f64, net_metric: f64| backtest::gates::evaluators::CostRung {
        multiple,
        net_metric,
    };
    if shaped.cost_ladder.len() >= 2 {
        return shaped
            .cost_ladder
            .iter()
            .map(|r| rung(r.multiple, r.net_metric))
            .collect();
    }
    match recorded_breakeven {
        Some(b) if b > 1.0 && b.is_finite() => vec![rung(1.0, 1.0), rung(b, 0.0)],
        _ => Vec::new(),
    }
}

/// Gate 9's measurements, derived from the stored series.
///
/// `None` when there is no series: an absent measurement makes the gate
/// inconclusive, which is never a pass. What it must never do is substitute a
/// default, because every one of these quantities has a value that passes.
///
/// The event count is the number of **non-zero** returns. A flat period is not
/// an independent observation of anything — it is the strategy not trading — and
/// counting flat bars toward the event floor is how a 5-minute backtest claims
/// 300 events in a week.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn length_inputs(
    timestamps: &[chrono::DateTime<chrono::Utc>],
    returns: &[f64],
    n_eff: f64,
) -> Option<backtest::gates::evaluators::LengthInputs> {
    if returns.len() < 2 || timestamps.len() != returns.len() {
        return None;
    }
    let first = *timestamps.first()?;
    let last = *timestamps.last()?;
    let days = (last - first).num_seconds() as f64 / 86_400.0;
    if days <= 0.0 {
        return None;
    }
    let years = days / 365.25;

    // Annualized from the observed sampling interval rather than an assumed one:
    // a daily series and a minute series with the same per-step Sharpe are not
    // the same annual Sharpe, and assuming 252 for both inflates one of them.
    let steps_per_year = returns.len() as f64 / years;
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    // A relative floor, not `> 0.0`. Summing five hundred identical values leaves
    // a standard deviation around 1e-19 from rounding alone, and dividing by it
    // produces a Sharpe of 1e15 that passes every gate in the stack. The floor is
    // relative to the series' own magnitude so it means the same thing for
    // returns measured in percent and in basis points.
    let magnitude = returns.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
    if !(sd.is_finite() && sd > magnitude * 1e-12) {
        return None;
    }
    let sharpe = mean / sd * steps_per_year.sqrt();
    if !sharpe.is_finite() {
        return None;
    }

    let independent_events =
        u32::try_from(returns.iter().filter(|r| **r != 0.0).count()).unwrap_or(u32::MAX);

    Some(backtest::gates::evaluators::LengthInputs {
        sharpe,
        n_eff,
        years,
        independent_events,
    })
}

/// A deterministic seed from a trial id.
///
/// Deterministic so a re-evaluation of the same trial produces the same
/// bootstrap, which is what makes a recorded verdict reproducible by a third
/// party. Derived from the id rather than supplied, so there is no parameter to
/// resample against.
fn trial_seed(trial_id: Uuid) -> u64 {
    let b = trial_id.as_bytes();
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leakage_check_list_is_the_suites_own() {
        // Gate 2 refuses a run missing any of these, so drifting from the
        // suite's actual checks would make every candidate inconclusive — loudly,
        // which is the right direction, but this is where to notice it.
        assert_eq!(LEAKAGE_CHECKS.len(), 5);
        assert!(LEAKAGE_CHECKS.contains(&"cv_wf_gap"));
        assert!(LEAKAGE_CHECKS.contains(&"synthetic_injection"));
    }

    #[test]
    fn an_unknown_profile_is_refused_rather_than_defaulted() {
        assert!(profile_for("strict_v1").is_ok());
        assert!(profile_for("paper_v1").is_ok());
        let err = profile_for("lenient_v1").unwrap_err();
        assert_eq!(err.terminal, ledger::TerminalReason::IntegrityRejected);
        assert_eq!(err.code, "unknown_gate_profile");
    }

    #[test]
    fn the_manifest_must_name_what_is_being_judged() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({ "experiment_id": "e" })).is_err());
        assert!(parse(&json!({
            "experiment_id": "e",
            "trial_id": "not-a-uuid",
            "dataset_id": "ds"
        }))
        .is_err());

        let ok = parse(&json!({
            "experiment_id": "e",
            "trial_id": Uuid::nil().to_string(),
            "dataset_id": "ds:abc"
        }))
        .expect("a complete manifest");
        assert_eq!(ok.experiment_id, "e");
        assert_eq!(ok.dataset_id, "ds:abc");
        // The profile defaults to the strict one when unnamed. Defaulting
        // *toward* the stricter profile is the only safe direction.
        assert_eq!(ok.profile_id, "strict_v1");
    }

    /// Set N N-3.6: the manifest carries the bound models, and a malformed list is refused
    /// rather than read as "no bindings".
    #[test]
    fn model_bindings_are_parsed_and_a_malformed_list_is_refused() {
        let base = |extra: Value| {
            let mut m = json!({"experiment_id": "e", "trial_id": Uuid::nil().to_string(), "dataset_id": "ds"});
            m["model_bindings"] = extra;
            m
        };
        assert!(parse(&base(Value::Null)).unwrap().model_bindings.is_empty());
        let ok = parse(&base(
            json!([{"model_id": "vol", "version": 2, "uses": ["vol", "tail_risk"]}]),
        ))
        .unwrap();
        assert_eq!(ok.model_bindings[0].label(), "vol@2");
        assert_eq!(ok.model_bindings[0].uses.len(), 2);
        assert!(parse(&base(
            json!([{"model_id": "vol", "version": 2, "uses": ["magic"]}])
        ))
        .is_err());
        assert!(parse(&base(json!("vol@2"))).is_err());
    }

    /// A cost ladder recorded by the sweep is the measurement; the scalar is
    /// only its summary, so the rungs win.
    #[test]
    fn stored_cost_rungs_beat_the_reconstructed_pair() {
        let mut shaped = ledger::evidence::TrialEvidence::produced_by("funnel");
        shaped.cost_ladder = vec![
            ledger::evidence::CostRung {
                multiple: 1.0,
                net_metric: 0.20,
            },
            ledger::evidence::CostRung {
                multiple: 4.0,
                net_metric: -0.05,
            },
        ];
        let ladder = cost_ladder(&shaped, Some(9.0));
        assert_eq!(ladder.len(), 2);
        assert!(
            (ladder[1].multiple - 4.0).abs() < f64::EPSILON,
            "not the scalar's 9x"
        );
    }

    /// The adversarial case for the fallback. A breakeven at or below 1x says
    /// the edge does not survive its own cost model; reconstructing "alive at
    /// 1x, dead at 0.8x" would hand Gate 3 a ladder that contradicts the
    /// measurement it came from. No ladder is built, and Gate 3 says it cannot
    /// judge.
    #[test]
    fn a_breakeven_at_or_below_one_is_not_reconstructed_into_a_ladder() {
        let shaped = ledger::evidence::TrialEvidence::produced_by("funnel");
        for b in [0.0, 0.5, 1.0, f64::NAN] {
            assert!(
                cost_ladder(&shaped, Some(b)).is_empty(),
                "a breakeven of {b} must not become a ladder"
            );
        }
        assert!(cost_ladder(&shaped, None).is_empty());
        assert_eq!(cost_ladder(&shaped, Some(3.5)).len(), 2);
    }

    /// Nothing recorded means no ladder at all, which makes Gate 3
    /// inconclusive. The failure mode this forbids is a default ladder that
    /// reads as "the edge survives every rung tried".
    #[test]
    fn no_cost_evidence_produces_no_ladder_rather_than_a_default_one() {
        let shaped = ledger::evidence::TrialEvidence::default();
        assert!(cost_ladder(&shaped, None).is_empty());
        let verdict = backtest::gates::evaluators::gate_3_cost_sensitivity(
            &GateProfile::strict_v1(),
            &cost_ladder(&shaped, None),
        );
        assert!(
            verdict.inconclusive && !verdict.passed,
            "{}",
            verdict.detail
        );
    }

    #[test]
    fn the_bootstrap_seed_is_deterministic_and_not_a_parameter() {
        let id = Uuid::new_v4();
        assert_eq!(
            trial_seed(id),
            trial_seed(id),
            "a re-evaluation must reproduce"
        );
        assert_ne!(trial_seed(id), trial_seed(Uuid::new_v4()));
    }
}

#[cfg(test)]
mod derivation_tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn daily(n: usize, r: f64) -> (Vec<chrono::DateTime<chrono::Utc>>, Vec<f64>) {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts = (0..n).map(|i| start + Duration::days(i as i64)).collect();
        let rs = (0..n)
            .map(|i| if i % 2 == 0 { r } else { r * 0.5 })
            .collect();
        (ts, rs)
    }

    #[test]
    fn no_series_means_no_measurement_rather_than_a_default() {
        assert!(length_inputs(&[], &[], 10.0).is_none());
        let (ts, _) = daily(5, 0.01);
        assert!(
            length_inputs(&ts, &[0.01], 10.0).is_none(),
            "mismatched lengths"
        );
    }

    /// A constant series has no Sharpe. Returning one would be a division by a
    /// standard deviation of zero dressed up as a measurement.
    #[test]
    fn a_flat_series_has_no_sharpe() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts: Vec<_> = (0..500).map(|i| start + Duration::days(i)).collect();
        assert!(length_inputs(&ts, &vec![0.001; 500], 10.0).is_none());
    }

    #[test]
    fn the_span_is_measured_not_assumed() {
        let (ts, rs) = daily(731, 0.001);
        let l = length_inputs(&ts, &rs, 42.0).expect("measurable");
        assert!(
            (l.years - 2.0).abs() < 0.02,
            "two years of daily bars: {}",
            l.years
        );
        assert!(
            (l.n_eff - 42.0).abs() < f64::EPSILON,
            "N_eff is the platform's, not derived"
        );
    }

    /// Annualizing from the observed sampling interval rather than assuming 252:
    /// the same per-step Sharpe on minute bars and on daily bars is a wildly
    /// different annual number, and assuming one rate inflates the other.
    #[test]
    fn the_sharpe_is_annualized_from_the_observed_interval() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let n = 2_000;
        let rs: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { 0.001 } else { 0.0005 })
            .collect();

        let daily_ts: Vec<_> = (0..n).map(|i| start + Duration::days(i as i64)).collect();
        let minute_ts: Vec<_> = (0..n)
            .map(|i| start + Duration::minutes(i as i64))
            .collect();

        let d = length_inputs(&daily_ts, &rs, 10.0).unwrap();
        let m = length_inputs(&minute_ts, &rs, 10.0).unwrap();
        assert!(
            m.sharpe > d.sharpe * 10.0,
            "a minute series annualizes to far more: {} vs {}",
            m.sharpe,
            d.sharpe
        );
    }

    /// A flat bar is the strategy not trading. Counting it toward the event floor
    /// is how a week of 5-minute bars claims three hundred independent events.
    #[test]
    fn flat_periods_are_not_independent_events() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts: Vec<_> = (0..1_000).map(|i| start + Duration::days(i)).collect();
        // Ten real moves, the rest flat.
        let rs: Vec<f64> = (0..1_000)
            .map(|i| if i % 100 == 0 { 0.02 } else { 0.0 })
            .collect();
        let l = length_inputs(&ts, &rs, 10.0).expect("measurable");
        assert_eq!(l.independent_events, 10, "only the moves count");
    }

    fn observation(
        source: domain::forward::ObservationSource,
        matched: bool,
    ) -> domain::forward::ForwardObservation {
        domain::forward::ForwardObservation {
            deployment_id: Uuid::new_v4(),
            strategy_id: "s".into(),
            bar: chrono::Utc::now(),
            realised_return: 0.001,
            expected_return: 0.001,
            numeraire: domain::numeraire::Numeraire::Usd,
            source,
            signal_matched: matched,
            realised_slippage_bps: 2.0,
            modelled_slippage_bps: 2.0,
            realised_turnover: 1.0,
            expected_turnover: 1.0,
            model_attributable_rejects: 0,
            observed_at: chrono::Utc::now(),
        }
    }

    /// Gate 15 used to be `paper: None` for every candidate. A deployment with
    /// observed rows now yields inputs, with the kill criteria taken from the
    /// deployment.
    #[test]
    fn observed_rows_become_gate_15_inputs() {
        use domain::forward::ObservationSource::Observed;
        let rows: Vec<_> = (0..5).map(|i| observation(Observed, i != 0)).collect();
        let p = paper_from_rows(&rows, "stop at 2x backtest max drawdown").expect("evidence");
        assert!((p.signal_match - 0.8).abs() < 1e-9);
        assert_eq!(p.observations, 5);
        assert!(p.kill_criteria_registered);
        // Blank criteria are not criteria: the gate then refuses to judge.
        let blank = paper_from_rows(&rows, "   ").unwrap();
        assert!(!blank.kill_criteria_registered);
    }

    /// Backfilled rows are not forward evidence (D-T1), and a window with no
    /// rows is no evidence at all: both leave Gate 15 inconclusive.
    #[test]
    fn backfilled_or_absent_rows_give_the_gate_nothing_to_judge() {
        use domain::forward::ObservationSource::Backfill;
        let rows: Vec<_> = (0..5).map(|_| observation(Backfill, true)).collect();
        assert!(paper_from_rows(&rows, "k").is_none());
        assert!(paper_from_rows(&[], "k").is_none());
    }
}
