//! Venue connectors — normalize raw WS messages into typed `EventEnvelope`s.
//!
//! Each venue implementation lives in its own sub-module and is deliberately
//! different to prove that the `Collector` abstraction is venue-agnostic.

pub mod crypto;
pub mod equity;
pub mod gap;
pub mod normalizer;
pub mod reconnect;

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

/// Errors that a collector can return.
#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("connect error: {0}")]
    Connect(String),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("fatal error: {0}")]
    Fatal(String),
}

/// A running venue connector.
///
/// Implementations connect to the upstream venue, normalize events, and
/// publish them through [`event_bus::Publisher`].  Any events that cannot be
/// normalized are sent to the quarantine lane via [`event_bus::QuarantinePublisher`].
#[async_trait]
pub trait Collector: Send + Sync {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError>;
}
