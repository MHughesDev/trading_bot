//! In-memory OHLCV bar aggregation.
//!
//! Rolls base-timeframe bars (e.g. 1 min) up into a higher timeframe
//! (e.g. 5 min) without hitting ClickHouse.  The same aggregation rules
//! apply as the live bar builder: open = first, high = max, low = min,
//! close = last, volume/trade_count = sum.  The output bar's `ts_ns` is
//! the bucket's close time (matching `available_time` semantics used
//! throughout the platform).

use domain::payloads::bar::Timeframe;

use crate::store::LoadedBar;
use crate::types::TimeframeExt;

/// Roll `bars` (sorted ascending by `ts_ns`) from `base` timeframe into
/// `target` timeframe.
///
/// Returns `bars` unchanged when `target == base`.  Panics in debug builds
/// if `target` is not a whole multiple of `base` (e.g. 5m from 1m is fine;
/// 7m from 1m is not).
///
/// Partial buckets — where fewer base bars arrived than expected — are still
/// emitted.  Callers that need only complete candles should load a few extra
/// base bars as a buffer and discard the first output bucket; the warmup
/// loader does this automatically via its `BUFFER_BARS` constant.
pub fn aggregate_bars(bars: &[LoadedBar], target: Timeframe, base: Timeframe) -> Vec<LoadedBar> {
    if target == base {
        return bars.to_vec();
    }

    let target_ns = i64::try_from(target.seconds()).expect("overflow") * 1_000_000_000;
    let base_ns = i64::try_from(base.seconds()).expect("overflow") * 1_000_000_000;

    debug_assert_eq!(
        target_ns % base_ns,
        0,
        "target timeframe must be a whole multiple of base timeframe"
    );

    let mut out: Vec<LoadedBar> =
        Vec::with_capacity(bars.len() / (target_ns / base_ns) as usize + 1);

    // Accumulator for the bar currently being built.
    let mut acc: Option<BucketAcc> = None;

    for bar in bars {
        let bucket_close = bucket_close_ns(bar.ts_ns, target_ns);

        if let Some(ref mut a) = acc {
            if a.bucket_close == bucket_close {
                a.absorb(bar);
                continue;
            }
            // Bar belongs to a new bucket — emit the completed one.
            out.push(acc.take().unwrap().finish());
        }
        acc = Some(BucketAcc::from_bar(bar, bucket_close));
    }

    if let Some(a) = acc {
        out.push(a.finish());
    }

    out
}

/// Computes the close timestamp (nanoseconds) of the target-timeframe bucket
/// that contains a bar closing at `ts_ns`.
///
/// A bar closing exactly ON a boundary (ts_ns % target_ns == 0) is the LAST
/// bar of that bucket, so its bucket close is ts_ns itself.
fn bucket_close_ns(ts_ns: i64, target_ns: i64) -> i64 {
    let remainder = ts_ns.rem_euclid(target_ns);
    if remainder == 0 {
        ts_ns
    } else {
        ts_ns - remainder + target_ns
    }
}

struct BucketAcc {
    bucket_close: i64,
    open: rust_decimal::Decimal,
    high: rust_decimal::Decimal,
    low: rust_decimal::Decimal,
    close: rust_decimal::Decimal,
    volume: rust_decimal::Decimal,
    trade_count: u64,
}

impl BucketAcc {
    fn from_bar(bar: &LoadedBar, bucket_close: i64) -> Self {
        Self {
            bucket_close,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            trade_count: bar.trade_count,
        }
    }

    fn absorb(&mut self, bar: &LoadedBar) {
        if bar.high > self.high {
            self.high = bar.high;
        }
        if bar.low < self.low {
            self.low = bar.low;
        }
        self.close = bar.close;
        self.volume += bar.volume;
        self.trade_count += bar.trade_count;
    }

