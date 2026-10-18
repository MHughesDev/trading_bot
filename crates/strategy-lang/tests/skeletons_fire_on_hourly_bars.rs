//! A skeleton is a starting point that works (S-5.12; AGENT-011 §8.2, AC-2).
//!
//! The acceptance: the skeletons "compile, validate and preview on BTC-USD 1h
//! with non-zero entries and no warm-up violation".
//!
//! # What this proves, and what it does not
//!
//! Compile, validate and preview are all the production functions: the
//! SLv2 compiler, `strategy_validator::validate`, and
//! `backtest::preview::preview_signals` (the one the API's preview route calls).
//!
//! The bars are the fixture in `support` — hourly, regime-switching, at about
//! a large crypto pair's volatility — and **not BTC-USD**: no database is
//! available to a test. The real-bars version is `skeletons_on_btc_usd.rs`,
//! ignored unless a ClickHouse is configured.
//!
//! # History
//!
//! Running this first found that the preview refused three of the four skeletons
//! because `backtest::requirements::parse_feature` accepted only a few families
//! (S-5.2); it now resolves through the registry, and all four fire.

mod support;

use backtest::preview::{preview_signals, SignalPreview};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use strategy_lang::{skeleton, HypothesisFamily};
use support::{hourly_bars, materialized};

/// Every family the platform can write. A new one is covered the day it exists.
fn worked() -> Vec<strategy_lang::Skeleton> {
    strategy_lang::skeleton::worked()
}

fn run(s: &strategy_lang::Skeleton, bars: &[LoadedBar]) -> Result<SignalPreview, String> {
    let def = s.definition().unwrap_or_else(|e| panic!("{e:#?}"));
    preview_signals(&materialized(&def), bars, Timeframe::Hours1).map_err(|e| e.to_string())
}

fn assert_fires(s: &strategy_lang::Skeleton, bars: &[LoadedBar]) {
    let p = run(s, bars).unwrap_or_else(|e| panic!("{}: the preview refused: {e}", s.family));
    assert!(
        p.entries > 0,
        "{} never fires over {} bars: {:?}",
        s.family,
        p.n_bars,
        p.never_fires_because
    );
    assert_eq!(p.never_fires_code, None, "{}", s.family);
    // No warm-up violation: the window is longer than the warm-up, and bars past
    // it were actually evaluated.
    assert!(
        (p.warmup_bars as usize) < p.n_bars,
        "{} needs {} bars of warm-up in a window of {}",
        s.family,
        p.warmup_bars,
        p.n_bars
    );
    assert!(p.bars_evaluated > 0, "{}", s.family);
    assert!(
        p.bars_evaluated + p.warmup_bars as usize <= p.n_bars + 1,
        "{} evaluated bars it should have been warming up on",
        s.family
    );
}

#[test]
fn every_worked_skeleton_validates() {
    for s in worked() {
        let def = s.definition().unwrap_or_else(|e| panic!("{e:#?}"));
        // Validated the way the save route validates: after parameters take
        // their defaults.
        strategy_validator::validate(&materialized(&def))
            .unwrap_or_else(|e| panic!("{} was refused: {e:#?}", s.family));
    }
}

#[test]
fn the_trend_skeleton_fires_with_no_warm_up_violation() {
    let bars = hourly_bars(3_000, 11);
    assert_fires(&skeleton(HypothesisFamily::Trend).expect("worked"), &bars);
}

/// The acceptance, for every worked family. It was `#[ignore]`d while the
/// preview's requirements gate refused every family but `ema` and `rsi`; the gate
/// now resolves through the feature registry (S-5.2).
#[test]
fn every_worked_skeleton_fires_with_no_warm_up_violation() {
    let bars = hourly_bars(3_000, 11);
    for s in worked() {
        assert_fires(&s, &bars);
    }
}

/// The acceptance names these three; asserted by name so a refactor of
/// `worked()` cannot quietly drop one.
#[test]
fn the_three_named_skeletons_are_among_them() {
    for family in [
        HypothesisFamily::Trend,
        HypothesisFamily::MeanReversion,
        HypothesisFamily::VolTarget,
    ] {
        let s = skeleton(family).expect("a worked family");
        let def = s.definition().expect("compiles");
        strategy_validator::validate(&materialized(&def))
            .unwrap_or_else(|e| panic!("{family} was refused: {e:#?}"));
    }
}
