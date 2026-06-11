#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


use std::sync::Arc;

use anyhow::Context;
use collectors::futures::tradovate::TradovateCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-futures");
    } else {
        observability::init("collector-futures");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-futures starting"
    );

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    let symbol = std::env::var("TRADOVATE_SYMBOL").unwrap_or_else(|_| "ESH4".to_owned());
    let collector = TradovateCollector::new(&symbol);
    info!(%symbol, "starting Tradovate futures collector");

    if let Err(e) = collector.run(publisher, quarantine).await {
        error!(error = %e, "Tradovate collector exited with error");
    }

    Ok(())
}
