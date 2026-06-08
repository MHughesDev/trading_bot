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
}
