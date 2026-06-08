//! Publisher for failed normalization events (quarantine lane).

use crate::lanes;
use crate::nats::BusError;
use base64::Engine as _;

/// Publishes failed-normalization events to the quarantine lane.
pub struct QuarantinePublisher {
    js: async_nats::jetstream::Context,
}

impl QuarantinePublisher {
    /// Wrap an existing JetStream context.
    pub fn new(js: async_nats::jetstream::Context) -> Self {
        Self { js }
    }

    /// Publish raw bytes that failed normalization to the quarantine lane.
    pub async fn publish_failure(
        &self,
        raw: &[u8],
        error: &domain::NormalizeError,
        source: &str,
    ) -> Result<(), BusError> {
        let raw_b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let payload = serde_json::json!({
            "source": source,
            "error": error.to_string(),
            "raw_b64": raw_b64,
        });

        let bytes = serde_json::to_vec(&payload).map_err(|e| BusError::Serialize(e.to_string()))?;

        self.js
            .publish(lanes::quarantine_subject(), bytes.into())
            .await
            .map_err(|e| BusError::Publish(e.to_string()))?;

        Ok(())
    }
}
