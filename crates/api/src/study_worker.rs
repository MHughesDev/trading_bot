//! The `Study` worker: the EXPERIMENT phase's child job (SPEC §10, §11.5;
//! checklist 2.1; ADR-P2-04, ADR-P2-34).
//!
//! A campaign's EXPERIMENT phase dispatches one child job, and this is it. Its
//! job is not to find a winner — nothing here ranks, selects or promotes. Its
//! job is to **produce the paired evidence the COMPARE phase judges**: one
//! challenger drawn from the campaign's pre-registered search space, the
//! campaign's declared benchmark beside it, and both measured on the *same*
//! replicates of the §11.5 nuisance grid.
//!
//! ## Why both arms, every cycle
//!
//! §11.5's difference is **paired**: the same nuisance draw applied to A and to
//! B, so everything the two share cancels and the variance being estimated is
//! the variance of the difference rather than of the level. The cheapest way to
//! guarantee that pairing is for one worker to measure both arms against one
//! grid, which is what this does. Re-measuring the benchmark each cycle is not
//! waste: the run cache serves it, the ledger settles the look as
//! `deduplicated` naming the prior trial, and the measurement row is refused as
//! a duplicate. The count of looks climbs, because looking again is looking
//! again (ADR-P0-01).
//!
//! ## Why the grid is a campaign constant
//!
//! The replicate grid is derived from the campaign's immutable `define_hash`
//! and nothing else — not from the phase position, not from the clock, and not
//! from anything a caller passes. Two consequences, both load-bearing:
//!
//! * every cycle measures the incumbent on the *same* replicates, so the two
//!   arms stay pairable across cycles rather than accumulating unpaired samples;
//! * a re-queued study re-derives the same grid, hits the unique index, and
//!   writes nothing — the phase's side effect is idempotent at its key, which is
//!   what ADR-P2-04 buys instead of a workflow engine.
//!
//! ## What this build cannot honestly vary
//!
//! §11.5 names three nuisance sources: the history window, the cost assumption
//! and the stochastic-fill seed. **Only the window is real here.** No executor
//! in this repository reads `RunConfig::cost_model_ref` — the simulator takes
//! its costs from the strategy definition — and the simulation is deterministic
//! given its bars, so a differing seed changes the `run_id` and nothing else.
//!
//! Perturbing `cost_model_ref` anyway would produce measurements that *claim* a
//! cost ladder nobody applied, which is a worse failure than a narrow grid: it
//! is a narrow grid that reads as a wide one. So the grid varies the window,
//! records `cost_bp = 0`, and the COMPARE phase raises
//! `cost_nuisance_unrandomized` on every verdict built from it (ADR-P2-36). The
//! flag comes off when an executor honours the cost reference, not before.

// `JobError` is the job service's wire type, capped at 400 bytes on purpose
// (JB-07); boxing it would put a `Box` in every fallible worker helper here.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use backtest::run::{
    Backtest, DataSlice, EvalResolution, InMemoryRunStore, MetricKind, ParamMap, RunConfig,
    RunConfigBuilder, RunDispatch, RunExecutor, RunStatus,
};
use chrono::{DateTime, Utc};
use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use ledger::campaign::{CampaignDefinition, Dispatcher};
use ledger::evidence::{ReplicateKey, ReplicateMeasurement, INCUMBENT_ARM};
use ledger::DecisionKind;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// `produced_by` on every measurement this worker writes.
pub const PRODUCED_BY: &str = "campaign_study_worker";

/// Replicates a campaign gets when its search space does not declare a count.
/// §11.5's target; a campaign may pre-register more, never fewer than
/// [`backtest::stats::compare::MIN_REPLICATES`].
pub const DEFAULT_REPLICATES: usize = backtest::stats::compare::TARGET_REPLICATES;

/// Runs the EXPERIMENT phase of one campaign.
pub struct StudyWorker {
    pg: PgPool,
    executor: Arc<dyn RunExecutor>,
}

