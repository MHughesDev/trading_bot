//! Shared normalization helpers.
//!
//! The entry point is [`quarantine_or_publish`], which routes a normalization
//! result to either the main bus (on success) or the quarantine lane (on failure).

use std::sync::Arc;

use tracing::{error, warn};

/// Route a normalization result to the correct downstream.
///
/// * `Ok(envelope)` — publish to the instrument lane via `publisher`.
/// * `Err(e)` — publish the raw bytes to the quarantine lane via `quarantine`.
///
/// When the main bus publish fails (e.g. NATS reconnect window), the envelope
/// is routed to the quarantine lane so no event is silently lost (H-7).
pub async fn quarantine_or_publish<T>(
    result: Result<domain::EventEnvelope<T>, domain::NormalizeError>,
    raw: &[u8],
    instrument_id: &str,
    source: &str,
    publisher: &Arc<event_bus::Publisher>,
    quarantine: &Arc<event_bus::QuarantinePublisher>,
) where
    T: domain::payloads::Payload + serde::Serialize + Send + Sync,
{
    match result {
        Ok(envelope) => {
            if let Err(e) = publisher.publish(&envelope, instrument_id).await {
                warn!(instrument_id, source, error = %e, "main publish failed — routing to quarantine");
                let publish_error = domain::NormalizeError::Deserialize(e.to_string());
                if let Err(qe) = quarantine
                    .publish_failure(raw, &publish_error, source)
                    .await
                {
                    error!(
                        instrument_id,
                        source,
                        error = %qe,
                        "quarantine publish also failed — event lost"
                    );
                }
            }
        }
        Err(e) => {
            if let Err(qe) = quarantine.publish_failure(raw, &e, source).await {
                warn!(source, error = %qe, "failed to publish to quarantine");
            }
        }
    }
}
