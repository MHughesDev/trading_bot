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

/// Tradovate's `action` → `Side`. Exactly `Buy` or `Sell`; anything else is an
/// error (Set E audit #9 — this was `if action == "Buy" { Buy } else { Sell }`).
pub(crate) fn parse_action(s: &str) -> Result<Side, BrokerError> {
    match s {
        "Buy" => Ok(Side::Buy),
        "Sell" => Ok(Side::Sell),
        other => Err(BrokerError::Serialization(format!(
            "tradovate reported order action {other:?}; refusing to guess a direction"
        ))),
    }
}

/// Tradovate's `ordType` → `OrderType`. It used to be `Market` or else `Limit`.
pub(crate) fn parse_ord_type(s: &str) -> Result<OrderType, BrokerError> {
    match s {
        "Market" => Ok(OrderType::Market),
        "Limit" => Ok(OrderType::Limit),
        "Stop" => Ok(OrderType::Stop),
        "StopLimit" => Ok(OrderType::StopLimit),
        "TrailingStop" => Ok(OrderType::TrailingStop),
        other => Err(BrokerError::Serialization(format!(
            "tradovate reported order type {other:?}; refusing to guess one"
        ))),
    }
}

#[async_trait]
impl Broker for TradovateBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let action = match order.intent.side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        };
        // Stop-type orders are refused by name. This adapter used to send a
        // stop-limit as a plain limit — the trigger silently dropped — which is a
        // different order from the one the strategy placed (Set U U-2.3).
        if order.intent.order_type.is_triggered() {
            return Err(BrokerError::Rejected(format!(
                "{} orders are not supported by the {} adapter",
                order.intent.order_type.as_str(),
                "Tradovate"
            )));
        }
        let ord_type = match order.intent.order_type {
            OrderType::Market => "Market",
            _ => "Limit",
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

        let side = parse_action(&o.action)?;
        let order_type = parse_ord_type(&o.ord_type)?;
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
            // This adapter does not report fees on an order query;
            // `None`, never zero -- see BrokerOrderStatus::commission.
            commission: None,
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
        // Never implemented, and answering "none" reads as a clean book.
        Err(BrokerError::Serialization(
            "tradovate open-order listing is not implemented; use query_order".to_owned(),
        ))
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

#[cfg(test)]
mod inbound_tests {
    use super::*;

    #[test]
    fn only_exact_buy_and_sell_actions_are_read() {
        assert_eq!(parse_action("Buy").unwrap(), Side::Buy);
        assert_eq!(parse_action("Sell").unwrap(), Side::Sell);
        for bad in ["", "buy", "SELL", "Short", "Sell "] {
            assert!(parse_action(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn an_unknown_ord_type_is_refused() {
        assert_eq!(parse_ord_type("StopLimit").unwrap(), OrderType::StopLimit);
        assert!(parse_ord_type("MIT").is_err());
        assert!(parse_ord_type("").is_err());
    }
}
