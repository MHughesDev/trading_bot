//! OANDA v3 `AccountSource` adapter — FX demo account.

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

pub struct OandaAccountSource {
    client: Client,
}

impl Default for OandaAccountSource {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl OandaAccountSource {
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

    fn auth_headers(token: &str) -> header::HeaderMap {
        let mut h = header::HeaderMap::new();
        let bearer = format!("Bearer {token}");
        if let Ok(v) = header::HeaderValue::from_str(&bearer) {
            h.insert(header::AUTHORIZATION, v);
        }
        h
    }

    fn base_url() -> &'static str {
        "https://api-fxpractice.oanda.com"
    }
}

#[async_trait]
impl AccountSource for OandaAccountSource {
    fn venue_id(&self) -> &str {
        "oanda"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let (token, account_id) = Self::parse_creds(creds)?;
        let resp = self
            .client
            .get(format!(
                "{}/v3/accounts/{}/summary",
                Self::base_url(),
                account_id
            ))
            .headers(Self::auth_headers(&token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let bal = parsed["account"]["balance"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO);
        let currency = parsed["account"]["currency"]
            .as_str()
            .unwrap_or("USD")
            .to_owned();

        Ok(vec![Balance {
            asset: currency,
            available: Size::from_decimal(bal),
            locked: Size::from_decimal(Decimal::ZERO),
            usd_value: Some(Price::from_decimal(bal)),
        }])
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let (token, account_id) = Self::parse_creds(creds)?;
        let resp = self
            .client
            .get(format!(
                "{}/v3/accounts/{}/openTrades",
                Self::base_url(),
                account_id
            ))
            .headers(Self::auth_headers(&token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let trades = parsed["trades"].as_array().cloned().unwrap_or_default();
        Ok(trades
            .iter()
            .filter_map(|t| {
                let instrument = t["instrument"].as_str()?.to_owned();
                let qty = Decimal::from_str(t["currentUnits"].as_str()?).ok()?;
                let price = Decimal::from_str(t["price"].as_str()?).ok()?;
                let upnl = t["unrealizedPL"]
                    .as_str()
                    .and_then(|s| Decimal::from_str(s).ok());
                Some(VenuePosition {
                    instrument_id: instrument,
                    quantity: qty,
                    avg_entry_price: Price::from_decimal(price),
                    unrealized_pnl_usd: upnl,
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
        let (token, account_id) = Self::parse_creds(creds)?;
        let mut req = self
            .client
            .get(format!(
                "{}/v3/accounts/{}/transactions",
                Self::base_url(),
                account_id
            ))
            .headers(Self::auth_headers(&token));

        if let Some(s) = since {
            req = req.query(&[("from", s.to_rfc3339())]);
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let parsed: serde_json::Value = resp.json().await?;

        let pages = parsed["pages"].as_array().cloned().unwrap_or_default();
        Ok(pages
            .iter()
            .filter_map(|p| {
                let id = p["id"].as_str()?.to_owned();
                let tx_type = p["type"].as_str().unwrap_or("UNKNOWN").to_owned();
                let instrument = p["instrument"].as_str().map(|s| s.to_owned());
                let units = Decimal::from_str(p["units"].as_str().unwrap_or("0"))
                    .unwrap_or_default()
                    .abs();
                let occurred_at = p["time"]
                    .as_str()
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);
                Some(VenueTransaction {
                    id,
                    transaction_type: tx_type,
                    instrument_id: instrument,
                    amount: units,
                    currency: "USD",
                    occurred_at,
                })
            })
            .collect())
    }
}
