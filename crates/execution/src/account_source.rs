//! `AccountSource` — per-user, per-venue balance/position/transaction fetch
//! abstraction (C-017 / C-092).
//!
//! Fires on-demand when the user navigates to the Dashboard — no polling loop.
//! Concrete REST adapters for each venue land in Phase 4.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

use domain::money::{Price, Size};

/// Opaque venue credential bundle passed to `AccountSource` methods.
/// Concrete contents are determined per-venue; the trait receives an opaque
/// reference so the adapter can cast internally.
pub struct VenueCredentials {
    pub venue: String,
    /// Raw credential bytes; decrypted by the credential service before being
    /// passed here.  Never logged.
    pub plaintext: Vec<u8>,
}

/// A single balance entry for one asset at a venue.
#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: String,
    pub available: Size,
    pub locked: Size,
    pub usd_value: Option<Price>,
}

/// A position held at a venue.
#[derive(Debug, Clone)]
pub struct VenuePosition {
    pub instrument_id: String,
    pub quantity: Decimal,
    pub avg_entry_price: Price,
    pub unrealized_pnl_usd: Option<Decimal>,
}

/// A historical transaction from the venue (fill, fee, funding, etc.).
#[derive(Debug, Clone)]
pub struct VenueTransaction {
    pub id: String,
    pub transaction_type: String,
    pub instrument_id: Option<String>,
    pub amount: Decimal,
    pub currency: &'static str,
    pub occurred_at: DateTime<Utc>,
}

/// Errors from account-source queries.
#[derive(Debug, Error)]
pub enum AccountSourceError {
    /// HTTP transport error (typed, formatted only on display).
    #[error("venue HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// HTTP error response body (plain text from the venue).
    #[error("venue HTTP error: {0}")]
    HttpStatus(String),
    /// Credential decode / validation error.
    #[error("credential error: {0}")]
    Credentials(String),
    /// Bytes-to-UTF-8 conversion of the credential blob failed.
    #[error("credential error: {0}")]
    CredentialEncoding(#[from] std::string::FromUtf8Error),
    /// JSON / decimal parse error (typed, formatted only on display).
    #[error("parse error: {0}")]
    ParseJson(#[from] serde_json::Error),
    /// Decimal parse error (typed, formatted only on display).
    #[error("parse error: {0}")]
    ParseDecimal(#[from] rust_decimal::Error),
    /// Legacy string parse error — kept for any callers that still use it.
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not implemented — full REST adapter deferred to Phase 4")]
    NotImplemented,
}

/// On-demand per-venue account data fetcher.
///
/// Implementations are per-venue (Coinbase, Alpaca, etc.) and land in Phase 4.
/// `VenueCredentials` are decrypted by the credential service before being
/// passed here — the adapter never decrypts credentials itself.
#[async_trait]
pub trait AccountSource: Send + Sync {
    fn venue_id(&self) -> &str;

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError>;

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError>;

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError>;
}
