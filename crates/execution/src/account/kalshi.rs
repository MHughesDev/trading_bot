//! Kalshi `AccountSource` adapter — prediction market balances and positions.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{header, Client};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

use domain::money::{Price, Size};

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

pub struct KalshiAccountSource {
    client: Client,
}

impl Default for KalshiAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl KalshiAccountSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn base_url() -> &'static str {
        "https://trading-api.kalshi.com/trade-api/v2"
    }

    fn auth_headers(creds: &VenueCredentials) -> Result<header::HeaderMap, AccountSourceError> {
        let key = std::str::from_utf8(&creds.plaintext).map_err(|_| {
            AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned())
        })?;
        let mut h = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(key) {
            h.insert("Authorization", v);
        } else {
            return Err(AccountSourceError::Credentials("invalid key".to_owned()));
        }
        Ok(h)
    }
}

#[async_trait]
impl AccountSource for KalshiAccountSource {
    fn venue_id(&self) -> &str {
        "kalshi"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let headers = Self::auth_headers(creds)?;
        let resp = self
            .client
            .get(format!("{}/portfolio/balance", Self::base_url()))
            .headers(headers)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }
        let parsed: serde_json::Value = resp.json().await?;
        let balance_cents = parsed["balance"].as_i64().unwrap_or(0);
        let balance = Decimal::from(balance_cents) / Decimal::from(100u64);
        Ok(vec![Balance {
            asset: "USD".to_owned(),
            available: Size::from_decimal(balance),
            locked: Size::from_decimal(Decimal::ZERO),
            usd_value: Some(Price::from_decimal(balance)),
        }])
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let headers = Self::auth_headers(creds)?;
        let resp = self
            .client
            .get(format!("{}/portfolio/positions", Self::base_url()))
            .headers(headers)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }
        let parsed: serde_json::Value = resp.json().await?;
        let positions = parsed["market_positions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(positions
            .iter()
            .filter_map(|p| {
                let ticker = p["ticker"].as_str()?.to_owned();
                let net = p["position"].as_i64().unwrap_or(0);
                let exposure = p["market_exposure"]
                    .as_f64()
                    .and_then(|v| Decimal::from_str(&v.to_string()).ok())
                    .unwrap_or(Decimal::ZERO);
                let qty = Decimal::from(net);
                let avg = if qty.is_zero() {
                    Decimal::ZERO
                } else {
                    exposure / qty.abs()
                };
                Some(VenuePosition {
                    instrument_id: ticker,
                    quantity: qty,
                    avg_entry_price: Price::from_decimal(avg),
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
        let headers = Self::auth_headers(creds)?;
        let mut req = self
            .client
            .get(format!("{}/portfolio/fills", Self::base_url()))
            .headers(headers);
        if let Some(s) = since {
            req = req.query(&[("min_ts", s.timestamp().to_string())]);
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
                let ticker = f["ticker"].as_str()?.to_owned();
                let count = Decimal::from(f["count"].as_u64().unwrap_or(0));
                let occurred_at = f["created_time"]
                    .as_str()
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);
                Some(VenueTransaction {
                    id,
                    transaction_type: "fill".to_owned(),
                    instrument_id: Some(ticker),
                    amount: count,
                    currency: "USD".to_owned(),
                    occurred_at,
                })
            })
            .collect())
    }
}
