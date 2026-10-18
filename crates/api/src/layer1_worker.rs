//! The `layer1_run` worker: a research-strategy file in, one counted trial out
//! (Set M M-1.5, M-1.7; FEAT-004 §2.2-2.3; FEAT-005 §4.2).
//!
//! # What it does, in the order COMP-005 §2 gives
//!
//! 1. Reads the window **through `BarStore`** at the manifest's `as_of`, the one
//!    point-in-time reader, so the function cannot see a bar the horizon forbids.
//! 2. Runs the agent's function, plus its Gate-0 truncation re-runs, in the skill
//!    runner's sandbox ([`compute`]). This is compute, not a look: it registers no trial.
//! 3. Compares the re-runs in Rust (`position_series::check_truncation`). The cut points
//!    come from a seed derived from the experiment and the code hash, **not** from the
//!    manifest, so the agent cannot choose cuts that miss its look-ahead without changing
//!    its code (ADR-M-08).
//! 4. Stores the code (`code_snapshot`) and the series with its bars (`position_series`)
//!    as content-addressed artifacts and puts both hashes in `strategy_version`.
//! 5. Dispatches the evaluation through `backtest::run::Backtest`: the ledger **registers
//!    the trial first** (INV-16), the `PositionRunExecutor` computes P&L itself (ST-02),
//!    the Gate-0 verdict rides in the executor and rejects the run if missing or failed
//!    (M-1.7), and the return series is persisted to the ledger so Gates 13 and 14 and the
//!    theme posterior read this run with the rest of its experiment's family.
//!
//! `JobKind::Layer1Run` is not a counted kind at submission; the one counted trial is the
//! evaluation in step 5. A manifest that carries a `pnl` field never gets this far.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use backtest::position_run::{ArtifactBar, Layer1Artifact, PositionRunExecutor};
use backtest::position_series::{
    check_truncation, truncation_points, PositionPoint, PositionSeries, PositionUnits,
    TruncationCase, TruncationVerdict,
};
use backtest::run::{Backtest, InMemoryRunStore, RunDispatch, RunStatus};
use jobs::kinds::layer1::Layer1Manifest;
use jobs::store::JobStore;
use jobs::{ArtifactRegistry, Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const RESEARCH_STRATEGY_PY: &str = include_str!("../../../apps/skill-runner/research_strategy.py");
const DRIVER_PY: &str = include_str!("../../../apps/skill-runner/layer1_driver.py");

/// A bar with everything the harness's `Data` object exposes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawBar {
    pub ts_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum Layer1Error {
    /// The function or its declaration was refused, with a stable code the agent can branch on.
    #[error("{reason}: {message}")]
    Refused { reason: String, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Infra(String),
}

impl From<Layer1Error> for JobError {
    fn from(e: Layer1Error) -> Self {
        match e {
            Layer1Error::Refused { reason, message } => {
                JobError::logic(ledger::TerminalReason::IntegrityRejected, reason, message)
            }
            Layer1Error::Invalid(m) => JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_series",
                m,
            ),
            Layer1Error::Infra(m) => JobError::infrastructure(m),
        }
    }
}

/// What the compute step returns.
pub struct Computed {
    pub artifact: Layer1Artifact,
    pub verdict: TruncationVerdict,
    pub lint: Value,
    /// The declared params as the harness reports them: `{name: {kind, low, high}}`.
    pub declared: serde_json::Map<String, Value>,
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The truncation seed: a function of the experiment and the code, never of the manifest.
#[must_use]
pub fn cut_seed(experiment_id: &str, code_sha256: &str) -> u64 {
    let h = Sha256::digest(format!("layer1-cuts|{experiment_id}|{code_sha256}").as_bytes());
    u64::from_le_bytes(h[..8].try_into().expect("8 bytes"))
}

fn series_from(
    detail: &Value,
    key: &str,
    units: PositionUnits,
    allow_short: bool,
) -> Result<PositionSeries, Layer1Error> {
    let bad =
        |m: &str| Layer1Error::Invalid(format!("the harness returned a malformed `{key}`: {m}"));
    let ts = detail
        .get("ts")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no ts"))?;
    let tg = detail
        .get("target")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no target"))?;
    if ts.len() != tg.len() {
        return Err(bad("ts and target differ in length"));
    }
    let points = ts
        .iter()
        .zip(tg)
        .map(|(t, v)| {
            Ok(PositionPoint {
                ts_ns: t.as_i64().ok_or_else(|| bad("a ts is not an integer"))?,
                target: if v.is_null() {
                    None
                } else {
                    Some(v.as_f64().ok_or_else(|| bad("a target is not a number"))?)
                },
            })
        })
        .collect::<Result<Vec<_>, Layer1Error>>()?;
    Ok(PositionSeries {
        position_units: units,
        allow_short,
        points,
    })
}

/// Turn the harness's answer into the full series and the truncation verdict.
///
/// # Errors
/// A refusal, a malformed answer, or a series that fails the validity rules.
pub fn assemble(
    out: &Value,
    decision_times: &[i64],
) -> Result<(PositionSeries, TruncationVerdict, Value), Layer1Error> {
    if out.get("refused").and_then(Value::as_bool) == Some(true) {
        return Err(Layer1Error::Refused {
            reason: out
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("refused")
                .to_string(),
            message: out
                .pointer("/detail/message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    let d = out
        .get("detail")
        .ok_or_else(|| Layer1Error::Invalid("no detail".into()))?;
    let units = match d.get("position_units").and_then(Value::as_str) {
        Some("units") => PositionUnits::Units,
        _ => PositionUnits::Fraction,
    };
    let allow_short = d
        .get("allow_short")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let full = series_from(
        d.get("full")
            .ok_or_else(|| Layer1Error::Invalid("no full series".into()))?,
        "full",
        units,
        allow_short,
    )?;
    full.validate(Some(decision_times))
        .map_err(|e| Layer1Error::Invalid(e.to_string()))?;
    let mut cases = Vec::new();
    for c in d
        .get("cases")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let cut_ns = c
            .get("cut_ns")
            .and_then(Value::as_i64)
            .ok_or_else(|| Layer1Error::Invalid("a case has no cut_ns".into()))?;
        cases.push(TruncationCase {
            cut_ns,
            truncated: series_from(&c, "case", units, allow_short)?,
        });
    }
    let verdict = check_truncation(&full, &cases);
    Ok((
        full,
        verdict,
        d.get("lint").cloned().unwrap_or_else(|| json!([])),
    ))
}

fn bars_parquet(bars: &[RawBar]) -> Result<Vec<u8>, Layer1Error> {
    use arrow::array::{ArrayRef, Float64Array, Int64Array};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    let infra = |e: &dyn std::fmt::Display| Layer1Error::Infra(e.to_string());
    let schema = Arc::new(Schema::new(vec![
        Field::new("ts_ns", DataType::Int64, false),
        Field::new("open", DataType::Float64, false),
        Field::new("high", DataType::Float64, false),
        Field::new("low", DataType::Float64, false),
        Field::new("close", DataType::Float64, false),
        Field::new("volume", DataType::Float64, false),
    ]));
    let f = |g: fn(&RawBar) -> f64| -> ArrayRef {
        Arc::new(Float64Array::from(bars.iter().map(g).collect::<Vec<_>>()))
    };
    let cols: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(
            bars.iter().map(|b| b.ts_ns).collect::<Vec<_>>(),
        )),
        f(|b| b.open),
        f(|b| b.high),
        f(|b| b.low),
        f(|b| b.close),
        f(|b| b.volume),
    ];
    let batch = RecordBatch::try_new(schema.clone(), cols).map_err(|e| infra(&e))?;
    let mut buf = Vec::new();
    {
        let mut w = ArrowWriter::try_new(&mut buf, schema, None).map_err(|e| infra(&e))?;
        w.write(&batch).map_err(|e| infra(&e))?;
        w.close().map_err(|e| infra(&e))?;
    }
    Ok(buf)
}

/// Run the function and its truncation re-runs. No database, no trial.
///
/// # Errors
/// A refused function, an invalid series, or a runner failure.
pub async fn compute(
    m: &Layer1Manifest,
    bars: &[RawBar],
    root: &Path,
    scratch: &Path,
    cost: Option<&backtest::cost::CostModel>,
) -> Result<Computed, Layer1Error> {
    if bars.len() < 30 {
        return Err(Layer1Error::Invalid(format!(
            "{} bars is too few to run a research strategy on",
            bars.len()
        )));
    }
    let code_sha = sha256_hex(m.source.as_bytes());
    let times: Vec<i64> = bars.iter().map(|b| b.ts_ns).collect();
    let cuts = truncation_points(
        cut_seed(&m.experiment_id, &code_sha),
        m.truncation_cuts,
        &times,
    );

    std::fs::create_dir_all(scratch).map_err(|e| Layer1Error::Infra(e.to_string()))?;
    let path = scratch.join("bars.parquet");
    std::fs::write(&path, bars_parquet(bars)?).map_err(|e| Layer1Error::Infra(e.to_string()))?;

    let source = format!("{RESEARCH_STRATEGY_PY}\n{DRIVER_PY}");
    let contract = skills::Contract::new(["code"], ["detail"], "layer1_run harness")
        .expect("a valid contract");
    let skill = skills::Skill::author(
        "layer1_run",
        "Runs a Layer 1 research strategy and its truncation re-runs over materialised bars.",
        source,
        contract,
        None,
    )
    .map_err(|e| Layer1Error::Infra(e.to_string()))?;
    let mut config = skills::RunnerConfig::from_workspace(root);
    config.limits = skills::runner::Limits {
        wall_seconds: 300,
        cpu_seconds: 280,
        memory_mb: 1024,
        output_bytes: 32 * 1024 * 1024,
    };
    let payload = json!({
        "code": m.source,
        "params": m.params,
        "seed": m.seed,
        "cuts": cuts,
        "instrument": m.instrument,
        "timeframe": m.timeframe,
        "data": { "primary": path.to_string_lossy() },
    });
    let done = skills::runner::run(&skill, &payload, &config).await;
    let out = done.result.ok_or_else(|| {
        Layer1Error::Infra(format!(
            "the harness did not return a result: {:?}",
            done.record.outcome
        ))
    })?;
    let (full, verdict, lint) = assemble(&out, &times)?;
    let declared = out
        .pointer("/detail/declared_params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let (fee_bps, half_spread_bps, impact) = match cost {
        Some(c) => {
            let (costs, impact) = c.to_eval();
            (costs.fee_bps, costs.half_spread_bps, Some(impact))
        }
        None => (m.fee_bps, m.half_spread_bps, None),
    };
    let artifact = Layer1Artifact {
        code_sha256: code_sha,
        instrument: m.instrument.clone(),
        timeframe: m.timeframe.clone(),
        series: full,
        bars: bars
            .iter()
            .map(|b| ArtifactBar {
                ts_ns: b.ts_ns,
                open: b.open,
                close: b.close,
            })
            .collect(),
        initial_equity: m.initial_equity,
        fee_bps,
        half_spread_bps,
        cost_model_ref: cost.map_or_else(
            || {
                format!(
                    "cost:flat:fee={}bps:half_spread={}bps",
                    m.fee_bps, m.half_spread_bps
                )
            },
            backtest::cost::CostModel::reference,
        ),
        data_snapshot: format!("asof:{}", m.as_of.to_rfc3339()),
        lot_size: None,
        impact,
        params: m.params.clone().into_iter().collect(),
        seed: m.seed,
    };
    Ok(Computed {
        artifact,
        verdict,
        lint,
        declared,
    })
}

/// The manifest's cost model, parsed and checked point-in-time against the window it prices.
///
/// # Errors
/// A model that does not parse, is dated after the window, or asks for latency this evaluator
/// does not apply (stated rather than silently ignored).
pub fn resolve_cost_model(
    m: &Layer1Manifest,
) -> Result<Option<backtest::cost::CostModel>, JobError> {
    let Some(v) = &m.cost_model else {
        return Ok(None);
    };
    let refuse = |code: &str, why: String| {
        JobError::logic(ledger::TerminalReason::IntegrityRejected, code, why)
    };
    let model: backtest::cost::CostModel = serde_json::from_value(v.clone())
        .map_err(|e| refuse("invalid_cost_model", e.to_string()))?;
    model
        .check_point_in_time(m.start.date_naive())
        .map_err(|e| refuse("cost_model_not_point_in_time", e.to_string()))?;
    if model.instrument != m.instrument {
        return Err(refuse(
            "cost_model_instrument",
            format!(
                "the model prices {} and the run trades {}",
                model.instrument, m.instrument
            ),
        ));
    }
    if model.latency_bars > 0 {
        return Err(refuse(
            "latency_not_applied",
            format!("the model asks for {} bars of extra latency and the position-series evaluator does not apply it; refusing rather than ignoring it", model.latency_bars),
        ));
    }
    Ok(Some(model))
}

pub struct Layer1Worker {
    store: Arc<JobStore>,
    pg: PgPool,
    clickhouse_url: String,
    artifacts: Option<Arc<ArtifactRegistry>>,
    root: PathBuf,
}

impl Layer1Worker {
    #[must_use]
    pub fn new(
        store: Arc<JobStore>,
        pg: PgPool,
        clickhouse_url: String,
        artifacts: Option<Arc<ArtifactRegistry>>,
    ) -> Self {
        let root =
            std::env::var("WORKSPACE_ROOT").map_or_else(|_| PathBuf::from("."), PathBuf::from);
        Self {
            store,
            pg,
            clickhouse_url,
            artifacts,
            root,
        }
    }

    async fn load_bars(&self, m: &Layer1Manifest) -> Result<Vec<RawBar>, Layer1Error> {
        use backtest::types::TimeframeExt;
        use rust_decimal::prelude::ToPrimitive;
        let tf = <domain::payloads::bar::Timeframe as TimeframeExt>::from_key(&m.timeframe)
            .ok_or_else(|| Layer1Error::Invalid(format!("unknown timeframe {:?}", m.timeframe)))?;
        let loaded = backtest::store::BarStore::connect(&self.clickhouse_url)
            .as_of(m.as_of)
            .load_bars(&m.instrument, tf, m.start, m.end)
            .await
            .map_err(|e| Layer1Error::Infra(format!("reading bars: {e}")))?;
        let f = |d: &rust_decimal::Decimal| d.to_f64().unwrap_or(f64::NAN);
        Ok(loaded
            .iter()
            .map(|b| RawBar {
                ts_ns: b.ts_ns,
                open: f(&b.open),
                high: f(&b.high),
                low: f(&b.low),
                close: f(&b.close),
                volume: f(&b.volume),
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl Worker for Layer1Worker {
    fn kind(&self) -> JobKind {
        JobKind::Layer1Run
    }

    fn estimate(&self, m: &Value) -> Cost {
        let cuts = m
            .get("truncation_cuts")
            .and_then(Value::as_f64)
            .unwrap_or(8.0);
        Cost {
            compute_s: Some(10.0 * (cuts + 1.0)),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let m = Layer1Manifest::from_value(manifest).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_manifest",
                e,
            )
        })?;
        let job = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        ctx.progress(Progress {
            pct: Some(5.0),
            stage: Some("bars".into()),
            message: Some(format!("{} {}", m.instrument, m.timeframe)),
        })
        .await;
        let cost_model = resolve_cost_model(&m)?;
        let bars = self.load_bars(&m).await?;
        ctx.progress(Progress {
            pct: Some(20.0),
            stage: Some("research".into()),
            message: Some(format!("{} bars", bars.len())),
        })
        .await;
        match m.sweep {
            None => {
                let member = self
                    .evaluate(&job, &m, &bars, cost_model.as_ref(), "single")
                    .await?;
                Ok(member.into_output())
            }
            Some(sw) => {
                self.sweep(&job, &m, &bars, cost_model.as_ref(), sw.max_runs)
                    .await
            }
        }
    }
}

/// One evaluated member: the result document, plus what a sweep needs to fold it.
struct MemberOutput {
    summary: String,
    result: Value,
    metric: Option<f64>,
    rejected: bool,
    artifacts: Vec<(String, String)>,
}

impl MemberOutput {
    fn into_output(self) -> JobOutput {
        JobOutput {
            summary: Some(self.summary),
            result: self.result,
            artifacts: self.artifacts,
            ..JobOutput::default()
        }
    }
}

impl Layer1Worker {
    /// Compute one point's series and its truncation verdict, store the artifacts, dispatch the
    /// one counted evaluation, write the rich outputs. One call is one trial.
    async fn evaluate(
        &self,
        job: &jobs::Job,
        m: &Layer1Manifest,
        bars: &[RawBar],
        cost_model: Option<&backtest::cost::CostModel>,
        tag: &str,
    ) -> Result<MemberOutput, JobError> {
        let tenant = job.user_id.to_string();
        let scratch = std::env::temp_dir().join(format!("layer1-{}-{tag}", job.job_id));
        let computed = compute(m, bars, &self.root, &scratch, cost_model).await;
        let _ = std::fs::remove_dir_all(&scratch);
        let Computed {
            artifact,
            verdict,
            lint,
            ..
        } = computed?;

        let bytes = artifact.to_bytes();
        let series_sha = artifact.sha256();
        let (mut code_handle, mut series_handle) = (None, None);
        if let Some(reg) = &self.artifacts {
            let storing = |e: jobs::store::JobStoreError| {
                JobError::infrastructure(format!("storing an artifact: {e}"))
            };
            code_handle = Some(
                reg.put(
                    "code_snapshot",
                    job.project_id,
                    m.source.as_bytes(),
                    json!({"layer": 1, "instrument": m.instrument}),
                    Some(&job.job_id),
                )
                .await
                .map_err(storing)?
                .handle,
            );
            series_handle = Some(
                reg.put(
                    "position_series",
                    job.project_id,
                    &bytes,
                    json!({"layer": 1, "code_sha256": artifact.code_sha256, "series_sha256": series_sha}),
                    Some(&job.job_id),
                )
                .await
                .map_err(storing)?
                .handle,
            );
        }

        let a = Arc::new(artifact);
        let cfg = a.run_config().map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "unsupported_run",
                e.to_string(),
            )
        })?;
        let verdict_for_run = verdict.clone();
        let (pg, a2, experiment, job_id) = (
            self.pg.clone(),
            a.clone(),
            m.experiment_id.clone(),
            job.job_id.clone(),
        );
        let cfg2 = cfg.clone();
        let (result, trial_id) = tokio::task::spawn_blocking(move || {
            let bt = Backtest::new(
                InMemoryRunStore::new(),
                PositionRunExecutor::new(a2, Some(verdict_for_run)),
                ledger::pg::PgTrialLedger::new(pg),
            );
            let dctx = ledger::DispatchContext::policy_without_effect_size(
                &tenant,
                &job_id,
                "layer1_run",
                1,
            )
            .with_experiment(experiment);
            bt.run_traced_with_trial(&RunDispatch::new(&dctx, &cfg2, 1.0))
                .map(|(r, _, t)| (r, t))
        })
        .await
        .map_err(|e| JobError::infrastructure(format!("the evaluation task: {e}")))?
        .map_err(|e| JobError::infrastructure(format!("the ledger refused the trial: {e}")))?;

        // Rich outputs (M-2.5): idempotent by run id. A failure to write them is reported in the
        // result and does not undo the counted trial.
        let outputs_written = match PositionRunExecutor::new(a.clone(), None).evaluate() {
            Ok(run) => {
                let rows = backtest::outputs_v2::build(result.run_id.as_str(), &a, &run);
                match backtest::outputs_v2::ensure_schema(&self.clickhouse_url).await {
                    Ok(()) => backtest::outputs_v2::write(&self.clickhouse_url, &rows)
                        .await
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
            Err(e) => Err(e.to_string()),
        };
        let rejected = result.status == RunStatus::RejectedIntegrity;
        let res = a
            .resolution()
            .unwrap_or(backtest::run::EvalResolution::Hour1);
        Ok(MemberOutput {
            summary: format!(
                "layer1 run {}: {} trades, sharpe {:.2}; truncation {}{}",
                result.run_id.as_str().get(..8).unwrap_or("?"),
                result.trades.len(),
                result.metrics.sharpe,
                if verdict.passed() { "passed" } else { "FAILED" },
                if rejected {
                    " (run rejected at Gate 0)"
                } else {
                    ""
                }
            ),
            metric: result
                .metrics
                .sharpe
                .is_finite()
                .then_some(result.metrics.sharpe),
            rejected,
            result: json!({
                "run_id": result.run_id.as_str(),
                "trial_id": trial_id,
                "status": result.status,
                "strategy_version": a.strategy_version(),
                "code_sha256": a.code_sha256,
                "series_sha256": series_sha,
                "flags": result.integrity_flags,
                "lint": lint,
                "trades": result.trades.len(),
                "execution_policy": backtest::execution_policy::ExecutionPolicy::of(&cfg).block(),
                "annualisation": {
                    "periods_per_year": backtest::annualisation::periods_per_year(res, backtest::annualisation::Calendar::Continuous),
                    "warnings": backtest::annualisation::warnings("sortino", res),
                },
                "outputs_written": outputs_written.is_ok(),
                "outputs_error": outputs_written.err(),
                "metrics": result.metrics,
            }),
            artifacts: [
                code_handle.map(|h| (h, "code_snapshot".to_string())),
                series_handle.map(|h| (h, "position_series".to_string())),
            ]
            .into_iter()
            .flatten()
            .collect(),
        })
    }

    /// Sample the declared params and evaluate N members (Set M M-1.6; INV-2).
    ///
    /// The declared params are read by one **probe** (the function at its defaults, one
    /// truncation cut, no trial: it only reads the declaration). The sampler seed is derived by
    /// the platform from the experiment and the code, so the agent cannot grind seeds for a
    /// flattering draw. Each member is one counted trial; the report is sealed
    /// (`research::layer1_sweep`): no member list, no ranking, no argmax.
    async fn sweep(
        &self,
        job: &jobs::Job,
        m: &Layer1Manifest,
        bars: &[RawBar],
        cost_model: Option<&backtest::cost::CostModel>,
        n: u32,
    ) -> Result<JobOutput, JobError> {
        use research::layer1_sweep::{draw_points, report_from, DeclaredParam, Member};
        let probe_manifest = Layer1Manifest {
            truncation_cuts: 1,
            params: serde_json::Map::new(),
            sweep: None,
            ..m.clone()
        };
        let scratch = std::env::temp_dir().join(format!("layer1-{}-probe", job.job_id));
        let probed = compute(&probe_manifest, bars, &self.root, &scratch, cost_model).await;
        let _ = std::fs::remove_dir_all(&scratch);
        let declared: Vec<DeclaredParam> = probed?
            .declared
            .iter()
            .map(|(name, spec)| DeclaredParam {
                name: name.clone(),
                kind: spec["kind"].as_str().unwrap_or("float").to_string(),
                low: spec["low"].as_f64().unwrap_or(f64::NAN),
                high: spec["high"].as_f64().unwrap_or(f64::NAN),
            })
            .collect();
        let seed = cut_seed(
            &m.experiment_id,
            &format!("sweep|{}", sha256_hex(m.source.as_bytes())),
        );
        let points = draw_points(&declared, n, seed).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_sweep",
                e.to_string(),
            )
        })?;
        let mut outcomes = Vec::with_capacity(points.len());
        let mut artifacts = Vec::new();
        for (i, point) in points.iter().enumerate() {
            let member_manifest = Layer1Manifest {
                params: point.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                sweep: None,
                ..m.clone()
            };
            match self
                .evaluate(job, &member_manifest, bars, cost_model, &format!("m{i}"))
                .await
            {
                Ok(o) => {
                    artifacts.extend(o.artifacts);
                    outcomes.push(Ok(Member {
                        metric: o.metric,
                        rejected: o.rejected,
                    }));
                }
                // A member that could not run is counted by the report and not retried.
                Err(e) => outcomes.push(Err(e.code.clone())),
            }
        }
        let report = report_from(declared, n, seed, outcomes);
        Ok(JobOutput {
            summary: Some(format!(
                "layer1 sweep: {} of {} members run ({} trials), median sharpe {}, {} rejected at Gate 0; seed {} recorded",
                report.n_run,
                report.n_requested,
                report.n_run,
                report.median.map_or("n/a".to_string(), |x| format!("{x:.2}")),
                report.n_rejected_gate0,
                report.seed
            )),
            result: json!({ "sweep": report, "note": "sealed: per-member metrics and parameters are not returned (INV-2)" }),
            artifacts,
            ..JobOutput::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(n: usize) -> Vec<RawBar> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let p = 100.0 + (i as f64 * 0.3).sin() * 5.0 + i as f64 * 0.05;
                RawBar {
                    ts_ns: (i64::try_from(i).unwrap() + 1) * 3_600_000_000_000,
                    open: p,
                    high: p + 0.5,
                    low: p - 0.5,
                    close: p + 0.1,
                    volume: 10.0,
                }
            })
            .collect()
    }

    fn manifest(source: &str) -> Layer1Manifest {
        Layer1Manifest::from_value(
            &json!({"experiment_id":"e","source":source,"instrument":"BTC-USD",
            "timeframe":"1h","start":"2025-01-01T00:00:00Z","end":"2025-02-01T00:00:00Z",
            "as_of":"2025-03-01T00:00:00Z","params":{"lb":5},"seed":3,"fee_bps":10.0,
            "half_spread_bps":2.0}),
        )
        .unwrap()
    }

    const CAUSAL: &str = "@research_strategy(instruments=[\"BTC-USD\"], timeframe=\"1h\", params={\"lb\": Param.int(2, 50, 5)})\ndef positions(data, p):\n    return (data.close > data.close.shift(p.lb)).astype(float)\n";

    fn root() -> PathBuf {
        // The runner hides user site-packages from agent code (PYTHONNOUSERSITE), so a developer
        // machine's numpy has to be named, exactly as `mcp-server/tests/templates_run.rs` does.
        if std::env::var_os("SKILL_RUNNER_STACK_PATH").is_none() {
            let py = std::env::var("SKILL_RUNNER_PYTHON").unwrap_or_else(|_| "python".into());
            if let Ok(o) = std::process::Command::new(py)
                .args([
                    "-c",
                    "import numpy, os; print(os.path.dirname(os.path.dirname(numpy.__file__)))",
                ])
                .output()
            {
                let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !dir.is_empty() {
                    std::env::set_var("SKILL_RUNNER_STACK_PATH", dir);
                }
            }
        }
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf()
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("layer1-test-{tag}-{}", std::process::id()))
    }

    #[test]
    fn the_cut_seed_depends_on_the_code_and_not_on_anything_the_agent_sets() {
        assert_eq!(cut_seed("e", "aa"), cut_seed("e", "aa"));
        assert_ne!(cut_seed("e", "aa"), cut_seed("e", "ab"));
        assert_ne!(cut_seed("e", "aa"), cut_seed("f", "aa"));
    }

    /// The sandbox needs the analysis stack; on a machine whose interpreter lacks it these
    /// three return early with the reason rather than failing for an unrelated cause.
    fn stack_missing() -> Option<String> {
        let py = std::env::var("SKILL_RUNNER_PYTHON").unwrap_or_else(|_| "python".into());
        let ok = std::process::Command::new(py)
            .args(["-c", "import numpy, pandas, pyarrow"])
            .output()
            .is_ok_and(|o| o.status.success());
        (!ok).then(|| "the interpreter has no numpy/pandas/pyarrow".to_string())
    }

    #[tokio::test]
    async fn a_causal_function_passes_the_truncation_test_and_produces_a_valid_artifact() {
        if let Some(why) = stack_missing() {
            eprintln!("skipped: {why}");
            return;
        }
        let c = compute(&manifest(CAUSAL), &bars(260), &root(), &scratch("ok"), None)
            .await
            .expect("computes");
        assert!(c.verdict.passed(), "{:?}", c.verdict);
        assert_eq!(c.artifact.series.points.len(), 260);
        assert!(c.artifact.strategy_version().starts_with("layer1."));
    }

    #[tokio::test]
    async fn a_look_ahead_function_fails_naming_the_first_differing_timestamp() {
        if stack_missing().is_some() {
            return;
        }
        let leaky = CAUSAL.replace("shift(p.lb)", "shift(-p.lb)");
        let c = compute(
            &manifest(&leaky),
            &bars(260),
            &root(),
            &scratch("leak"),
            None,
        )
        .await
        .expect("computes");
        let TruncationVerdict::Fail(f) = &c.verdict else {
            panic!("a shift(-n) must fail: {:?}", c.verdict)
        };
        assert!(f.first_differing_ns > 0);
        let flag = backtest::gates::position_leakage::check(Some(&c.verdict)).unwrap();
        assert_eq!(flag.code, backtest::gates::position_leakage::CODE);
    }

    #[tokio::test]
    async fn a_file_that_declares_no_params_is_refused_with_a_stable_code() {
        if stack_missing().is_some() {
            return;
        }
        let none = CAUSAL.replace("params={\"lb\": Param.int(2, 50, 5)}", "params={}");
        let e = compute(&manifest(&none), &bars(100), &root(), &scratch("np"), None)
            .await
            .err()
            .expect("refused");
        assert!(
            matches!(&e, Layer1Error::Refused { reason, .. } if reason == "meta.no_params"),
            "{e}"
        );
    }

    #[test]
    fn a_series_that_is_not_on_decision_times_is_refused_at_assembly() {
        let times = [1_i64, 2, 3];
        let out = json!({"detail": {"full": {"ts": [1, 9], "target": [1.0, 0.0]}, "cases": [], "position_units": "fraction"}});
        assert!(matches!(
            assemble(&out, &times),
            Err(Layer1Error::Invalid(_))
        ));
    }

    #[test]
    fn no_cases_is_a_not_computable_verdict_never_a_pass() {
        let out = json!({"detail": {"full": {"ts": [1, 2], "target": [1.0, 0.0]}, "cases": [], "position_units": "fraction"}});
        let (_, v, _) = assemble(&out, &[1, 2, 3]).unwrap();
        assert!(!v.passed());
    }
}
