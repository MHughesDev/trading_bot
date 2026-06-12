//! NATS connection helpers and JetStream stream setup.

use async_nats::jetstream::{self, stream};
use domain::lanes::{
    FEATURES_TECHNICAL, MARKET_BARS_1M, MARKET_BARS_1M_REVISED, MARKET_BARS_1S,
    MARKET_ORDERBOOK_L2, MARKET_QUOTES, MARKET_TRADES, ORDERS_COMMANDS, ORDERS_EVENTS,
    POSITIONS_EVENTS, QUARANTINE, STRATEGY_SIGNALS,
};
use thiserror::Error;
use tracing::{info, warn};

/// All errors produced by the event-bus layer.
#[derive(Debug, Error)]
pub enum BusError {
    #[error("NATS connect error: {0}")]
    Connect(String),
    #[error("NATS publish error: {0}")]
    Publish(String),
    #[error("JetStream error: {0}")]
    JetStream(String),
    #[error("serialize error: {0}")]
    Serialize(String),
}

/// A connected NATS client together with its JetStream context.
pub struct NatsClient {
    pub client: async_nats::Client,
    pub js: async_nats::jetstream::Context,
}

/// Connect to a NATS server and return a [`NatsClient`].
pub async fn connect(url: &str) -> Result<NatsClient, BusError> {
    let client = async_nats::connect(url)
        .await
        .map_err(|e| BusError::Connect(e.to_string()))?;
    let js = jetstream::new(client.clone());
    Ok(NatsClient { client, js })
}

/// Create JetStream streams for every lane.
///
/// If a stream already exists the function logs and skips it — idempotent.
pub async fn setup_streams(js: &async_nats::jetstream::Context) -> Result<(), BusError> {
    let lanes = [
        MARKET_TRADES,
        MARKET_QUOTES,
        MARKET_ORDERBOOK_L2,
        MARKET_BARS_1S,
        MARKET_BARS_1M,
        MARKET_BARS_1M_REVISED,
        FEATURES_TECHNICAL,
        STRATEGY_SIGNALS,
        ORDERS_COMMANDS,
        ORDERS_EVENTS,
        POSITIONS_EVENTS,
        QUARANTINE,
    ];

    for lane in &lanes {
        let stream_name = lane.replace('.', "-");
        let subject = if *lane == QUARANTINE {
            QUARANTINE.to_owned()
        } else {
            format!("{lane}.>")
        };

        // Use Limits retention so every durable consumer sees every message
        // (fan-out). WorkQueue deletes a message once any one consumer acks it,
        // which silently starves all other consumers on the same lane.
        // ORDERS_COMMANDS is intentionally handled by a single executor, but
        // even there Limits+durable-consumer is safer than WorkQueue.
        let stream_config = stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject],
            num_replicas: 1,
            retention: stream::RetentionPolicy::Limits,
            ..Default::default()
        };

        match js.create_stream(stream_config).await {
            Ok(_) => {
                info!(stream = %stream_name, "JetStream stream created");
            }
            Err(e) => {
                // Stream already exists or other non-fatal error — log and continue.
                warn!(stream = %stream_name, error = %e, "stream already exists or could not be created, skipping");
            }
        }
    }

    Ok(())
}
