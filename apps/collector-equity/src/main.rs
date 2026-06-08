//! Equity data collector satellite.
//!
//! Connects to the Alpaca WS data feed and publishes equity trade events on the
//! same `market.trades` lane as the crypto collector. Downstream consumers
//! (storage writers, bar builder, feature engine) receive identical `EventEnvelope`
//! payloads regardless of asset class — no changes to core code were required.
//!
//! Usage: set `ALPACA_API_KEY_ID` and `ALPACA_API_SECRET_KEY`, then run with
//! the symbols to subscribe to as command-line arguments.
//!
//!   collector-equity AAPL SPY MSFT

use std::sync::Arc;

use anyhow::Context;
use collectors::{equity::alpaca_data::AlpacaDataCollector, Collector};
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-equity");
    } else {
        observability::init("collector-equity");
    }

    let symbols: Vec<String> = std::env::args().skip(1).collect();
    if symbols.is_empty() {
        eprintln!("Usage: collector-equity <SYMBOL> [<SYMBOL>...]");
        eprintln!("Example: collector-equity AAPL SPY");
        std::process::exit(1);
    }

    info!(?symbols, "starting equity data collector");

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;

    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    info!("NATS connected and streams provisioned");

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    let mut handles = Vec::new();
    for symbol in symbols {
        let pub_clone = publisher.clone();
        let q_clone = quarantine.clone();
        let collector = AlpacaDataCollector::new(symbol.clone());
        let handle = tokio::spawn(async move {
            if let Err(e) = collector.run(pub_clone, q_clone).await {
                error!(symbol = %symbol, error = %e, "collector exited");
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}
