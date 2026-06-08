use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use domain::lanes;
use domain::money::{Price, Size};
use domain::{
    compute_available_time,
    payloads::bar::{BarPayload, Timeframe},
    payloads::trade::TradePayload,
    AvailableTimeParams, EventEnvelope, TrustTier,
};
use uuid::Uuid;

pub struct BarBuilderConfig {
    pub timeframe: Timeframe,
    pub watermark: Duration,
    pub processing_delay: Duration,
    pub instrument_id: String,
    pub venue_id: String,
    pub source: String,
}

pub enum BarEvent {
    Closed(EventEnvelope<BarPayload>),
    Revision(EventEnvelope<BarPayload>),
}

struct WindowAccum {
    window_start: DateTime<Utc>,
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Size,
    trade_count: u64,
    last_observed_time: DateTime<Utc>,
}

struct PublishedBar {
    payload: BarPayload,
    revision: u32,
}

pub struct BarState {
    config: BarBuilderConfig,
    current: Option<WindowAccum>,
    // keyed by window_start timestamp_millis
    published: HashMap<i64, PublishedBar>,
    sequence: u64,
}

impl BarState {
    pub fn new(config: BarBuilderConfig) -> Self {
        Self {
            config,
            current: None,
            published: HashMap::new(),
            sequence: 0,
        }
    }

    /// Feed a trade into the builder. Returns any bar events that were emitted.
    pub fn feed_trade(&mut self, trade: &EventEnvelope<TradePayload>) -> Vec<BarEvent> {
        let trade_time = trade.event_time.unwrap_or(trade.available_time);
        let observed = trade.observed_time;
        let trade_window = window_start_for(trade_time, self.config.timeframe);
        let duration = timeframe_duration(self.config.timeframe);

        let mut events = Vec::new();

        match self.current.take() {
            None => {
                self.current = Some(WindowAccum {
                    window_start: trade_window,
                    open: trade.payload.price,
                    high: trade.payload.price,
                    low: trade.payload.price,
                    close: trade.payload.price,
                    volume: trade.payload.size,
                    trade_count: 1,
                    last_observed_time: observed,
                });
            }
            Some(mut accum) if trade_window == accum.window_start => {
                update_accum(&mut accum, trade, observed);
                self.current = Some(accum);
            }
            Some(accum) if trade_window > accum.window_start => {
                let closed = self.build_bar_envelope(accum, duration);
                events.push(BarEvent::Closed(closed));
                self.current = Some(WindowAccum {
                    window_start: trade_window,
                    open: trade.payload.price,
                    high: trade.payload.price,
                    low: trade.payload.price,
                    close: trade.payload.price,
                    volume: trade.payload.size,
                    trade_count: 1,
                    last_observed_time: observed,
                });
            }
            Some(accum) => {
                // Late trade: put current back, then revise the published bar.
                self.current = Some(accum);
                let key = trade_window.timestamp_millis();
                if let Some(pub_bar) = self.published.get_mut(&key) {
                    let new_rev = pub_bar.revision + 1;
                    let p = &mut pub_bar.payload;
                    if trade.payload.price > p.high {
                        p.high = trade.payload.price;
                    }
                    if trade.payload.price < p.low {
                        p.low = trade.payload.price;
                    }
                    p.close = trade.payload.price;
                    p.volume = Size(p.volume.0 + trade.payload.size.0);
                    p.trade_count += 1;
                    p.revision = new_rev;
                    pub_bar.revision = new_rev;

                    let revised_payload = p.clone();
                    let window_close = trade_window + duration;
                    let available_time = compute_available_time(&AvailableTimeParams {
                        window_close,
                        observed_time: observed,
                        watermark: self.config.watermark,
                        processing_delay: self.config.processing_delay,
                    });
                    self.sequence += 1;
                    let env = EventEnvelope::new(
                        Uuid::new_v4(),
                        lanes::MARKET_BARS_1M_REVISED,
                        self.config.instrument_id.clone(),
                        self.config.venue_id.clone(),
                        self.config.source.clone(),
                        TrustTier::CentralizedExchange,
                        Some(window_close),
                        observed,
                        Utc::now(),
                        available_time,
                        self.sequence,
                        revised_payload,
                    );
                    events.push(BarEvent::Revision(env));
                }
            }
        }

        events
    }

    /// Flush the current in-progress window early (e.g. on shutdown).
    pub fn flush(&mut self) -> Option<EventEnvelope<BarPayload>> {
        let accum = self.current.take()?;
        let duration = timeframe_duration(self.config.timeframe);
        Some(self.build_bar_envelope(accum, duration))
    }

    fn build_bar_envelope(
        &mut self,
        accum: WindowAccum,
        duration: Duration,
    ) -> EventEnvelope<BarPayload> {
        let window_close = accum.window_start + duration;
        let available_time = compute_available_time(&AvailableTimeParams {
            window_close,
            observed_time: accum.last_observed_time,
            watermark: self.config.watermark,
            processing_delay: self.config.processing_delay,
        });
        let payload = BarPayload::new(
            self.config.timeframe,
            accum.open,
            accum.high,
            accum.low,
            accum.close,
            accum.volume,
            accum.trade_count,
        );
        let lane = match self.config.timeframe {
            Timeframe::Seconds1 => lanes::MARKET_BARS_1S,
            Timeframe::Minutes1 => lanes::MARKET_BARS_1M,
            _ => lanes::MARKET_BARS_1M,
        };
        let key = accum.window_start.timestamp_millis();
        self.published.insert(
            key,
            PublishedBar {
                payload: payload.clone(),
                revision: 0,
            },
        );
        self.sequence += 1;
        EventEnvelope::new(
            Uuid::new_v4(),
            lane,
            self.config.instrument_id.clone(),
            self.config.venue_id.clone(),
            self.config.source.clone(),
            TrustTier::CentralizedExchange,
            Some(window_close),
            accum.last_observed_time,
            Utc::now(),
            available_time,
            self.sequence,
            payload,
        )
    }
}

