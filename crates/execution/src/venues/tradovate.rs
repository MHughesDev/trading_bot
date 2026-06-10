//! Tradovate REST broker adapter — futures (demo account first).

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

pub const DEMO_BASE_URL: &str = "https://demo-api-d.tradovate.com/v1";

pub struct TradovateBroker {
    client: Client,
    access_token: String,
    account_id: i64,
    pub base_url: String,
}

impl TradovateBroker {
    pub fn new(access_token: impl Into<String>, account_id: i64) -> Self {
        Self {
            client: Client::new(),
            access_token: access_token.into(),
            account_id,
            base_url: DEMO_BASE_URL.to_owned(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        let bearer = format!("Bearer {}", self.access_token);
        if let (Ok(auth), Ok(ct)) = (
            header::HeaderValue::from_str(&bearer),
            header::HeaderValue::from_str("application/json"),
        ) {
            headers.insert(header::AUTHORIZATION, auth);
            headers.insert(header::CONTENT_TYPE, ct);
        }
        headers
    }
}

#[derive(Debug, Deserialize)]
struct TradovateOrderResponse {
    #[serde(rename = "orderId")]
    order_id: Option<i64>,
    #[serde(rename = "failureReason")]
    failure_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TradovateOrder {
    id: i64,
    #[serde(rename = "contractId")]
    contract_id: i64,
    action: String,
    #[serde(rename = "ordType")]
    ord_type: String,
    #[serde(default, rename = "fillQty")]
    fill_qty: i64,
    #[serde(rename = "totalQty")]
    total_qty: i64,
    #[serde(rename = "avgPx")]
    avg_px: Option<String>,
    status: String,
}

fn parse_state(s: &str) -> BrokerOrderState {
    match s {
        "Working" | "Accepted" => BrokerOrderState::New,
        "Completed" => BrokerOrderState::Filled,
        "Cancelled" | "Expired" => BrokerOrderState::Cancelled,
        "Rejected" => BrokerOrderState::Rejected,
        other => BrokerOrderState::Unknown(other.to_owned()),
    }
}

#[async_trait]
impl Broker for TradovateBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let action = match order.intent.side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        };
        let ord_type = match order.intent.order_type {
            OrderType::Market => "Market",
            OrderType::Limit | OrderType::StopLimit => "Limit",
        };

        let mut body = serde_json::json!({
            "accountSpec": self.account_id,
            "symbol": order.intent.instrument_id,
            "action": action,
            "orderQty": order.intent.size.inner().to_string(),
            "orderType": ord_type,
            "timeInForce": "GTC",
            "isAutomated": true,
            "clientOrderId": order.intent.idempotency_key.to_string(),
        });

        if let Some(lp) = order.intent.limit_price {
            body["price"] = serde_json::json!(lp.inner().to_string());
        }

        let resp = self
            .client
            .post(format!("{}/order/placeorder", self.base_url))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let parsed: TradovateOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        if let Some(reason) = parsed.failure_reason {
            return Err(BrokerError::Rejected(reason));
        }
        let id = parsed
            .order_id
            .ok_or_else(|| BrokerError::Serialization("missing orderId".to_owned()))?;
        Ok(id.to_string())
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let id: i64 = broker_order_id
            .parse()
            .map_err(|_| BrokerError::OrderNotFound(broker_order_id.to_owned()))?;

        let resp = self
            .client
            .post(format!("{}/order/cancelorder", self.base_url))
            .headers(self.auth_headers())
            .json(&serde_json::json!({ "orderId": id }))
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
        let id: i64 = broker_order_id
            .parse()
            .map_err(|_| BrokerError::OrderNotFound(broker_order_id.to_owned()))?;

        let resp = self
            .client
            .get(format!("{}/order/item", self.base_url))
            .headers(self.auth_headers())
            .query(&[("id", id)])
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

        let o: TradovateOrder = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let side = if o.action == "Buy" {
            Side::Buy
        } else {
            Side::Sell
        };
        let order_type = if o.ord_type == "Market" {
            OrderType::Market
        } else {
            OrderType::Limit
        };
        let avg_fill_price = o
            .avg_px
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(Price::from_decimal);
        let broker_state = if o.fill_qty > 0 && o.fill_qty < o.total_qty {
            BrokerOrderState::PartiallyFilled
        } else {
            parse_state(&o.status)
        };

        Ok(BrokerOrderStatus {
            broker_order_id: o.id.to_string(),
            instrument_id: o.contract_id.to_string(),
            side,
            order_type,
            submitted_qty: Decimal::from(o.total_qty),
            filled_qty: Decimal::from(o.fill_qty),
            avg_fill_price,
            state: broker_state,
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        let resp = self
            .client
            .get(format!("{}/order/list", self.base_url))
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
            .get(format!("{}/position/list", self.base_url))
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Http(text));
        }

        let parsed: Vec<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        Ok(parsed
            .iter()
            .filter_map(|p| {
                let contract_id = p["contractId"].as_i64()?.to_string();
                let qty = Decimal::from(p["netPos"].as_i64().unwrap_or(0));
                let price = p["netPrice"]
                    .as_f64()
                    .and_then(|v| Decimal::from_str(&v.to_string()).ok())
                    .map(Price::from_decimal)
                    .unwrap_or_else(|| Price::from_decimal(Decimal::ZERO));
                Some(BrokerPosition {
                    instrument_id: contract_id,
                    quantity: qty,
                    avg_entry_price: price,
                })
            })
            .collect())
    }
}
