//! NATS.ws configuration and subject allow-list for browser subscriptions (P2-T03).
//!
//! The browser subscribes to OHLCV directly over the NATS.ws port (default 9222).
//! A browser token may only subscribe to subjects matching the allow-list prefix
//! `md.>` (all market-data subjects).  Order/position/strategy subjects are
//! blocked at the NATS authorization layer.
//!
//! # Subject scheme
//! `md.<data_type_key>.<venue_slug>.<asset_class_key>.<instrument_id>`
//!
//! Example: `md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD`
//!
//! Use [`event_bus::subjects::ohlcv_subject`] to construct subjects.
//!
//! # Configuration
//! Set `NATS_WS_PORT` (default 9222) and `NATS_WS_AUTH_TOKEN` in environment or
//! platform config.  The NATS server must have `websocket { port: 9222 }` and a
//! user entry with `subscribe = ["md.>"]` permissions.

/// Default NATS.ws port exposed to browsers.
pub const DEFAULT_NATS_WS_PORT: u16 = 9222;

/// NATS subject prefix for all market-data browser feeds.
pub const MARKET_DATA_SUBJECT_PREFIX: &str = "md.>";

/// NATS.ws connection parameters surfaced to the browser client.
#[derive(Debug, Clone)]
pub struct NatsWsConfig {
    /// Hostname or IP the browser connects to (e.g. `"localhost"` in dev,
    /// CDN/LB hostname in production).
    pub host: String,
    /// NATS.ws port.
    pub port: u16,
    /// Optional auth token the browser includes in the CONNECT frame.
    pub auth_token: Option<String>,
}

impl NatsWsConfig {
    pub fn new(host: impl Into<String>, port: u16, auth_token: Option<String>) -> Self {
        Self {
            host: host.into(),
            port,
            auth_token,
        }
    }

    /// Build from environment variables (`NATS_WS_HOST`, `NATS_WS_PORT`, `NATS_WS_TOKEN`).
    pub fn from_env() -> Self {
        let host = std::env::var("NATS_WS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("NATS_WS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_NATS_WS_PORT);
        let auth_token = std::env::var("NATS_WS_TOKEN").ok();
        Self::new(host, port, auth_token)
    }

    /// WebSocket URL for the browser (e.g. `"ws://localhost:9222"`).
    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}", self.host, self.port)
    }
}

/// Verify that a subject requested by the browser is within the allowed market-data
/// namespace.  Returns `true` if the subject matches `md.*` (not private lanes).
pub fn is_allowed_subject(subject: &str) -> bool {
    subject.starts_with("md.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_data_subject_is_allowed() {
        assert!(is_allowed_subject(
            "md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD"
        ));
        assert!(is_allowed_subject(
            "md.market.funding_rate.kraken.perpetual_swap.BTC-PERP"
        ));
    }

    #[test]
    fn private_subjects_are_blocked() {
        assert!(!is_allowed_subject("orders.user123"));
        assert!(!is_allowed_subject("positions.user123"));
        assert!(!is_allowed_subject("strategy.user123.my-strat"));
    }

    #[test]
    fn config_ws_url() {
        let cfg = NatsWsConfig::new("localhost", 9222, None);
        assert_eq!(cfg.ws_url(), "ws://localhost:9222");
    }
}
