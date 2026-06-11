//! JetStream publisher.

use crate::lanes;
use crate::nats::BusError;

/// Publishes typed [`domain::EventEnvelope`]s to JetStream.
pub struct Publisher {
    js: async_nats::jetstream::Context,
}

impl Publisher {
    /// Wrap an existing JetStream context.
    pub fn new(js: async_nats::jetstream::Context) -> Self {
        Self { js }
    }

    /// Serialize and publish an envelope to the correct lane subject.
    pub async fn publish<T>(
        &self,
        envelope: &domain::EventEnvelope<T>,
        instrument_id: &str,
    ) -> Result<(), BusError>
    where
        T: domain::payloads::Payload + serde::Serialize,
    {
        let bytes = serde_json::to_vec(envelope).map_err(|e| BusError::Serialize(e.to_string()))?;

        let subject = lanes::subject_for(envelope.lane.as_str(), instrument_id);

        self.js
            .publish(subject, bytes.into())
            .await
            .map_err(|e| BusError::Publish(e.to_string()))?;

        Ok(())
    }

    /// Serialize and spawn a background task to publish — never blocks the caller.
    ///
    /// Used by the tee task so JetStream writes never stall the hot-path rings.
    /// Serialization errors are logged and dropped; publish errors are logged.
    pub fn publish_fire_and_forget<T>(
        &self,
        envelope: &domain::EventEnvelope<T>,
        instrument_id: &str,
    ) where
        T: domain::payloads::Payload + serde::Serialize + Clone + Send + 'static,
    {
        let bytes = match serde_json::to_vec(envelope) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "tee serialize failed — event dropped");
                return;
            }
        };
        let subject = lanes::subject_for(envelope.lane.as_str(), instrument_id);
        let js = self.js.clone();
        tokio::spawn(async move {
            if let Err(e) = js.publish(subject, bytes.into()).await {
                tracing::warn!(error = %e, "tee JetStream publish failed");
            }
        });
    }
}
