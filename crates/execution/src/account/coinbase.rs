//! Coinbase Advanced Trade `AccountSource` adapter.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{header, Client};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use uuid::Uuid;

use domain::money::{Price, Size};

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

pub struct CoinbaseAccountSource {
    client: Client,
}

impl Default for CoinbaseAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl CoinbaseAccountSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn base_url() -> &'static str {
        "https://api.coinbase.com"
    }

    fn auth_headers(creds: &VenueCredentials) -> Result<header::HeaderMap, AccountSourceError> {
        let key = std::str::from_utf8(&creds.plaintext)
            .map_err(|_| AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned()))?;
        let mut headers = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&key) {
            headers.insert("CB-ACCESS-KEY", v);
        } else {
            return Err(AccountSourceError::Credentials("invalid key".to_owned()));
        }
        Ok(headers)
    }
}

#[derive(Debug, Deserialize)]
struct CoinbaseAccount {
    currency: String,
    available_balance: CoinbaseAmount,
    hold: CoinbaseAmount,
}

#[derive(Debug, Deserialize)]
struct CoinbaseAmount {
    value: String,
}

#[derive(Debug, Deserialize)]
struct CoinbasePosition {
    product_id: String,
    net_size: String,
    avg_entry_price: String,
    unrealized_pnl: String,
}

#[async_trait]
impl AccountSource for CoinbaseAccountSource {
    fn venue_id(&self) -> &str {
        "coinbase"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let headers = Self::auth_headers(creds)?;
        let resp = self
            .client
            .get(format!("{}/api/v3/brokerage/accounts", Self::base_url()))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let accounts: Vec<CoinbaseAccount> = serde_json::from_value(parsed["accounts"].clone())?;

        accounts
            .into_iter()
            .map(|a| {
                let available = Decimal::from_str(&a.available_balance.value)?;
                let locked = Decimal::from_str(&a.hold.value)?;
                Ok(Balance {
                    asset: a.currency,
                    available: Size::from_decimal(available),
                    locked: Size::from_decimal(locked),
                    usd_value: None,
                })
            })
            .collect()
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let headers = Self::auth_headers(creds)?;
        let resp = self
            .client
            .get(format!(
                "{}/api/v3/brokerage/portfolios/breakdown",
                Self::base_url()
            ))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let spot: Vec<CoinbasePosition> =
            serde_json::from_value(parsed["breakdown"]["spot_positions"].clone())
                .unwrap_or_default();

        spot.into_iter()
            .map(|p| {
                let qty = Decimal::from_str(&p.net_size)?;
                let price = Decimal::from_str(&p.avg_entry_price)?;
                let upnl = Decimal::from_str(&p.unrealized_pnl).ok();
                Ok(VenuePosition {
                    instrument_id: p.product_id,
                    quantity: qty,
                    avg_entry_price: Price::from_decimal(price),
                    unrealized_pnl_usd: upnl,
                })
            })
            .collect()
    }

    async fn fetch_transactions(
        &self,
        creds: &VenueCredentials,
        _user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        let headers = Self::auth_headers(creds)?;
        let mut req = self
            .client
            .get(format!(
                "{}/api/v3/brokerage/orders/historical/fills",
                Self::base_url()
            ))
            .headers(headers);

        if let Some(s) = since {
            req = req.query(&[("start_sequence_timestamp", s.to_rfc3339())]);
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let fills = parsed["fills"].as_array().cloned().unwrap_or_default();
        Ok(fills
            .iter()
            .filter_map(|f| {
                let id = f["fill_id"].as_str()?.to_owned();
                let product_id = f["product_id"].as_str()?.to_owned();
                let size = f["size"].as_str()?.to_owned();
                let occurred_at = f["trade_time"]
                    .as_str()
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);
                Some(VenueTransaction {
                    id,
                    transaction_type: "fill".to_owned(),
                    instrument_id: Some(product_id),
                    amount: Decimal::from_str(&size).unwrap_or_default(),
                    currency: "USD".to_owned(),
                    occurred_at,
                })
            })
            .collect())
    }
}