fn update_accum(
    accum: &mut WindowAccum,
    trade: &EventEnvelope<TradePayload>,
    observed: DateTime<Utc>,
) {
    if trade.payload.price > accum.high {
        accum.high = trade.payload.price;
    }
    if trade.payload.price < accum.low {
        accum.low = trade.payload.price;
    }
    accum.close = trade.payload.price;
    accum.volume = Size(accum.volume.0 + trade.payload.size.0);
    accum.trade_count += 1;
    if observed > accum.last_observed_time {
        accum.last_observed_time = observed;
    }
}

/// Compute the window-start for a timestamp given a timeframe.
pub fn window_start_for(t: DateTime<Utc>, timeframe: Timeframe) -> DateTime<Utc> {
    use chrono::Timelike;
    match timeframe {
        Timeframe::Seconds1 => t.with_nanosecond(0).expect("valid"),
        Timeframe::Minutes1 => t
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .expect("valid"),
        Timeframe::Minutes5 => {
            let minute = (t.minute() / 5) * 5;
            t.with_minute(minute)
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .expect("valid")
        }
        Timeframe::Minutes15 => {
            let minute = (t.minute() / 15) * 15;
            t.with_minute(minute)
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .expect("valid")
        }
        Timeframe::Hours1 => t
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .expect("valid"),
        Timeframe::Hours4 => {
            let hour = (t.hour() / 4) * 4;
            t.with_hour(hour)
                .and_then(|t| t.with_minute(0))
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .expect("valid")
        }
        Timeframe::Daily => t
            .with_hour(0)
            .and_then(|t| t.with_minute(0))
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .expect("valid"),
    }
}

/// Duration of one bar window for the given timeframe.
pub fn timeframe_duration(timeframe: Timeframe) -> Duration {
    match timeframe {
        Timeframe::Seconds1 => Duration::seconds(1),
        Timeframe::Minutes1 => Duration::minutes(1),
        Timeframe::Minutes5 => Duration::minutes(5),
        Timeframe::Minutes15 => Duration::minutes(15),
        Timeframe::Hours1 => Duration::hours(1),
        Timeframe::Hours4 => Duration::hours(4),
        Timeframe::Daily => Duration::hours(24),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use domain::{payloads::trade::TradeSide, TrustTier};
    use std::str::FromStr;

    fn make_trade(price: &str, size: &str, ts: DateTime<Utc>) -> EventEnvelope<TradePayload> {
        EventEnvelope::new(
            Uuid::new_v4(),
            lanes::MARKET_TRADES,
            "BTC-USD",
            "kraken",
            "kraken_ws",
            TrustTier::CentralizedExchange,
            Some(ts),
            ts,
            ts,
            ts,
            1,
            TradePayload::new(
                Price::from_str(price).unwrap(),
                Size::from_str(size).unwrap(),
                TradeSide::Buy,
                "t1",
            ),
        )
    }

    fn cfg() -> BarBuilderConfig {
        BarBuilderConfig {
            timeframe: Timeframe::Minutes1,
            watermark: Duration::seconds(2),
            processing_delay: Duration::milliseconds(50),
            instrument_id: "BTC-USD".into(),
            venue_id: "kraken".into(),
            source: "kraken_ws".into(),
        }
    }

    #[test]
    fn three_trades_in_same_window_produce_no_close_until_next_window() {
        let mut state = BarState::new(cfg());
        let t0 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 0).unwrap();
        assert!(state.feed_trade(&make_trade("100", "1", t0)).is_empty());
        assert!(state.feed_trade(&make_trade("110", "2", t0)).is_empty());
        assert!(state.feed_trade(&make_trade("95", "3", t0)).is_empty());
        // No close yet — still in the same window.
    }

    #[test]
    fn trade_in_next_window_closes_current() {
        let mut state = BarState::new(cfg());
        let t0 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 30).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 1, 5).unwrap();
        state.feed_trade(&make_trade("100", "1", t0));
        let events = state.feed_trade(&make_trade("105", "1", t1));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BarEvent::Closed(_)));
    }

    #[test]
    fn late_trade_produces_revision() {
        let mut state = BarState::new(cfg());
        let t0 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 30).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 1, 5).unwrap();
        let t_late = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 55).unwrap();
        state.feed_trade(&make_trade("100", "1", t0));
        state.feed_trade(&make_trade("105", "1", t1));
        let events = state.feed_trade(&make_trade("90", "0.5", t_late));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BarEvent::Revision(_)));
        if let BarEvent::Revision(env) = &events[0] {
            assert_eq!(env.payload.revision, 1);
        }
    }

    #[test]
    fn ohlcv_are_computed_correctly() {
        let mut state = BarState::new(cfg());
        let t0 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 0, 10).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 8, 10, 1, 0).unwrap();
        state.feed_trade(&make_trade("100", "1", t0));
        state.feed_trade(&make_trade("120", "2", t0));
        state.feed_trade(&make_trade("80", "0.5", t0));
        let events = state.feed_trade(&make_trade("110", "1", t1));
        if let BarEvent::Closed(env) = &events[0] {
            assert_eq!(env.payload.open.to_string(), "100");
            assert_eq!(env.payload.high.to_string(), "120");
            assert_eq!(env.payload.low.to_string(), "80");
            assert_eq!(env.payload.close.to_string(), "80"); // last trade in window
            assert_eq!(env.payload.trade_count, 3);
        } else {
            panic!("expected Closed");
        }
    }
}
