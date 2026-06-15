#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod bar_persist;
mod hot_path;
mod pipeline_manager;
mod tee;

use std::sync::Arc;

use anyhow::Context;
use domain::instrument::AssetClass;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load layered config first (before tracing, so we can read json_logs).
    let cfg = cfg::load().context("failed to load config")?;

    // Init tracing.
    if cfg.observability.json_logs {
        observability::init_json("platform");
    } else {
        observability::init("platform");
    }

    info!(version = env!("CARGO_PKG_VERSION"), "platform starting");

    // Connect to Postgres.
    let pg = storage::postgres::connect(&cfg.database.url)
        .await
        .context("failed to connect to postgres")?;

    info!("postgres connected");

    // Apply pending schema migrations on boot so a fresh database is always at
    // the current schema (backtest_runs, etc.) without a manual step (#20).
    storage::postgres::run_migrations(&pg)
        .await
        .context("failed to apply database migrations")?;
    info!("database migrations applied");

    // Load kill switch state from Postgres (trading_enabled column).
    let initially_halted = load_kill_switch_state(&pg).await;
    let kill_switch = Arc::new(risk::KillSwitch::new(initially_halted));

    // Build risk gate with default limits.
    let risk_gate = Arc::new(risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        Arc::clone(&kill_switch),
    ));

    // Build the in-house paper trading engine — the paper half of execution.
    // One internal account per asset class, fills simulated locally with
    // per-class realism (tuned spreads/fees, size impact, session calendars,
    // mark-freshness gates); balances/positions/ledger all in-process.
    // Live and paper share the same collector data: every pipeline feeds the
    // engine's mark board and registers its instrument's asset class.  The
    // multi-asset broker then routes each paper order to the account of its
    // instrument's class.  Live broker adapters are loaded per-user from the
    // database credential store when live credentials exist; default is paper.
    let paper_engine = Arc::new(execution::paper::PaperTradingEngine::realistic());
    let paper_broker = paper_engine.multi_asset_broker();
    let execution_engine = Arc::new(execution::ExecutionEngine::new(Arc::new(paper_broker)));
    info!("in-house paper trading engine initialised (per-class accounts, realism gates on, no external APIs)");

    // Perpetual-swap funding: charge open perp positions hourly at a flat
    // default rate (1 bp per 8h, pro-rated).  Mirrors live venue cash flows.
    {
        let engine = Arc::clone(&paper_engine);
        tokio::spawn(async move {
            // 1 bp per 8h, pro-rated hourly: 0.0001 / 8.
            let hourly_rate = rust_decimal::Decimal::new(125, 7);
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            tick.tick().await; // skip the immediate first tick
            loop {
                tick.tick().await;
                for position in engine.positions(AssetClass::PerpetualSwap) {
                    match engine.apply_funding(&position.instrument_id, hourly_rate) {
                        Ok(payment) => tracing::debug!(
                            instrument_id = %position.instrument_id,
                            %payment,
                            "paper perp funding applied"
                        ),
                        Err(e) => tracing::debug!(
                            instrument_id = %position.instrument_id,
                            error = %e,
                            "paper perp funding skipped"
                        ),
                    }
                }
            }
        });
    }

    // Build demand manager and UI gateway.
    let demand_registry = Arc::new(demand_manager::DemandRegistry::new(Arc::new(
        demand_manager::NoopPipelineFactory,
    )));
    let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(demand_registry));

    // ── In-process hot-path pipeline ──────────────────────────────────────────
    //
    // Connect to NATS for the JetStream tee (best-effort persistence).
    // If NATS is unavailable the tee is skipped; the hot path still runs.
    let (tee_tx, tee_rx) = tokio::sync::mpsc::unbounded_channel::<hot_path::RawTick>();

    match event_bus::connect(&cfg.nats.url).await {
        Ok(nats) => {
            if let Err(e) = event_bus::setup_streams(&nats.js).await {
                tracing::warn!(error = %e, "JetStream stream setup failed — tee disabled");
            } else {
                let publisher = Arc::new(event_bus::Publisher::new(nats.js));
                tokio::spawn(tee::run_tee(publisher, tee_rx));
                info!("JetStream tee task started");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "NATS unavailable — JetStream tee disabled");
            // Drop tee_rx so tee senders see a closed channel harmlessly.
            drop(tee_rx);
        }
    }

    // ── Continuous live-data pipelines ────────────────────────────────────────
    //
    // The single writer of live 1-minute bars to ClickHouse.  Every initialized
    // asset's pipeline aggregates trades into 1m OHLCV and sends completed bars
    // here, so minute-level history accrues for as long as the platform runs —
    // independent of whether any strategy or automation is subscribed.
    let (bar_tx, bar_rx) = tokio::sync::mpsc::unbounded_channel::<bar_persist::PersistBar>();
    tokio::spawn(bar_persist::run_bar_persist(
        cfg.clickhouse.url.clone(),
        bar_rx,
    ));

    // Owns one in-process pipeline per initialized instrument.
    let pipeline_manager = Arc::new(pipeline_manager::PipelineManager::new(
        tee_tx,
        bar_tx,
        Arc::clone(&execution_engine),
        Arc::clone(&risk_gate),
        Arc::clone(&paper_engine),
        cfg.clickhouse.url.clone(),
    ));

    // Resume a pipeline for every already-initialized asset (asset_lifecycle).
    let initialized: Vec<(String, String)> =
        sqlx::query_as("SELECT symbol, asset_class FROM asset_lifecycle")
            .fetch_all(&pg)
            .await
            .unwrap_or_default();
    for (symbol, asset_class) in &initialized {
        pipeline_manager.ensure(symbol, asset_class);
    }
    // Keep BTC-USD streaming by default (the bundled in-process crypto feed)
    // even before it is formally initialized.
    pipeline_manager.ensure("BTC-USD", "crypto_spot_cex");
    info!(
        pipelines = pipeline_manager.active_count(),
        "live 1m aggregation pipelines running"
    );

    // Start pipelines on demand when new assets are initialized (no restart).
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<api::StreamRequest>();
    {
        let mgr = Arc::clone(&pipeline_manager);
        tokio::spawn(async move {
            while let Some(req) = stream_rx.recv().await {
                mgr.ensure(&req.instrument_id, &req.asset_class);
            }
        });
    }

    // Reset all automations to disarmed on startup.  Users must explicitly
    // re-arm after each server restart — this prevents stale automations from
    // executing without deliberate user action.
    match storage::automation::disarm_all_automations(&pg).await {
        Ok(count) => info!(count, "all automations reset to disarmed on server start"),
        Err(e) => tracing::warn!(error = %e, "could not reset automations at startup"),
    }

    // Backtest orchestrator — owns simulation jobs and drives the
    // market_simulator engine (used purely as an embedded SDK). Reads bars
    // from this platform's ClickHouse store; the simulator owns no data.
    let backtest_manager = backtest::BacktestManager::new(cfg.clickhouse.url.clone(), pg.clone());
    info!("backtest orchestrator initialised (market_simulator SDK)");

    // Build the API router.
    let app_state = api::AppState::new(
        pg,
        risk_gate,
        kill_switch,
        execution_engine,
        Arc::clone(&paper_engine),
        gateway,
        backtest_manager,
        cfg.email.clone(),
        cfg.clickhouse.url.clone(),
        Some(stream_tx),
    );
    let router = api::router(app_state);

    // Safety guardrail (M-17): refuse to bind on a network-accessible address
    // while bearer-token auth is still the placeholder (any non-empty token
    // accepted).  Remove this check when Phase 2 session validation lands.
    let is_loopback = matches!(cfg.api.host.as_str(), "127.0.0.1" | "::1" | "localhost");
    if !is_loopback {
        anyhow::bail!(
            "SECURITY: auth is placeholder-only (M-17) — refusing to bind on \
             non-loopback address '{}'. Set api.host to 127.0.0.1 or implement \
             Phase 2 session validation first.",
            cfg.api.host
        );
    }

    // Bind and serve.
    let addr = format!("{}:{}", cfg.api.host, cfg.api.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;

    info!(addr, "listening");

    axum::serve(listener, router)
        .await
        .context("axum serve error")?;

    Ok(())
}

/// Read `trading_enabled` from Postgres.  Returns `true` (halted) if the row
/// says disabled, or if the DB is unreachable (fail-closed).
async fn load_kill_switch_state(pg: &sqlx::PgPool) -> bool {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT trading_enabled FROM global_risk_config LIMIT 1")
            .fetch_optional(pg)
            .await
            .ok()
            .flatten();

    match row {
        Some((enabled,)) => !enabled,
        None => {
            tracing::warn!("could not read global_risk_config — defaulting kill switch to ACTIVE (fail-closed)");
            true
        }
    }
}