impl StudyWorker {
    /// Build over the platform's pool and Run executor.
    ///
    /// The executor is injected rather than constructed here for the same
    /// reason `SuiteManager` injects one: the simulator-backed executor needs a
    /// runtime handle and a ClickHouse connection, and a test needs neither.
    #[must_use]
    pub fn new(pg: PgPool, executor: Arc<dyn RunExecutor>) -> Self {
        Self { pg, executor }
    }

    fn ledger(&self) -> ledger::pg::PgTrialLedger {
        ledger::pg::PgTrialLedger::new(self.pg.clone())
    }
}

// ── the manifest ─────────────────────────────────────────────────────────────

/// What the campaign driver hands its EXPERIMENT child.
///
/// Four fields, and none of them is evidence. The driver names *which campaign*
/// and *where in the fold*; everything the study measures against — the search
/// space, the benchmark, the objective, the effect size — is read from the
/// campaign's immutable row (ADR-P2-31). A manifest that carried the search
/// space would be a search space the submitter chose after DEFINE.
struct Request {
    campaign_id: Uuid,
    seq: i64,
}

fn parse(manifest: &Value) -> Result<Request, JobError> {
    let bad = |m: &str| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_manifest",
            m.to_string(),
        )
    };
    let campaign_id: Uuid = manifest
        .get("campaign_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("a study job needs a `campaign_id`"))?;
    let seq = manifest
        .get("seq")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("a study job needs the fold position `seq` it was dispatched at"))?;
    if seq < 0 {
        return Err(bad("`seq` is a fold position and cannot be negative"));
    }
    Ok(Request { campaign_id, seq })
}

// ── the campaign's pre-registered plan ───────────────────────────────────────

/// The runnable half of a campaign's DEFINE facts, read from `search_space`.
///
/// SPEC §10's DEFINE block names the objective, the benchmark, the effect size,
/// the budget, the floor and the gate profile — but not what to *run*. The
/// `search_space` column carries that, and this is the shape it must have
/// (ADR-P2-34). Every field is required and none is defaulted: a campaign that
/// has not said which strategy, which universe and which window it is searching
/// has not said what it is doing, and inventing the answer here would make the
/// pre-registration hash cover a plan nobody wrote.
struct StudyPlan {
    strategy_ref: String,
    strategy_version: String,
    universe_ref: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    eval_resolution: EvalResolution,
    cost_model_ref: String,
    sizing_ref: String,
    data_snapshot: String,
    /// The candidate parameter sets the dispatcher draws from.
    grid: Vec<ParamMap>,
    /// The benchmark's parameters — the incumbent arm.
    benchmark_params: ParamMap,
    /// How many paired replicates this campaign pre-registered.
    replicates: usize,
}

fn plan_error(what: &str) -> JobError {
    JobError::logic(
        ledger::TerminalReason::IntegrityRejected,
        "campaign_search_space_unrunnable",
        format!(
            "this campaign's search_space {what}; EXPERIMENT cannot invent it, because a plan \
             invented after DEFINE is not the plan the define_hash covers (§10)"
        ),
    )
}

fn param_map(v: Option<&Value>) -> Option<ParamMap> {
    let obj = v?.as_object()?;
    Some(obj.iter().map(|(k, x)| (k.clone(), x.clone())).collect())
}

