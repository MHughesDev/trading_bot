//! Per-asset-class paper account policies.
//!
//! Each asset class gets an ad hoc account model that matches how that class
//! actually trades — cash settlement for spot, leveraged margin for
//! derivatives, premium × multiplier for options, and binary payout for
//! prediction markets.  Everything here is data: adding or tuning a class is
//! a table edit, not a code change elsewhere.

use domain::instrument::AssetClass;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// All asset classes the paper engine opens an account for at startup.
pub const ALL_ASSET_CLASSES: [AssetClass; 11] = [
    AssetClass::CryptoSpotCex,
    AssetClass::Equity,
    AssetClass::Etf,
    AssetClass::CryptoSpotDex,
    AssetClass::FuturesExpiring,
    AssetClass::PerpetualSwap,
    AssetClass::Option,
    AssetClass::Bond,
    AssetClass::Fx,
    AssetClass::Nft,
    AssetClass::PredictionMarket,
];

/// How cash and positions interact for an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountKind {
    /// Fully-funded cash account: buys debit cash up front, sells require the
    /// position to exist (long-only).  Equity = cash + market value of holdings.
    Cash,
    /// Leveraged margin account: long and short allowed; opening exposure
    /// reserves `notional / leverage`; realized P&L settles to cash on reduce.
    Margin,
    /// Binary prediction contracts priced in [0, 1]: buys debit `price × qty`,
    /// settlement pays 1 per contract on the winning side, 0 otherwise.
    Binary,
}

/// Ad hoc paper account policy for one asset class.
#[derive(Clone, Debug)]
pub struct AccountPolicy {
    pub kind: AccountKind,
    /// Maximum gross leverage (`Margin` accounts only; 1 elsewhere).
    pub leverage: Decimal,
    /// Cash value of one unit of quantity per point of price
    /// (100 for US equity options; 1 elsewhere).
    pub contract_multiplier: Decimal,
    /// Currency the account's cash balance is denominated in.
    pub quote_currency: &'static str,
    /// Cash the account is seeded with at engine startup.
    pub default_starting_cash: Decimal,
    /// Report balances wallet-style (one row per token held) instead of a
    /// single cash row — used for DEX and NFT accounts.
    pub token_balances: bool,
}

impl AccountPolicy {
    /// The policy table — one ad hoc model per asset class.
    pub fn for_asset_class(asset_class: AssetClass) -> Self {
        match asset_class {
            AssetClass::CryptoSpotCex => Self::cash("USD", dec!(100_000), false),
            AssetClass::Equity | AssetClass::Etf | AssetClass::Bond => {
                Self::cash("USD", dec!(100_000), false)
            }
            // DEX spot settles token-vs-token; cash is the quote stablecoin.
            AssetClass::CryptoSpotDex => Self::cash("USDC", dec!(100_000), true),
            // NFT trades are quoted in ETH, not USD.
            AssetClass::Nft => Self::cash("ETH", dec!(100), true),
            AssetClass::FuturesExpiring | AssetClass::PerpetualSwap => {
                Self::margin("USD", dec!(100_000), dec!(10))
            }
            AssetClass::Fx => Self::margin("USD", dec!(100_000), dec!(30)),
            AssetClass::Option => Self {
                contract_multiplier: dec!(100),
                ..Self::cash("USD", dec!(100_000), false)
            },
            AssetClass::PredictionMarket => Self {
                kind: AccountKind::Binary,
                ..Self::cash("USD", dec!(10_000), false)
            },
        }
    }

    fn cash(quote: &'static str, starting: Decimal, tokens: bool) -> Self {
        Self {
            kind: AccountKind::Cash,
            leverage: Decimal::ONE,
            contract_multiplier: Decimal::ONE,
            quote_currency: quote,
            default_starting_cash: starting,
            token_balances: tokens,
        }
    }

    fn margin(quote: &'static str, starting: Decimal, leverage: Decimal) -> Self {
        Self {
            kind: AccountKind::Margin,
            leverage,
            contract_multiplier: Decimal::ONE,
            quote_currency: quote,
            default_starting_cash: starting,
            token_balances: false,
        }
    }
}

/// Base token of an instrument id like `"WETH-USDC"` or `"BTC/USD"`.
/// Falls back to the whole id when no separator is present.
pub fn base_token(instrument_id: &str) -> &str {
    instrument_id
        .split(['-', '/'])
        .next()
        .unwrap_or(instrument_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_class_has_a_policy() {
        for ac in ALL_ASSET_CLASSES {
            let p = AccountPolicy::for_asset_class(ac);
            assert!(
                p.default_starting_cash > Decimal::ZERO,
                "{ac:?} must seed cash"
            );
            assert!(p.leverage >= Decimal::ONE, "{ac:?} leverage must be >= 1");
        }
    }

    #[test]
    fn derivatives_are_margin_accounts() {
        for ac in [
            AssetClass::FuturesExpiring,
            AssetClass::PerpetualSwap,
            AssetClass::Fx,
        ] {
            let p = AccountPolicy::for_asset_class(ac);
            assert_eq!(p.kind, AccountKind::Margin);
            assert!(p.leverage > Decimal::ONE);
        }
    }

    #[test]
    fn options_carry_contract_multiplier() {
        let p = AccountPolicy::for_asset_class(AssetClass::Option);
        assert_eq!(p.contract_multiplier, dec!(100));
        assert_eq!(p.kind, AccountKind::Cash);
    }

    #[test]
    fn prediction_market_is_binary() {
        let p = AccountPolicy::for_asset_class(AssetClass::PredictionMarket);
        assert_eq!(p.kind, AccountKind::Binary);
    }

    #[test]
    fn base_token_parses_common_separators() {
        assert_eq!(base_token("WETH-USDC"), "WETH");
        assert_eq!(base_token("BTC/USD"), "BTC");
        assert_eq!(base_token("CRYPTOPUNK1234"), "CRYPTOPUNK1234");
    }
}
