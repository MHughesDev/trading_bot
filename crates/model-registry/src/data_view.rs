//! Leakage-safe, point-in-time data view for the AI Models suite (Set I, Phase 0).
//!
//! `DataView` is the suite's *only* sanctioned way to read bars for training,
//! evaluation, and materialization. It wraps the existing backtest PIT store
//! (`backtest::store::BarStore`) and forming-bar-safe resampler
//! (`backtest::aggregate::aggregate_bars`) behind a **mandatory `as_of` ceiling**:
//! it can never return a bar whose `available_time` exceeds `as_of`, and it never
//! returns a forming (incomplete) higher-timeframe bucket. There is no new query
//! path — this is a thin, guarded façade over the canonical store.
//!
//! The `as_of` parameter is **not optional** (I-0.8): leakage-safety is
//! structural, not a flag a caller can forget. ADR-0008 makes event lookahead
//! impossible by `available_time` ordering; this view extends that guarantee to
//! every read the suite performs. The Python sidecars never issue their own bar
//! queries — they are handed pre-windowed, PIT-correct data.

use backtest::aggregate::aggregate_bars;
use backtest::store::{BarStore, LoadedBar};
use backtest::types::TimeframeExt;
use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

/// An `available_time` ceiling in Unix nanoseconds. A [`DataView`] read never
/// returns a bar whose close (`ts_ns`) is strictly greater than this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AsOf(pub i64);

impl AsOf {
    pub fn from_datetime(t: DateTime<Utc>) -> Self {
        Self(t.timestamp_nanos_opt().unwrap_or(i64::MAX))
    }
    pub fn nanos(self) -> i64 {
        self.0
    }
}

/// Thin point-in-time façade over [`BarStore`].
pub struct DataView<'a> {
    store: &'a BarStore,
}

impl<'a> DataView<'a> {
    pub fn new(store: &'a BarStore) -> Self {
        Self { store }
    }

    /// Load bars for `instrument` at `target_timeframe`, resampled from
    /// `base_timeframe`, over `[from, to)`, with every bar guaranteed to satisfy
    /// `ts_ns <= as_of`. A higher-timeframe bucket whose close exceeds `as_of` is
    /// excluded — so a forming bucket is never returned (forming-bar-safe).
    ///
    /// `as_of` is required: there is no way to ask this view for "all bars
    /// regardless of availability".
    pub async fn bars(
        &self,
        instrument: &str,
        base_timeframe: Timeframe,
        target_timeframe: Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        as_of: AsOf,
    ) -> anyhow::Result<Vec<LoadedBar>> {
        let base = self
            .store
            .load_bars(instrument, base_timeframe, from, to)
            .await?;
        let resampled = aggregate_bars(&base, target_timeframe, base_timeframe);
        let view = filter_as_of(resampled, as_of);
        // Defence in depth: the filter must hold structurally.
        guard_as_of(&view, as_of)?;
        Ok(view)
    }
}

/// Drop every bar whose close (`ts_ns`) is after the `as_of` ceiling. Because a
/// resampled bucket's `ts_ns` is its close time and every constituent base bar
/// closes at or before that, a surviving bucket is fully settled at `as_of`.
pub fn filter_as_of(bars: Vec<LoadedBar>, as_of: AsOf) -> Vec<LoadedBar> {
    bars.into_iter().filter(|b| b.ts_ns <= as_of.0).collect()
}

/// Structural guard: error (never return data) if any bar exceeds `as_of`.
/// Calling this on the output of [`filter_as_of`] is the belt-and-braces check
/// that the leakage rule held — a bug that lets a future bar through fails loudly.
pub fn guard_as_of(bars: &[LoadedBar], as_of: AsOf) -> anyhow::Result<()> {
    if let Some(bad) = bars.iter().find(|b| b.ts_ns > as_of.0) {
        anyhow::bail!(
            "leakage guard: bar at ts_ns={} exceeds as_of={} (ADR-0008/ADR-0017)",
            bad.ts_ns,
            as_of.0
        );
    }
    Ok(())
}