fn parse_plan(def: &CampaignDefinition) -> Result<StudyPlan, JobError> {
    let space = &def.search_space;
    let text = |k: &str| -> Result<String, JobError> {
        space
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| plan_error(&format!("does not name `{k}`")))
    };
    let time = |k: &str| -> Result<DateTime<Utc>, JobError> {
        space
            .get(k)
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(|| plan_error(&format!("does not name `{k}` as an RFC-3339 instant")))
    };

    let start = time("start")?;
    let end = time("end")?;
    if end <= start {
        return Err(plan_error("names an empty or reversed window"));
    }

    let eval_resolution = match space.get("eval_resolution").and_then(Value::as_str) {
        Some("1m") => EvalResolution::Min1,
        Some("5m") => EvalResolution::Min5,
        Some("10m") => EvalResolution::Min10,
        Some("15m") => EvalResolution::Min15,
        Some("30m") => EvalResolution::Min30,
        Some("1h") => EvalResolution::Hour1,
        Some("1d") => EvalResolution::Day1,
        _ => return Err(plan_error("does not name a known `eval_resolution`")),
    };

    let grid: Vec<ParamMap> = space
        .get("grid")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(|e| param_map(Some(e))).collect())
        .unwrap_or_default();
    if grid.is_empty() {
        return Err(plan_error("has an empty candidate `grid`"));
    }

    // `benchmark.params` is the incumbent arm. An absent one is the strategy at
    // its declared defaults, which is a legitimate benchmark and is written
    // down as the empty map rather than assumed somewhere later.
    let benchmark_params = param_map(def.benchmark.get("params")).unwrap_or_default();

    // A replicate count is a DEFINE fact like everything else here: it is inside
    // `search_space`, so it is inside the define_hash, so it was fixed before
    // anything ran. The floor is the protocol's, not the campaign's.
    let replicates = space
        .get("replicates")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_REPLICATES, |k| usize::try_from(k).unwrap_or(DEFAULT_REPLICATES))
        .max(backtest::stats::compare::MIN_REPLICATES);

    Ok(StudyPlan {
        strategy_ref: text("strategy_ref")?,
        strategy_version: text("strategy_version")?,
        universe_ref: text("universe_ref")?,
        start,
        end,
        eval_resolution,
        cost_model_ref: text("cost_model_ref")?,
        sizing_ref: text("sizing_ref")?,
        data_snapshot: text("data_snapshot")?,
        grid,
        benchmark_params,
        replicates,
    })
}

/// The objective the campaign declared it would be judged on.
///
/// The **first** entry of `maximize`, not a scalarization of all of them:
/// ADR-012 removed the composite score on purpose, and inventing one here to
/// have a single number to compare would put it back. A campaign maximizing two
/// things gets its pairs judged on the first, and the second is a constraint or
/// a second comparison — never a weighted sum nobody declared.
pub fn objective_metric(def: &CampaignDefinition) -> Result<(String, MetricKind), JobError> {
    let name = def
        .objective
        .get("maximize")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or_else(|| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "campaign_objective_unreadable",
                "the campaign's objective names nothing to maximize (§10)",
            )
        })?;
    let kind = metric_kind(name).ok_or_else(|| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "unknown_objective_metric",
            format!(
                "`{name}` is not a metric this engine computes; a campaign judged on a metric \
                 nobody can measure would be judged on a number somebody supplied"
            ),
        )
    })?;
    Ok((name.to_string(), kind))
}

/// The ledger's objective vocabulary mapped onto the engine's metric set.
///
/// Refused rather than defaulted: a misspelled objective that silently became
/// Sharpe would judge every candidate in the campaign on a quantity the
/// campaign did not declare.
fn metric_kind(name: &str) -> Option<MetricKind> {
    Some(match name {
        "sharpe_net" | "sharpe" => MetricKind::Sharpe,
        "sortino_net" | "sortino" => MetricKind::Sortino,
        "calmar" => MetricKind::Calmar,
        "cagr" => MetricKind::Cagr,
        "total_return" => MetricKind::TotalReturn,
        "dsr" | "deflated_sharpe" | "detrended_sharpe" => MetricKind::DetrendedSharpe,
        "profit_factor" => MetricKind::ProfitFactor,
        "expectancy" => MetricKind::Expectancy,
        _ => return None,
    })
}

// ── the replicate grid ───────────────────────────────────────────────────────

/// The campaign's replicate grid: `k` contiguous windows of the declared span.
///
/// Derived from the campaign's `define_hash` alone. `cost_bp` is `0` on every
/// rung, and that zero is a statement rather than an omission — see the module
/// header, and `cost_nuisance_unrandomized` on the verdicts built from it.
fn campaign_grid(define_hash: &str, k: usize) -> Vec<ReplicateKey> {
    let base = seed_from(define_hash);
    (0..k)
        .map(|i| ReplicateKey {
            window_index: i32::try_from(i).unwrap_or(i32::MAX),
            cost_bp: 0,
            seed: (base.wrapping_add(i as u64) >> 1) as i64,
        })
        .collect()
}

