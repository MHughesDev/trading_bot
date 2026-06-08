//! `TrustTier` — ordered enum from highest to lowest trust.
//!
//! Strategies declare `min_trust_tier`; the system compares the tier of each
//! incoming event against the strategy's threshold and discards sub-threshold
//! events before they reach strategy logic.

use serde::{Deserialize, Serialize};

/// Source-trust classification.
///
/// Variants are ordered **ascending** (lowest discriminant = lowest trust) so
/// that `derive(Ord)` gives `Regulated > CentralizedExchange > ... > SocialDerived`.
/// Strategies declare a `min_trust_tier` and the system discards any event whose
/// tier is *less than* the declared minimum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Social or derived data (sentiment, news) — lowest trust.
    SocialDerived,
    /// On-chain data that is not yet finalized (fewer confirmations).
    OnchainTentative,
    /// On-chain data with sufficient block confirmations.
    OnchainConfirmed,
    /// Top-tier centralized exchange (e.g. Coinbase, Kraken, Alpaca).
    CentralizedExchange,
    /// Regulated exchange or licensed broker — highest trust.
    Regulated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_highest_to_lowest() {
        assert!(TrustTier::Regulated >= TrustTier::CentralizedExchange);
        assert!(TrustTier::CentralizedExchange >= TrustTier::OnchainConfirmed);
        assert!(TrustTier::OnchainConfirmed >= TrustTier::OnchainTentative);
        assert!(TrustTier::OnchainTentative >= TrustTier::SocialDerived);
    }

    #[test]
    fn serde_round_trip() {
        let t = TrustTier::CentralizedExchange;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#""centralized_exchange""#);
        let back: TrustTier = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
