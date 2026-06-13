//! 0x Swap API broker adapter — DEX swap execution via firm quote.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use domain::order::{OrderType, Side};
use reqwest::{header, Client};
use risk::ApprovedOrder;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

pub const BASE_URL: &str = "https://api.0x.org";

/// In-flight order metadata: (instrument_id, side, qty).
type PendingEntry = (String, Side, Decimal);

pub struct ZeroXBroker {
    client: Client,
    api_key: String,
    pub base_url: String,
    /// ERC-20 token decimal places by symbol. Defaults to 18 for unlisted tokens.
    token_decimals: HashMap<String, u32>,
    /// Submitted orders: broker_order_id → (instrument_id, side, qty).
    /// Populated by `submit`; read by `query_order` until on-chain poll (task 3.4).
    pending: Arc<Mutex<HashMap<String, PendingEntry>>>,
}

impl ZeroXBroker {
    pub fn new(api_key: impl Into<String>) -> Self {
        let mut token_decimals = HashMap::new();
        // Common ERC-20 tokens with non-18 decimal precision.
        token_decimals.insert("USDC".to_owned(), 6u32);
        token_decimals.insert("USDT".to_owned(), 6u32);
        token_decimals.insert("WBTC".to_owned(), 8u32);
        token_decimals.insert("WETH".to_owned(), 18u32);
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: BASE_URL.to_owned(),
            token_decimals,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Override token decimal precision (e.g. from the instrument registry).
    pub fn with_token_decimals(mut self, decimals: HashMap<String, u32>) -> Self {
        self.token_decimals.extend(decimals);
        self
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

        let decimals = self.token_decimals.get(sell_token).copied().unwrap_or(18);
        let sell_amount = (order.intent.size.inner() * Decimal::from(10u64.pow(decimals)))
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

        // Store order metadata so query_order can return accurate fields.
        let broker_id = format!("0x:{}", quote.to);
        if let Ok(mut map) = self.pending.lock() {
            map.insert(
                broker_id.clone(),
                (
                    order.intent.instrument_id.clone(),
                    order.intent.side,
                    order.intent.size.inner(),
                ),
            );
        }
        Ok(broker_id)
    }

    async fn cancel(&self, _broker_order_id: &str) -> Result<(), BrokerError> {
        // DEX swaps are atomic and cannot be cancelled once broadcast.
        Err(BrokerError::Rejected(
            "DEX swaps cannot be cancelled after broadcast".to_owned(),
        ))
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        // Full on-chain receipt polling is implemented in task 3.4.
        // Return Unknown rather than fabricating a Filled status.
        let info = self
            .pending
            .lock()
            .ok()
            .and_then(|m| m.get(broker_order_id).cloned());

        let (instrument_id, side, qty) =
            info.unwrap_or_else(|| (String::new(), Side::Buy, Decimal::ZERO));

        Ok(BrokerOrderStatus {
            broker_order_id: broker_order_id.to_owned(),
            instrument_id,
            side,
            order_type: OrderType::Market,
            submitted_qty: qty,
            filled_qty: Decimal::ZERO,
            avg_fill_price: None,
            state: BrokerOrderState::Unknown(
                "on-chain receipt poll not yet implemented (task 3.4)".to_owned(),
            ),
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
