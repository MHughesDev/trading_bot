//! 0x swap aggregator DEX quote-snapshot collector (P2-T09).
//!
//! Polls `GET /swap/v1/quote` for configured token pairs and emits
//! firm-quote snapshots as `EventEnvelope<DexQuotePayload>` with
//! `DataType::DexQuote` / `AssetClass::CryptoSpotDex`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    event_id_from_key,
    money::{Price, Size},
    payloads::dex_quote::DexQuotePayload,
    sequenced_key, EventEnvelope, NormalizeError, TrustTier,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

use crate::{Collector, CollectorError};

const ZEROX_REST_BASE: &str = "https://api.0x.org/swap/v1";
const VENUE_ID: &str = "zerox";
const SOURCE: &str = "zerox_rest";

// ── 0x response shapes ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZeroXQuoteResponse {
    sell_token_address: Option<String>,
    buy_token_address: Option<String>,
    sell_amount: String,
    buy_amount: String,
    price: String,
    estimated_gas: Option<String>,
}

// ── Collector ────────────────────────────────────────────────────────────────

/// 0x DEX quote collector for a single token pair.
pub struct ZeroXCollector {
    /// Sell token symbol / address, e.g. `"WETH"`.
    pub sell_token: String,
    /// Buy token symbol / address, e.g. `"USDC"`.
    pub buy_token: String,
    /// Amount of sell token to quote (in base units, e.g. `"1000000000000000000"` for 1 WETH).
    pub sell_amount: String,
    /// Domain instrument ID: `"{sell_token}-{buy_token}"`.
    pub instrument_id: String,
    /// 0x API key (optional, rate limits apply without it).
    pub chain_id: u32,
}

impl ZeroXCollector {
    pub fn new(
        sell_token: impl Into<String>,
        buy_token: impl Into<String>,
        sell_amount: impl Into<String>,
    ) -> Self {
        let sell_token = sell_token.into();
        let buy_token = buy_token.into();
        let instrument_id = format!("{sell_token}-{buy_token}");
        Self {
            sell_token,
            buy_token,
            sell_amount: sell_amount.into(),
            instrument_id,
            chain_id: 1, // Ethereum mainnet
        }
    }

    /// Normalize a 0x quote response into a `DexQuotePayload` envelope.
    pub fn normalize_quote(
        &self,
        response: &ZeroXQuoteResponse,
        seq: u64,
    ) -> Result<EventEnvelope<DexQuotePayload>, NormalizeError> {
        let sell_amount = Decimal::from_str(&response.sell_amount)
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "sellAmount".to_owned(),
                reason: e.to_string(),
            })?;

        let buy_amount = Decimal::from_str(&response.buy_amount)
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "buyAmount".to_owned(),
                reason: e.to_string(),
            })?;

        let price = Decimal::from_str(&response.price)
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "price".to_owned(),
                reason: e.to_string(),
            })?;

        let sell_token = response
            .sell_token_address
            .clone()
            .unwrap_or_else(|| self.sell_token.clone());
        let buy_token = response
            .buy_token_address
            .clone()
            .unwrap_or_else(|| self.buy_token.clone());

        let payload = DexQuotePayload::new(
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            price,
            response.estimated_gas.clone(),
        );

        let dedup = sequenced_key("dex.quote", &self.instrument_id, VENUE_ID, seq, SOURCE);
        let event_id = event_id_from_key(&dedup);
        let now = Utc::now();

        Ok(EventEnvelope::new(
            event_id,
            "dex.quote",
            &self.instrument_id,
            VENUE_ID,
            SOURCE,
            TrustTier::OnchainConfirmed,
            None,
            now,
            now,
            now,
            seq,
            payload,
        ))
    }
}

#[async_trait]
impl Collector for ZeroXCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let api_key = std::env::var("ZEROX_API_KEY").unwrap_or_default();
        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(
            sell_token = %self.sell_token,
            buy_token = %self.buy_token,
            "0x DEX quote collector starting"
        );

        loop {
            let url = format!(
                "{ZEROX_REST_BASE}/quote?sellToken={}&buyToken={}&sellAmount={}&chainId={}",
                self.sell_token, self.buy_token, self.sell_amount, self.chain_id
            );

            let result = client.get(&url).header("0x-api-key", &api_key).send().await;

            match result {
                Err(e) => {
                    warn!(error = %e, instrument = %self.instrument_id, "0x REST request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    match serde_json::from_slice::<ZeroXQuoteResponse>(&raw) {
                        Err(e) => {
                            let norm_err = NormalizeError::Deserialize(e.to_string());
                            if let Err(qe) =
                                quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                            {
                                warn!(error = %qe, "quarantine publish failed");
                            }
                        }
                        Ok(quote) => {
                            seq += 1;
                            let result = self.normalize_quote(&quote, seq);
                            crate::normalizer::quarantine_or_publish(
                                result,
                                &raw,
                                &self.instrument_id,
                                SOURCE,
                                &publisher,
                                &quarantine,
                            )
                            .await;
                        }
                    }
                }
            }

            // Poll every 30 seconds — 0x quotes are stateless and cheap.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote() -> ZeroXQuoteResponse {
        ZeroXQuoteResponse {
            sell_token_address: Some("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".to_owned()),
            buy_token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_owned()),
            sell_amount: "1000000000000000000".to_owned(),
            buy_amount: "2500000000".to_owned(),
            price: "2500".to_owned(),
            estimated_gas: Some("120000".to_owned()),
        }
    }

    #[test]
    fn normalize_quote_produces_correct_amounts_and_price() {
        let collector = ZeroXCollector::new("WETH", "USDC", "1000000000000000000");
        let result = collector.normalize_quote(&sample_quote(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(env.instrument_id, "WETH-USDC");
        assert_eq!(env.trust_tier, TrustTier::OnchainConfirmed);
        let payload = &env.payload;
        assert_eq!(payload.price.to_string(), "2500");
        assert!(payload.estimated_gas.is_some());
    }

    #[test]
    fn normalize_quote_invalid_price_returns_error() {
        let collector = ZeroXCollector::new("WETH", "USDC", "1000000000000000000");
        let bad_quote = ZeroXQuoteResponse {
            sell_token_address: None,
            buy_token_address: None,
            sell_amount: "1000".to_owned(),
            buy_amount: "2500".to_owned(),
            price: "not_a_number".to_owned(),
            estimated_gas: None,
        };
        assert!(collector.normalize_quote(&bad_quote, 1).is_err());
    }

    #[test]
    fn instrument_id_derived_from_tokens() {
        let c = ZeroXCollector::new("WETH", "DAI", "1000");
        assert_eq!(c.instrument_id, "WETH-DAI");
    }
}
