//! P1-T09: `AccountSource` trait mock test.
//!
//! Proves the trait contract is usable via a `MockAccountSource` implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use domain::money::{Price, Size};
use execution::account_source::{
    AccountSource, AccountSourceError, Balance, VenueCredentials, VenuePosition, VenueTransaction,
};

/// Minimal mock returning canned data.
struct MockAccountSource;

#[async_trait]
impl AccountSource for MockAccountSource {
    fn venue_id(&self) -> &str {
        "mock"
    }

    async fn fetch_balances(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<Balance>, AccountSourceError> {
        Ok(vec![Balance {
            asset: "USD".into(),
            available: Size::from_decimal(rust_decimal_macros::dec!(10000)),
            locked: Size::from_decimal(rust_decimal_macros::dec!(0)),
            usd_value: Some(Price::from_decimal(rust_decimal_macros::dec!(10000))),
        }])
    }

    async fn fetch_positions(
        &self,
        _creds: &VenueCredentials,
    ) -> Result<Vec<VenuePosition>, AccountSourceError> {
        Ok(vec![])
    }

    async fn fetch_transactions(
        &self,
        _creds: &VenueCredentials,
        _user_id: Uuid,
        _since: Option<DateTime<Utc>>,
    ) -> Result<Vec<VenueTransaction>, AccountSourceError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn mock_returns_usd_balance() {
    let source = MockAccountSource;
    let creds = VenueCredentials {
        venue: "mock".into(),
        plaintext: vec![],
    };
    let balances = source.fetch_balances(&creds).await.unwrap();
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].asset, "USD");
}

#[tokio::test]
async fn mock_returns_empty_positions() {
    let source = MockAccountSource;
    let creds = VenueCredentials {
        venue: "mock".into(),
        plaintext: vec![],
    };
    let positions = source.fetch_positions(&creds).await.unwrap();
    assert!(positions.is_empty());
}
