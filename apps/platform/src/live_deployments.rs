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
    // A selection-bound deployment and its members are the allocator's to
    // resume: a member needs the sleeve its allocator holds (U-6.4).
    for row in rows
        .into_iter()
        .filter(|r| r.target.is_none() && r.parent_deployment_id.is_none())
    {
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
            sleeve: None,
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
    let refresh = ExternalRefresh {
        pg: pg.clone(),
        ch_url: ch_url.clone(),
    };
    if let Some(reason) = drive(runner, &mut bars, &registry, Some(refresh)).await {
        let _ = storage::postgres::deployments::stop(&pg, id, &reason).await;
        registry.deregister_v3(id);
    }
}

/// Where a runner's external rows are re-read from (P-3.6).
pub struct ExternalRefresh {
    pub pg: sqlx::PgPool,
    pub ch_url: String,
}

/// How often a deployment that reads macro or fundamentals series re-reads
/// them. Their sources publish at most daily; an hour bounds how late a new
/// vintage reaches a decision.
const EXTERNAL_REFRESH: Duration = Duration::from_secs(3_600);

/// Feed minute bars to a runner until it is withdrawn (`None`) or halts
/// (`Some(reason)`), re-reading its external rows every
/// [`EXTERNAL_REFRESH`] when it reads any.
pub async fn drive(
    mut runner: LiveRunner,
    bars: &mut tokio::sync::broadcast::Receiver<MinuteBar>,
    registry: &LiveStrategyRegistry,
    external: Option<ExternalRefresh>,
) -> Option<String> {
    let id = runner.deployment_id();
    let wanted = runner.instruments();
    let reads_external = !runner.plan().external_indicators().is_empty();
    let mut refreshed = std::time::Instant::now();
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
        // Left its selection (U-6.4): the runner carries out how.
        if let Some(reason) = retire(&mut runner).await {
            info!(deployment = %id, reason, "member left its selection; runner stopped");
            return Some(reason);
        }
        if !wanted.contains(&m.instrument_id) {
            continue;
        }
        let Some(bar) = minute_bar(&m.bar) else {
            continue;
        };
        if let (true, Some(src)) = (reads_external, &external) {
            if refreshed.elapsed() >= EXTERNAL_REFRESH {
                refreshed = std::time::Instant::now();
                let now = chrono::Utc::now();
                let from = now - chrono::Duration::days(1);
                match backtest::runner::load_external(
                    &src.pg,
                    &src.ch_url,
                    runner.plan(),
                    from,
                    now,
                )
                .await
                {
                    Ok(data) => {
                        if let Err(e) = runner.refresh_external(data).await {
                            error!(deployment = %id, error = %e, "external rows not handed over");
                        }
                    }
                    // The rows already held stay point-in-time correct; only a
                    // vintage published since is late.
                    Err(e) => {
                        warn!(deployment = %id, error = %e, "external rows not re-read; keeping the last")
                    }
                }
            }
        }
        if let Err(e) = runner.on_minute_bar(&m.instrument_id, bar).await {
            error!(deployment = %id, error = %e, "live runner error");
        }
        if let Some(reason) = runner.halted() {
            return Some(format!("halted: {reason}"));
        }
    }
}

