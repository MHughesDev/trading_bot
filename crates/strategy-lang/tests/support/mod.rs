//! Synthetic hourly bars and the real preview, for the tests that need to see a
//! strategy fire.
//!
//! The bars are a fixture and say so: a deterministic regime-switching walk, not
//! BTC-USD. They stand in for it because the database is not available to a test,
//! and what the tests assert is about the *language* (a clause is reported as the
//! blocker, a skeleton fires) rather than about any market. The evaluator is the
//! production one — `backtest::preview::preview_signals`, the function the API's
//! preview route calls — never a copy of it.

#![allow(dead_code)]

use std::collections::BTreeMap;

use backtest::preview::{preview_signals, SignalPreview};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use rust_decimal::Decimal;

const HOUR_NS: i64 = 3_600_000_000_000;
/// 2025-01-01T00:00:00Z.
const START_NS: i64 = 1_735_689_600_000_000_000;

/// xorshift64*, so the fixture is the same on every machine.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Approximately standard normal: the sum of twelve uniforms.
    fn normal(&mut self) -> f64 {
        (0..12).map(|_| self.unit()).sum::<f64>() - 6.0
    }
}

fn dec(x: f64) -> Decimal {
    Decimal::from_f64_retain(x)
        .unwrap_or_default()
        .round_dp(4)
}

/// `n` hourly bars. Regimes of 150 bars alternate between an up-drift, a
/// mean-reverting stretch and a down-drift, at roughly the volatility of a
/// large crypto pair, so trend, reversion and volatility rules all have
/// something to find.
#[must_use]
pub fn hourly_bars(n: usize, seed: u64) -> Vec<LoadedBar> {
    let mut rng = Rng(seed | 1);
    let mut log_price = 10_000.0_f64.ln();
    let mut anchor = log_price;
    let mut bars = Vec::with_capacity(n);
    for i in 0..n {
        let regime = (i / 150) % 3;
        let sigma = 0.006;
        let step = match regime {
            0 => 0.0012 + sigma * rng.normal(),
            // Pulled back toward where the stretch began.
            1 => 0.25 * (anchor - log_price) + sigma * 1.2 * rng.normal(),
            _ => -0.0010 + sigma * rng.normal(),
        };
        if i % 150 == 0 {
            anchor = log_price;
        }
        let open = log_price.exp();
        log_price += step;
        let close = log_price.exp();
        let wick_hi = sigma * 0.6 * rng.unit();
        let wick_lo = sigma * 0.6 * rng.unit();
        let high = open.max(close) * (1.0 + wick_hi);
        let low = open.min(close) * (1.0 - wick_lo);
        let volume = 50.0 + 400.0 * rng.unit() + if i % 97 == 0 { 900.0 } else { 0.0 };
        let ts = START_NS + (i as i64 + 1) * HOUR_NS;
        bars.push(LoadedBar {
            ts_ns: ts,
            open_ns: ts - HOUR_NS,
            knowledge_ns: ts,
            open: dec(open),
            high: dec(high),
            low: dec(low),
            close: dec(close),
            volume: dec(volume),
            trade_count: 10,
            ..Default::default()
        });
    }
    bars
}

/// The definition with its parameters at their defaults — what a backtest runs.
///
/// The preview does not do this itself: it takes feature names straight from
/// the definition, so `ema_{{fast}}` reaches the engine unresolved. Callers
/// of `preview_signals` materialize first.
#[must_use]
pub fn materialized(def: &StrategyDefinition) -> StrategyDefinition {
    domain::strategy_def::params::materialize(def, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("the defaults do not materialize: {e}"))
}

/// The production preview over `bars`, at one hour.
#[must_use]
pub fn preview(def: &StrategyDefinition, bars: &[LoadedBar]) -> SignalPreview {
    preview_signals(&materialized(def), bars, Timeframe::Hours1)
        .unwrap_or_else(|e| panic!("the preview refused: {e}"))
}

/// Compile SLv2 and panic with every diagnostic if it does not.
#[must_use]
pub fn compiled(text: &str) -> StrategyDefinition {
    strategy_lang::text_to_definition(text).unwrap_or_else(|e| panic!("{e:#?}\n---\n{text}"))
}
