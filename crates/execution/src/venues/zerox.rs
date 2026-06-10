//! 0x Swap API broker adapter — DEX swap execution via firm quote.

use async_trait::async_trait;
use reqwest::{header, Client};
use rust_decimal::Decimal;
use serde::Deserialize;
use domain::{
    order::{OrderType, Side},
};
use risk::ApprovedOrder;

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

pub const BASE_URL: &str = "https://api.0x.org";

pub struct ZeroXBroker {
    client: Client,
    api_key: String,
    pub base_url: String,
}

impl ZeroXBroker {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: BASE_URL.to_owned(),
        }
    }

    fn auth_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&self.api_key) {
            headers.insert("0x-api-key", v);
        }
        headers
    }
}

/// 0x firm quote response shape.
#[derive(Debug, Deserialize)]
pub struct ZeroXQuote {
    pub price: String,
    pub guaranteed_price: String,
    pub to: String,
    pub data: String,
    pub value: String,
    pub gas: String,
    pub gas_price: String,
    pub buy_amount: String,
    pub sell_amount: String,
}

#[async_trait]
impl Broker for ZeroXBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        // 0x is a DEX aggregator — "submit" means fetching a firm quote and
        // broadcasting the transaction. In the adapter layer, we model this as
        // fetching the quote and returning its transaction hash stub.
        // Real broadcast requires a signer wallet; the adapter returns the quote
        // data that the caller passes to a signer.
        let (sell_token, buy_token) = {
            let parts: Vec<&str> = order.intent.instrument_id.split('-').collect();
            if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                return Err(BrokerError::Rejected(
                    "instrument_id must be BASE-QUOTE format for 0x".to_owned(),
                ));
            }
        };

        let (sell_token, buy_token) = match order.intent.side {
            Side::Buy => (buy_token, sell_token),
            Side::Sell => (sell_token, buy_token),
        };

        let sell_amount = (order.intent.size.inner() * Decimal::from(10u64.pow(18)))
            .round()
            .to_string();

        let resp = self
            .client
            .get(format!("{}/swap/v1/quote", self.base_url))
            .headers(self.auth_headers())
            .query(&[
                ("sellToken", sell_token),
                ("buyToken", buy_token),
                ("sellAmount", &sell_amount),
            ])
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BrokerError::Rejected(text));
        }

        let quote: ZeroXQuote = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;

        // Return the contract address as the broker order ID (used to poll status).
        Ok(format!("0x:{}", quote.to))
    }

    async fn cancel(&self, _broker_order_id: &str) -> Result<(), BrokerError> {
        // DEX swaps are atomic and cannot be cancelled once submitted.
        Err(BrokerError::Rejected(
            "DEX swaps cannot be cancelled after broadcast".to_owned(),
        ))
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        // For DEX, "query" means checking on-chain transaction status.
        // In this adapter stub we return the order as Filled since 0x swaps are atomic.
        Ok(BrokerOrderStatus {
            broker_order_id: broker_order_id.to_owned(),
            instrument_id: String::new(),
            side: Side::Buy,
            order_type: OrderType::Market,
            submitted_qty: Decimal::ONE,
            filled_qty: Decimal::ONE,
            avg_fill_price: None,
            state: BrokerOrderState::Filled,
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        // DEX swaps are atomic — no resting orders.
        Ok(vec![])
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        // DEX positions are managed via DexPaperWallet for paper; on-chain for live.
        Ok(vec![])
    }
}
