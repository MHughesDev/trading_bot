//! AMM/DEX paper swap simulator — used for DEX spot (CryptoSpotDex / 0x).
//!
//! Phase 1 skeleton: takes a `FirmQuote` input and returns the quoted
//! out-amount.  Real 0x HTTP wiring lands in Phase 4.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, Side},
};

use super::{wallet::InsufficientBalance, DexPaperWallet, PaperFill, PaperFillSimulator};

/// A firm price quote from a DEX aggregator.
///
/// In Phase 1 this is caller-supplied (mocked); Phase 4 populates it from
/// the 0x Swap API `/price` endpoint.
#[derive(Debug, Clone)]
pub struct FirmQuote {
    /// Quoted output amount for the swap (in base asset for buys).
    pub out_amount: Decimal,
    /// Effective price (out/in ratio expressed as a `Price`).
    pub effective_price: Price,
    /// Estimated gas/protocol fee in USD.
    pub fee_usd: Decimal,
}

/// AMM/DEX paper fill simulator skeleton.
#[derive(Debug, Clone, Default)]
pub struct AmmQuoteSwapSimulator {
    /// Simulated price impact in basis points (applied when no firm quote given).
    pub price_impact_bps: Decimal,
}

impl AmmQuoteSwapSimulator {
    pub fn new(price_impact_bps: Decimal) -> Self {
        Self { price_impact_bps }
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
    /// Simulate without a firm quote by applying a configurable price impact.
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let m = mark.inner();
        let impact = m * self.price_impact_bps / dec!(10000);
        let fill_price_raw = match intent.side {
            Side::Buy => m + impact,
            Side::Sell => m - impact,
        };
        let fill_price = Price::from_decimal(fill_price_raw.max(Decimal::ZERO));
        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: intent.size.inner(),
            fill_price,
            fee: Decimal::ZERO,
        }
    }
}
