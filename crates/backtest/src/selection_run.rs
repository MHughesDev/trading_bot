//! Running a selection strategy now, or as of a past instant (Set U U-6.3;
//! FEAT-007 §4, §10).
//!
//! A person asking the screener, the agent calling its tool, the scheduler and a
//! deployment's rebalance all come through [`run_selection`]: the same revision,
//! the same universe resolved at the same instant, the same bars read as the
//! world stood then, the same engine. So a person and the agent asking about the
//! same strategy at the same instant get the same list — not by convention but
//! because there is one path.
//!
//! "As of" means as of: the evaluation instant is the last close of the
//! strategy's timeframe at or before the time asked for; membership is what was
//! known at that instant (`dataplane::universe`, both clocks); bars are what had
//! closed *and been recorded* by then (`BarStore::as_of`). A run as of last March
//! cannot see April.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::strategy_graph::{StrategyGraph, StrategyKind};
use serde::Serialize;
use strategy_engine::env::ModelProvider;
use strategy_engine::plan::PRIMARY;
use strategy_engine::selection::{SelectionEngine, SelectionRun, SelectionStep};
use strategy_engine::{EngineError, Membership, ModelResponse, Plan};
use uuid::Uuid;

use crate::store::LoadedBar;

/// Who asked. Journaled with every run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedBy {
    Person,
    Agent,
    Schedule,
    Deployment,
}

impl RequestedBy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
            Self::Schedule => "schedule",
            Self::Deployment => "deployment",
        }
    }
}

