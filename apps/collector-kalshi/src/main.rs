#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use anyhow::Context;
use collectors::prediction::kalshi::KalshiCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-kalshi");
    } else {
        observability::init("collector-kalshi");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-kalshi starting"
    );

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    let collector = KalshiCollector::new_prediction("PRES-2024-D");
    info!(
        ticker = "PRES-2024-D",
        "starting Kalshi prediction collector"
    );

    if let Err(e) = collector.run(publisher, quarantine).await {
        error!(error = %e, "Kalshi collector exited with error");
    }

    Ok(())
}
