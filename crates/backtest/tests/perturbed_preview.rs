//! The runtime half of `future_perturbation_test` (S-3.10) over a real strategy.
//!
//! `crates/backtest/src/perturb.rs` unit-tests the column arithmetic and every
//! refusal. What it cannot show on its own is that the mode has **power**: that
//! moving a column a strategy reads actually changes where that strategy fires,
//! and that the columns the request did not name come back unchanged. A
//! perturbation mode without power produces a no-divergence result on every run,
//! and a probe whose negative answer is guaranteed reads as a clean bill of
//! health for every strategy on the platform.
//!
//! No simulator here on purpose — the preview is the surface the tool calls, and
//! `preview_matches_simulator.rs` is what ties the preview to the engine.

use backtest::perturb::{Perturbation, PerturbationError, PerturbationSet};
use backtest::preview::{preview_signals, preview_signals_perturbed, PreviewError, SignalPreview};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn bar(i: i64, close: Decimal) -> LoadedBar {
    LoadedBar {
        ts_ns: i * 60_000_000_000,
        open: close,
        high: close,
        low: close,
        close,
        volume: dec!(10),
        trade_count: 1,
        ..Default::default()
    }
}

/// Two features, so a request naming one of them can be shown to leave the other
/// alone.
fn crossover_definition() -> StrategyDefinition {
    let json = r#"{
        "strategy_id": "ema_cross_perturb_v1",
        "definition_version": "1.3",
        "asset_class": "crypto_spot_cex",
        "inputs": [
            { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_3", "ema_10"] }
        ],
        "nodes": [
            { "id": "n1", "type": "condition", "expr": "feature('ema_3') > feature('ema_10')" },
            { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
        ],
        "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }
        ],
        "exits": { "mode": "rules", "rules": [{ "rule": "max_holding_bars", "bars": 4 }] }
    }"#;
    serde_json::from_str(json).expect("valid fixture definition")
}

/// Up and down repeatedly, long enough that the crossings land past `ema_10`'s
/// 50-bar warm-up. Same shape as `preview_matches_simulator.rs`'s fixture.
fn zigzag() -> Vec<LoadedBar> {
    let mut bars = Vec::new();
    let mut i = 0i64;
    let mut price = 100i64;
    let mut push = |bars: &mut Vec<LoadedBar>, i: &mut i64, steps: usize, step: i64| {
        for _ in 0..steps {
            price += step;
            bars.push(bar(*i, Decimal::from(price)));
            *i += 1;
        }
    };
    push(&mut bars, &mut i, 60, 1);
    for leg in 0..8 {
        let step = if leg % 2 == 0 { -3 } else { 3 };
        push(&mut bars, &mut i, 14, step);
    }
    bars
}

fn run(items: Vec<Perturbation>, n_bars: usize) -> Result<SignalPreview, PreviewError> {
    let names = vec!["ema_3".to_string(), "ema_10".to_string()];
    let set = PerturbationSet::validate(items, &names, n_bars).expect("the request validates");
    preview_signals_perturbed(
        &crossover_definition(),
        &zigzag(),
        Timeframe::Minutes1,
        &set,
    )
}

/// The entry bars, which is what a divergence is measured over.
fn entry_bars(p: &SignalPreview) -> Vec<usize> {
    p.sample_events.iter().map(|e| e.bar).collect()
}

#[test]
fn an_unperturbed_preview_reports_no_perturbations_and_still_fires() {
    let bars = zigzag();
    let p = preview_signals(&crossover_definition(), &bars, Timeframe::Minutes1)
        .expect("the preview runs");
    assert!(
        p.perturbations.is_empty(),
        "an ordinary preview carries an empty list, never a missing one"
    );
    assert!(p.entries > 0, "the fixture fires — the baseline is real");
}