/// Bar-level data-quality diagnostics returned *before* training (I-0.7).
///
/// Complements the day-level coverage analyzer (`backtest::gaps::analyze`): this
/// works on the concrete bar series the model will actually train on, catching
/// intra-day grid gaps, duplicate `available_time`s, and return outliers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DataQualityReport {
    /// Number of missing bars relative to the fixed timeframe grid in
    /// `[first, last]` (sum of grid slots with no bar).
    pub gaps: u64,
    /// Count of duplicate `available_time` values (consecutive equal `ts_ns`).
    pub dupes: u64,
    /// Count of bars whose log-return is beyond the robust N-σ band.
    pub outliers: u64,
    /// Number of bars examined.
    pub bar_count: u64,
    /// `present / expected` over `[first, last]`, in `[0, 100]`.
    pub coverage_pct: f64,
}

/// Compute [`DataQualityReport`] over `bars` (ascending `ts_ns`) for a series of
/// `step_seconds` cadence, flagging returns beyond `sigma` robust deviations.
///
/// Robust band: |r − median(r)| > `sigma` × 1.4826 × MAD(r). The 1.4826 factor
/// makes MAD a consistent estimator of σ for normal data. Returns are log-returns
/// of close-to-close (f64 — statistical, not money; consistent with Set I D-4).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn data_quality(bars: &[LoadedBar], step_seconds: u64, sigma: f64) -> DataQualityReport {
    let bar_count = bars.len() as u64;
    if bars.is_empty() {
        return DataQualityReport::default();
    }
    let step_ns = (step_seconds.max(1) as i64) * 1_000_000_000;

    // Gaps + duplicates over the grid.
    let mut gaps = 0u64;
    let mut dupes = 0u64;
    for w in bars.windows(2) {
        let delta = w[1].ts_ns - w[0].ts_ns;
        if delta == 0 {
            dupes += 1;
        } else if delta > step_ns {
            // Number of whole grid slots missing strictly between the two bars.
            gaps += ((delta / step_ns) - 1).max(0) as u64;
        }
    }

    // Coverage: present distinct slots vs expected over [first, last].
    let span_ns = bars[bars.len() - 1].ts_ns - bars[0].ts_ns;
    let expected = (span_ns / step_ns).max(0) as u64 + 1;
    let present = bar_count.saturating_sub(dupes);
    let coverage_pct = if expected == 0 {
        0.0
    } else {
        (present as f64 / expected as f64 * 100.0).min(100.0)
    };

    // Outliers: robust band on log-returns.
    let mut returns: Vec<f64> = Vec::with_capacity(bars.len().saturating_sub(1));
    for w in bars.windows(2) {
        let p0 = w[0].close.to_f64().unwrap_or(0.0);
        let p1 = w[1].close.to_f64().unwrap_or(0.0);
        if p0 > 0.0 && p1 > 0.0 {
            returns.push((p1 / p0).ln());
        }
    }
    let outliers = count_robust_outliers(&returns, sigma);

    DataQualityReport {
        gaps,
        dupes,
        outliers,
        bar_count,
        coverage_pct,
    }
}

/// Count values beyond `sigma` × 1.4826 × MAD from the median. When the MAD is
/// zero (a near-constant series with a few spikes — MAD's known degenerate case),
/// fall back to a mean/std band so isolated spikes are still caught.
#[allow(clippy::cast_precision_loss)]
fn count_robust_outliers(values: &[f64], sigma: f64) -> u64 {
    if values.len() < 3 {
        return 0;
    }
    let median = median_of(values);
    let abs_dev: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
    let mad = median_of(&abs_dev);

    let (center, threshold) = if mad > 0.0 {
        (median, sigma * 1.4826 * mad)
    } else {
        // Degenerate MAD: use mean ± sigma·std.
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let std = var.sqrt();
        if std <= 0.0 {
            return 0;
        }
        (mean, sigma * std)
    };

    values
        .iter()
        .filter(|v| (**v - center).abs() > threshold)
        .count() as u64
}

