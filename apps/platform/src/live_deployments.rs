//! The live-runner supervisor (Set U U-4.1, U-4.6).
//!
//! The one live registry (ADR-0038) *declares* v3 deployments; this is what
//! *runs* them — one [`LiveRunner`] task per declared deployment, fed by the
//! process-wide minute-bar hub. Hot-path stage 3 keeps running v1 strategies
//! until Phase 8 and never sees a v3 deployment.
//!
//! - **At boot** every deployment the record says is running is declared
//!   again, so a restart resumes them — from their checkpoints.
//! - **Withdrawal** is read before every bar: a deployment withdrawn from the
//!   registry stops at its next decision tick.
//! - **A halted runner** (its journal or checkpoint could not be written)
//!   closes its record with the reason and stops.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use strategy_runtime::live::{LiveStrategyRegistry, V3Deployment};
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::live_bars::{self, MinuteBar};
use crate::live_runner::{minute_bar, LiveDeps, LiveRunner};

/// Start the supervisor.
pub fn spawn(deps: LiveDeps, pg: sqlx::PgPool, ch_url: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let registry = strategy_runtime::live::global();
        resume_recorded(&registry, &pg).await;
        let mut running: BTreeMap<Uuid, tokio::task::JoinHandle<()>> = BTreeMap::new();
        let mut seen = u64::MAX;
        let mut pruned_at: Option<std::time::Instant> = None;
        loop {
            // The decision journal's retention (U-4.5): once a day, records
            // older than the data budget's window go. Nothing else deletes
            // from it, and an append-only table with no prune grows for ever.
            if pruned_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(86_400)) {
                pruned_at = Some(std::time::Instant::now());
                let cutoff = chrono::Utc::now()
                    - chrono::Duration::days(
                        storage::postgres::decision_journal::DEFAULT_RETENTION_DAYS,
                    );
                match storage::postgres::decision_journal::prune_older_than(&pg, cutoff).await {
                    Ok(n) if n > 0 => info!(
                        removed = n,
                        "decision journal pruned to its retention window"
                    ),
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "decision journal prune failed; will retry tomorrow")
                    }
                }
            }
            running.retain(|_, h| !h.is_finished());
            let generation = registry.v3_generation();
            if generation != seen {
                seen = generation;
                for d in registry.v3_snapshot() {
                    if running.contains_key(&d.deployment_id) {
                        continue;
                    }
                    let task = tokio::spawn(run_one(
                        Arc::clone(&d),
                        deps.clone(),
                        pg.clone(),
                        ch_url.clone(),
                        live_bars::subscribe(),
                        Arc::clone(&registry),
                    ));
                    running.insert(d.deployment_id, task);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

/// Declare again every deployment the record says is running.
async fn resume_recorded(registry: &LiveStrategyRegistry, pg: &sqlx::PgPool) {
    let rows = match storage::postgres::deployments::running(pg).await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "could not read running deployments; none resumed");
            return;
        }
    };
    for row in rows {
        let graph = match serde_json::from_value(row.definition_json.clone()) {
            Ok(g) => g,
            Err(e) => {
                error!(deployment = %row.deployment_id, error = %e, "recorded graph does not parse; closing the record");
                let _ = storage::postgres::deployments::stop(
                    pg,
                    row.deployment_id,
                    "graph does not parse at resume",
                )
                .await;
                continue;
            }
        };
        let declared = registry.register_v3(V3Deployment {
            deployment_id: row.deployment_id,
            user_id: row.user_id,
            instrument_id: row.instrument_id,
            graph,
            revision: row.revision_hash,
            starting_capital: row.starting_capital,
        });
        if let Err(e) = declared {
            warn!(deployment = %row.deployment_id, error = %e, "recorded deployment not resumed");
        }
    }
}

/// Run one deployment until it is withdrawn or halts.
pub async fn run_one(
    d: Arc<V3Deployment>,
    deps: LiveDeps,
    pg: sqlx::PgPool,
    ch_url: String,
    mut bars: tokio::sync::broadcast::Receiver<MinuteBar>,
    registry: Arc<LiveStrategyRegistry>,
) {
    let id = d.deployment_id;
    let runner = match build(&d, deps, &ch_url, &pg).await {
        Ok(r) => r,
        Err(reason) => {
            error!(deployment = %id, reason, "deployment could not start; closing the record");
            let _ = storage::postgres::deployments::stop(
                &pg,
                id,
                &format!("could not start: {reason}"),
            )
            .await;
            registry.deregister_v3(id);
            return;
        }
    };
    info!(deployment = %id, instruments = ?runner.instruments(), "v3 deployment running on paper");
    if let Some(reason) = drive(runner, &mut bars, &registry).await {
        let _ = storage::postgres::deployments::stop(&pg, id, &reason).await;
        registry.deregister_v3(id);
    }
}

