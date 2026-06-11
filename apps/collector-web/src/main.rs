//! Web scraper satellite — fetches configured URLs, honours robots.txt,
//! enforces per-domain rate limits, and emits `web.page_snapshot` events.
//!
//! Configuration (env vars):
//!   WEB_SCRAPER_URLS         — comma-separated list of URLs to scrape
//!   WEB_SCRAPER_RATE_SECS    — minimum seconds between requests per domain (default: 2)
//!   WEB_SCRAPER_POLL_SECS    — seconds between full scrape passes (default: 300)

#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use collectors::{
    web::scraper::{WebScraper, WebScraperConfig},
    Collector,
};
use event_bus::{connect, setup_streams, Publisher, QuarantinePublisher};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("collector-web");
    } else {
        observability::init("collector-web");
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "collector-web starting"
    );

    let urls: Vec<String> = std::env::var("WEB_SCRAPER_URLS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_owned())
        .collect();

    if urls.is_empty() {
        info!("WEB_SCRAPER_URLS not configured — scraper will idle until URLs are set");
    }

    let rate_secs: u64 = std::env::var("WEB_SCRAPER_RATE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);

    let poll_secs: u64 = std::env::var("WEB_SCRAPER_POLL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);

    let scraper_cfg = WebScraperConfig {
        urls,
        rate_limit: Duration::from_secs(rate_secs),
        poll_interval: Duration::from_secs(poll_secs),
    };

    let nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    let publisher = Arc::new(Publisher::new(nats.js.clone()));
    let quarantine = Arc::new(QuarantinePublisher::new(nats.js.clone()));

    let scraper = WebScraper::new(scraper_cfg);
    info!("starting web scraper");

    if let Err(e) = scraper.run(publisher, quarantine).await {
        error!(error = %e, "web scraper exited with error");
    }

    Ok(())
}
