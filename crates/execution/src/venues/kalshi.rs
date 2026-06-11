//! Kalshi REST broker adapter — prediction markets and perpetual swaps.

use async_trait::async_trait;
use reqwest::{header, Client};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use domain::{
    money::Price,
    order::{OrderType, Side},
};
use risk::ApprovedOrder;

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

pub const BASE_URL: &str = "https://trading-api.kalshi.com/trade-api/v2";

pub struct KalshiBroker {
    client: Client,
    api_key: String,
    pub base_url: String,
}

impl KalshiBroker {
    pub fn new(api_key: impl Into<String>) -> Self {
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
            base_url: BASE_URL.to_owned(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&self.api_key) {
            headers.insert("Authorization", v);
        }
        headers
    }
}

#[derive(Debug, Deserialize)]
struct KalshiOrderResponse {
    order: KalshiOrder,
}

#[derive(Debug, Deserialize)]
struct KalshiOrder {
    order_id: String,
    ticker: String,
    side: String,
    action: String,
    count: u64,
    filled_count: u64,
    no_price: Option<u64>,
    yes_price: Option<u64>,
    status: String,
}

fn parse_state(s: &str) -> BrokerOrderState {
    match s {
        "resting" | "pending" => BrokerOrderState::New,
        "executed" => BrokerOrderState::Filled,
        "canceled" | "cancelled" => BrokerOrderState::Cancelled,
        other => BrokerOrderState::Unknown(other.to_owned()),
    }
}

/// Kalshi prices are in cents (0–99); convert to [0, 1) decimal.
fn cents_to_price(cents: u64) -> Price {
    Price::from_decimal(Decimal::from(cents) / Decimal::from(100u64))
}

#[async_trait]
impl Broker for KalshiBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let side = match order.intent.side {
            Side::Buy => "yes",
            Side::Sell => "no",
        };
        let count = order.intent.size.inner().try_into().unwrap_or(1u64);

        let mut body = serde_json::json!({
            "ticker": order.intent.instrument_id,
            "side": side,
            "action": "buy",
            "type": "market",
            "count": count,
            "client_order_id": order.intent.idempotency_key.to_string(),
        });

        if let Some(lp) = order.intent.limit_price {
            let cents = (lp.inner() * Decimal::from(100u64))
                .try_into()
                .unwrap_or(50u64);
            body["yes_price"] = serde_json::json!(cents);
            body["type"] = serde_json::json!("limit");
        }

        let resp = self
            .client
            .post(format!("{}/portfolio/orders", self.base_url))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let parsed: KalshiOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        Ok(parsed.order.order_id)
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let resp = self
            .client
            .delete(format!(
                "{}/portfolio/orders/{broker_order_id}",
                self.base_url
            ))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }
        Ok(())
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        let resp = self
            .client
            .get(format!(
                "{}/portfolio/orders/{broker_order_id}",
                self.base_url
            ))
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

        let parsed: KalshiOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let o = &parsed.order;
        let avg_fill_price = o.yes_price.map(cents_to_price);
        let side = if o.side == "yes" {
            Side::Buy
        } else {
            Side::Sell
        };

        Ok(BrokerOrderStatus {
            broker_order_id: o.order_id.clone(),
            instrument_id: o.ticker.clone(),
            side,
            order_type: OrderType::Market,
            submitted_qty: Decimal::from(o.count),
            filled_qty: Decimal::from(o.filled_count),
            avg_fill_price,
            state: parse_state(&o.status),
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/portfolio/orders", self.base_url))
            .query(&[("status", "resting")])
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let orders = parsed["orders"].as_array().cloned().unwrap_or_default();
        let result = orders
            .iter()
            .filter_map(|v| {
                let id = v["order_id"].as_str()?.to_owned();
                let ticker = v["ticker"].as_str()?.to_owned();
                let side = if v["side"].as_str()? == "yes" {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let count = v["count"].as_u64().unwrap_or(0);
                let filled = v["filled_count"].as_u64().unwrap_or(0);
                let price = v["yes_price"].as_u64().map(cents_to_price);
                Some(BrokerOrderStatus {
                    broker_order_id: id,
                    instrument_id: ticker,
                    side,
                    order_type: OrderType::Limit,
                    submitted_qty: Decimal::from(count),
                    filled_qty: Decimal::from(filled),
                    avg_fill_price: price,
                    state: BrokerOrderState::New,
                })
            })
            .collect();

        Ok(result)
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/portfolio/positions", self.base_url))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let positions = parsed["market_positions"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        Ok(positions
            .iter()
            .filter_map(|p| {
                let ticker = p["ticker"].as_str()?.to_owned();
                let qty = Decimal::from(p["position"].as_i64().unwrap_or(0));
                let price = p["market_exposure"]
                    .as_f64()
                    .and_then(|v| Decimal::from_str(&v.to_string()).ok())
                    .map(Price::from_decimal)
                    .unwrap_or_else(|| Price::from_decimal(Decimal::ZERO));
                Some(BrokerPosition {
                    instrument_id: ticker,
                    quantity: qty,
                    avg_entry_price: price,
                })
            })
            .collect())
    }
}

#[allow(dead_code)]
fn _use_fields(o: &KalshiOrder) -> (&str, Option<u64>) {
    (&o.action, o.no_price)
}
