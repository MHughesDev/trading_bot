#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


use std::sync::Arc;

use anyhow::Context;
use collectors::dex::zerox::ZeroXCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-dex");
    } else {
        observability::init("collector-dex");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-dex starting"
    );

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    // WETH → USDC on Ethereum mainnet; 1 WETH = 1e18 wei.
    let collector = ZeroXCollector::new("WETH", "USDC", "1000000000000000000");
    info!(pair = "WETH-USDC", "starting 0x DEX quote collector");

    if let Err(e) = collector.run(publisher, quarantine).await {
        error!(error = %e, "0x collector exited with error");
    }

    Ok(())
}
