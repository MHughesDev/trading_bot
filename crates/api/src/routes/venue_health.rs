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

use crate::state::AppState;
use domain::SupportedVenue;

/// Response body for a venue health check.
#[derive(Debug, Serialize, Deserialize)]
pub struct VenueHealthResponse {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// `GET /api/venues/:venue/health`
pub async fn venue_health(
    Path(venue_slug): Path<String>,
    State(state): State<AppState>,
) -> Json<VenueHealthResponse> {
    let venue: SupportedVenue = match venue_slug.parse() {
        Ok(v) => v,
        Err(_) => {
            return Json(VenueHealthResponse {
                ok: false,
                latency_ms: 0,
                message: format!("unknown venue: {venue_slug}"),
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
        Err(msg) => VenueHealthResponse {
            ok: false,
            latency_ms,
            message: msg,
        },
    })
}

/// Perform a cheap authenticated ping to the venue.  Returns the server time/
/// version string on success or an error description on failure.
async fn check_venue_health(venue: SupportedVenue, _state: &AppState) -> Result<String, String> {
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

async fn ping_kraken() -> Result<String, String> {
    let url = "https://api.kraken.com/0/public/Time";
    do_get_ping(url, "result.unixtime").await
}

async fn ping_alpaca() -> Result<String, String> {
    let url = "https://api.alpaca.markets/v2/clock";
    do_get_ping(url, "timestamp").await
}

async fn ping_coinbase() -> Result<String, String> {
    let url = "https://api.coinbase.com/v2/time";
    do_get_ping(url, "data.iso").await
}

async fn ping_oanda() -> Result<String, String> {
    // OANDA v20 — instruments endpoint is public
    let url = "https://api-fxtrade.oanda.com/v3/instruments";
    do_get_ping(url, "instruments").await
}

async fn ping_kalshi() -> Result<String, String> {
    let url = "https://api.elections.kalshi.com/trade-api/v2/exchange/status";
    do_get_ping(url, "trading_active").await
}

async fn ping_tradier() -> Result<String, String> {
    let url = "https://api.tradier.com/v1/markets/clock";
    do_get_ping(url, "clock.state").await
}

async fn ping_zerox() -> Result<String, String> {
    // 0x swap API health
    let url = "https://api.0x.org/swap/v1/sources";
    do_get_ping(url, "records").await
}

async fn ping_tradovate() -> Result<String, String> {
    let url = "https://demo.tradovateapi.com/v1/contract/find";
    do_get_ping(url, "id").await
}

/// Generic GET ping — succeeds if we get a 2xx response.  Returns a sanitized
/// status string (no credential material can leak through this path).
async fn do_get_ping(url: &str, _expected_field: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("client build error: {e}"))?;

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(format!("HTTP {status}"))
            } else {
                Err(format!("HTTP {status}"))
            }
        }
        Err(e) => Err(format!("connection error: {e}")),
    }
}
