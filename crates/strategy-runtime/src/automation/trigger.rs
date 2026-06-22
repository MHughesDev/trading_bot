//! Trigger specification and higher-timeframe bar aggregation.
//!
//! `TriggerSpec` is the serializable config stored on an `AutomationSpec` that
//! answers the question "when should this automation evaluate?"  Two variants:
//!
//! - `OhlcvBar` — fires each time a bar of the given timeframe closes.
//!   For 1m bars this is a direct pass-through from the live aggregator.
//!   For any longer timeframe the bar is constructed in-process by
//!   `HtfBarAggregator`, which folds completed 1m bars into the target window
//!   using the same `window_start_for` boundaries the charting system uses.
//!   The strategy therefore always sees the same OHLCV data the chart shows —
//!   no look-ahead, no separate HTF feed needed.
//!
//! - `Timer` — fires on a fixed wall-clock interval, independent of bars.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use builders::window_start_for;
use domain::money::{Price, Size};
use domain::payloads::bar::{BarPayload, Timeframe};

/// Defines when an automation's strategy should be evaluated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSpec {
    /// Fires when a new OHLCV bar closes for the given timeframe.
    ///
    /// For `minutes_1`: one evaluation per completed 1-minute bar.
    /// For longer timeframes: the bar is built in-process from 1m data
    /// by `HtfBarAggregator` — same window math as the chart.
    OhlcvBar { timeframe: Timeframe },
    /// Fires on a fixed wall-clock interval, independent of market data.
    Timer { interval_secs: u64 },
}

impl Default for TriggerSpec {
    fn default() -> Self {
        TriggerSpec::OhlcvBar {
            timeframe: Timeframe::Minutes1,
        }
    }
}

/// Data delivered to the strategy when a trigger fires.
#[derive(Clone, Debug)]
pub enum TriggerFired {
    /// A completed OHLCV bar (1m direct or HTF constructed from 1m bars).
    Bar {
        bar: BarPayload,
        /// UTC open-time of this bar's window.
        window_start: DateTime<Utc>,
    },
    /// A wall-clock timer tick.
    Tick { fired_at: DateTime<Utc> },
}

// ── HtfBarAggregator ─────────────────────────────────────────────────────────

struct HtfAccum {
    window_start: DateTime<Utc>,
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Size,
    trade_count: u64,
}

impl HtfAccum {
    fn from_bar(window_start: DateTime<Utc>, bar: &BarPayload) -> Self {
        Self {
            window_start,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
            trade_count: bar.trade_count,
        }
    }

    fn merge(mut self, bar: &BarPayload) -> Self {
        if bar.high > self.high {
            self.high = bar.high;
        }
        if bar.low < self.low {
            self.low = bar.low;
        }
        self.close = bar.close;
        self.volume = Size(self.volume.0 + bar.volume.0);
        self.trade_count += bar.trade_count;
        self
    }

    fn build(self, timeframe: Timeframe) -> BarPayload {
        BarPayload::new(
            timeframe,
            self.open,
            self.high,
            self.low,
            self.close,
            self.volume,
            self.trade_count,
        )
    }
}

/// Aggregates completed 1m bars into a higher-timeframe OHLCV bar.
///
/// Feed one completed 1m bar at a time via `feed_1m_bar`.  The aggregator
/// maps each bar's open-time to an HTF window using `window_start_for` — the
/// same function the chart uses — and emits a `TriggerFired::Bar` whenever the
/// window rolls.
///
/// For `Timeframe::Minutes1` every call returns `Some` immediately
/// (transparent pass-through).
pub struct HtfBarAggregator {
    timeframe: Timeframe,
    current: Option<HtfAccum>,
}

impl HtfBarAggregator {
    pub fn new(timeframe: Timeframe) -> Self {
        Self {
            timeframe,
            current: None,
        }
    }

    /// Feed one completed 1m bar.
    ///
    /// `bar_window_start` — UTC open-time of the 1m bar that just closed.
    ///
    /// Returns `Some(TriggerFired::Bar)` when the HTF window rolls (i.e. the
    /// previous window's aggregated bar is complete), otherwise `None`.
    pub fn feed_1m_bar(
        &mut self,
        bar: &BarPayload,
        bar_window_start: DateTime<Utc>,
    ) -> Option<TriggerFired> {
        if self.timeframe == Timeframe::Minutes1 {
            return Some(TriggerFired::Bar {
                bar: bar.clone(),
                window_start: bar_window_start,
            });
        }

        let htf_window = window_start_for(bar_window_start, self.timeframe);

        match self.current.take() {
            None => {
                self.current = Some(HtfAccum::from_bar(htf_window, bar));
                None
            }
            Some(accum) if accum.window_start == htf_window => {
                self.current = Some(accum.merge(bar));
                None
            }
            Some(accum) => {
                let window_start = accum.window_start;
                let fired = TriggerFired::Bar {
                    bar: accum.build(self.timeframe),
                    window_start,
                };
                self.current = Some(HtfAccum::from_bar(htf_window, bar));
                Some(fired)
            }
        }
    }