fn median_of(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Helper: number of base-timeframe bars a label horizon token spans, used by
/// CV validation and materialization to size purge/embargo. Returns `None` for
/// an unparseable token or a base coarser than the horizon.
pub fn horizon_in_bars(horizon: &str, base_timeframe: Timeframe) -> Option<u64> {
    let horizon_secs = parse_horizon_seconds(horizon)?;
    let base_secs = base_timeframe.seconds();
    if base_secs == 0 || horizon_secs < base_secs {
        return None;
    }
    Some(horizon_secs / base_secs)
}

/// Parse an ISO-8601-ish horizon token (`"90s"`, `"15m"`, `"4h"`, `"1d"`, `"1w"`)
/// into seconds.
fn parse_horizon_seconds(token: &str) -> Option<u64> {
    let token = token.trim();
    let (num, unit) = token.split_at(token.find(|c: char| c.is_alphabetic())?);
    let n: u64 = num.parse().ok()?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(n * mult)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    const MIN_NS: i64 = 60_000_000_000;

    fn bar(ts_ns: i64, close: i64) -> LoadedBar {
        LoadedBar {
            ts_ns,
            open: Decimal::from(close),
            high: Decimal::from(close),
            low: Decimal::from(close),
            close: Decimal::from(close),
            volume: Decimal::from(1),
            trade_count: 1,
        }
    }

    #[test]
    fn filter_as_of_excludes_future_bars() {
        let bars = vec![bar(MIN_NS, 100), bar(2 * MIN_NS, 101), bar(3 * MIN_NS, 102)];
        let view = filter_as_of(bars, AsOf(2 * MIN_NS));
        assert_eq!(view.len(), 2, "bar past as_of is excluded");
        assert_eq!(view.last().unwrap().ts_ns, 2 * MIN_NS);
    }

    #[test]
    fn filter_as_of_excludes_forming_higher_tf_bucket() {
        // Three 5m buckets closing at 5,10,15m; as_of mid-way through the third.
        let buckets = vec![
            bar(5 * MIN_NS, 100),
            bar(10 * MIN_NS, 101),
            bar(15 * MIN_NS, 102),
        ];
        // as_of at minute 12: the 15m bucket is still forming and must be dropped.
        let view = filter_as_of(buckets, AsOf(12 * MIN_NS));
        assert_eq!(view.len(), 2);
        assert_eq!(view.last().unwrap().ts_ns, 10 * MIN_NS);
    }

    #[test]
    fn guard_as_of_errors_on_future_bar() {
        let bars = vec![bar(MIN_NS, 100), bar(3 * MIN_NS, 102)];
        assert!(guard_as_of(&bars, AsOf(2 * MIN_NS)).is_err());
        assert!(guard_as_of(&bars, AsOf(3 * MIN_NS)).is_ok());
    }

    #[test]
    fn data_quality_detects_planted_gap_and_dupe() {
        // Grid at 1m: bars at minutes 1,2,2(dupe),4(gap at minute 3),5.
        let bars = vec![
            bar(1 * MIN_NS, 100),
            bar(2 * MIN_NS, 101),
            bar(2 * MIN_NS, 101), // duplicate available_time
            bar(4 * MIN_NS, 103), // minute 3 missing
            bar(5 * MIN_NS, 104),
        ];
        let dq = data_quality(&bars, 60, 5.0);
        assert_eq!(dq.dupes, 1, "one duplicate ts");
        assert_eq!(dq.gaps, 1, "minute 3 missing");
        assert_eq!(dq.bar_count, 5);
        assert!(dq.coverage_pct < 100.0);
    }

    #[test]
    fn data_quality_flags_return_outlier() {
        // A gently wiggling series (closes alternate 100/101) with one large
        // spike to 200 → the spike's returns are clear outliers.
        let mut bars: Vec<LoadedBar> = (1..=20).map(|i| bar(i * MIN_NS, 100 + (i % 2))).collect();
        bars[10] = bar(11 * MIN_NS, 200);
        let dq = data_quality(&bars, 60, 3.0);
        assert!(dq.outliers >= 1, "the spike is an outlier");
    }

    #[test]
    fn empty_series_is_clean() {
        let dq = data_quality(&[], 60, 5.0);
        assert_eq!(dq, DataQualityReport::default());
    }

    #[test]
    fn horizon_in_bars_converts_tokens() {
        assert_eq!(horizon_in_bars("1h", Timeframe::Minutes1), Some(60));
        assert_eq!(horizon_in_bars("15m", Timeframe::Minutes15), Some(1));
        assert_eq!(horizon_in_bars("1d", Timeframe::Minutes1), Some(1440));
        // Horizon finer than base is rejected.
        assert_eq!(horizon_in_bars("30s", Timeframe::Minutes1), None);
        assert_eq!(horizon_in_bars("bogus", Timeframe::Minutes1), None);
    }
}
