#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use collectors::social::reddit::RedditCollector;
use collectors::Collector;
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-reddit");
    } else {
        observability::init("collector-reddit");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-reddit starting"
    );

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    // Seed known instruments for cashtag linking.
    let mut known = HashMap::new();
    for sym in ["BTC", "ETH", "SOL", "DOGE", "AAPL", "TSLA", "SPY", "QQQ"] {
        known.insert(sym.to_owned(), sym.to_owned());
    }

    let subreddit =
        std::env::var("REDDIT_SUBREDDIT").unwrap_or_else(|_| "CryptoCurrency".to_owned());

    let collector = RedditCollector::new(&subreddit, known);
    info!(%subreddit, "starting Reddit collector");

    if let Err(e) = collector.run(publisher, quarantine).await {
        error!(error = %e, "Reddit collector exited with error");
    }

    Ok(())
}
