//! AMM/DEX paper swap simulator — used for DEX spot (CryptoSpotDex / 0x).
//!
//! Fills against a `FirmQuote`. A quote can come from the caller, or from the
//! real 0x `/swap/permit2/price` endpoint through
//! `ZeroXQuoter::fetch_quote` (Set E 3.3), whose answer [`firm_quote_from_price`]
//! shapes. The network call lives in `venues::zerox`, not here: this file is the
//! venue core and takes prices from what it is handed. The bps model in
//! [`PaperFillSimulator::simulate_fill`] is the offline / backtest fallback and
//! needs no network.
//!
//! Default configuration models a typical Uniswap v2-style pool:
//! - 30 bps price impact (linear approximation of x·y=k curve)
//! - 30 bps protocol fee (Uniswap v2 0.3%)
//! - $2 flat gas/network fee per swap
//!
//! NFT trades use a separate override in `SimulatorSet::realistic()` that
//! applies a higher platform + royalty fee (250 bps) and higher gas ($15).

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, Side},
};

use super::{wallet::InsufficientBalance, DexPaperWallet, PaperFill, PaperFillSimulator};

/// A firm price quote from a DEX aggregator.
///
/// Caller-supplied, or built from a real 0x price answer by
/// [`firm_quote_from_price`].
#[derive(Debug, Clone)]
pub struct FirmQuote {
    /// Quoted output amount for the swap (in base asset for buys).
    pub out_amount: Decimal,
    /// Effective price (out/in ratio expressed as a `Price`).
    pub effective_price: Price,
    /// Estimated gas/protocol fee in USD.
    pub fee_usd: Decimal,
}

/// AMM/DEX paper fill simulator.
#[derive(Debug, Clone)]
pub struct AmmQuoteSwapSimulator {
    /// Simulated price impact in basis points (linear approximation of x·y=k).
    pub price_impact_bps: Decimal,
    /// AMM protocol fee in basis points charged on the trade notional
    /// (e.g. 30 = Uniswap v2 0.3%, 5 = Uniswap v3 stable pool 0.05%).
    pub protocol_fee_bps: Decimal,
    /// Flat network/gas fee per swap, denominated in the account's quote
    /// currency (USD for most DEX accounts; ETH for NFT accounts).
    /// Ethereum mainnet: ~$1–$50; L2s are much cheaper.
    pub flat_fee: Decimal,
}

impl Default for AmmQuoteSwapSimulator {
    fn default() -> Self {
        Self {
            price_impact_bps: dec!(30), // 0.3% impact — mid-size Uniswap v2 swap
            protocol_fee_bps: dec!(30), // Uniswap v2 standard 0.3% pool fee
            flat_fee: dec!(2),          // ~$2 gas (Ethereum moderate congestion)
        }
    }
}

/// Build a [`FirmQuote`] from a parsed 0x price. Pure.
///
/// `native_usd` is the USD price of the chain's native token (ETH on mainnet),
/// supplied by the caller from its own mark board -- 0x reports the fee in the
/// native token, and a USD figure invented here would be a fabricated cost.
///
/// For a **sell** of the base, `out_amount` is the quote token received and the
/// price is quote per base; for a **buy**, `out_amount` is the base received
/// and the price is quote spent per base received.
#[must_use]
pub fn firm_quote_from_price(
    price: &crate::venues::zerox::ZeroXPrice,
    side: Side,
    native_usd: Decimal,
) -> FirmQuote {
    let (out_amount, per_base) = match side {
        Side::Sell => (price.buy_amount, price.buy_amount / price.sell_amount),
        Side::Buy => (price.buy_amount, price.sell_amount / price.buy_amount),
    };
    FirmQuote {
        out_amount,
        effective_price: Price::from_decimal(per_base),
        fee_usd: price.network_fee_native * native_usd,
    }
}

impl AmmQuoteSwapSimulator {
    pub fn new(price_impact_bps: Decimal) -> Self {
        Self {
            price_impact_bps,
            ..Self::default()
        }
    }

