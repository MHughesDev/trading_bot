//! Kraken REST `AccountSource` adapter.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

use domain::money::{Price, Size};

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

pub struct KrakenAccountSource {
    client: Client,
}

impl Default for KrakenAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl KrakenAccountSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn base_url() -> &'static str {
        "https://api.kraken.com"
    }

    fn parse_key(creds: &VenueCredentials) -> Result<(String, String), AccountSourceError> {
        let text = String::from_utf8(creds.plaintext.clone())?;
        let parts: Vec<&str> = text.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(AccountSourceError::Credentials(
                "expected api_key:api_secret".to_owned(),
            ));
        }
        Ok((parts[0].to_owned(), parts[1].to_owned()))
    }
}

#[async_trait]
impl AccountSource for KrakenAccountSource {
    fn venue_id(&self) -> &str {
        "kraken"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let (api_key, _api_secret) = Self::parse_key(creds)?;

        let resp = self
            .client
            .post(format!("{}/0/private/Balance", Self::base_url()))
            .header("API-Key", &api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        if let Some(errors) = parsed["error"].as_array() {
            if !errors.is_empty() {
                let msg = errors
                    .iter()
                    .filter_map(|e| e.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(AccountSourceError::Credentials(msg));
            }
        }

        let result = parsed["result"].as_object().cloned().unwrap_or_default();
        Ok(result
            .into_iter()
            .filter_map(|(asset, val)| {
                let amount = Decimal::from_str(val.as_str()?).ok()?;
                Some(Balance {
                    asset,
                    available: Size::from_decimal(amount),
                    locked: Size::from_decimal(Decimal::ZERO),
                    usd_value: None,
                })
            })
            .collect())
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let (api_key, _) = Self::parse_key(creds)?;

        let resp = self
            .client
            .post(format!("{}/0/private/OpenPositions", Self::base_url()))
            .header("API-Key", &api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let result = parsed["result"].as_object().cloned().unwrap_or_default();
        Ok(result
            .into_iter()
            .filter_map(|(_, pos)| {
                let pair = pos["pair"].as_str()?.to_owned();
                let vol = Decimal::from_str(pos["vol"].as_str()?).ok()?;
                let cost = Decimal::from_str(pos["cost"].as_str()?).ok()?;
                let avg_price = if vol.is_zero() {
                    Decimal::ZERO
                } else {
                    cost / vol
                };
                Some(VenuePosition {
                    instrument_id: pair,
                    quantity: vol,
                    avg_entry_price: Price::from_decimal(avg_price),
                    unrealized_pnl_usd: None,
                })
            })
            .collect())
    }

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        _user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        let (api_key, _) = Self::parse_key(creds)?;

        let mut params = vec![];
        if let Some(s) = since {
            params.push(("start", s.timestamp().to_string()));
        }

        let resp = self
            .client
            .post(format!("{}/0/private/TradesHistory", Self::base_url()))
            .header("API-Key", &api_key)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let trades = parsed["result"]["trades"]
            .as_object()
            .cloned()
            .unwrap_or_default();

        Ok(trades
            .into_iter()
            .filter_map(|(id, t)| {
                let pair = t["pair"].as_str()?.to_owned();
                let vol = Decimal::from_str(t["vol"].as_str()?).ok()?;
                let ts = t["time"].as_f64()? as i64;
                let occurred_at = DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now);
                Some(VenueTransaction {
                    id,
                    transaction_type: "fill".to_owned(),
                    instrument_id: Some(pair),
                    amount: vol,
                    currency: "USD".to_owned(),
                    occurred_at,
                })
            })
            .collect())
    }
}
