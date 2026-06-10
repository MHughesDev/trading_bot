use std::sync::Arc;

use anyhow::Context;
use collectors::fx::oanda::OandaCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-fx");
    } else {
        observability::init("collector-fx");
    }

    info!(version = env!("CARGO_PKG_VERSION"), "collector-fx starting");

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    let collector = OandaCollector::new("EUR_USD");
    info!(pair = "EUR_USD", "starting OANDA FX collector");

    if let Err(e) = collector.run(publisher, quarantine).await {
        error!(error = %e, "OANDA collector exited with error");
    }

    Ok(())
}
