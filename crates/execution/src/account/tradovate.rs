//! Tradovate REST `AccountSource` adapter (stub — full impl in Phase 4).

use async_trait::async_trait;
use reqwest::Client;

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

pub struct TradovateAccountSource {
    _client: Client,
}

impl Default for TradovateAccountSource {
    fn default() -> Self {
        Self {
            _client: Client::new(),
        }
    }
}

impl TradovateAccountSource {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AccountSource for TradovateAccountSource {
    fn venue_id(&self) -> &str {
        "tradovate"
    }

    async fn fetch_balances(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        Err(AccountSourceError::NotImplemented)
    }

    async fn fetch_positions(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        Err(AccountSourceError::NotImplemented)
    }

    async fn fetch_transactions(
        &self,
        _creds: &VenueCredentials,
        _user_id: uuid::Uuid,
        _since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        Err(AccountSourceError::NotImplemented)
    }
}