    fn finish(self) -> LoadedBar {
        LoadedBar {
            ts_ns: self.bucket_close,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            trade_count: self.trade_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn bar(ts_ns: i64, open: i64, high: i64, low: i64, close: i64, vol: i64) -> LoadedBar {
        LoadedBar {
            ts_ns,
            open: dec!(1) * rust_decimal::Decimal::from(open),
            high: dec!(1) * rust_decimal::Decimal::from(high),
            low: dec!(1) * rust_decimal::Decimal::from(low),
            close: dec!(1) * rust_decimal::Decimal::from(close),
            volume: dec!(1) * rust_decimal::Decimal::from(vol),
            trade_count: 1,
        }
    }

    const MIN_NS: i64 = 60_000_000_000;

    #[test]
    fn identity_when_same_timeframe() {
        let bars = vec![
            bar(1 * MIN_NS, 100, 110, 95, 105, 10),
            bar(2 * MIN_NS, 105, 115, 100, 108, 12),
        ];
        let out = aggregate_bars(&bars, Timeframe::Minutes1, Timeframe::Minutes1);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].ts_ns, 1 * MIN_NS);
        assert_eq!(out[1].ts_ns, 2 * MIN_NS);
    }

    #[test]
    fn five_1m_bars_become_one_5m_bar() {
        // 1m bars closing at minutes 1..=5 → one 5m bar closing at minute 5
        let bars: Vec<LoadedBar> = (1i64..=5)
            .map(|i| bar(i * MIN_NS, 100 + i, 110 + i, 90 + i, 100 + i, i))
            .collect();

        let out = aggregate_bars(&bars, Timeframe::Minutes5, Timeframe::Minutes1);

        assert_eq!(out.len(), 1, "five 1m bars → one 5m bar");
        let b = &out[0];
        assert_eq!(b.ts_ns, 5 * MIN_NS, "5m bar closes at minute 5");
        assert_eq!(b.open, dec!(101), "open = first bar");
        assert_eq!(b.high, dec!(115), "high = max");
        assert_eq!(b.low, dec!(91), "low = min");
        assert_eq!(b.close, dec!(105), "close = last bar");
        assert_eq!(b.volume, dec!(15), "volume = sum");
        assert_eq!(b.trade_count, 5, "trade_count = sum");
    }

    #[test]
    fn ten_1m_bars_become_two_5m_bars() {
        let bars: Vec<LoadedBar> = (1i64..=10)
            .map(|i| bar(i * MIN_NS, 100, 110, 90, 100, 1))
            .collect();

        let out = aggregate_bars(&bars, Timeframe::Minutes5, Timeframe::Minutes1);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].ts_ns, 5 * MIN_NS);
        assert_eq!(out[1].ts_ns, 10 * MIN_NS);
    }

    #[test]
    fn partial_leading_bucket_is_emitted() {
        // Only bars 3, 4, 5 — bar 3 belongs to the 5m bucket closing at minute 5.
        // This is a partial bucket (missing bars 1 and 2) but it is still emitted.
        let bars = vec![
            bar(3 * MIN_NS, 100, 110, 90, 102, 3),
            bar(4 * MIN_NS, 102, 112, 92, 104, 4),
            bar(5 * MIN_NS, 104, 114, 94, 106, 5),
        ];
        let out = aggregate_bars(&bars, Timeframe::Minutes5, Timeframe::Minutes1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ts_ns, 5 * MIN_NS);
        assert_eq!(out[0].open, dec!(100), "open = first seen bar");
        assert_eq!(out[0].trade_count, 3);
    }

    #[test]
    fn bar_on_exact_boundary_closes_its_own_bucket() {
        // A bar at exactly 5 * MIN_NS closes the bucket [1m..5m], not the next one.
        let ts = 5 * MIN_NS;
        assert_eq!(
            bucket_close_ns(ts, 5 * MIN_NS),
            ts,
            "bar exactly on boundary closes that bucket"
        );
    }

    #[test]
    fn bar_one_minute_past_boundary_closes_next_bucket() {
        // Bar at 6 * MIN_NS belongs to bucket closing at 10 * MIN_NS.
        assert_eq!(bucket_close_ns(6 * MIN_NS, 5 * MIN_NS), 10 * MIN_NS);
    }

    #[test]
    fn aggregate_to_15m_from_1m() {
        let bars: Vec<LoadedBar> = (1i64..=15)
            .map(|i| bar(i * MIN_NS, 100, 100, 100, 100 + i, 1))
            .collect();
        let out = aggregate_bars(&bars, Timeframe::Minutes15, Timeframe::Minutes1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ts_ns, 15 * MIN_NS);
        assert_eq!(out[0].close, dec!(115), "close = bar 15");
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = aggregate_bars(&[], Timeframe::Minutes5, Timeframe::Minutes1);
        assert!(out.is_empty());
    }
}
