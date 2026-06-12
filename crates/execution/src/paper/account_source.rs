//! `PaperAccountSource` — serves balances, positions, and transaction history
//! for the dashboard **entirely from the internal paper engine**.
//!
//! This is the internal counterpart of the per-venue REST adapters in
//! [`crate::account`]: same [`AccountSource`] trait, zero network calls, no
//! credentials required (the credential argument is ignored).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

use domain::money::{Price, Size};
use rust_decimal::Decimal;

use crate::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

use super::engine::PaperTradingEngine;
use super::policy::{base_token, AccountPolicy, ALL_ASSET_CLASSES};

/// On-demand account data straight from the internal paper engine.
pub struct PaperAccountSource {
    engine: Arc<PaperTradingEngine>,
}

impl PaperAccountSource {
    pub fn new(engine: Arc<PaperTradingEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl AccountSource for PaperAccountSource {
    fn venue_id(&self) -> &str {
        "paper"
    }

    /// One cash row per asset-class account, plus per-token rows for
    /// wallet-style accounts (DEX, NFT).  Margin accounts report free
    /// collateral as available and reserved margin as locked.
    async fn fetch_balances(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        let mut balances = Vec::new();
        for snap in self.engine.snapshots() {
            let (available, locked) = if snap.used_margin > Decimal::ZERO {
                (snap.free_collateral, snap.used_margin)
            } else {
                (snap.cash, Decimal::ZERO)
            };
            balances.push(Balance {
                asset: format!("{} ({})", snap.currency, snap.asset_class.as_str()),
                available: Size::from_decimal(available),
                locked: Size::from_decimal(locked),
                usd_value: (snap.currency == "USD").then(|| Price::from_decimal(snap.equity)),
            });

            // Wallet-style accounts additionally list each token held.
            if AccountPolicy::for_asset_class(snap.asset_class).token_balances {
                for pos in &snap.positions {
                    balances.push(Balance {
                        asset: base_token(&pos.instrument_id).to_owned(),
                        available: Size::from_decimal(pos.quantity),
                        locked: Size::from_decimal(Decimal::ZERO),
                        usd_value: None,
                    });
                }
            }
        }
        Ok(balances)
    }

    /// Every open paper position across all asset classes.
    async fn fetch_positions(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        let mut positions = Vec::new();
        for snap in self.engine.snapshots() {
            let usd_quoted = snap.currency == "USD";
            for pos in snap.positions {
                positions.push(VenuePosition {
                    instrument_id: pos.instrument_id,
                    quantity: pos.quantity,
                    avg_entry_price: Price::from_decimal(pos.average_entry_price),
                    unrealized_pnl_usd: usd_quoted.then_some(pos.unrealized_pnl),
                });
            }
        }
        Ok(positions)
    }

    /// Internal journal entries across all paper accounts, newest data taken
    /// straight from the in-process ledgers.
    async fn fetch_transactions(
        &self,
        _creds: &VenueCredentials,
        _user_id: Uuid,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        let mut transactions = Vec::new();
        for asset_class in ALL_ASSET_CLASSES {
            let currency = AccountPolicy::for_asset_class(asset_class).quote_currency;
            for entry in self.engine.transactions_since(asset_class, since) {
                transactions.push(VenueTransaction {
                    id: format!("paper-{}-{}", asset_class.as_str(), entry.seq),
                    transaction_type: entry.kind.as_str().to_owned(),
                    instrument_id: entry.instrument_id,
                    amount: entry.cash_delta,
                    currency,
                    occurred_at: entry.at,
                });
            }
        }
        transactions.sort_by_key(|t| t.occurred_at);
        Ok(transactions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        instrument::AssetClass,
        money::Size,
        order::{OrderIntent, OrderType, Side},
    };
    use rust_decimal_macros::dec;

    fn creds() -> VenueCredentials {
        VenueCredentials {
            venue: "paper".into(),
            plaintext: Vec::new(),
        }
    }

    fn engine_with_trades() -> Arc<PaperTradingEngine> {
        let engine = Arc::new(PaperTradingEngine::new());
        engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        engine
            .submit(
                AssetClass::CryptoSpotCex,
                &OrderIntent::new(
                    "BTC-USD",
                    Side::Buy,
                    OrderType::Market,
                    Size::from_decimal(dec!(1)),
                    None,
                    None,
                ),
            )
            .unwrap();
        engine.on_mark("WETH-USDC", Price::from_decimal(dec!(3_000)));
        engine
            .submit(
                AssetClass::CryptoSpotDex,
                &OrderIntent::new(
                    "WETH-USDC",
                    Side::Buy,
                    OrderType::Market,
                    Size::from_decimal(dec!(2)),
                    None,
                    None,
                ),
            )
            .unwrap();
        engine
    }

    #[tokio::test]
    async fn balances_cover_all_asset_classes_without_credentials() {
        let source = PaperAccountSource::new(engine_with_trades());
        let balances = source.fetch_balances(&creds()).await.unwrap();
        // 11 cash rows + 1 WETH token row for the DEX wallet account.
        assert_eq!(balances.len(), 12);
        assert!(balances.iter().any(|b| b.asset == "WETH"));
    }

    #[tokio::test]
    async fn positions_span_asset_classes() {
        let source = PaperAccountSource::new(engine_with_trades());
        let positions = source.fetch_positions(&creds()).await.unwrap();
        assert_eq!(positions.len(), 2);
        assert!(positions.iter().any(|p| p.instrument_id == "BTC-USD"));
        assert!(positions.iter().any(|p| p.instrument_id == "WETH-USDC"));
    }

    #[tokio::test]
    async fn transactions_come_from_internal_ledger() {
        let source = PaperAccountSource::new(engine_with_trades());
        let txs = source
            .fetch_transactions(&creds(), Uuid::new_v4(), None)
            .await
            .unwrap();
        // 11 opening deposits + trade & fee lines from the two fills.
        assert!(
            txs.iter()
                .filter(|t| t.transaction_type == "deposit")
                .count()
                == 11
        );
        assert!(txs.iter().any(|t| t.transaction_type == "trade"));
        assert!(txs.iter().all(|t| t.id.starts_with("paper-")));
    }
}
