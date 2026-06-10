//! Tradier REST `AccountSource` adapter (stub — full impl in Phase 4).

use async_trait::async_trait;
use reqwest::Client;

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

pub struct TradierAccountSource {
    _client: Client,
}

impl Default for TradierAccountSource {
    fn default() -> Self {
        Self {
            _client: Client::new(),
        }
    }
}

impl TradierAccountSource {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AccountSource for TradierAccountSource {
    fn venue_id(&self) -> &str {
        "tradier"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let raw = std::str::from_utf8(&creds.plaintext)
            .map_err(|e| AccountSourceError::Credentials(e.to_string()))?;
        let mut parts = raw.splitn(2, ':');
        let _token = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AccountSourceError::Credentials("missing access_token".to_owned()))?;
        let _account_id = parts.next().filter(|s| !s.is_empty()).ok_or_else(|| {
            AccountSourceError::Credentials(
                "missing account_id; expected token:account_id".to_owned(),
            )
        })?;
        Ok(vec![])
    }

    async fn fetch_positions(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        Ok(vec![])
    }

    async fn fetch_transactions(
        &self,
        _creds: &VenueCredentials,
        _user_id: uuid::Uuid,
        _since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        Ok(vec![])
    }
}
