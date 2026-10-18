//! OANDA v20 REST broker adapter — FX demo account.

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

pub const DEMO_BASE_URL: &str = "https://api-fxpractice.oanda.com";

pub struct OandaBroker {
    client: Client,
    api_token: String,
    account_id: String,
    pub base_url: String,
}

impl OandaBroker {
    pub fn new(api_token: impl Into<String>, account_id: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .http2_keep_alive_interval(std::time::Duration::from_secs(30))
            .http2_keep_alive_while_idle(true)
            .tcp_nodelay(true)
            .pool_idle_timeout(None)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            api_token: api_token.into(),
            account_id: account_id.into(),
            base_url: DEMO_BASE_URL.to_owned(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        let bearer = format!("Bearer {}", self.api_token);
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
struct OandaOrderResponse {
    #[serde(rename = "orderCreateTransaction")]
    order_create_transaction: OandaOrderTransaction,
}

#[derive(Debug, Deserialize)]
struct OandaOrderTransaction {
    id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OandaTradeResponse {
    id: String,
    instrument: String,
    #[serde(rename = "currentUnits")]
    current_units: String,
    price: String,
    state: String,
}

/// The direction of an OANDA order from its signed `units` string. A missing,
/// unparsable or zero value is an error: this used to default to a **buy** when
/// `units` was absent or malformed (Set E audit #9).
pub(crate) fn side_from_units(units: Option<&str>) -> Result<Side, BrokerError> {
    let d = units
        .and_then(|s| Decimal::from_str(s).ok())
        .ok_or_else(|| {
            BrokerError::Serialization(format!(
                "oanda reported order units {units:?}; cannot tell buy from sell"
            ))
        })?;
    match d.cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Greater => Ok(Side::Buy),
        std::cmp::Ordering::Less => Ok(Side::Sell),
        std::cmp::Ordering::Equal => Err(BrokerError::Serialization(
            "oanda reported zero units; cannot tell buy from sell".to_owned(),
        )),
    }
}

#[async_trait]
impl Broker for OandaBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let units = match order.intent.side {
            Side::Buy => order.intent.size.inner().to_string(),
            Side::Sell => format!("-{}", order.intent.size.inner()),
        };

        let body = serde_json::json!({
            "order": {
                "type": "MARKET",
                "instrument": order.intent.instrument_id,
                "units": units,
                "timeInForce": "FOK",
                "clientExtensions": {
                    "id": order.intent.idempotency_key.to_string()
                }
            }
        });

        let resp = self
            .client
            .post(format!(
                "{}/v3/accounts/{}/orders",
                self.base_url, self.account_id
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let parsed: OandaOrderResponse = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        Ok(parsed.order_create_transaction.id)
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        let resp = self
            .client
            .put(format!(
                "{}/v3/accounts/{}/orders/{}/cancel",
                self.base_url, self.account_id, broker_order_id
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
                "{}/v3/accounts/{}/orders/{}",
                self.base_url, self.account_id, broker_order_id
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

        let parsed: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        let order = &parsed["order"];
        let state = order["state"].as_str().unwrap_or("PENDING");
        let broker_state = match state {
            "FILLED" => BrokerOrderState::Filled,
            "CANCELLED" => BrokerOrderState::Cancelled,
            "REJECTED" => BrokerOrderState::Rejected,
            "PENDING" | "TRIGGERED" => BrokerOrderState::New,
            // An unrecognised state is reported as unknown, not as "new": a
            // reconciler treats New as a live order it may wait on.
            other => BrokerOrderState::Unknown(other.to_owned()),
        };
        let filled_qty = order["filledUnits"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO)
            .abs();
        let submitted_qty = order["units"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO)
            .abs();
        let avg_fill_price = order["price"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .map(Price::from_decimal);

        Ok(BrokerOrderStatus {
            broker_order_id: broker_order_id.to_owned(),
            instrument_id: order["instrument"].as_str().unwrap_or_default().to_owned(),
            side: side_from_units(order["units"].as_str())?,
            order_type: OrderType::Market,
            submitted_qty,
            filled_qty,
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
            .get(format!(
                "{}/v3/accounts/{}/pendingOrders",
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
        // Never implemented; "none" would read as a clean book.
        Err(BrokerError::Serialization(
            "oanda pending-order listing is not implemented; use query_order".to_owned(),
        ))
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        let resp = self
            .client
            .get(format!(
                "{}/v3/accounts/{}/openTrades",
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

        let trades = parsed["trades"].as_array().cloned().unwrap_or_default();
        let positions = trades
            .iter()
            .filter_map(|t| {
                let instrument = t["instrument"].as_str()?.to_owned();
                let qty = Decimal::from_str(t["currentUnits"].as_str()?).ok()?;
                let avg_entry_price =
                    Price::from_decimal(Decimal::from_str(t["price"].as_str()?).ok()?);
                Some(BrokerPosition {
                    instrument_id: instrument,
                    quantity: qty,
                    avg_entry_price,
                })
            })
            .collect();

        Ok(positions)
    }
}

// Suppress dead_code for the struct field used only in non-test builds
#[allow(dead_code)]
fn _assert_oanda_fields(b: &OandaTradeResponse) -> &str {
    &b.id
}

#[cfg(test)]
mod inbound_tests {
    use super::*;

    #[test]
    fn oanda_direction_comes_only_from_a_readable_nonzero_units_string() {
        assert_eq!(side_from_units(Some("100")).unwrap(), Side::Buy);
        assert_eq!(side_from_units(Some("-100")).unwrap(), Side::Sell);
        for bad in [None, Some(""), Some("abc"), Some("0")] {
            assert!(side_from_units(bad).is_err(), "{bad:?} must be refused");
        }
    }
}
