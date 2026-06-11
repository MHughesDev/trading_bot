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

    fn parse_creds(creds: &VenueCredentials) -> Result<(String, String), AccountSourceError> {
        let text = std::str::from_utf8(&creds.plaintext).map_err(|_| {
            AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned())
        })?;
        let mut parts = text.splitn(2, ':');
        let token = parts.next().unwrap_or("").to_owned();
        let account_id = parts
            .next()
            .ok_or_else(|| {
                AccountSourceError::Credentials("expected api_token:account_id".to_owned())
            })?
            .to_owned();
        Ok((token, account_id))
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
        Self::parse_creds(creds)?;
        Err(AccountSourceError::NotImplemented)
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        Self::parse_creds(creds)?;
        Err(AccountSourceError::NotImplemented)
    }

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        _user_id: uuid::Uuid,
        _since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        Self::parse_creds(creds)?;
        Err(AccountSourceError::NotImplemented)
    }
}
