//! Proves the versioning contract:
//! - the same bar stream yields identical FeatureValues across runs
//! - all values carry a non-zero feature_version

use chrono::Utc;
use features::{Ema, FeatureValue, Rsi, EMA_FEATURE_VERSION, RSI_FEATURE_VERSION};

fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}

// ── EMA ──────────────────────────────────────────────────────────────────────

#[test]
fn ema_feature_version_is_non_zero() {
    const { assert!(EMA_FEATURE_VERSION > 0) };
}

#[test]
fn ema_value_carries_version() {
    let mut ema = Ema::new(7);
    let t = now();
    let prices = [10.0_f64, 11.0, 10.5, 11.2, 10.8, 11.5, 11.0];
    let mut last = None;
    for &p in &prices {
        last = Some(FeatureValue::new(
            "ema_7",
            ema.update(p),
            EMA_FEATURE_VERSION,
            t,
        ));
    }
    let fv = last.unwrap();
    assert_eq!(fv.feature_version, EMA_FEATURE_VERSION);
    assert_eq!(fv.name, "ema_7");
}

#[test]
fn ema_same_stream_yields_identical_values() {
    let prices = [100.0, 102.0, 101.5, 103.0, 102.5, 104.0, 103.5, 105.0];
    let run = |prices: &[f64]| {
        let mut ema = Ema::new(5);
        prices.iter().map(|&p| ema.update(p)).collect::<Vec<_>>()
    };
    let a = run(&prices);
    let b = run(&prices);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() < 1e-12, "EMA values differ: {x} vs {y}");
    }
}

// ── RSI ──────────────────────────────────────────────────────────────────────

#[test]
fn rsi_feature_version_is_non_zero() {
    const { assert!(RSI_FEATURE_VERSION > 0) };
}

#[test]
fn rsi_value_carries_version() {
    let mut rsi = Rsi::new(14);
    let t = now();
    let prices: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
    let mut last = None;
    for &p in &prices {
        if let Some(v) = rsi.update(p) {
            last = Some(FeatureValue::new("rsi_14", v, RSI_FEATURE_VERSION, t));
        }
    }
    let fv = last.unwrap();
    assert_eq!(fv.feature_version, RSI_FEATURE_VERSION);
    assert_eq!(fv.name, "rsi_14");
}

#[test]
fn rsi_same_stream_yields_identical_values() {
    let prices: Vec<f64> = (0..20).map(|i| 100.0 + (i as f64) * 0.7).collect();
    let run = |prices: &[f64]| {
        let mut r = Rsi::new(14);
        prices
            .iter()
            .filter_map(|&p| r.update(p))
            .collect::<Vec<_>>()
    };
    let a = run(&prices);
    let b = run(&prices);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() < 1e-12, "RSI values differ: {x} vs {y}");
    }
}
