//! P4-T07 acceptance tests — per-venue AccountSource adapters.
//!
//! Tests verify: adapter venue_id(), credential parse errors return typed errors,
//! and DTOs are correctly constructed.

use execution::{
    account::{
        AlpacaAccountSource, CoinbaseAccountSource, KalshiAccountSource, KrakenAccountSource,
        OandaAccountSource, TradierAccountSource, TradovateAccountSource,
    },
    account_source::{AccountSource, AccountSourceError, VenueCredentials},
};

fn creds(s: &str) -> VenueCredentials {
    VenueCredentials {
        venue: "test".to_owned(),
        plaintext: s.as_bytes().to_vec(),
    }
}

fn bad_creds() -> VenueCredentials {
    VenueCredentials {
        venue: "test".to_owned(),
        plaintext: vec![0xFE, 0xFF], // invalid UTF-8
    }
}

// ── venue_id() ───────────────────────────────────────────────────────────────

#[test]
fn coinbase_venue_id() {
    assert_eq!(CoinbaseAccountSource::new().venue_id(), "coinbase");
}
#[test]
fn kraken_venue_id() {
    assert_eq!(KrakenAccountSource::new().venue_id(), "kraken");
}
#[test]
fn alpaca_venue_id() {
    assert_eq!(AlpacaAccountSource::new(true).venue_id(), "alpaca");
}
#[test]
fn oanda_venue_id() {
    assert_eq!(OandaAccountSource::new().venue_id(), "oanda");
}
#[test]
fn kalshi_venue_id() {
    assert_eq!(KalshiAccountSource::new().venue_id(), "kalshi");
}
#[test]
fn tradier_venue_id() {
    assert_eq!(TradierAccountSource::new().venue_id(), "tradier");
}
#[test]
fn tradovate_venue_id() {
    assert_eq!(TradovateAccountSource::new().venue_id(), "tradovate");
}

// ── Credential parse errors — not panics ────────────────────────────────────

#[tokio::test]
async fn kraken_bad_utf8_creds_returns_credentials_error() {
    let src = KrakenAccountSource::new();
    let result = src.fetch_balances(&bad_creds()).await;
    assert!(matches!(result, Err(AccountSourceError::Credentials(_))));
}

#[tokio::test]
async fn alpaca_missing_separator_returns_credentials_error() {
    let src = AlpacaAccountSource::new(true);
    // Missing ':' separator — parse should fail with typed error.
    let result = src.fetch_balances(&creds("onlyone")).await;
    assert!(
        matches!(result, Err(AccountSourceError::Credentials(_))),
        "missing ':' must yield Credentials error"
    );
}

#[tokio::test]
async fn oanda_missing_separator_returns_credentials_error() {
    let src = OandaAccountSource::new();
    let result = src.fetch_balances(&creds("tokenonly")).await;
    assert!(matches!(result, Err(AccountSourceError::Credentials(_))));
}

#[tokio::test]
async fn tradier_missing_separator_returns_credentials_error() {
    let src = TradierAccountSource::new();
    let result = src.fetch_balances(&creds("tokenonly")).await;
    assert!(matches!(result, Err(AccountSourceError::Credentials(_))));
}

// ── AccountSourceError is not a panic ────────────────────────────────────────

#[test]
fn account_source_error_display_is_not_empty() {
    let e = AccountSourceError::Credentials("missing key".to_owned());
    assert!(!e.to_string().is_empty());
    let e2 = AccountSourceError::Http("503".to_owned());
    assert!(!e2.to_string().is_empty());
    let e3 = AccountSourceError::Parse("invalid json".to_owned());
    assert!(!e3.to_string().is_empty());
}
