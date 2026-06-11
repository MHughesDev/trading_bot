//! Alpaca paper trading broker adapter.
//!
//! Implements `Broker` against the Alpaca paper trading REST API:
//! `https://paper-api.alpaca.markets/v2`
//!
//! Credentials are read from env vars `ALPACA_API_KEY_ID` and
//! `ALPACA_API_SECRET_KEY`.

use async_trait::async_trait;
use reqwest::{header, Client};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

use domain::{
    money::Price,
    order::{OrderType, Side},
};
use risk::ApprovedOrder;

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

pub const PAPER_BASE_URL: &str = "https://paper-api.alpaca.markets";

pub struct AlpacaBroker {
    client: Client,
    api_key: String,
    api_secret: String,
    pub base_url: String,
}

impl AlpacaBroker {
    /// Create from explicit credentials (used in tests / injected from config).
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .http2_keep_alive_interval(std::time::Duration::from_secs(30))
            .http2_keep_alive_while_idle(true)
            .tcp_nodelay(true)
            .pool_idle_timeout(None)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            base_url: PAPER_BASE_URL.to_owned(),
        }
    }

    /// Load credentials from environment variables.
    pub fn from_env() -> Result<Self, std::env::VarError> {
        let key = std::env::var("ALPACA_API_KEY_ID")?;
        let secret = std::env::var("ALPACA_API_SECRET_KEY")?;
        Ok(Self::new(key, secret))
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        if let (Ok(k), Ok(s)) = (
            header::HeaderValue::from_str(&self.api_key),
            header::HeaderValue::from_str(&self.api_secret),
        ) {
            headers.insert("APCA-API-KEY-ID", k);
            headers.insert("APCA-API-SECRET-KEY", s);
        }
        headers
    }
}

// ── Alpaca response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AlpacaOrderResponse {
    id: String,
    symbol: String,
    side: String,
    #[serde(rename = "type")]
    order_type: String,
    qty: String,
    filled_qty: String,
    filled_avg_price: Option<String>,
    status: String,
}

#[derive(Debug, Deserialize)]
struct AlpacaPositionResponse {
    symbol: String,
    qty: String,
    avg_entry_price: String,
}

fn parse_broker_state(status: &str) -> BrokerOrderState {
    match status {
        "new" | "accepted" | "pending_new" | "accepted_for_bidding" => BrokerOrderState::New,
        "partially_filled" => BrokerOrderState::PartiallyFilled,
        "filled" => BrokerOrderState::Filled,
        "canceled" | "cancelled" | "expired" | "done_for_day" => BrokerOrderState::Cancelled,
        "rejected" => BrokerOrderState::Rejected,
        other => BrokerOrderState::Unknown(other.to_owned()),
    }
}

fn alpaca_symbol(instrument_id: &str) -> String {
    instrument_id.replace('-', "")
}

fn domain_side(side: &str) -> Side {
    if side == "buy" {
        Side::Buy
    } else {
        Side::Sell
    }
}

fn domain_order_type(t: &str) -> OrderType {
    match t {
        "limit" => OrderType::Limit,
        "stop_limit" => OrderType::StopLimit,
        _ => OrderType::Market,
    }
}

fn parse_price(s: &str) -> Option<Price> {
    Decimal::from_str(s).ok().map(Price::from_decimal)
}

#[async_trait]
impl Broker for AlpacaBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let symbol = alpaca_symbol(&order.intent.instrument_id);
        let side = match order.intent.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let order_type = match order.intent.order_type {
            OrderType::Market => "market",
            OrderType::Limit => "limit",
            OrderType::StopLimit => "stop_limit",
        };

        let mut body = serde_json::json!({
            "symbol": symbol,
            "qty": order.intent.size.inner().to_string(),
            "side": side,
            "type": order_type,
            "time_in_force": "gtc",
            "client_order_id": order.intent.idempotency_key.to_string(),
        });

        if let Some(lp) = order.intent.limit_price {
            body["limit_price"] = serde_json::json!(lp.inner().to_string());
        }

        let resp = self
            .client
            .post(format!("{}/v2/orders", self.base_url))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(%symbol, "Alpaca rejected order: {text}");
            return Err(BrokerError::Rejected(text));
        }

        let parsed: AlpacaOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        info!(broker_order_id = %parsed.id, %symbol, "order submitted to Alpaca");
        Ok(parsed.id)
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let resp = self
            .client
            .delete(format!("{}/v2/orders/{broker_order_id}", self.base_url))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BrokerError::OrderNotFound(broker_order_id.to_owned()));
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }
        Ok(())
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/v2/orders/{broker_order_id}", self.base_url))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BrokerError::OrderNotFound(broker_order_id.to_owned()));
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let parsed: AlpacaOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let submitted_qty = Decimal::from_str(&parsed.qty).unwrap_or(Decimal::ZERO);
        let filled_qty = Decimal::from_str(&parsed.filled_qty).unwrap_or(Decimal::ZERO);
        let avg_fill_price = parsed.filled_avg_price.as_deref().and_then(parse_price);

        Ok(BrokerOrderStatus {
            broker_order_id: parsed.id,
            instrument_id: parsed.symbol,
            side: domain_side(&parsed.side),
            order_type: domain_order_type(&parsed.order_type),
            submitted_qty,
            filled_qty,
            avg_fill_price,
            state: parse_broker_state(&parsed.status),
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/v2/orders", self.base_url))
            .query(&[("status", "open")])
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }

        let parsed: Vec<AlpacaOrderResponse> = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        Ok(parsed
            .into_iter()
            .map(|o| {
                let submitted_qty = Decimal::from_str(&o.qty).unwrap_or(Decimal::ZERO);
                let filled_qty = Decimal::from_str(&o.filled_qty).unwrap_or(Decimal::ZERO);
                let avg_fill_price = o.filled_avg_price.as_deref().and_then(parse_price);
                BrokerOrderStatus {
                    broker_order_id: o.id,
                    instrument_id: o.symbol,
                    side: domain_side(&o.side),
                    order_type: domain_order_type(&o.order_type),
                    submitted_qty,
                    filled_qty,
                    avg_fill_price,
                    state: parse_broker_state(&o.status),
                }
            })
            .collect())
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/v2/positions", self.base_url))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }

        let parsed: Vec<AlpacaPositionResponse> = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        Ok(parsed
            .into_iter()
            .filter_map(|p| {
                let qty = Decimal::from_str(&p.qty).ok()?;
                let avg_entry_price = parse_price(&p.avg_entry_price)?;
                Some(BrokerPosition {
                    instrument_id: p.symbol,
                    quantity: qty,
                    avg_entry_price,
                })
            })
            .collect())
    }
}
