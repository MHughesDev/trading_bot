#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod hot_path;
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

    // Load kill switch state from Postgres (trading_enabled column).
    let initially_halted = load_kill_switch_state(&pg).await;
    let kill_switch = Arc::new(risk::KillSwitch::new(initially_halted));

    // Build risk gate with default limits.
    let risk_gate = Arc::new(risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        Arc::clone(&kill_switch),
    ));

    // Build in-house paper execution engine — no external broker API credentials needed.
    // Live broker adapters are loaded per-user from the database credential store when
    // the user has deposited live trading credentials; the default path is always paper.
    let (paper_broker, mark_price) = execution::paper::PaperBroker::new(AssetClass::CryptoSpotCex);
    let execution_engine = Arc::new(execution::ExecutionEngine::new(Arc::new(paper_broker)));
    info!("in-house paper execution engine initialised (all asset classes covered)");

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

    // Spawn the Kraken in-process pipeline for BTC/USD.
    let _pipeline = hot_path::spawn_pipeline(
        "BTC/USD".to_owned(),
        "BTC-USD".to_owned(),
        tee_tx,
        Arc::clone(&execution_engine),
        Arc::clone(&risk_gate),
        mark_price,
    );
    info!("hot-path pipeline (BTC/USD) started");

    // Build the API router.
    let app_state = api::AppState::new(pg, risk_gate, kill_switch, execution_engine, gateway);
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