/// Feed minute bars to a runner until it is withdrawn (`None`) or halts
/// (`Some(reason)`).
pub async fn drive(
    mut runner: LiveRunner,
    bars: &mut tokio::sync::broadcast::Receiver<MinuteBar>,
    registry: &LiveStrategyRegistry,
) -> Option<String> {
    let id = runner.deployment_id();
    let wanted = runner.instruments();
    loop {
        let m = match bars.recv().await {
            Ok(m) => m,
            Err(RecvError::Lagged(n)) => {
                warn!(deployment = %id, missed = n, "live runner fell behind the bar feed; missed minutes fold into the next bar");
                continue;
            }
            Err(RecvError::Closed) => return None,
        };
        // Withdrawn: stop before deciding again.
        if !registry.is_v3_declared(id) {
            info!(deployment = %id, "v3 deployment withdrawn; runner stopped");
            return None;
        }
        if !wanted.contains(&m.instrument_id) {
            continue;
        }
        let Some(bar) = minute_bar(&m.bar) else {
            continue;
        };
        if let Err(e) = runner.on_minute_bar(&m.instrument_id, bar).await {
            error!(deployment = %id, error = %e, "live runner error");
        }
        if let Some(reason) = runner.halted() {
            return Some(format!("halted: {reason}"));
        }
    }
}

/// Compile, size and warm a deployment's runner.
async fn build(
    d: &Arc<V3Deployment>,
    deps: LiveDeps,
    ch_url: &str,
    pg: &sqlx::PgPool,
) -> Result<LiveRunner, String> {
    let registry = Arc::new(backtest::models::RegistrySnapshot::load_or_empty(pg).await);
    let plan = backtest::runner::compile_for_run(
        &d.graph,
        &backtest::run::ParamMap::new(),
        registry.as_ref(),
    )?;
    // Models answer through the gateway over the registry this deployment
    // compiled against: pinned versions, batched per tick (U-5.3).
    let mut deps = deps;
    if let Some(client) = deps.sidecar.clone() {
        deps.models = Arc::new(model_registry::provider::gateway::GatewayModels::new(
            client, registry,
        ));
    }
    let asset_class: domain::instrument::AssetClass =
        serde_json::from_value(serde_json::Value::String(d.graph.asset_class.clone()))
            .map_err(|_| format!("unknown asset class '{}'", d.graph.asset_class))?;
    let limits = match risk::context::instrument_limits(&d.instrument_id) {
        risk::Measured::Yes(l) => l,
        risk::Measured::Unavailable => {
            return Err(format!(
                "no instrument metadata for '{}': orders cannot be sized or rounded",
                d.instrument_id
            ))
        }
    };
    let spec = strategy_engine::InstrumentSpec {
        instrument: d.instrument_id.clone(),
        lot_size: limits.lot_size,
        tick_size: limits.tick_size,
        min_qty: limits.lot_size,
        caps: strategy_engine::VenueCaps::all(),
    };
    // Warm-up from stored bars, every declared input, up to now.
    let store = backtest::store::BarStore::connect(ch_url);
    let now = chrono::Utc::now();
    let mut history = Vec::with_capacity(plan.inputs.len());
    for (i, need) in plan.required_history() {
        let input = &plan.inputs[i];
        let instrument = match &input.instrument {
            domain::strategy_graph::InstrumentRef::Bound => d.instrument_id.clone(),
            domain::strategy_graph::InstrumentRef::Fixed { instrument } => instrument.clone(),
        };
        let lead = chrono::Duration::seconds(
            i64::from(need.saturating_add(2)) * i64::from(input.timeframe.seconds()),
        );
        let loaded = store
            .load_bars(&instrument, input.timeframe.timeframe(), now - lead, now)
            .await
            .map_err(|e| format!("warm-up bars for {instrument}: {e:#}"))?;
        history.push(
            loaded
                .iter()
                .map(|b| {
                    use rust_decimal::prelude::ToPrimitive;
                    let f = |x: rust_decimal::Decimal| x.to_f64().unwrap_or(f64::NAN);
                    strategy_engine::Bar {
                        close_ns: b.ts_ns,
                        open: f(b.open),
                        high: f(b.high),
                        low: f(b.low),
                        close: f(b.close),
                        volume: f(b.volume),
                    }
                })
                .collect(),
        );
    }
    let mut runner = LiveRunner::start(
        Arc::clone(d),
        Arc::new(plan),
        asset_class,
        spec,
        deps,
        history,
    )
    .await
    .map_err(|e| e.to_string())?;
    // The minutes already elapsed in each longer timeframe's open bucket.
    for (instrument, seconds) in runner.input_timeframes() {
        if seconds <= 60 {
            continue;
        }
        let period = i64::from(seconds);
        let bucket_start = now.timestamp().div_euclid(period) * period;
        let from = chrono::DateTime::from_timestamp(bucket_start, 0).unwrap_or(now);
        let minutes = store
            .load_bars(
                &instrument,
                domain::payloads::bar::Timeframe::Minutes1,
                from,
                now,
            )
            .await
            .map_err(|e| format!("open-bucket minutes for {instrument}: {e:#}"))?;
        for m in &minutes {
            use rust_decimal::prelude::ToPrimitive;
            let f = |x: rust_decimal::Decimal| x.to_f64().unwrap_or(f64::NAN);
            runner.prime_minute(
                &instrument,
                strategy_engine::Bar {
                    close_ns: m.ts_ns,
                    open: f(m.open),
                    high: f(m.high),
                    low: f(m.low),
                    close: f(m.close),
                    volume: f(m.volume),
                },
            );
        }
    }
    Ok(runner)
}