/// The probe has power: handing `ema_3` values from three bars ahead moves where
/// the crossover fires. Without this, every `future_perturbation_test` result
/// would be "no divergence" and would mean nothing.
#[test]
fn a_forward_shift_moves_where_the_strategy_fires() {
    let bars = zigzag();
    let clean = preview_signals(&crossover_definition(), &bars, Timeframe::Minutes1)
        .expect("the preview runs");
    let shifted = run(
        vec![Perturbation::ShiftColumn {
            column: "ema_3".into(),
            shift_bars: 3,
        }],
        bars.len(),
    )
    .expect("the perturbed preview runs");

    assert!(
        shifted.entries > 0,
        "the shifted run still trades — a divergence produced by the strategy going \
         silent would be about the blanked tail, not about lookahead"
    );
    assert_ne!(
        entry_bars(&clean),
        entry_bars(&shifted),
        "a column read three bars early has to change where a crossover fires"
    );
    let applied = &shifted.perturbations;
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].mode, "shift_column");
    assert_eq!(applied[0].column, "ema_3");
    assert_eq!(applied[0].shift_bars, Some(3));
    assert!(
        applied[0].bars_with_value > 0 && applied[0].bars_with_value < bars.len(),
        "a forward shift blanks the window's tail and says how much it kept"
    );
}

/// The other half of the acceptance, at this layer: the perturbation reaches only
/// the column it names. If it reached others, a divergence could not be blamed on
/// the named indicator, and "the offending indicator named" would be a guess.
#[test]
fn a_perturbation_leaves_the_column_it_did_not_name_untouched() {
    let bars = zigzag();
    let clean = preview_signals(&crossover_definition(), &bars, Timeframe::Minutes1)
        .expect("the preview runs");
    let shifted = run(
        vec![Perturbation::ShiftColumn {
            column: "ema_3".into(),
            shift_bars: 3,
        }],
        bars.len(),
    )
    .expect("the perturbed preview runs");

    let untouched = |p: &SignalPreview| {
        p.features
            .iter()
            .find(|f| f.name == "ema_10")
            .map(|f| f.bars_with_value)
            .expect("ema_10 is in the feature list")
    };
    assert_eq!(
        untouched(&clean),
        untouched(&shifted),
        "`ema_10` was not named, so it is evaluated exactly as the clean run did"
    );
}

/// A masked column is the one perturbation that can stop a strategy firing
/// entirely, and the reason must be the mask rather than the warm-up. The
/// pre-existing diagnosis would have sent a reader to widen a window that was
/// never the problem — and would have let a perturbed run that fired nothing read
/// as a run the perturbation never reached.
#[test]
fn a_masked_column_is_not_reported_as_a_warm_up_problem() {
    let bars = zigzag();
    let masked = run(
        vec![Perturbation::MaskColumn {
            column: "ema_3".into(),
        }],
        bars.len(),
    )
    .expect("the perturbed preview runs");

    assert_eq!(
        masked.entries, 0,
        "a condition naming a blank column is false"
    );
    assert_eq!(
        masked.never_fires_code,
        Some("perturbed_column_has_no_value"),
        "the diagnosis blames the perturbation, not the window"
    );
    let why = masked.never_fires_because.as_deref().unwrap_or_default();
    assert!(
        why.contains("ema_3"),
        "the offending column is named: {why}"
    );
    assert_eq!(masked.perturbations[0].bars_with_value, 0);
}

/// A 1% jitter is the same jitter on every call, so a repeat comparison measures
/// the strategy rather than the tool's own noise.
#[test]
fn a_seeded_jitter_is_reproducible_across_runs() {
    let bars = zigzag();
    let jitter = || {
        run(
            vec![Perturbation::JitterColumn {
                column: "ema_3".into(),
                perturbation_pct: 0.01,
                seed: 4242,
            }],
            bars.len(),
        )
        .expect("the perturbed preview runs")
    };
    assert_eq!(entry_bars(&jitter()), entry_bars(&jitter()));
}

/// Validation runs against the columns the strategy actually reads, which is only
/// knowable after `derive_requirements`. A caller that skipped `validate` must not
/// be able to slip an unread column past the preview.
#[test]
fn the_preview_itself_refuses_a_column_the_strategy_does_not_read() {
    let names = vec!["rsi_14".to_string()];
    let set = PerturbationSet::validate(
        vec![Perturbation::MaskColumn {
            column: "rsi_14".into(),
        }],
        &names,
        500,
    )
    .expect("valid against the wrong column list");

    let err = preview_signals_perturbed(
        &crossover_definition(),
        &zigzag(),
        Timeframe::Minutes1,
        &set,
    )
    .expect_err("the preview re-validates against its own requirements");
    assert!(matches!(
        err,
        PreviewError::Perturbation(PerturbationError::UnknownColumn { .. })
    ));
    let msg = err.to_string();
    assert!(msg.contains("ema_10") && msg.contains("ema_3"), "{msg}");
}
