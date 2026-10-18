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
    // Held for the on-chain submit path (Set E 3.4, BLOCKED:HUMAN); `submit` no
    // longer sends a request, so nothing reads them yet.
    #[allow(dead_code)]
    client: Client,
    #[allow(dead_code)]
    api_key: String,
    pub base_url: String,
    /// ERC-20 token decimal places by symbol. Defaults to 18 for unlisted tokens.
    token_decimals: HashMap<String, u32>,
    /// Orders the broker has been told about: broker_order_id -> (instrument_id,
    /// side, qty). `submit` refuses (Set E 3.4), so this stays empty until the
    /// on-chain path exists; `query_order` reads it.
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
}

/// Mainnet token addresses for the symbols the platform quotes. 0x v2 takes
/// addresses, not symbols. `ETH` is 0x's native-token sentinel.
pub fn default_token_addresses() -> HashMap<String, String> {
    [
        ("ETH", "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE"),
        ("WETH", "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
        ("USDC", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        ("USDT", "0xdAC17F958D2ee523a2206206994597C13D831ec7"),
        ("DAI", "0x6B175474E89094C44Da98b954EedeAC495271d0F"),
        ("WBTC", "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect()
}

/// A parsed 0x `/swap/permit2/price` answer, in human units.
#[derive(Debug, Clone, PartialEq)]
pub struct ZeroXPrice {
    /// Amount of the buy token received, in whole tokens.
    pub buy_amount: Decimal,
    /// Amount of the sell token given, in whole tokens.
    pub sell_amount: Decimal,
    /// Estimated network fee in the chain's native token (`gas x gasPrice`).
    pub network_fee_native: Decimal,
}

fn pow10(decimals: u32) -> Result<Decimal, BrokerError> {
    if decimals > 18 {
        return Err(BrokerError::Rejected(format!(
            "token decimals {decimals} exceed 18; refusing to scale"
        )));
    }
    Ok(Decimal::from(10u64.pow(decimals)))
}

/// Parse a 0x v2 price response using the **real** decimals of each token (the
/// 0.2 fix: `10^18` mis-sized USDC/USDT by twelve orders of magnitude).
///
/// An answer with `liquidityAvailable: false`, or without a buy or sell amount,
/// is an error -- never a zero-price quote.
pub fn parse_price_response(
    body: &serde_json::Value,
    sell_decimals: u32,
    buy_decimals: u32,
) -> Result<ZeroXPrice, BrokerError> {
    use std::str::FromStr;
    let bad = |m: &str| BrokerError::Serialization(format!("0x price response: {m}"));
    if body["liquidityAvailable"] == serde_json::Value::Bool(false) {
        return Err(BrokerError::Rejected(
            "0x reports no liquidity for this pair and size".to_owned(),
        ));
    }
    let amount = |k: &str| -> Result<Decimal, BrokerError> {
        let raw = body[k]
            .as_str()
            .ok_or_else(|| bad(&format!("missing {k}")))?;
        Decimal::from_str(raw).map_err(|_| bad(&format!("{k} is not a number")))
    };
    let buy = amount("buyAmount")? / pow10(buy_decimals)?;
    let sell = amount("sellAmount")? / pow10(sell_decimals)?;
    if buy <= Decimal::ZERO || sell <= Decimal::ZERO {
        return Err(bad("a zero amount"));
    }
    // `totalNetworkFee` is gas x gasPrice in wei when present; otherwise compute it.
    let fee_wei = match body["totalNetworkFee"]
        .as_str()
        .and_then(|s| Decimal::from_str(s).ok())
    {
        Some(f) => f,
        None => {
            let gas = body["gas"].as_str().and_then(|s| Decimal::from_str(s).ok());
            let price = body["gasPrice"]
                .as_str()
                .and_then(|s| Decimal::from_str(s).ok());
            match (gas, price) {
                (Some(g), Some(p)) => g * p,
                _ => return Err(bad("no network fee (totalNetworkFee, or gas and gasPrice)")),
            }
        }
    };
    Ok(ZeroXPrice {
        buy_amount: buy,
        sell_amount: sell,
        network_fee_native: fee_wei / pow10(18)?,
    })
}

/// The pair and exact-input amount for a request.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceRequest {
    pub sell_symbol: String,
    pub buy_symbol: String,
    /// Exact input, in the sell token's smallest unit.
    pub sell_amount_units: String,
    pub sell_decimals: u32,
    pub buy_decimals: u32,
}

/// Read-only 0x quoting for the **paper** DEX path (Set E 3.3). It holds an API
/// key and nothing that can move funds: no signer, no wallet, no broadcast.
pub struct ZeroXQuoter {
    client: Client,
    api_key: String,
    pub base_url: String,
    pub chain_id: u64,
    token_decimals: HashMap<String, u32>,
    token_addresses: HashMap<String, String>,
}

impl ZeroXQuoter {
    pub fn new(api_key: impl Into<String>) -> Self {
        let mut token_decimals = HashMap::new();
        for (s, d) in [
            ("USDC", 6u32),
            ("USDT", 6),
            ("WBTC", 8),
            ("WETH", 18),
            ("ETH", 18),
            ("DAI", 18),
        ] {
            token_decimals.insert(s.to_owned(), d);
        }
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: BASE_URL.to_owned(),
            chain_id: 1,
            token_decimals,
            token_addresses: default_token_addresses(),
        }
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    #[must_use]
    pub fn with_token(mut self, symbol: &str, address: &str, decimals: u32) -> Self {
        self.token_decimals.insert(symbol.to_owned(), decimals);
        self.token_addresses
            .insert(symbol.to_owned(), address.to_owned());
        self
    }

    fn decimals(&self, symbol: &str) -> Result<u32, BrokerError> {
        self.token_decimals.get(symbol).copied().ok_or_else(|| {
            BrokerError::Rejected(format!(
                "no decimals known for token {symbol}; refusing to default to 18"
            ))
        })
    }

    /// Build the request for one order. Pure.
    ///
    /// A **sell** gives `size` of the base token. A **buy** wants `size` of the
    /// base, but the price endpoint is exact-input, so the quote-token input is
    /// sized as `size x reference_price`; the answer's `buyAmount` is what that
    /// input actually buys, and the effective price falls out of the ratio.
    pub fn price_request(
        &self,
        instrument_id: &str,
        side: Side,
        size: Decimal,
        reference_price: Decimal,
    ) -> Result<PriceRequest, BrokerError> {
        let (base, quote) = instrument_id
            .split_once('-')
            .filter(|(b, q)| !b.is_empty() && !q.is_empty())
            .ok_or_else(|| {
                BrokerError::Rejected("instrument_id must be BASE-QUOTE format for 0x".to_owned())
            })?;
        if size <= Decimal::ZERO || reference_price <= Decimal::ZERO {
            return Err(BrokerError::Rejected(
                "size and reference price must be positive".to_owned(),
            ));
        }
        let (sell, buy, sell_human) = match side {
            Side::Sell => (base, quote, size),
            Side::Buy => (quote, base, size * reference_price),
        };
        let sell_decimals = self.decimals(sell)?;
        let buy_decimals = self.decimals(buy)?;
        let units = (sell_human * pow10(sell_decimals)?).round();
        Ok(PriceRequest {
            sell_symbol: sell.to_owned(),
            buy_symbol: buy.to_owned(),
            sell_amount_units: units.to_string(),
            sell_decimals,
            buy_decimals,
        })
    }

    /// A real indicative quote for `intent`, shaped as the paper simulator's
    /// [`FirmQuote`](crate::paper::amm_swap::FirmQuote) (Set E 3.3). Paper only:
    /// this type holds no signer. `reference_price` sizes a buy's quote-token
    /// input (the endpoint is exact-input); `native_usd` converts the network
    /// fee. On any failure the caller falls back to the bps model,
    /// `AmmQuoteSwapSimulator::simulate_fill`.
    ///
    /// # Errors
    /// An unknown token, an HTTP failure or an unusable answer.
    pub async fn fetch_quote(
        &self,
        intent: &domain::order::OrderIntent,
        reference_price: domain::money::Price,
        native_usd: Decimal,
    ) -> Result<crate::paper::amm_swap::FirmQuote, BrokerError> {
        let (_, price) = self
            .fetch_price(
                &intent.instrument_id,
                intent.side,
                intent.size.inner(),
                reference_price.inner(),
            )
            .await?;
        Ok(crate::paper::amm_swap::firm_quote_from_price(
            &price,
            intent.side,
            native_usd,
        ))
    }

    /// `GET /swap/permit2/price` for one order, parsed with real decimals.
    ///
    /// # Errors
    /// An unknown token, an HTTP failure, or an unusable answer.
    pub async fn fetch_price(
        &self,
        instrument_id: &str,
        side: Side,
        size: Decimal,
        reference_price: Decimal,
    ) -> Result<(PriceRequest, ZeroXPrice), BrokerError> {
        let req = self.price_request(instrument_id, side, size, reference_price)?;
        let addr = |sym: &str| {
            self.token_addresses.get(sym).cloned().ok_or_else(|| {
                BrokerError::Rejected(format!("no contract address known for token {sym}"))
            })
        };
        let (sell_addr, buy_addr) = (addr(&req.sell_symbol)?, addr(&req.buy_symbol)?);
        let mut headers = header::HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&self.api_key) {
            headers.insert("0x-api-key", v);
        }
        headers.insert("0x-version", header::HeaderValue::from_static("v2"));
        let resp = self
            .client
            .get(format!("{}/swap/permit2/price", self.base_url))
            .headers(headers)
            .query(&[
                ("chainId", self.chain_id.to_string()),
                ("sellToken", sell_addr),
                ("buyToken", buy_addr),
                ("sellAmount", req.sell_amount_units.clone()),
            ])
            .send()
            .await
            .map_err(|e| BrokerError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(BrokerError::Rejected(resp.text().await.unwrap_or_default()));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| BrokerError::Serialization(e.to_string()))?;
        let price = parse_price_response(&body, req.sell_decimals, req.buy_decimals)?;
        Ok((req, price))
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
    async fn submit(&self, _order: &ApprovedOrder) -> Result<String, BrokerError> {
        // This used to fetch a quote and return `0x:{quote.to}` -- the 0x
        // *contract address*, identical for every order -- as a broker order id,
        // claiming success for a swap that was never broadcast (Set E audit #10).
        //
        // A real submit signs the swap with a wallet key and broadcasts it
        // through a chain RPC; neither exists in this build, and key handling is
        // gated on a `/security-review` (Set E 3.4, BLOCKED:HUMAN). Until then
        // this refuses, so an order routed here fails loudly instead of being
        // recorded as placed. The paper path never reaches this adapter; it
        // quotes through `ZeroXQuoter`.
        Err(BrokerError::Rejected(
            "0x on-chain submit is not available: no signer wallet or chain RPC is configured \
             (Set E 3.4 is blocked on key-handling review); nothing was broadcast"
                .to_owned(),
        ))
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
            // This adapter does not report fees on an order query;
            // `None`, never zero -- see BrokerOrderStatus::commission.
            commission: None,
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