    /// Emit the currently open (incomplete) window.
    ///
    /// Use on graceful shutdown or end-of-session flush.  The emitted bar
    /// covers whatever 1m bars have accumulated so far in the current HTF
    /// window.
    pub fn flush(&mut self) -> Option<TriggerFired> {
        let accum = self.current.take()?;
        let window_start = accum.window_start;
        Some(TriggerFired::Bar {
            bar: accum.build(self.timeframe),
            window_start,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::str::FromStr;

    fn price(s: &str) -> Price {
        Price::from_str(s).unwrap()
    }

    fn size(s: &str) -> Size {
        Size::from_str(s).unwrap()
    }

    fn bar(o: &str, h: &str, l: &str, c: &str, vol: &str) -> BarPayload {
        BarPayload::new(
            Timeframe::Minutes1,
            price(o),
            price(h),
            price(l),
            price(c),
            size(vol),
            1,
        )
    }

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 20, h, m, 0).unwrap()
    }

    #[test]
    fn minutes1_is_passthrough() {
        let mut agg = HtfBarAggregator::new(Timeframe::Minutes1);
        let b = bar("100", "110", "95", "105", "1");
        let result = agg.feed_1m_bar(&b, t(10, 0));
        assert!(matches!(result, Some(TriggerFired::Bar { .. })));
    }

    #[test]
    fn minutes5_accumulates_four_bars_then_emits_on_fifth() {
        let mut agg = HtfBarAggregator::new(Timeframe::Minutes5);

        // 10:00–10:04 — all in the same 5m window (10:00)
        assert!(agg.feed_1m_bar(&bar("100", "105", "98", "102", "1"), t(10, 0)).is_none());
        assert!(agg.feed_1m_bar(&bar("102", "108", "100", "106", "2"), t(10, 1)).is_none());
        assert!(agg.feed_1m_bar(&bar("106", "110", "104", "108", "1"), t(10, 2)).is_none());
        assert!(agg.feed_1m_bar(&bar("108", "112", "106", "110", "3"), t(10, 3)).is_none());

        // 10:05 — rolls into next 5m window; prior window emitted
        let fired = agg.feed_1m_bar(&bar("110", "115", "109", "113", "1"), t(10, 5));
        let TriggerFired::Bar { bar: htf, window_start } = fired.unwrap() else {
            panic!("expected Bar");
        };

        assert_eq!(window_start, t(10, 0));
        assert_eq!(htf.timeframe, Timeframe::Minutes5);
        assert_eq!(htf.open, price("100"));
        assert_eq!(htf.high, price("112"));
        assert_eq!(htf.low, price("98"));
        assert_eq!(htf.close, price("110"));
        assert_eq!(htf.trade_count, 4);
    }

    #[test]
    fn flush_emits_open_window() {
        let mut agg = HtfBarAggregator::new(Timeframe::Minutes5);
        agg.feed_1m_bar(&bar("100", "105", "98", "102", "1"), t(10, 0));
        agg.feed_1m_bar(&bar("102", "106", "100", "104", "1"), t(10, 1));

        let flushed = agg.flush();
        assert!(flushed.is_some());
        let TriggerFired::Bar { bar: htf, .. } = flushed.unwrap() else {
            panic!("expected Bar");
        };
        assert_eq!(htf.open, price("100"));
        assert_eq!(htf.high, price("106"));
        assert_eq!(htf.trade_count, 2);
        // flush clears state
        assert!(agg.flush().is_none());
    }

    #[test]
    fn trigger_spec_serde_round_trip() {
        let spec = TriggerSpec::OhlcvBar {
            timeframe: Timeframe::Minutes5,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: TriggerSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);

        let timer = TriggerSpec::Timer { interval_secs: 300 };
        let json2 = serde_json::to_string(&timer).unwrap();
        let back2: TriggerSpec = serde_json::from_str(&json2).unwrap();
        assert_eq!(timer, back2);
    }

    #[test]
    fn trigger_spec_default_is_1m_bar() {
        let spec = TriggerSpec::default();
        assert_eq!(
            spec,
            TriggerSpec::OhlcvBar {
                timeframe: Timeframe::Minutes1,
            }
        );
    }

    #[test]
    fn trigger_spec_json_shape() {
        let spec = TriggerSpec::OhlcvBar {
            timeframe: Timeframe::Minutes5,
        };
        let v: serde_json::Value = serde_json::to_value(&spec).unwrap();
        assert_eq!(v["kind"], "ohlcv_bar");
        assert_eq!(v["timeframe"], "minutes5");

        let timer = TriggerSpec::Timer { interval_secs: 60 };
        let v2: serde_json::Value = serde_json::to_value(&timer).unwrap();
        assert_eq!(v2["kind"], "timer");
        assert_eq!(v2["interval_secs"], 60);
    }
}
