#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod bar_persist;
mod feature_jobs;
mod health_jobs;
mod leakage_jobs;
mod ledger_jobs;
mod regime_jobs;
mod book_feed;
mod hot_path;
mod pipeline_manager;
mod tee;

use std::sync::Arc;

use anyhow::Context;
use domain::instrument::AssetClass;
use tracing::{info, warn};

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

    // Connect to Postgres as the owner for migrations only.
    let owner_pg = storage::postgres::connect(&cfg.database.url)
        .await
        .context("failed to connect to postgres")?;

    info!("postgres connected");

    // Apply pending schema migrations on boot so a fresh database is always at
    // the current schema (backtest_runs, etc.) without a manual step (#20).
    storage::postgres::run_migrations(&owner_pg)
        .await
        .context("failed to apply database migrations")?;
    info!("database migrations applied");

    // Everything after migrations runs as the restricted `platform_app` role, so
    // grants and row-level security actually apply (migration 0044, ADR-P0-13).
    let app_password = std::env::var("PLATFORM_DB_APP_PASSWORD").unwrap_or_default();
    let pg = storage::postgres::connect_app(&owner_pg, &cfg.database.url, &app_password)
        .await
        .context("failed to connect as the restricted platform_app role")?;
    owner_pg.close().await;
    info!("runtime postgres pool connected as platform_app");

    // Ledger fixation (SPEC §4.6): signed daily anchors to WORM and whole-chain
    // verification. Required configuration; there is no way to run without it.
    let fixation = std::sync::Arc::new(
        ledger_jobs::LedgerFixation::from_env(pg.clone()).context("ledger fixation is not configured")?,
    );
    fixation.clone().spawn();
    info!("ledger anchor and verification jobs started");
    feature_jobs::spawn(pg.clone(), cfg.clickhouse.url.clone());
    leakage_jobs::spawn(pg.clone());
    health_jobs::spawn(pg.clone());
    // The rule-tier regime labeller (SPEC §5.4, ADR-P3-02). Scoped to the
    // instruments that have bars: a regime is read against the market a strategy
    // trades, and there is no platform-wide "the market" that means the same
    // thing for crypto and equities.
    regime_jobs::spawn(
        pg.clone(),
        cfg.clickhouse.url.clone(),
        cfg.regime_scopes.clone(),
    );
    info!("nightly feature consistency diff scheduled");

    // Same reasoning for ClickHouse (Set L, L-0.2). The `clickhouse/` directory is
    // mounted at /docker-entrypoint-initdb.d, so its DDL runs only when the data
    // volume is first created — an existing volume never sees a new file. Replaying
    // the embedded DDL on boot is what makes `market_bars_v2` (DATA-005 §3) actually
    // exist on the boxes that already hold bars. Every statement is
    // CREATE TABLE IF NOT EXISTS, so this is a no-op once applied.
    storage::clickhouse::migrate::run_migrations(&cfg.clickhouse.url)
        .await
        .context("failed to apply clickhouse migrations")?;
    info!("clickhouse migrations applied");

    // Move existing bars into the append-only v2 table (Set L, L-0.3). Chunked by
    // (timeframe, month) and skipped per chunk once copied, so this is a few cheap
    // counts on a normal boot and a no-op on a fresh install.
    //
    // `BARS_V2_BACKFILL_SOURCE` lets an operator migrate from a verified snapshot
    // instead of the live table — a plain-MergeTree copy cannot collapse mid-
    // migration the way the ReplacingMergeTree source can. Defaults to `market_bars`.
    let bars_source =
        std::env::var("BARS_V2_BACKFILL_SOURCE").unwrap_or_else(|_| "market_bars".to_string());
    let bars_report =
        storage::clickhouse::backfill::backfill_market_bars_v2(&cfg.clickhouse.url, &bars_source)
            .await
            .context("failed to backfill market_bars_v2")?;
    if bars_report.did_work() {
        info!(
            rows = bars_report.rows_copied,
            chunks = bars_report.chunks_copied,
            source = %bars_source,
            "market_bars_v2 backfill applied"
        );
    }

    // Surrogate identity for every bar writer, then the one-time move of v2 into the
    // canonical four-timestamp table (clickhouse/07, ADR-P0-16). Chunked and verified;
    // a no-op once done.
    let identity = storage::identity::PgIdentityService::new(pg.clone(), cfg.clickhouse.url.clone());
    identity.sync_dims().await.context("failed to sync identity dimensions")?;
    storage::identity::install(identity);
    let canonical = storage::clickhouse::canonical::migrate_to_canonical(&cfg.clickhouse.url, &pg)
        .await
        .context("failed to migrate bars into market_bar")?;
    if canonical.chunks_copied > 0 {
        info!(
            rows = canonical.rows_copied,
            chunks = canonical.chunks_copied,
            "market_bars_v2 migrated into canonical market_bar"
        );
    }

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

    // The live frame bus. Created before any producer so every stage can
    // publish into it; the API's AppState holds the same sender, and each
    // `/ws/live` socket subscribes to it.
    let live_bus = api::live_bus::channel();

    // Build demand manager and UI gateway.
    let demand_registry = Arc::new(demand_manager::DemandRegistry::new(Arc::new(
        demand_manager::NoopPipelineFactory,
    )));
    let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(demand_registry));

    // -- In-process hot-path pipeline --
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

    // -- Continuous live-data pipelines --
    //
    // The single writer of live 1-minute bars to ClickHouse.  Every initialized
    // asset's pipeline aggregates trades into 1m OHLCV and sends completed bars
    // here, so minute-level history accrues for as long as the platform runs --
    // independent of whether any strategy or automation is subscribed.
    let (bar_tx, bar_rx) = tokio::sync::mpsc::unbounded_channel::<bar_persist::PersistBar>();
    tokio::spawn(bar_persist::run_bar_persist(
        cfg.clickhouse.url.clone(),
        bar_rx,
        live_bus.clone(),
    ));

    // Owns one in-process pipeline per initialized instrument.
    let pipeline_manager = Arc::new(pipeline_manager::PipelineManager::new(
        live_bus.clone(),
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
    // L2 depth feeds. Separate from the trade pipelines because a book is a
    // different subscription with a different shape and a different cadence —
    // and because an instrument can have trades without a book adapter.
    let book_feeds = Arc::new(book_feed::BookFeeds::new(live_bus.clone()));

    for (symbol, asset_class) in &initialized {
        pipeline_manager.ensure(symbol, asset_class);
        book_feeds.ensure(symbol, asset_class);
    }
    // Keep BTC-USD streaming by default (the bundled in-process crypto feed)
    // even before it is formally initialized.
    pipeline_manager.ensure("BTC-USD", "crypto_spot_cex");
    book_feeds.ensure("BTC-USD", "crypto_spot_cex");
    info!(
        pipelines = pipeline_manager.active_count(),
        books = book_feeds.active_count(),
        "live aggregation pipelines and depth feeds running"
    );

    // Start pipelines on demand when new assets are initialized (no restart).
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<api::StreamRequest>();
    {
        let mgr = Arc::clone(&pipeline_manager);
        let books = Arc::clone(&book_feeds);
        tokio::spawn(async move {
            while let Some(req) = stream_rx.recv().await {
                mgr.ensure(&req.instrument_id, &req.asset_class);
                books.ensure(&req.instrument_id, &req.asset_class);
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

    // AI Model Studio orchestrator — owns model identity, training/eval jobs,
    // alias management, and drives async job execution.
    let model_manager = model_registry::ModelManager::from_env(pg.clone());
    info!("model registry initialised");

    // Inference gateway — alias resolution, prediction caching, circuit breaking.
    let sidecar = Arc::new(model_registry::sidecar::SidecarClient::from_env());
    let inference_gateway = model_registry::InferenceGateway::new(pg.clone(), sidecar);
    info!("inference gateway initialised");

    // Best-effort NATS progress bridge: drives ModelManager job state from
    // training/eval progress frames published by the Python trainer sidecar.
    {
        let mm = Arc::clone(&model_manager);
        let nats_url = cfg.nats.url.clone();
        tokio::spawn(async move {
            match async_nats::connect(&nats_url).await {
                Ok(client) => {
                    info!("model NATS progress bridge connected");
                    model_registry::nats_bridge::run(client, mm).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "model NATS progress bridge disabled");
                }
            }
        });
    }

    // Internal agent orchestrator — LLM-driven strategy design + backtest
    // loop. Its tool calls go through this platform's own HTTP API on
    // loopback with a run-scoped service token (same path as the MCP server).
    let agent_manager = Arc::new(api::agent::AgentManager::new(
        pg.clone(),
        format!("http://127.0.0.1:{}", cfg.api.port),
        cfg.agent.clone(),
    ));
    agent_manager.recover_orphans().await;

    // Build the API router.
    let app_state = api::AppState::new(
        pg,
        risk_gate,
        kill_switch,
        execution_engine,
        Arc::clone(&paper_engine),
        gateway,
        backtest_manager,
        model_manager,
        inference_gateway,
        cfg.email.clone(),
        cfg.clickhouse.url.clone(),
        Some(stream_tx),
        agent_manager,
        Some(live_bus.clone()),
    );
    // ── Portfolio sampler + alert evaluator (migration 0042) ─────────────────
    //
    // The paper engine knows account equity at any instant, but nothing was
    // recording it over time — which is why the dashboard equity curve had no
    // series to draw. This samples every asset class on an interval and writes
    // one row per class, and trips any armed price alert the latest mark
    // satisfies. Running it here rather than in the browser is what makes an
    // alert fire with the tab closed.
    {
        let pg = app_state.pg.clone();
        let engine = app_state.paper_engine.clone();
        let interval_secs: u64 = std::env::var("EQUITY_SAMPLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(5)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if let Err(e) =
                    api::routes::portfolio::record_equity_snapshot(&pg, &engine, "paper").await
                {
                    warn!(error = %e, "equity snapshot write failed");
                }
                match api::routes::portfolio::evaluate_alerts(&pg, &engine).await {
                    Ok(n) if n > 0 => info!(fired = n, "price alerts triggered"),
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "alert evaluation failed"),
                }
            }
        });
        info!(interval_secs, "portfolio sampler started");
    }

    // ── Job service workers (Set L Phase 1, COMP-005 §7, §12) ────────────────
    //
    // One bounded pool per worker class, replacing the fixed 3-concurrent backtest
    // semaphore that nothing outside the backtest manager could see or tune (JB-05).
    // The lease reaper is what makes a job outlive the worker running it: a worker
    // that dies simply stops renewing, and the job returns to the queue (JB-04).
    if let Some(job_store) = app_state.jobs.clone() {
        tokio::spawn(jobs::run_lease_reaper(job_store.clone()));

        let pools: &[(jobs::WorkerClass, usize)] = &[
            (jobs::WorkerClass::Backtest, cfg.jobs.backtest_parallel),
            (jobs::WorkerClass::Research, cfg.jobs.research_parallel),
            (jobs::WorkerClass::Data, cfg.jobs.data_parallel),
            (jobs::WorkerClass::Eval, cfg.jobs.eval_parallel),
        ];

        for (class, parallel) in pools.iter().copied() {
            let mut pool = jobs::WorkerPool::new(job_store.clone(), class, parallel);
            // Register the workers this class knows how to run. Kinds with no
            // registered worker still queue; they fail fast with `no_worker` when
            // claimed, which is visible, rather than sitting queued forever, which
            // is not.
            if class == jobs::WorkerClass::Backtest {
                // The sixteen-gate stack's production caller (SPEC §12.3). It is
                // a counted kind: evaluating a candidate against the gates is a
                // look at its out-of-sample behaviour, and the counter climbs
                // for it like everything else (INV-1).
                pool = pool.register(std::sync::Arc::new(
                    api::gate_worker::GateAdvanceWorker::new(app_state.pg.clone()),
                ));
            }
            if class == jobs::WorkerClass::Data {
                pool = pool.register(std::sync::Arc::new(api::workers::DataQcWorker::new(
                    cfg.clickhouse.url.clone(),
                )));
            }
            if class == jobs::WorkerClass::Research {
                // The synthetic venue (DATA-005 §9). It is a research-class worker
                // rather than a data one because it produces instruments for
                // research to consume, and because the data pool is the one that
                // must stay free for live collection.
                pool = pool.register(std::sync::Arc::new(
                    api::synthetic_worker::SimulatePathsWorker::new(
                        cfg.clickhouse.url.clone(),
                        app_state.pg.clone(),
                    ),
                ));
                // The campaign driver (SPEC §10, ADR-P2-04). It spends almost
                // all of its time waiting on child jobs rather than computing,
                // so it sits in the research pool beside the other coordinators
                // instead of occupying a compute class.
                pool = pool.register(std::sync::Arc::new(api::campaign_driver::CampaignWorker::new(
                    job_store.clone(),
                    app_state.pg.clone(),
                )));
            }
            tokio::spawn(std::sync::Arc::new(pool).run_forever());
        }

        info!(
            backtest = cfg.jobs.backtest_parallel,
            research = cfg.jobs.research_parallel,
            data = cfg.jobs.data_parallel,
            eval = cfg.jobs.eval_parallel,
            "job worker pools started"
        );
    }

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
