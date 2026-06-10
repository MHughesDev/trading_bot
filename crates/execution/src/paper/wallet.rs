//! DEX paper wallet — tracks simulated token balances for `CryptoSpotDex` paper
//! trading.  Credits/debits on every swap fill; never calls an external chain.

use std::collections::HashMap;

use rust_decimal::Decimal;

/// Simulated token balances for a DEX paper account.
///
/// Keys are token symbols (e.g. `"USDC"`, `"WETH"`).
#[derive(Debug, Default, Clone)]
pub struct DexPaperWallet {
    balances: HashMap<String, Decimal>,
}

impl DexPaperWallet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set an initial balance for `token`.
    pub fn seed(&mut self, token: impl Into<String>, amount: Decimal) {
        self.balances.insert(token.into(), amount);
    }

    /// Return the current balance of `token` (0 if not tracked).
    pub fn balance(&self, token: &str) -> Decimal {
        self.balances.get(token).copied().unwrap_or(Decimal::ZERO)
    }

    /// Debit `amount` from `token`.  Returns `Err` if insufficient balance.
    pub fn debit(&mut self, token: &str, amount: Decimal) -> Result<(), InsufficientBalance> {
        let bal = self.balances.entry(token.to_owned()).or_default();
        if *bal < amount {
            return Err(InsufficientBalance {
                token: token.to_owned(),
                available: *bal,
                required: amount,
            });
        }
        *bal -= amount;
        Ok(())
    }

    /// Credit `amount` to `token`.
    pub fn credit(&mut self, token: impl Into<String>, amount: Decimal) {
        *self.balances.entry(token.into()).or_default() += amount;
    }

    /// Apply a swap fill: debit `in_token`, credit `out_token`.
    pub fn apply_swap(
        &mut self,
        in_token: &str,
        in_amount: Decimal,
        out_token: &str,
        out_amount: Decimal,
    ) -> Result<(), InsufficientBalance> {
        self.debit(in_token, in_amount)?;
        self.credit(out_token, out_amount);
        Ok(())
    }
}

/// Error when a debit would exceed the available balance.
#[derive(Debug, thiserror::Error)]
#[error("insufficient balance of {token}: have {available}, need {required}")]
pub struct InsufficientBalance {
    pub token: String,
    pub available: Decimal,
    pub required: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn swap_debits_in_token_and_credits_out_token() {
        let mut wallet = DexPaperWallet::new();
        wallet.seed("USDC", dec!(1000));
        wallet
            .apply_swap("USDC", dec!(500), "WETH", dec!(0.2))
            .unwrap();
        assert_eq!(wallet.balance("USDC"), dec!(500));
        assert_eq!(wallet.balance("WETH"), dec!(0.2));
    }

    #[test]
    fn debit_below_zero_returns_error() {
        let mut wallet = DexPaperWallet::new();
        wallet.seed("USDC", dec!(100));
        let result = wallet.debit("USDC", dec!(200));
        assert!(result.is_err());
    }
}