/// One selection run, as served and journaled.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectionReport {
    pub strategy_ref: String,
    pub revision: String,
    /// The instant the selection describes.
    pub as_of: DateTime<Utc>,
    /// `false`: today's membership applied to the past (a list or an asset
    /// class) — a survivorship caveat, stated rather than hidden.
    pub universe_point_in_time: bool,
    /// SHA-256 of the whole evaluation.
    pub digest: String,
    pub run: SelectionRun,
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionRunError {
    #[error("{0}")]
    NotFound(String),
    #[error("'{0}' is a trading strategy; a selection run needs a selection strategy")]
    NotSelection(String),
    #[error("the strategy does not compile: {0}")]
    Compile(String),
    #[error("universe: {0}")]
    Universe(String),
    #[error("bars: {0}")]
    Bars(String),
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

/// The last close of `timeframe_secs` at or before `at_ns`.
#[must_use]
pub fn evaluation_instant(at_ns: i64, timeframe_secs: u32) -> i64 {
    let tf = i64::from(timeframe_secs) * 1_000_000_000;
    at_ns.div_euclid(tf) * tf
}

/// Evaluate a compiled selection over loaded bars, as of `at_ns`.
///
/// Pure. Every bar closing at or before `at_ns` builds history; every earlier
/// close of the primary timeframe is a warm-up evaluation over the members of
/// that instant, so stateful operators carry what they would have carried had
/// the selection been running; the evaluation at `at_ns` is the answer. A bar
/// closing after `at_ns` is never read.
///
/// # Errors
/// An inconsistent universe or an engine defect.
pub fn evaluate_over(
    plan: &Arc<Plan>,
    membership: &Membership,
    bars: &BTreeMap<String, Vec<Vec<LoadedBar>>>,
    at_ns: i64,
    models: &mut dyn ModelProvider,
) -> Result<SelectionRun, SelectionRunError> {
    let mut engine = SelectionEngine::new(Arc::clone(plan))?;
    let mut by_instant: BTreeMap<i64, Vec<(&str, usize, &LoadedBar)>> = BTreeMap::new();
    for (m, inputs) in bars {
        for (input, series) in inputs.iter().enumerate() {
            for b in series
                .iter()
                .filter(|b| b.ts_ns <= at_ns && b.knowledge_ns <= at_ns)
            {
                by_instant.entry(b.ts_ns).or_default().push((m, input, b));
            }
        }
    }
    let primary_closes: BTreeSet<i64> = by_instant
        .iter()
        .filter(|(_, v)| v.iter().any(|(_, input, _)| *input == PRIMARY))
        .map(|(t, _)| *t)
        .collect();
    let universe = |t: i64| {
        membership
            .members_at(t)
            .map_err(|e| SelectionRunError::Universe(e.to_string()))
    };
    for (t, rows) in &by_instant {
        for (m, input, b) in rows {
            engine.on_bar(m, *input, bar(b))?;
        }
        if *t < at_ns && primary_closes.contains(t) {
            let members = universe(*t)?;
            let step = engine.evaluate(*t, &members, true)?;
            debug_assert_eq!(step, SelectionStep::Idle);
        }
    }
    let members = universe(at_ns)?;
    let mut step = engine.evaluate(at_ns, &members, false)?;
    loop {
        match step {
            SelectionStep::NeedModels(reqs) => {
                let answers = reqs
                    .iter()
                    .map(|q| ModelResponse {
                        node: q.node.clone(),
                        outcome: models.answer(q),
                    })
                    .collect();
                step = engine.resume(answers)?;
            }
            SelectionStep::Selected(run) => return Ok(*run),
            SelectionStep::Idle => unreachable!("a non-warm-up evaluation selects"),
        }
    }
}

fn bar(b: &LoadedBar) -> strategy_engine::Bar {
    use rust_decimal::prelude::ToPrimitive;
    let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(f64::NAN);
    strategy_engine::Bar {
        close_ns: b.ts_ns,
        open: f(b.open),
        high: f(b.high),
        low: f(b.low),
        close: f(b.close),
        volume: f(b.volume),
    }
}

/// Load a selection strategy by revision or id and compile it.
///
/// # Errors
/// Not found, not a selection, or does not compile.
pub async fn load_selection(
    pg: &sqlx::PgPool,
    strategy_ref: &str,
) -> Result<
    (
        StrategyGraph,
        Arc<Plan>,
        Arc<crate::models::RegistrySnapshot>,
    ),
    SelectionRunError,
> {
    let json = crate::runner::stored_definition(pg, strategy_ref, strategy_ref)
        .await
        .map_err(|e| SelectionRunError::NotFound(format!("{e:#}")))?;
    if !crate::runner::is_v3(&json) {
        return Err(SelectionRunError::NotFound(format!(
            "'{strategy_ref}' is not a {} graph",
            domain::strategy_graph::FORMAT
        )));
    }
    let graph: StrategyGraph = serde_json::from_value(json)
        .map_err(|e| SelectionRunError::Compile(format!("graph does not parse: {e}")))?;
    if graph.kind != StrategyKind::Selection {
        return Err(SelectionRunError::NotSelection(strategy_ref.to_string()));
    }
    let registry = Arc::new(crate::models::RegistrySnapshot::load_or_empty(pg).await);
    let plan =
        crate::runner::compile_for_run(&graph, &crate::run::ParamMap::new(), registry.as_ref())
            .map_err(SelectionRunError::Compile)?;
    Ok((graph, Arc::new(plan), registry))
}

/// Load every member's bars for a selection evaluated at `at_ns`: enough history
/// for each input, read as the world stood at `at_ns`.
///
/// # Errors
/// ClickHouse.
pub async fn load_bars(
    ch_url: &str,
    plan: &Plan,
    members: &[String],
    at_ns: i64,
) -> Result<BTreeMap<String, Vec<Vec<LoadedBar>>>, SelectionRunError> {
    let at = DateTime::from_timestamp_nanos(at_ns);
    let store = crate::store::BarStore::connect(ch_url).as_of(at);
    let mut out = BTreeMap::new();
    for m in members {
        let mut inputs = Vec::with_capacity(plan.inputs.len());
        for (i, need) in plan.required_history() {
            let input = &plan.inputs[i];
            let lead = chrono::Duration::seconds(
                i64::from(need.saturating_add(2)) * i64::from(input.timeframe.seconds()),
            );
            // A member with no stored bars is still a member: it is evaluated,
            // and excluded for having no data.
            let loaded = store
                .load_bars(m, input.timeframe.timeframe(), at - lead, at)
                .await
                .map_err(|e| SelectionRunError::Bars(format!("{m}: {e:#}")))?;
            inputs.push(loaded);
        }
        out.insert(m.clone(), inputs);
    }
    Ok(out)
}

/// Run a selection strategy as of `as_of` (now, when the caller asks for now).
///
/// # Errors
/// As [`load_selection`], an unloadable universe or bars, or an engine defect.
pub async fn run_selection(
    pg: &sqlx::PgPool,
    ch_url: &str,
    strategy_ref: &str,
    as_of: DateTime<Utc>,
    models: Option<&crate::runner::ModelsFactory>,
) -> Result<SelectionReport, SelectionRunError> {
    let (graph, plan, registry) = load_selection(pg, strategy_ref).await?;
    let domain::strategy_graph::graph::Sink::Select { universe, .. } = &graph.sink else {
        return Err(SelectionRunError::NotSelection(strategy_ref.to_string()));
    };
    let membership = crate::env::universe::load(pg, universe)
        .await
        .map_err(|e| SelectionRunError::Universe(e.to_string()))?;
    let at_ns = evaluation_instant(
        as_of.timestamp_nanos_opt().unwrap_or(i64::MAX),
        plan.timeframe.seconds(),
    );
    // Everyone who is a member at any warm-up instant, so each warm-up
    // evaluation sees its own membership.
    let warm_from = at_ns
        - i64::from(plan.warmup_ticks.saturating_add(1))
            * i64::from(plan.timeframe.seconds())
            * 1_000_000_000;
    let members = membership.ever_between(warm_from, at_ns);
    let bars = load_bars(ch_url, &plan, &members, at_ns).await?;
    let mut provider: Box<dyn ModelProvider> = match models {
        Some(factory) => factory(registry, false),
        None => Box::new(strategy_engine::env::NoModels),
    };
    // The sidecar is reached by blocking inside the provider; keep that off the
    // async workers' backs.
    let run = tokio::task::block_in_place(|| {
        evaluate_over(&plan, &membership, &bars, at_ns, provider.as_mut())
    })?;
    Ok(SelectionReport {
        strategy_ref: strategy_ref.to_string(),
        revision: plan.revision.clone(),
        as_of: DateTime::from_timestamp_nanos(at_ns),
        universe_point_in_time: membership.point_in_time(),
        digest: run.digest(),
        run,
    })
}

/// Write a run to the selection journal (migration 0147).
///
/// # Errors
/// The database.
pub async fn journal(
    pg: &sqlx::PgPool,
    report: &SelectionReport,
    requested_by: RequestedBy,
) -> Result<Uuid, sqlx::Error> {
    let s = &report.run.selection;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO selection_run (strategy_ref, revision, as_of, requested_by, universe_size, \
           universe_point_in_time, selected_count, selection, digest) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING run_id",
    )
    .bind(&report.strategy_ref)
    .bind(&report.revision)
    .bind(report.as_of)
    .bind(requested_by.as_str())
    .bind(i32::try_from(s.universe_size).unwrap_or(i32::MAX))
    .bind(report.universe_point_in_time)
    .bind(i32::try_from(s.selected.len()).unwrap_or(i32::MAX))
    .bind(serde_json::to_value(s).unwrap_or_default())
    .bind(&report.digest)
    .fetch_one(pg)
    .await?;
    Ok(id)
}

/// One journaled run, as listed.
#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct JournaledRun {
    pub run_id: Uuid,
    pub strategy_ref: String,
    pub revision: String,
    pub as_of: DateTime<Utc>,
    pub requested_by: String,
    pub universe_size: i32,
    pub universe_point_in_time: bool,
    pub selected_count: i32,
    pub selection: serde_json::Value,
    pub digest: String,
    pub created_at: DateTime<Utc>,
}

/// Journaled runs, newest first; of one strategy (by the reference it was
/// asked under, or its revision) when `strategy` is given.
///
/// # Errors
/// The database.
pub async fn list(
    pg: &sqlx::PgPool,
    strategy: Option<&str>,
    limit: i64,
) -> Result<Vec<JournaledRun>, sqlx::Error> {
    sqlx::query_as(
        "SELECT run_id, strategy_ref, revision, as_of, requested_by, universe_size, \
                universe_point_in_time, selected_count, selection, digest, created_at \
           FROM selection_run \
          WHERE $1::text IS NULL OR strategy_ref = $1 OR revision = $1 \
          ORDER BY created_at DESC LIMIT $2",
    )
    .bind(strategy)
    .bind(limit.clamp(1, 500))
    .fetch_all(pg)
    .await
}
