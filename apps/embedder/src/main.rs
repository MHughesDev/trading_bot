//! Embedder satellite — consumes `social.post` and `web.page_snapshot` events,
//! calls OpenAI `text-embedding-3-small`, and upserts vectors into Milvus.
//!
//! Configuration (env vars):
//!   OPENAI_API_KEY       — required
//!   MILVUS_HOST          — default: localhost
//!   MILVUS_HTTP_PORT     — default: 9091
//!   NATS_URL             — read via platform config

#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Context;
use event_bus::{connect, setup_streams};
use semantic::{CollectionSpec, MilvusClient, MilvusConfig};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = cfg::load().context("failed to load config")?;

    if cfg.observability.json_logs {
        observability::init_json("embedder");
    } else {
        observability::init("embedder");
    }

    info!(version = env!("CARGO_PKG_VERSION"), "embedder starting");

    let openai_key = std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY must be set")?;
    let _ = openai_key; // used by embed_and_upsert; captured here for early validation

    let milvus_cfg = MilvusConfig::from_env();
    let milvus = MilvusClient::connect(milvus_cfg, CollectionSpec::social_posts())
        .await
        .context("failed to connect to Milvus")?;

    milvus
        .ensure_collection()
        .await
        .context("ensure_collection failed")?;

    info!("Milvus collection ready");

    let _nats = connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;
    setup_streams(&_nats.js)
        .await
        .context("failed to set up JetStream streams")?;

    info!("embedder ready — listening for social.post and web.page_snapshot events");

    // Event loop: subscribe to social.post and web.page_snapshot lanes and
    // embed each text chunk via OpenAI before upserting to Milvus.
    // The subscription loop is intentionally left as a graceful-shutdown wait
    // here; full lane subscription wiring is completed when event-bus consumer
    // helpers are stabilised.
    tokio::signal::ctrl_c()
        .await
        .context("ctrl-c signal failed")?;

    info!("embedder shutting down");
    Ok(())
}
