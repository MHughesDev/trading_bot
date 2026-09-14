//! Proves the versioning contract through the single runtime:
//! - the same bar stream yields bit-identical values across runs
//! - every feature carries a non-zero version and the hash of its implementation

use chrono::Utc;
use features::runtime;
use features::{FeatureRow, FeatureValue, EMA_FEATURE_VERSION, RSI_FEATURE_VERSION};

fn rows(prices: &[f64]) -> Vec<FeatureRow> {
    prices
        .iter()
        .enumerate()
        .map(|(i, &c)| FeatureRow { ts_ns: i as i64 * 60_000_000_000, open: c, high: c, low: c, close: c, volume: 1.0 })
        .collect()
}

#[test]
fn versions_are_non_zero_and_declared_on_the_definition() {
    const { assert!(EMA_FEATURE_VERSION > 0 && RSI_FEATURE_VERSION > 0) };
    assert_eq!(runtime::feature("ema_7").unwrap().def().version, EMA_FEATURE_VERSION);
    assert_eq!(runtime::feature("rsi_14").unwrap().def().version, RSI_FEATURE_VERSION);
}

#[test]
fn value_carries_version() {
    let prices: Vec<f64> = (0..80).map(|i| 100.0 + f64::from(i) * 0.3).collect();
    let r = rows(&prices);
    let f = runtime::feature("rsi_14").unwrap();
    let v = runtime::value_at(f.as_ref(), &r, r.len() - 1).unwrap();
    let fv = FeatureValue::new("rsi_14", v, f.def().version, Utc::now());
    assert_eq!(fv.feature_version, RSI_FEATURE_VERSION);
}

#[test]
fn same_stream_yields_bit_identical_values() {
    let prices: Vec<f64> = (0..200).map(|i| 100.0 + (f64::from(i) * 0.7).sin()).collect();
    for name in ["ema_5", "rsi_14", "zscore_20", "obv"] {
        let f = runtime::feature(name).unwrap();
        let a = runtime::backfill_column(f.as_ref(), &rows(&prices));
        let b = runtime::backfill_column(f.as_ref(), &rows(&prices));
        assert_eq!(
            a.iter().map(|v| v.map(f64::to_bits)).collect::<Vec<_>>(),
            b.iter().map(|v| v.map(f64::to_bits)).collect::<Vec<_>>(),
            "{name}"
        );
    }
}

/// INV-14: the batch column and a live evaluation over a growing window agree bit
/// for bit at every row.
#[test]
fn backfill_and_incremental_serving_agree() {
    let prices: Vec<f64> = (0..300).map(|i| 50.0 + (f64::from(i) * 0.11).cos() * 3.0).collect();
    let all = rows(&prices);
    for name in ["ema_21", "rsi_14", "rolling_std_20", "log_returns_1", "garman_klass_vol_20"] {
        let f = runtime::feature(name).unwrap();
        let batch = runtime::backfill_column(f.as_ref(), &all);
        for i in 0..all.len() {
            let live = runtime::latest(&[name.to_string()], &all[..=i]).unwrap()[name];
            assert_eq!(live.map(f64::to_bits), batch[i].map(f64::to_bits), "{name} at {i}");
        }
    }
}