/// A deterministic 64-bit seed from a string. Deterministic so a resumed study
/// re-derives the same grid; not a parameter, so there is nothing to resample
/// against.
fn seed_from(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A `[0, 1)` stream from a seed — splitmix64.
///
/// The dispatcher takes an RNG because §4.5's exploration floor is a *random*
/// reservation. A campaign study seeds it from `(define_hash, seq)`, so the draw
/// is random with respect to the candidates and identical across a restart at
/// the same fold position. A wall-clock seed would make a resumed study draw a
/// different candidate and measure a different arm from the one the log says it
/// dispatched.
fn deterministic_uniform(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        #[allow(clippy::cast_precision_loss)]
        {
            ((z >> 11) as f64) / ((1_u64 << 53) as f64)
        }
    }
}

/// The window of history replicate `i` measures.
fn window_slice(plan: &StudyPlan, index: usize) -> DataSlice {
    let span = (plan.end - plan.start).num_seconds().max(1);
    let k = plan.replicates.max(1) as i64;
    let each = (span / k).max(1);
    let i = i64::try_from(index).unwrap_or(0);
    let start = plan.start + chrono::Duration::seconds(each * i);
    // The last window runs to the declared end rather than to a rounded
    // boundary, so no part of the pre-registered span is silently dropped.
    let end = if i + 1 == k {
        plan.end
    } else {
        plan.start + chrono::Duration::seconds(each * (i + 1))
    };
    DataSlice::new(plan.universe_ref.clone(), start, end, plan.eval_resolution)
}

fn run_config(plan: &StudyPlan, params: &ParamMap, key: ReplicateKey, index: usize) -> RunConfig {
    RunConfigBuilder::new(
        plan.strategy_ref.clone(),
        plan.strategy_version.clone(),
        window_slice(plan, index),
        plan.cost_model_ref.clone(),
        plan.sizing_ref.clone(),
        plan.data_snapshot.clone(),
    )
    .params(params.clone())
    .seed(key.seed.unsigned_abs())
    .build()
}

/// The name a challenger is recorded under.
///
/// The fold position, not a uuid and not a timestamp: the same position names
/// the same arm however many times the study is re-queued, which is what lets
/// the unique index on the measurement table do the deduplicating.
#[must_use]
pub fn challenger_arm(seq: i64) -> String {
    format!("challenger:{seq}")
}

