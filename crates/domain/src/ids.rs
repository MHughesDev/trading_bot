//! Deterministic dedup-key helpers.
//!
//! Event identity is derived from the *source*, not from a random UUID generated
//! at ingest.  This guarantees that re-processing the same raw byte stream
//! produces the same event IDs, making dedup a purely deterministic operation.
//!
//! # Key forms
//!
//! | Stream kind | Key fields |
//! |-------------|------------|
//! | Sequenced (bars, quotes, order-book deltas) | `lane + instrument_id + venue_id + sequence + source` |
//! | Exchange trades | `venue_id + exchange_trade_id` |
//! | On-chain (future) | `chain + tx_hash + log_index` |

use uuid::Uuid;

/// Opaque dedup key — comparable, storable, loggable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DedupKey(pub String);

impl std::fmt::Display for DedupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Dedup key for sequenced streams (bars, quotes, order-book snapshots/deltas).
///
/// The key is deterministic: same inputs → same key, every time.
pub fn sequenced_key(
    lane: &str,
    instrument_id: &str,
    venue_id: &str,
    sequence: u64,
    source: &str,
) -> DedupKey {
    DedupKey(format!(
        "{lane}|{instrument_id}|{venue_id}|{sequence}|{source}"
    ))
}

/// Dedup key for exchange trade events identified by the venue's own trade ID.
pub fn trade_key(venue_id: &str, exchange_trade_id: &str) -> DedupKey {
    DedupKey(format!("{venue_id}|{exchange_trade_id}"))
}

/// Dedup key for on-chain events (future use).
pub fn onchain_key(chain: &str, tx_hash: &str, log_index: u32) -> DedupKey {
    DedupKey(format!("{chain}|{tx_hash}|{log_index}"))
}

/// Generate a deterministic `Uuid` from a `DedupKey` using UUID v5 (SHA-1 namespace).
///
/// The UUID is deterministic — the same `DedupKey` always produces the same UUID.
/// Use this as the `event_id` in `EventEnvelope` when a stable UUID is required.
pub fn event_id_from_key(key: &DedupKey) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, key.0.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequenced_key_is_deterministic() {
        let k1 = sequenced_key("market.bars.1m", "BTC-USDT", "coinbase", 42, "kraken_ws");
        let k2 = sequenced_key("market.bars.1m", "BTC-USDT", "coinbase", 42, "kraken_ws");
        assert_eq!(k1, k2);
    }

    #[test]
    fn sequenced_key_differs_on_sequence() {
        let k1 = sequenced_key("market.bars.1m", "BTC-USDT", "coinbase", 1, "kraken_ws");
        let k2 = sequenced_key("market.bars.1m", "BTC-USDT", "coinbase", 2, "kraken_ws");
        assert_ne!(k1, k2);
    }

    #[test]
    fn trade_key_is_deterministic() {
        let k1 = trade_key("coinbase", "trade-001");
        let k2 = trade_key("coinbase", "trade-001");
        assert_eq!(k1, k2);
    }

    #[test]
    fn trade_key_differs_on_id() {
        let k1 = trade_key("coinbase", "trade-001");
        let k2 = trade_key("coinbase", "trade-002");
        assert_ne!(k1, k2);
    }

    #[test]
    fn event_id_from_key_is_deterministic() {
        let key = trade_key("coinbase", "trade-abc");
        let id1 = event_id_from_key(&key);
        let id2 = event_id_from_key(&key);
        assert_eq!(id1, id2);
    }
}
