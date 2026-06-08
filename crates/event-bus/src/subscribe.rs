//! JetStream subscriber helpers.

use crate::lanes;
use crate::nats::BusError;
use async_nats::jetstream::consumer::push::OrderedConfig;
use async_nats::jetstream::consumer::Consumer;

/// Subscribes to JetStream lanes.
pub struct Subscriber {
    js: async_nats::jetstream::Context,
}

impl Subscriber {
    /// Wrap an existing JetStream context.
    pub fn new(js: async_nats::jetstream::Context) -> Self {
        Self { js }
    }

    /// Subscribe to `lane.instrument_id` using an ordered push consumer.
    ///
    /// The `consumer_name` is used as the name for the consumer.
    pub async fn subscribe(
        &self,
        lane: &str,
        instrument_id: &str,
        consumer_name: &str,
    ) -> Result<Consumer<OrderedConfig>, BusError> {
        let subject = lanes::subject_for(lane, instrument_id);
        let stream_name = lane.replace('.', "-");

        let stream = self
            .js
            .get_stream(&stream_name)
            .await
            .map_err(|e| BusError::JetStream(e.to_string()))?;

        let consumer = stream
            .create_consumer(OrderedConfig {
                filter_subject: subject,
                name: Some(consumer_name.to_owned()),
                ..Default::default()
            })
            .await
            .map_err(|e| BusError::JetStream(e.to_string()))?;

        Ok(consumer)
    }
}