/// A member whose allocator dropped it leaves as its deployment says:
/// `hold` stops deciding and keeps the position, `close` flattens through the
/// runner's own gate first, `let_strategy_exit` keeps deciding until its own
/// rules have it flat with nothing working. `Some(reason)` once it should stop.
pub async fn retire(runner: &mut LiveRunner) -> Option<String> {
    use domain::strategy_graph::deployment::OnRemoval;
    match runner.retiring()? {
        OnRemoval::Hold => Some("left its selection: holding the position".into()),
        OnRemoval::Close => {
            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let verdicts = runner.flatten(now).await;
            let refused: Vec<String> = verdicts
                .iter()
                .filter(|v| v.verdict != "approved")
                .map(|v| format!("{}: {}", v.tag, v.verdict))
                .collect();
            Some(if refused.is_empty() {
                "left its selection: flattened".into()
            } else {
                format!(
                    "left its selection: flatten refused ({})",
                    refused.join(", ")
                )
            })
        }
        OnRemoval::LetStrategyExit => runner
            .is_idle()
            .then(|| "left its selection: its strategy exited".into()),
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
        let mut gateway = model_registry::provider::gateway::GatewayModels::new(client, registry);
        if let Some((llm, caps)) = deps.llm.clone() {
            gateway = gateway.with_llm(llm, caps);
        }
        deps.models = Arc::new(gateway);
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
    // External rows its macro and fundamentals indicators read (P-3.6), as
    // they stand now, reaching back past the earliest warm-up bar.
    let first = history
        .iter()
        .flatten()
        .map(|b: &strategy_engine::Bar| b.close_ns)
        .min()
        .map_or(now, chrono::DateTime::from_timestamp_nanos);
    let external = backtest::runner::load_external(pg, ch_url, &plan, first, now).await?;
    deps.external = (!external.is_empty()).then_some(external);
    let gate = Arc::clone(&deps.gate);
    // A deployment trades in its own paper account (Set U U-4.7): its own cash,
    // positions and day's loss, seeded with its starting capital — or the one
    // restored at boot, which is never reseeded. A selection's members size
    // against their sleeves and trade in the asset class's account.
    if d.sleeve.is_none() {
        deps.paper
            .open_deployment_account(d.deployment_id, asset_class, d.starting_capital)
            .map_err(|e| format!("paper account: {e}"))?;
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
    // Forward evidence (Set T T-0.4): a deployment the record names as such
    // has every bar recorded against a shadow backtest of the same plan.
    if let Some(spec) = forward_spec(pg, d).await? {
        let recorders = reconciliation::ForwardRecorders::installed()
            .ok_or("forward evidence is recorded but no recorder registry is installed")?;
        let forward_id = spec.deployment_id;
        let fill_seq = storage::postgres::decision_journal::forward_fills(pg, forward_id)
            .await
            .map_err(|e| format!("forward fills: {e}"))?
            .last()
            .map_or(0, |f| f.seq);
        let halts = Arc::clone(gate.halt_registry());
        let (recorder, resumed) =
            match resume_forward(pg, d, &spec, recorders.as_ref(), &halts).await {
                Some(r) => (r, true),
                None => (
                    recorders
                        .deploy(spec, d.starting_capital, halts)
                        .map_err(|e| format!("forward recorder: {e}"))?,
                    false,
                ),
            };
        runner.attach_forward(
            recorder,
            forward_id,
            fill_seq,
            d.user_id.clone(),
            limits,
            resumed,
        )?;
        info!(deployment = %d.deployment_id, "forward evidence recording attached");
    }
    Ok(runner)
}

/// What a forward recorder stored before a restart.
pub(crate) struct StoredForward {
    pub marks: Vec<storage::postgres::decision_journal::ForwardMark>,
    pub fills: Vec<storage::postgres::decision_journal::ForwardFill>,
    pub observations: Vec<domain::forward::ForwardObservation>,
}

/// Read what [`resume_from`] needs.
async fn resume_forward(
    pg: &sqlx::PgPool,
    d: &V3Deployment,
    spec: &reconciliation::DeploymentSpec,
    recorders: &reconciliation::ForwardRecorders,
    halts: &Arc<risk::StrategyHaltRegistry>,
) -> Option<Arc<reconciliation::ForwardRecorder>> {
    use storage::postgres::decision_journal as journal;
    let id = spec.deployment_id;
    let stored = StoredForward {
        marks: journal::forward_marks(pg, id).await.ok()?,
        fills: journal::forward_fills(pg, id).await.ok()?,
        observations: reconciliation::store::load_observations(
            pg,
            &d.user_id,
            id,
            chrono::DateTime::<chrono::Utc>::MIN_UTC,
            chrono::DateTime::<chrono::Utc>::MAX_UTC,
        )
        .await
        .ok()?,
    };
    resume_from(&stored, d, spec, recorders, halts)
}

/// Rebuild a forward recorder from what it stored (Set T T-0.4 item 3): the
/// series since its latest opening, every fill it saw since, and the last mark.
/// `None` — and a fresh series — when there is nothing to resume, or when a
/// resume would have to guess: a fee the venue never reported, or a series the
/// recorder refuses to rebuild. A fresh series is weaker evidence; a guessed one
/// is wrong evidence.
pub(crate) fn resume_from(
    stored: &StoredForward,
    d: &V3Deployment,
    spec: &reconciliation::DeploymentSpec,
    recorders: &reconciliation::ForwardRecorders,
    halts: &Arc<risk::StrategyHaltRegistry>,
) -> Option<Arc<reconciliation::ForwardRecorder>> {
    let open = stored.marks.iter().rposition(|m| m.kind == "open")?;
    let open_bar = stored.marks[open].bar;
    let last_mark = stored.marks.last()?.mark;
    let observations: Vec<domain::forward::ForwardObservation> = stored
        .observations
        .iter()
        .filter(|o| o.bar > open_bar)
        .cloned()
        .collect();
    let mut fills = Vec::new();
    for f in &stored.fills {
        if f.filled_at <= open_bar {
            continue;
        }
        let Some(charge) = f.charge else {
            warn!(deployment = %d.deployment_id, seq = f.seq, "a forward fill has no reported fee — not resuming; the series restarts");
            return None;
        };
        fills.push(reconciliation::ResumeFill {
            side: if f.side == "buy" {
                domain::order::Side::Buy
            } else {
                domain::order::Side::Sell
            },
            filled: f.filled,
            fill_price: domain::money::Price::from_decimal(f.fill_price),
            charge,
            decision_price: domain::money::Price::from_decimal(f.decision_price),
            filled_at: f.filled_at,
        });
    }
    match recorders.resume_deploy(
        spec.clone(),
        d.starting_capital,
        open_bar,
        &observations,
        &fills,
        domain::money::Price::from_decimal(last_mark),
        Arc::clone(halts),
    ) {
        Ok(r) => {
            info!(deployment = %d.deployment_id, bars = observations.len(), fills = fills.len(), "forward recorder resumed");
            Some(r)
        }
        Err(e) => {
            warn!(deployment = %d.deployment_id, error = %e, "forward recorder could not resume — the series restarts");
            None
        }
    }
}

/// The forward-evidence declaration a deployment's record names, if any
/// (`strategy_deployment.forward_deployment_id` → `mlops.paper_deployment`),
/// read under the deployment owner's tenant.
async fn forward_spec(
    pg: &sqlx::PgPool,
    d: &V3Deployment,
) -> Result<Option<reconciliation::DeploymentSpec>, String> {
    use sqlx::Row;
    let mut tx = ledger::pg::tenant_tx(pg, &d.user_id)
        .await
        .map_err(|e| format!("forward evidence: {e}"))?;
    let row = sqlx::query(
        "SELECT pd.deployment_id, pd.strategy_id, pd.trial_id, pd.numeraire, pd.bar_seconds, \
                pd.max_abs_return_divergence, pd.max_cum_return_divergence, \
                pd.max_position_divergence, pd.min_signal_match \
           FROM strategy_deployment sd \
           JOIN mlops.paper_deployment pd ON pd.deployment_id = sd.forward_deployment_id \
          WHERE sd.deployment_id = $1",
    )
    .bind(d.deployment_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| format!("forward evidence: {e}"))?;
    let _ = tx.commit().await;
    let Some(r) = row else {
        return Ok(None);
    };
    let get = |e: sqlx::Error| format!("forward evidence row: {e}");
    let numeraire: String = r.try_get("numeraire").map_err(get)?;
    let bar_seconds: i32 = r.try_get("bar_seconds").map_err(get)?;
    Ok(Some(reconciliation::DeploymentSpec {
        deployment_id: r.try_get("deployment_id").map_err(get)?,
        strategy_id: r.try_get("strategy_id").map_err(get)?,
        trial_id: r.try_get("trial_id").map_err(get)?,
        venue: reconciliation::forward::PaperVenue::Platform,
        numeraire: domain::numeraire::Numeraire::of(&numeraire.to_uppercase()),
        bar_seconds: i64::from(bar_seconds),
        thresholds: reconciliation::DivergenceThresholds {
            max_abs_return_divergence: r.try_get("max_abs_return_divergence").map_err(get)?,
            max_cum_return_divergence: r.try_get("max_cum_return_divergence").map_err(get)?,
            max_position_divergence: r.try_get("max_position_divergence").map_err(get)?,
            min_signal_match: r.try_get("min_signal_match").map_err(get)?,
        },
    }))
}

#[cfg(test)]
mod tests {
    //! Set U U-4.2 against PostgreSQL: a deployment recorded over HTTP with
    //! forward evidence is found by its readers — the supervisor's resume reads
    //! the record and its starting capital, `forward_spec` finds the
    //! `mlops.paper_deployment` row the route wrote, and reconciliation's
    //! recorder is built from that capital.
    //!
    //! ```text
    //! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
    //!   cargo test -j 2 -p platform live_deployments -- --ignored --nocapture
    //! ```
    use std::str::FromStr;
    use std::sync::Arc;

    use rust_decimal::Decimal;
    use sqlx::{Connection, Executor, PgConnection, PgPool};
    use strategy_runtime::live::LiveStrategyRegistry;
    use uuid::Uuid;

    fn with_db(url: &str, db: &str) -> String {
        let cut = url.rfind('/').expect("url has a path");
        format!("{}/{db}", &url[..cut])
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs a Postgres that can create databases"]
    async fn a_recorded_forward_deployment_is_found_by_its_readers() {
        let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let name = format!("u42_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&with_db(&base, "postgres"))
            .await
            .unwrap();
        admin
            .execute(format!("CREATE DATABASE {name}").as_str())
            .await
            .unwrap();
        let pg = PgPool::connect(&with_db(&base, &name)).await.unwrap();
        for mig in sqlx::migrate!("../../migrations").iter() {
            if mig.migration_type.is_down_migration() {
                continue;
            }
            let mut tx = pg.begin().await.unwrap();
            match tx.execute(&*mig.sql).await {
                Ok(_) => tx.commit().await.unwrap(),
                Err(_) => {
                    tx.rollback().await.ok();
                }
            }
        }

        // Someone signed in, an instrument, and the trial the forward series
        // will be compared with.
        let (user,): (Uuid,) =
            sqlx::query_as("INSERT INTO users (email) VALUES ('u42@t.test') RETURNING user_id")
                .fetch_one(&pg)
                .await
                .unwrap();
        let token = format!("tok_{}", Uuid::new_v4().simple());
        sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
            .bind(&token)
            .bind(user)
            .execute(&pg)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO instruments (instrument_id, asset_class, venue_id, base_precision, \
               quote_precision, tick_size, lot_size, trading_hours_json, halt_policy, trust_tier) \
             VALUES ('BTC-USD', 'crypto_spot_cex', 'coinbase', 8, 2, 0.01, 0.0001, '{}', \
               'non_haltable', 'centralized_exchange') ON CONFLICT (instrument_id) DO NOTHING",
        )
        .execute(&pg)
        .await
        .unwrap();
        let tenant = user.to_string();
        let mut tx = ledger::pg::tenant_tx(&pg, &tenant).await.unwrap();
        let trial: Uuid = sqlx::query_scalar(
            "INSERT INTO mlops.trial (
                 seq, prev_hash, row_hash, tenant_id, experiment_id, config_hash, config,
                 dataset_id, code_hash, image_digest, seed_set, actor_kind, actor_id,
                 policy_id, policy_version, propensity, exploration_flag, prereg_hash,
                 delta_practical
             ) VALUES (0,'\\x'::bytea,'\\x'::bytea,$1,'e','c','{}'::jsonb,'d','c','i',
                       ARRAY[1]::bigint[],'human','u','p',1,0.5,false,'p',0.1)
             RETURNING trial_id",
        )
        .bind(&tenant)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();

        // The writer: POST /api/deployments with forward evidence, through
        // the platform's own router.
        let app = api::routes::router(test_state(pg.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let graph = serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "u42_ema",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "nodes": [
            {"id":"f","op":"ema","args":{"x":"close","n":5}},
            {"id":"s","op":"ema","args":{"x":"close","n":20}},
            {"id":"up","op":"crossed_above","args":{"a":"f","b":"s"}}
          ],
          "decide": {"rules": [{"when":"up","then":{"kind":"target","size":{"mode":"units","x":0.01}}}]}
        });
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/api/deployments"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "graph": graph,
                "instrument_id": "BTC-USD",
                "starting_capital": "2500",
                "forward": {
                    "trial_id": trial,
                    "numeraire": "usd",
                    "max_abs_return_divergence": 0.02,
                    "max_cum_return_divergence": 0.05,
                    "max_position_divergence": "0.5",
                    "min_signal_match": 0.9,
                    "kill_criteria": "halt on any divergence breach"
                }
            }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        assert_eq!(status, 201, "{body}");
        let deployment_id: Uuid = serde_json::from_value(body["deployment_id"].clone()).unwrap();

        // Reader 1: the supervisor's resume reads the record and its capital.
        let registry = LiveStrategyRegistry::new();
        super::resume_recorded(&registry, &pg).await;
        let d = registry
            .v3_snapshot()
            .into_iter()
            .find(|d| d.deployment_id == deployment_id)
            .expect("the running record is declared again");
        assert_eq!(d.starting_capital, Decimal::from(2500));

        // Reader 2: Set T's forward-evidence row, found through the record.
        let spec = super::forward_spec(&pg, &d)
            .await
            .unwrap()
            .expect("the paper_deployment row the route wrote");
        assert_eq!(spec.trial_id, trial);
        assert_eq!(spec.strategy_id, deployment_id.to_string());
        assert_eq!(spec.bar_seconds, 3600);
        assert_eq!(
            spec.thresholds.max_position_divergence,
            Decimal::from_str("0.5").unwrap()
        );

        // Reader 3: reconciliation's recorder is built from that capital. A
        // capital it cannot use is refused, so the value is consumed, not
        // carried past.
        let recorders = reconciliation::ForwardRecorders::install();
        let halts = Arc::new(risk::StrategyHaltRegistry::default());
        assert!(recorders
            .deploy(spec.clone(), Decimal::ZERO, Arc::clone(&halts))
            .is_err());
        let forward_id = spec.deployment_id;
        let recorder = recorders
            .deploy(spec, d.starting_capital, halts)
            .expect("built from the recorded capital");
        assert_eq!(recorder.position(), Decimal::ZERO);
        assert!(recorders.recorder(forward_id).is_some());

        pg.close().await;
        admin
            .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
            .await
            .unwrap();
    }

    /// The platform's API state, built as `main.rs` builds it, nothing spawned.
    fn test_state(pg: PgPool) -> api::AppState {
        let kill_switch = Arc::new(risk::KillSwitch::new(false));
        let risk_gate = Arc::new(risk::RiskGate::new(
            risk::GlobalRiskLimits::default(),
            Arc::clone(&kill_switch),
        ));
        let paper_engine = Arc::new(execution::paper::PaperTradingEngine::realistic());
        let execution_engine = Arc::new(execution::ExecutionEngine::new(Arc::new(
            paper_engine.multi_asset_broker(),
        )));
        let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(Arc::new(
            demand_manager::DemandRegistry::new(Arc::new(demand_manager::NoopPipelineFactory)),
        )));
        let ch = "http://127.0.0.1:9/none".to_string();
        let backtest = backtest::BacktestManager::new(ch.clone(), pg.clone());
        let models = model_registry::ModelManager::from_env(pg.clone());
        let inference = model_registry::InferenceGateway::new(
            pg.clone(),
            Arc::new(model_registry::sidecar::SidecarClient::from_env()),
        );
        let agent = Arc::new(api::agent::AgentManager::new(
            pg.clone(),
            "http://127.0.0.1:9".into(),
            cfg::model::AgentConfig::default(),
        ));
        api::AppState::new(
            pg,
            risk_gate,
            kill_switch,
            execution_engine,
            paper_engine,
            gateway,
            backtest,
            models,
            inference,
            cfg::model::EmailConfig::default(),
            ch,
            None,
            agent,
            None,
        )
    }
}
