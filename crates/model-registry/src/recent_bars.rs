//! The recent returns a conditional-volatility model is served from (Set N N-4.2).
//!
//! A GARCH-family model's forecast is the variance recursion filtered through the most recent
//! bars; there is no value to serve without them, and the old fallback (the unconditional
//! sigma) is gone. The gateway therefore reads the last bars of the instrument at the model's
//! timeframe through the one point-in-time reader and hands the log returns to the inference
//! sidecar as `series["recent_returns"]`. With no bar store, or too little history, nothing is
//! sent and the sidecar abstains with its reason.

use backtest::store::BarStore;
use chrono::{DateTime, Duration, Utc};
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;

/// Bars fetched per call. The sidecar needs the model's `min_bars` (30 by default); more
/// history only makes the filtered variance converge to the same value.
pub const WINDOW_BARS: usize = 250;

/// Log returns of consecutive closes. A non-positive or non-finite close ends the series at
/// that point (everything before it is dropped): a return across a bad price is not a return.
#[must_use]
pub fn log_returns(closes: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(closes.len().saturating_sub(1));
    for w in closes.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0 {
            out.push((b / a).ln());
        } else {
            out.clear();
        }
    }
    out
}

/// The bar source for serving. `None` store means nothing can be loaded.
pub struct RecentBars {
    store: Option<BarStore>,
}

impl RecentBars {
    /// `CLICKHOUSE_URL`, as `DatasetManager::new` reads it.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            store: std::env::var("CLICKHOUSE_URL")
                .ok()
                .filter(|u| !u.is_empty())
                .map(|url| BarStore::connect(&url)),
        }
    }

    #[must_use]
    pub fn with_store(store: Option<BarStore>) -> Self {
        Self { store }
    }

    /// Up to [`WINDOW_BARS`] log returns of `symbol` at `timeframe_key`, oldest first, from bars
    /// that had closed by `now`. `None` when there is no store, the timeframe is unknown or the
    /// read failed; the caller sends nothing and the model abstains.
    pub async fn recent_returns(
        &self,
        symbol: &str,
        timeframe_key: &str,
        now: DateTime<Utc>,
    ) -> Option<Vec<f64>> {
        use backtest::types::TimeframeExt;
        let store = self.store.as_ref()?;
        let tf = <Timeframe as TimeframeExt>::from_key(timeframe_key)?;
        let secs = i64::try_from(<Timeframe as TimeframeExt>::seconds(&tf)).ok()?;
        // Twice the span asked for, so weekends and short outages do not starve the window.
        let from = now - Duration::seconds(secs * (WINDOW_BARS as i64 + 1) * 2);
        let bars = store.load_bars(symbol, tf, from, now).await.ok()?;
        let closes: Vec<f64> = bars.iter().filter_map(|b| b.close.to_f64()).collect();
        let skip = closes.len().saturating_sub(WINDOW_BARS + 1);
        Some(log_returns(&closes[skip..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_returns_are_consecutive_log_differences() {
        let r = log_returns(&[100.0, 110.0, 99.0]);
        assert_eq!(r.len(), 2);
        assert!((r[0] - (1.1_f64).ln()).abs() < 1e-12);
        assert!((r[1] - (0.9_f64).ln()).abs() < 1e-12);
    }

    #[test]
    fn a_bad_price_drops_everything_before_it() {
        let r = log_returns(&[100.0, 101.0, 0.0, 50.0, 51.0]);
        assert_eq!(
            r.len(),
            1,
            "only the return after the bad close survives: {r:?}"
        );
        assert!(log_returns(&[100.0]).is_empty());
        assert!(log_returns(&[f64::NAN, 1.0, 2.0]).len() == 1);
    }

    #[tokio::test]
    async fn no_store_means_no_series() {
        let b = RecentBars::with_store(None);
        assert!(b
            .recent_returns("BTC-USD", "1m", Utc::now())
            .await
            .is_none());
    }
}
