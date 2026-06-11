//! Alpaca `AccountSource` adapter — equity account balances, positions, transactions.

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

pub const PAPER_BASE: &str = "https://paper-api.alpaca.markets";
pub const LIVE_BASE: &str = "https://api.alpaca.markets";
pub const DATA_BASE: &str = "https://data.alpaca.markets";

pub struct AlpacaAccountSource {
    client: Client,
    is_paper: bool,
}

impl AlpacaAccountSource {
    pub fn new(is_paper: bool) -> Self {
        Self {
            client: Client::new(),
            is_paper,
        }
    }

    fn base_url(&self) -> &'static str {
        if self.is_paper {
            PAPER_BASE
        } else {
            LIVE_BASE
        }
    }

    fn parse_creds(creds: &VenueCredentials) -> Result<(String, String), AccountSourceError> {
        let text = std::str::from_utf8(&creds.plaintext).map_err(|_| {
            AccountSourceError::Credentials("credentials are not valid UTF-8".to_owned())
        })?;
        let mut parts = text.splitn(2, ':');
        let key = parts.next().unwrap_or("").to_owned();
        let secret = parts
            .next()
            .ok_or_else(|| {
                AccountSourceError::Credentials("expected api_key:api_secret".to_owned())
            })?
            .to_owned();
        Ok((key, secret))
    }

    fn auth_headers(key: &str, secret: &str) -> header::HeaderMap {
        let mut h = header::HeaderMap::new();
        if let (Ok(k), Ok(s)) = (
            header::HeaderValue::from_str(key),
            header::HeaderValue::from_str(secret),
        ) {
            h.insert("APCA-API-KEY-ID", k);
            h.insert("APCA-API-SECRET-KEY", s);
        }
        h
    }
}

#[derive(Deserialize)]
struct AlpacaAccount {
    cash: String,
    buying_power: String,
    currency: String,
}

#[derive(Deserialize)]
struct AlpacaPosition {
    symbol: String,
    qty: String,
    avg_entry_price: String,
    unrealized_pl: String,
}

#[derive(Deserialize)]
struct AlpacaActivity {
    id: String,
    activity_type: String,
    symbol: Option<String>,
    qty: Option<String>,
    price: Option<String>,
    transaction_time: Option<String>,
}

#[async_trait]
impl AccountSource for AlpacaAccountSource {
    fn venue_id(&self) -> &str {
        "alpaca"
    }

    async fn fetch_balances(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let (key, secret) = Self::parse_creds(creds)?;
        let headers = Self::auth_headers(&key, &secret);

        let resp = self
            .client
            .get(format!("{}/v2/account", self.base_url()))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let acc: AlpacaAccount = resp.json().await?;

        let cash = Decimal::from_str(&acc.cash)?;
        let bp = Decimal::from_str(&acc.buying_power)?;

        Ok(vec![Balance {
            asset: acc.currency,
            available: Size::from_decimal(bp),
            locked: Size::from_decimal(cash - bp),
            usd_value: Some(Price::from_decimal(cash)),
        }])
    }

    async fn fetch_positions(
        &self,
        creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let (key, secret) = Self::parse_creds(creds)?;
        let headers = Self::auth_headers(&key, &secret);

        let resp = self
            .client
            .get(format!("{}/v2/positions", self.base_url()))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let positions: Vec<AlpacaPosition> = resp.json().await?;

        positions
            .into_iter()
            .map(|p| {
                let qty = Decimal::from_str(&p.qty)?;
                let price = Decimal::from_str(&p.avg_entry_price)?;
                let upnl = Decimal::from_str(&p.unrealized_pl).ok();
                Ok(VenuePosition {
                    instrument_id: p.symbol,
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
        let (key, secret) = Self::parse_creds(creds)?;
        let headers = Self::auth_headers(&key, &secret);

        let mut req = self
            .client
            .get(format!("{}/v2/account/activities/FILL", self.base_url()))
            .headers(headers);

        if let Some(s) = since {
            req = req.query(&[("after", s.to_rfc3339())]);
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            return Err(AccountSourceError::HttpStatus(
                resp.text().await.unwrap_or_default(),
            ));
        }

        let activities: Vec<AlpacaActivity> = resp.json().await?;

        Ok(activities
            .into_iter()
            .map(|a| {
                let qty = a
                    .qty
                    .as_deref()
                    .and_then(|s| Decimal::from_str(s).ok())
                    .unwrap_or(Decimal::ZERO);
                let occurred_at = a
                    .transaction_time
                    .as_deref()
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);
                VenueTransaction {
                    id: a.id,
                    transaction_type: a.activity_type,
                    instrument_id: a.symbol,
                    amount: qty,
                    currency: "USD".to_owned(),
                    occurred_at,
                }
            })
            .collect())
    }
}

#[allow(dead_code)]
fn _use_price(a: &AlpacaActivity) -> Option<&str> {
    a.price.as_deref()
}
