//! Tradier REST broker adapter — options execution.

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

pub const BASE_URL: &str = "https://api.tradier.com/v1";

pub struct TradierBroker {
    client: Client,
    access_token: String,
    account_id: String,
    pub base_url: String,
}

impl TradierBroker {
    pub fn new(access_token: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            access_token: access_token.into(),
            account_id: account_id.into(),
            base_url: BASE_URL.to_owned(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        let bearer = format!("Bearer {}", self.access_token);
        if let (Ok(auth), Ok(accept)) = (
            header::HeaderValue::from_str(&bearer),
            header::HeaderValue::from_str("application/json"),
        ) {
            headers.insert(header::AUTHORIZATION, auth);
            headers.insert(header::ACCEPT, accept);
        }
        headers
    }
}

#[derive(Debug, Deserialize)]
struct TradierOrderResponse {
    order: TradierOrder,
}

#[derive(Debug, Deserialize)]
struct TradierOrder {
    id: u64,
    symbol: String,
    side: String,
    #[serde(rename = "type")]
    order_type: String,
    quantity: Decimal,
    #[serde(default)]
    exec_quantity: Decimal,
    avg_fill_price: Option<Decimal>,
    status: String,
}

fn parse_state(s: &str) -> BrokerOrderState {
    match s {
        "open" | "partially_filled" | "pending" => BrokerOrderState::New,
        "filled" => BrokerOrderState::Filled,
        "canceled" => BrokerOrderState::Cancelled,
        "rejected" | "expired" => BrokerOrderState::Rejected,
        other => BrokerOrderState::Unknown(other.to_owned()),
    }
}

#[async_trait]
impl Broker for TradierBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let side = match order.intent.side {
            Side::Buy => "buy_to_open",
            Side::Sell => "sell_to_close",
        };
        let order_type = match order.intent.order_type {
            OrderType::Market => "market",
            OrderType::Limit | OrderType::StopLimit => "limit",
        };
        let duration = "gtc";

        let mut params = vec![
            ("class", "option"),
            ("symbol", &order.intent.instrument_id),
            ("side", side),
            ("type", order_type),
            ("duration", duration),
        ];
        let qty_str = order.intent.size.inner().to_string();
        params.push(("quantity", &qty_str));

        let limit_str;
        if let Some(lp) = order.intent.limit_price {
            limit_str = lp.inner().to_string();
            params.push(("price", &limit_str));
        }

        let resp = self
            .client
            .post(format!(
                "{}/accounts/{}/orders",
                self.base_url, self.account_id
            ))
            .headers(self.auth_headers())
            .form(&params)
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let id = parsed["order"]["id"]
            .as_u64()
            .ok_or_else(|| BrokerError::Serialization("missing order id".to_owned()))?;
        Ok(id.to_string())
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let resp = self
            .client
            .delete(format!(
                "{}/accounts/{}/orders/{broker_order_id}",
                self.base_url, self.account_id
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
                "{}/accounts/{}/orders/{broker_order_id}",
                self.base_url, self.account_id
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

        let parsed: TradierOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let o = &parsed.order;
        let side = if o.side.contains("buy") {
            Side::Buy
        } else {
            Side::Sell
        };
        let order_type = if o.order_type == "market" {
            OrderType::Market
        } else {
            OrderType::Limit
        };
        let broker_state = if o.exec_quantity > Decimal::ZERO && o.exec_quantity < o.quantity {
            BrokerOrderState::PartiallyFilled
        } else {
            parse_state(&o.status)
        };

        Ok(BrokerOrderStatus {
            broker_order_id: o.id.to_string(),
            instrument_id: o.symbol.clone(),
            side,
            order_type,
            submitted_qty: o.quantity,
            filled_qty: o.exec_quantity,
            avg_fill_price: o.avg_fill_price.map(Price::from_decimal),
            state: broker_state,
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        let resp = self
            .client
            .get(format!(
                "{}/accounts/{}/orders",
                self.base_url, self.account_id
            ))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }
        Ok(vec![])
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        let resp = self
            .client
            .get(format!(
                "{}/accounts/{}/positions",
                self.base_url, self.account_id
            ))
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

        let positions = parsed["positions"]["position"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        Ok(positions
            .iter()
            .filter_map(|p| {
                let symbol = p["symbol"].as_str()?.to_owned();
                let qty = Decimal::from_str(p["quantity"].as_str()?).ok()?;
                let price = Price::from_decimal(
                    Decimal::from_str(p["cost_basis"].as_str().unwrap_or("0")).unwrap_or_default(),
                );
                Some(BrokerPosition {
                    instrument_id: symbol,
                    quantity: qty,
                    avg_entry_price: price,
                })
            })
            .collect())
    }
}
