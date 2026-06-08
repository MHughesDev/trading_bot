use std::sync::Arc;

use anyhow::Context;
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

    // Build execution engine with the Alpaca paper broker.
    let broker: Arc<dyn execution::broker::Broker> =
        match execution::alpaca::AlpacaBroker::from_env() {
            Ok(b) => {
                info!("Alpaca paper broker configured from environment");
                Arc::new(b)
            }
            Err(_) => {
                tracing::warn!(
                    "ALPACA_API_KEY_ID / ALPACA_API_SECRET_KEY not set — using no-op broker"
                );
                Arc::new(NoBroker)
            }
        };
    let execution_engine = Arc::new(execution::ExecutionEngine::new(broker));

    // Build demand manager and UI gateway.
    let demand_registry = Arc::new(demand_manager::DemandRegistry::new(Arc::new(
        demand_manager::NoopPipelineFactory,
    )));
    let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(demand_registry));

    // Build the API router.
    let app_state = api::AppState::new(pg, risk_gate, kill_switch, execution_engine, gateway);
    let router = api::router(app_state);

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

/// Placeholder broker used when Alpaca credentials are unavailable.
struct NoBroker;

#[async_trait::async_trait]
impl execution::broker::Broker for NoBroker {
    async fn submit(
        &self,
        _order: &risk::ApprovedOrder,
    ) -> Result<String, execution::broker::BrokerError> {
        Err(execution::broker::BrokerError::Rejected(
            "no broker configured".to_owned(),
        ))
    }

    async fn cancel(&self, _broker_order_id: &str) -> Result<(), execution::broker::BrokerError> {
        Ok(())
    }

    async fn query_order(
        &self,
        broker_order_id: &str,
    ) -> Result<execution::broker::BrokerOrderStatus, execution::broker::BrokerError> {
        Err(execution::broker::BrokerError::OrderNotFound(
            broker_order_id.to_owned(),
        ))
    }

    async fn query_open_orders(
        &self,
    ) -> Result<Vec<execution::broker::BrokerOrderStatus>, execution::broker::BrokerError> {
        Ok(vec![])
    }

    async fn query_positions(
        &self,
    ) -> Result<Vec<execution::broker::BrokerPosition>, execution::broker::BrokerError> {
        Ok(vec![])
    }
}
