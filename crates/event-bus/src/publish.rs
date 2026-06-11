//! JetStream publisher.
//!
// serde_json is intentionally absent from the hot-path serialization in this
// crate — EventEnvelope is encoded with rkyv for all market-data lanes.
// serde_json may only appear in API handlers and diagnostic tooling.

use crate::lanes;
use crate::nats::BusError;

/// Publishes [`domain::EventEnvelope`]s to JetStream using rkyv binary encoding.
pub struct Publisher {
    js: async_nats::jetstream::Context,
}

impl Publisher {
    /// Wrap an existing JetStream context.
    pub fn new(js: async_nats::jetstream::Context) -> Self {
        Self { js }
    }

    /// Serialize and publish an envelope to the correct lane subject.
    ///
    /// `instrument_name` is the human-readable instrument string (e.g. `"BTC-USD"`)
    /// used to build the NATS subject; `lane` is the lane name (e.g. `"market.trades"`).
    pub async fn publish(
        &self,
        envelope: &domain::EventEnvelope,
        instrument_name: &str,
        lane: &str,
    ) -> Result<(), BusError> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(envelope)
            .map_err(|e| BusError::Serialize(e.to_string()))?
            .into_vec();

        let subject = lanes::subject_for(lane, instrument_name);

        self.js
            .publish(subject, bytes.into())
            .await
            .map_err(|e| BusError::Publish(e.to_string()))?;

        Ok(())
    }

    /// Serialize and spawn a background task to publish — never blocks the caller.
    ///
    /// Used by the tee task so JetStream writes never stall the hot-path rings.
    /// Serialize errors are logged and dropped; publish errors are logged.
    pub fn publish_fire_and_forget(
        &self,
        envelope: &domain::EventEnvelope,
        instrument_name: &str,
        lane: &str,
    ) {
        let bytes = match rkyv::to_bytes::<rkyv::rancor::Error>(envelope) {
            Ok(b) => b.into_vec(),
            Err(e) => {
                tracing::warn!(error = %e, "tee serialize failed — event dropped");
                return;
            }
        };
        let subject = lanes::subject_for(lane, instrument_name);
        let js = self.js.clone();
        tokio::spawn(async move {
            if let Err(e) = js.publish(subject, bytes.into()).await {
                tracing::warn!(error = %e, "tee JetStream publish failed");
            }
        });
    }
}