    /// Fill against a firm quote.  Returns a `PaperFill` with the quoted price.
    pub fn simulate_from_quote(&self, intent: &OrderIntent, quote: &FirmQuote) -> PaperFill {
        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: intent.size.inner(),
            fill_price: quote.effective_price,
            fee: quote.fee_usd,
        }
    }

    /// Fill against a firm quote, debiting `in_token` and crediting `out_token`
    /// in the `DexPaperWallet`.  Returns `Err` if the wallet has insufficient balance.
    pub fn simulate_with_wallet(
        &self,
        intent: &OrderIntent,
        quote: &FirmQuote,
        wallet: &mut DexPaperWallet,
        in_token: &str,
        out_token: &str,
    ) -> Result<PaperFill, InsufficientBalance> {
        let in_amount = intent.size.inner();
        wallet.apply_swap(in_token, in_amount, out_token, quote.out_amount)?;
        Ok(self.simulate_from_quote(intent, quote))
    }
}

impl PaperFillSimulator for AmmQuoteSwapSimulator {
    /// Simulate without a firm quote: apply price impact, protocol fee, and gas.
    ///
    /// Fee = notional × protocol_fee_bps / 10_000 + gas_fee_usd.
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let m = mark.inner();
        let impact = m * self.price_impact_bps / dec!(10000);
        let fill_price_raw = match intent.side {
            Side::Buy => m + impact,
            Side::Sell => m - impact,
        };
        let fill_price = Price::from_decimal(fill_price_raw.max(Decimal::ZERO));
        let notional = intent.size.inner() * fill_price.inner();
        let fee = notional * self.protocol_fee_bps / dec!(10000) + self.flat_fee;
        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: intent.size.inner(),
            fill_price,
            fee,
        }
    }
}

#[cfg(test)]
mod quote_tests {
    use super::*;
    use crate::venues::zerox::{parse_price_response, ZeroXPrice};
    use serde_json::json;

    #[test]
    fn a_usdc_sell_uses_six_decimals_not_eighteen() {
        // Sell 1 WETH (18 dp) for 2,500 USDC (6 dp).
        let body = json!({
            "sellAmount": "1000000000000000000",
            "buyAmount": "2500000000",
            "totalNetworkFee": "6000000000000000",
            "liquidityAvailable": true
        });
        let p = parse_price_response(&body, 18, 6).unwrap();
        assert_eq!(p.sell_amount, Decimal::ONE);
        assert_eq!(p.buy_amount, Decimal::from(2500));
        assert_eq!(p.network_fee_native, dec!(0.006));
        let q = firm_quote_from_price(&p, Side::Sell, dec!(2500));
        assert_eq!(q.effective_price.inner(), Decimal::from(2500));
        assert_eq!(q.out_amount, Decimal::from(2500));
        assert_eq!(q.fee_usd, Decimal::from(15), "0.006 ETH x $2500");
    }

    #[test]
    fn a_buy_prices_the_quote_token_spent_per_base_received() {
        // Spend 2,550 USDC, receive 1 WETH.
        let p = ZeroXPrice {
            buy_amount: Decimal::ONE,
            sell_amount: Decimal::from(2550),
            network_fee_native: Decimal::ZERO,
        };
        let q = firm_quote_from_price(&p, Side::Buy, dec!(2500));
        assert_eq!(q.effective_price.inner(), Decimal::from(2550));
        assert_eq!(q.out_amount, Decimal::ONE);
    }

    #[test]
    fn an_unusable_answer_is_an_error_not_a_zero_price() {
        assert!(parse_price_response(&json!({"liquidityAvailable": false}), 18, 6).is_err());
        assert!(parse_price_response(&json!({"sellAmount": "1", "buyAmount": "0", "totalNetworkFee": "0"}), 18, 6).is_err());
        assert!(parse_price_response(&json!({"sellAmount": "1", "buyAmount": "2"}), 18, 6).is_err(), "no fee");
        assert!(parse_price_response(&json!({"sellAmount": "1", "buyAmount": "2", "totalNetworkFee": "0"}), 19, 6).is_err());
    }

    #[test]
    fn the_offline_fallback_still_fills_with_no_network() {
        let sim = AmmQuoteSwapSimulator::default();
        let intent = OrderIntent::new(
            "WETH-USDC",
            Side::Buy,
            domain::order::OrderType::Market,
            domain::money::Size::from_decimal(Decimal::ONE),
            None,
            None,
        );
        let fill = sim.simulate_fill(&intent, Price::from_decimal(Decimal::from(2500)));
        assert!(fill.fill_price.inner() > Decimal::from(2500), "a buy pays impact");
        assert!(fill.fee >= dec!(2));
    }
}
