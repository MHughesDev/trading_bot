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

    // Build the API router.
    let app_state = api::AppState::new(pg);
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