// ── the worker ───────────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl Worker for StudyWorker {
    fn kind(&self) -> JobKind {
        JobKind::Study
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let req = parse(manifest)?;
        let led = self.ledger();

        // The job service authorises a counted kind with a registered trial
        // (INV-16); its tenant is the one everything below reads and writes
        // under. There is no other way to learn it here, which is the point.
        let tenant = ctx
            .trial
            .as_ref()
            .map(|t| t.tenant_id().to_string())
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unregistered_trial",
                    "study counts as a trial and cannot run without one (INV-16)",
                )
            })?;

        let campaign = led
            .load_campaign_async(&tenant, req.campaign_id)
            .await
            .map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unknown_campaign",
                    format!("campaign {}: {e}", req.campaign_id),
                )
            })?;
        let plan = parse_plan(campaign.definition())?;
        let (metric_name, metric) = objective_metric(campaign.definition())?;
        let grid = campaign_grid(campaign.define_hash(), plan.replicates);
        let arm = challenger_arm(req.seq);

        // An explicit checkpoint, which is what ADR-P2-04 uses instead of a
        // workflow engine's memory: if this position already has its evidence,
        // the study is done and re-running it would log a second dispatch
        // decision for a draw that was already made.
        let already = led
            .replicate_measurements_async(&tenant, req.campaign_id, &arm, &metric_name)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading prior measurements: {e}")))?;
        if already.len() >= grid.len() {
            return Ok(JobOutput {
                summary: Some(format!(
                    "{arm} was already measured on {} replicates; nothing to do",
                    already.len()
                )),
                result: json!({
                    "campaign_id": req.campaign_id,
                    "arm": arm,
                    "metric": metric_name,
                    "replicates": already.len(),
                    "resumed": true,
                }),
                ..Default::default()
            });
        }

        // ── the draw (§4.5, INV-20, INV-21) ────────────────────────────────
        //
        // The dispatcher decides exploration from the campaign's achieved
        // fraction against its own floor. This worker passes no floor, holds no
        // floor, and has no parameter that could carry one.
        let achieved = led
            .campaign_exploration_fraction_async(&tenant, req.campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the exploration fraction: {e}")))?;
        let candidates: Vec<Value> = plan
            .grid
            .iter()
            .map(|p| serde_json::to_value(p).unwrap_or(Value::Null))
            .collect();
        // No learned preference exists yet (ADR-P4-01's ladder is at its rule
        // tier), so every candidate is equally preferred and the draw is uniform
        // over the grid. A flat preference is an honest statement of what the
        // platform currently knows; a fabricated ranking would not be.
        let preference = vec![0.0_f64; candidates.len()];
        let mut rng = deterministic_uniform(seed_from(&format!(
            "{}:{}",
            campaign.define_hash(),
            req.seq
        )));
        let dispatcher = Dispatcher::new(&campaign, &led);
        let draw = dispatcher
            .draw(
                DecisionKind::Select,
                &candidates,
                &preference,
                achieved,
                &mut rng,
                Some(format!("campaign EXPERIMENT at seq {}", req.seq)),
            )
            .map_err(|e| JobError::infrastructure(format!("dispatching a candidate: {e}")))?;
        let challenger_params = plan.grid[draw.chosen_index].clone();

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("measuring".into()),
            message: Some(format!("{} replicates x 2 arms", grid.len())),
        })
        .await;

        // ── the measurements ───────────────────────────────────────────────
        //
        // Register-then-dispatch is `Backtest`'s contract, not this worker's
        // choice: it mints the ticket the executor cannot be called without, so
        // a measurement that reached this table without a ledger row is not a
        // path that exists (INV-16).
        let engine = Backtest::new(
            InMemoryRunStore::default(),
            Arc::clone(&self.executor),
            self.ledger(),
        );
        let base_ctx = campaign
            .dispatch_context(ledger::ActorKind::Scheduler, PRODUCED_BY)
            .with_experiment(format!("campaign:{}:experiment", req.campaign_id));

        let mut measurements: Vec<ReplicateMeasurement> = Vec::new();
        let mut failures = 0_usize;
        for (index, key) in grid.iter().copied().enumerate() {
            for (arm_name, params, explore) in [
                (INCUMBENT_ARM.to_string(), &plan.benchmark_params, false),
                (arm.clone(), &challenger_params, draw.exploration_flag),
            ] {
                if ctx.is_cancelled() {
                    break;
                }
                let cfg = run_config(&plan, params, key, index);
                let mut dispatch = RunDispatch::new(&base_ctx, &cfg, draw.propensity);
                if explore {
                    dispatch = dispatch.exploration();
                }
                let (result, _, trial_id) =
                    engine.run_traced_with_trial(&dispatch).map_err(|e| {
                        JobError::logic(
                            ledger::TerminalReason::IntegrityRejected,
                            "ledger_refused_dispatch",
                            format!("registering replicate {index} of {arm_name}: {e}"),
                        )
                    })?;
                // A failed member is recorded as a failed trial by the engine and
                // contributes no measurement. It is *not* replaced by a zero: a
                // zero is a number, and a missing replicate is not.
                if result.status != RunStatus::Ok {
                    failures += 1;
                    continue;
                }
                let value = result.metrics.value(metric);
                if !value.is_finite() {
                    failures += 1;
                    continue;
                }
                measurements.push(ReplicateMeasurement {
                    arm: arm_name,
                    key,
                    metric: metric_name.clone(),
                    value,
                    trial_id,
                });
            }
            ctx.progress(Progress {
                #[allow(clippy::cast_precision_loss)]
                pct: Some(0.1 + 0.85 * (index as f64 + 1.0) / grid.len() as f64),
                stage: Some("measuring".into()),
                message: None,
            })
            .await;
        }

        let mut written = 0_usize;
        for m in &measurements {
            if led
                .record_replicate_measurement_async(&tenant, req.campaign_id, m, PRODUCED_BY)
                .await
                .map_err(|e| JobError::infrastructure(format!("recording a measurement: {e}")))?
            {
                written += 1;
            }
        }

        Ok(JobOutput {
            summary: Some(format!(
                "{} measured {} replicates against {INCUMBENT_ARM} on {metric_name} ({written} new, \
                 {failures} member runs produced no measurement)",
                arm,
                grid.len(),
            )),
            result: json!({
                "campaign_id": req.campaign_id,
                "seq": req.seq,
                "arm": arm,
                "metric": metric_name,
                "replicates": grid.len(),
                "measurements": measurements.len(),
                "written": written,
                "failed_members": failures,
                "propensity": draw.propensity,
                "exploration": draw.exploration_flag,
                "candidate_set_hash": draw.candidate_set_hash,
                "decision_id": draw.decision_id,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campaign(space: Value) -> CampaignDefinition {
        CampaignDefinition {
            slug: "s".into(),
            hypothesis: "h".into(),
            objective: json!({ "maximize": ["sharpe_net"], "subject_to": {} }),
            benchmark: json!({ "kind": "flat", "params": { "fast": 8 } }),
            delta_practical: 0.25,
            approval_spend_usd: 0.0,
            budget: ledger::campaign::Budget {
                max_trials: 100,
                gpu_hours: 0.0,
                usd: 0.0,
                wall_clock_hours: 1.0,
            },
            exploration_floor: ledger::campaign::ExplorationFloor::default_floor(),
            gates_profile: "strict_v1".into(),
            preference_vector: None,
            search_space: space,
        }
    }

    fn runnable_space() -> Value {
        json!({
            "strategy_ref": "sma-cross",
            "strategy_version": "v1",
            "universe_ref": "BTC-USD",
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-03-01T00:00:00Z",
            "eval_resolution": "1d",
            "cost_model_ref": "cm:default",
            "sizing_ref": "sz:flat",
            "data_snapshot": "snap:1",
            "grid": [{ "fast": 8 }, { "fast": 12 }],
            "replicates": 6
        })
    }

    /// The manifest names a position, never evidence. A study that could be
    /// told its own search space would be a study the submitter aimed.
    #[test]
    fn the_manifest_names_a_campaign_and_a_position_and_nothing_else() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({ "campaign_id": Uuid::nil().to_string() })).is_err());
        assert!(parse(&json!({ "campaign_id": "nope", "seq": 3 })).is_err());
        assert!(parse(&json!({ "campaign_id": Uuid::nil().to_string(), "seq": -1 })).is_err());
        let ok = parse(&json!({ "campaign_id": Uuid::nil().to_string(), "seq": 4 })).unwrap();
        assert_eq!(ok.seq, 4);
    }

    /// Every missing field of the plan is refused, one at a time. None of them
    /// has a default, because a default would be a research plan nobody wrote
    /// covered by a pre-registration hash that says somebody did.
    #[test]
    fn an_incomplete_search_space_is_refused_rather_than_defaulted() {
        assert!(parse_plan(&campaign(runnable_space())).is_ok());
        for missing in [
            "strategy_ref",
            "strategy_version",
            "universe_ref",
            "start",
            "end",
            "eval_resolution",
            "cost_model_ref",
            "sizing_ref",
            "data_snapshot",
            "grid",
        ] {
            let mut space = runnable_space();
            space.as_object_mut().unwrap().remove(missing);
            let err = parse_plan(&campaign(space)).unwrap_err();
            assert_eq!(err.code, "campaign_search_space_unrunnable", "{missing}");
            assert_eq!(err.terminal, ledger::TerminalReason::IntegrityRejected);
        }
        // An empty grid is not a grid.
        let mut empty = runnable_space();
        empty["grid"] = json!([]);
        assert!(parse_plan(&campaign(empty)).is_err());
        // A reversed window is not a window.
        let mut reversed = runnable_space();
        reversed["end"] = json!("2023-01-01T00:00:00Z");
        assert!(parse_plan(&campaign(reversed)).is_err());
    }

    /// A campaign may pre-register more replicates than the target; it may not
    /// pre-register fewer than the protocol's floor, because below that the
    /// confidence sequence is too wide to separate anything and §11.5 says so.
    #[test]
    fn the_replicate_count_cannot_be_declared_below_the_protocols_floor() {
        let mut low = runnable_space();
        low["replicates"] = json!(1);
        let plan = parse_plan(&campaign(low)).unwrap();
        assert_eq!(plan.replicates, backtest::stats::compare::MIN_REPLICATES);

        let mut high = runnable_space();
        high["replicates"] = json!(40);
        assert_eq!(parse_plan(&campaign(high)).unwrap().replicates, 40);

        let mut unstated = runnable_space();
        unstated.as_object_mut().unwrap().remove("replicates");
        assert_eq!(parse_plan(&campaign(unstated)).unwrap().replicates, DEFAULT_REPLICATES);
    }

    /// An objective this engine cannot measure is refused. Defaulting it to
    /// Sharpe would judge the whole campaign on a quantity nobody declared.
    #[test]
    fn an_unmeasurable_objective_is_refused() {
        let c = campaign(runnable_space());
        assert_eq!(objective_metric(&c).unwrap().1, MetricKind::Sharpe);

        let mut vibes = c.clone();
        vibes.objective = json!({ "maximize": ["vibes"], "subject_to": {} });
        let err = objective_metric(&vibes).unwrap_err();
        assert_eq!(err.code, "unknown_objective_metric");

        // And an objective that names nothing at all is refused too.
        let mut empty = c;
        empty.objective = json!({ "maximize": [], "subject_to": {} });
        assert_eq!(
            objective_metric(&empty).unwrap_err().code,
            "campaign_objective_unreadable"
        );
    }

    /// The grid is a function of the campaign's immutable DEFINE hash and
    /// nothing else. This is what makes a resumed study re-derive the same
    /// replicates — and what keeps the incumbent pairable across cycles.
    #[test]
    fn the_replicate_grid_is_a_campaign_constant() {
        let a = campaign_grid("sha256:abc", 9);
        assert_eq!(a, campaign_grid("sha256:abc", 9));
        assert_ne!(a, campaign_grid("sha256:abd", 9));
        assert_eq!(a.len(), 9);
        // Distinct windows, all at the same cost rung — the honest shape of a
        // grid whose cost dimension no executor implements (ADR-P2-36).
        let windows: std::collections::BTreeSet<i32> = a.iter().map(|k| k.window_index).collect();
        assert_eq!(windows.len(), 9);
        assert!(a.iter().all(|k| k.cost_bp == 0));
    }

    /// The windows tile the declared span exactly: none of the pre-registered
    /// history is silently dropped by integer division.
    #[test]
    fn the_windows_tile_the_declared_span() {
        let plan = parse_plan(&campaign(runnable_space())).unwrap();
        let first = window_slice(&plan, 0);
        let last = window_slice(&plan, plan.replicates - 1);
        assert_eq!(first.start, plan.start);
        assert_eq!(last.end, plan.end);
        for i in 1..plan.replicates {
            assert_eq!(window_slice(&plan, i - 1).end, window_slice(&plan, i).start);
        }
    }

    /// The draw is random with respect to the candidates and identical across a
    /// restart. A wall-clock seed would make a resumed study measure a different
    /// arm from the one its own log says it dispatched.
    #[test]
    fn the_draw_is_deterministic_in_the_fold_position() {
        let pull = |seed: &str| {
            let mut r = deterministic_uniform(seed_from(seed));
            (0..5).map(|_| r()).collect::<Vec<_>>()
        };
        assert_eq!(pull("h:3"), pull("h:3"));
        assert_ne!(pull("h:3"), pull("h:4"));
        assert!(pull("h:3").iter().all(|u| (0.0..1.0).contains(u)));
    }

    /// An arm is named by its fold position, so a re-queued study writes into
    /// the same rows rather than beside them.
    #[test]
    fn a_challenger_is_named_by_its_position() {
        assert_eq!(challenger_arm(7), "challenger:7");
        assert_ne!(challenger_arm(7), challenger_arm(8));
        assert_ne!(challenger_arm(7), INCUMBENT_ARM);
    }
}
