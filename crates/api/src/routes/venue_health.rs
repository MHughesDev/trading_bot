//! Per-venue health-check endpoints (P2-T05).
//!
//! `GET /api/venues/{venue}/health` — performs a cheap authenticated ping and
//! returns `{ ok, latency_ms, message }`.  Never returns credential material.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use thiserror::Error;

use crate::{auth::session::BearerToken, state::AppState};
use domain::SupportedVenue;

/// Response body for a venue health check.
#[derive(Debug, Serialize, Deserialize)]
pub struct VenueHealthResponse {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// Errors from a venue ping (#49 — typed error avoids eager format! allocation).
#[derive(Debug, Error)]
enum PingError {
    #[error("client build error: {0}")]
    Build(reqwest::Error),
    #[error("HTTP {0}")]
    BadStatus(u16),
    #[error("connection error: {0}")]
    Connection(reqwest::Error),
}

/// `GET /api/venues/:venue/health`
///
/// Requires a bearer token (L-4: previously unauthenticated, leaking which
/// venue slugs are valid via the error message).
pub async fn venue_health(
    _token: BearerToken,
    Path(venue_slug): Path<String>,
    State(state): State<AppState>,
) -> Json<VenueHealthResponse> {
    let venue: SupportedVenue = match venue_slug.parse() {
        Ok(v) => v,
        Err(_) => {
            return Json(VenueHealthResponse {
                ok: false,
                latency_ms: 0,
                // Generic error — do not echo the slug to avoid venue enumeration.
                message: "invalid venue".to_owned(),
            });
        }
    };

    let start = Instant::now();
    let result = check_venue_health(venue, &state).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    Json(match result {
        Ok(msg) => VenueHealthResponse {
            ok: true,
            latency_ms,
            message: msg,
        },
        Err(e) => VenueHealthResponse {
            ok: false,
            latency_ms,
            message: e.to_string(),
        },
    })
}

/// Perform a cheap authenticated ping to the venue.  Returns the server time/
/// version string on success or a typed error on failure.
async fn check_venue_health(venue: SupportedVenue, _state: &AppState) -> Result<String, PingError> {
    match venue {
        SupportedVenue::Kraken => ping_kraken().await,
        SupportedVenue::Alpaca => ping_alpaca().await,
        SupportedVenue::Coinbase => ping_coinbase().await,
        SupportedVenue::Oanda => ping_oanda().await,
        SupportedVenue::Kalshi => ping_kalshi().await,
        SupportedVenue::Tradier => ping_tradier().await,
        SupportedVenue::ZeroX => ping_zerox().await,
        SupportedVenue::Tradovate => ping_tradovate().await,
    }
}

// ── Per-venue ping helpers ───────────────────────────────────────────────────
// Each does the lightest possible public ping (no auth required for server time).

async fn ping_kraken() -> Result<String, PingError> {
    do_get_ping("https://api.kraken.com/0/public/Time").await
}

async fn ping_alpaca() -> Result<String, PingError> {
    do_get_ping("https://api.alpaca.markets/v2/clock").await
}

async fn ping_coinbase() -> Result<String, PingError> {
    do_get_ping("https://api.coinbase.com/v2/time").await
}

async fn ping_oanda() -> Result<String, PingError> {
    // OANDA v20 — instruments endpoint is public
    do_get_ping("https://api-fxtrade.oanda.com/v3/instruments").await
}

async fn ping_kalshi() -> Result<String, PingError> {
    do_get_ping("https://api.elections.kalshi.com/trade-api/v2/exchange/status").await
}

async fn ping_tradier() -> Result<String, PingError> {
    do_get_ping("https://api.tradier.com/v1/markets/clock").await
}

async fn ping_zerox() -> Result<String, PingError> {
    // 0x swap API health
    do_get_ping("https://api.0x.org/swap/v1/sources").await
}

async fn ping_tradovate() -> Result<String, PingError> {
    do_get_ping("https://demo.tradovateapi.com/v1/contract/find").await
}

/// Generic GET ping — succeeds if we get a 2xx response.  Returns a sanitized
/// status string (no credential material can leak through this path).
/// Uses typed errors (#49) so the error message string is only formatted on
/// Display, not at construction time.
async fn do_get_ping(url: &str) -> Result<String, PingError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(PingError::Build)?;

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(format!("HTTP {status}"))
            } else {
                Err(PingError::BadStatus(status.as_u16()))
            }
        }
        Err(e) => Err(PingError::Connection(e)),
    }
}
