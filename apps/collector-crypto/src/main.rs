#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use anyhow::Context;
use collectors::crypto::kraken::KrakenCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-crypto");
    } else {
        observability::init("collector-crypto");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-crypto starting"
    );

    // Connect to NATS and ensure all JetStream streams exist.
    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;

    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    info!("NATS connected and streams provisioned");

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    // Start the Kraken BTC/USD collector.
    let kraken = KrakenCollector::new("BTC/USD");
    info!(symbol = "BTC/USD", "starting Kraken collector");

    if let Err(e) = kraken.run(publisher, quarantine).await {
        error!(error = %e, "Kraken collector exited with error");
    }

    Ok(())
}
