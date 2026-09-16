//! Golden parity: every legacy feature name still computes the value it computed
//! before the catalogue was built (DATA-006 §5 "Legacy migration"; AGENT-011 §8.1).
//!
//! # Why the tolerance is zero
//!
//! DATA-006 §10 F2 allows `1e-12` where the algorithm changed. Nothing here
//! changed algorithm: the F1 work moved the same arithmetic behind a registry and
//! shared two kernels, so a difference of one ulp would mean a summation order
//! moved, and a summation order that moved once will move again. Stored feature
//! values are compared bit for bit by `consistency::diff_serve`, which would
//! report the drift as `Diagnosis::CodeChanged` on every historical serve. So the
//! comparison is on `to_bits`, and the reference implementations below are
//! transcriptions of the pre-change source, not re-derivations of the formulas.
//!
//! Three instruments × three timeframes, as the spec requires: the instruments
//! differ in level, volatility and drift, and the timeframes differ in bar
//! spacing, which is what the calendar features read.

use features::runtime::{backfill_column, feature, value_at};
use features::FeatureRow;

// ── the fixtures ─────────────────────────────────────────────────────────────

/// Bar spacing in nanoseconds: 1m, 5m, 1h.
const TIMEFRAMES: [i64; 3] = [60_000_000_000, 300_000_000_000, 3_600_000_000_000];

/// Three price processes: a trending one, a mean-reverting one, and one with a
/// volatility cluster. Deterministic, so a failure is reproducible.
fn instrument(which: usize, n: usize, step_ns: i64) -> Vec<FeatureRow> {
    (0..n)
        .map(|i| {
            let t = i as f64;
            let close = match which {
                0 => 100.0 + t * 0.03 + (t * 0.11).sin() * 2.0,
                1 => 4_200.0 + (t * 0.07).sin() * 180.0 + (t * 0.31).cos() * 40.0,
                _ => 0.85 + (t * 0.05).sin() * 0.02 * if i > n / 2 { 4.0 } else { 1.0 },
            };
            let spread = close * 0.0015 * (1.0 + (t * 0.23).sin().abs());
            FeatureRow {
                ts_ns: i as i64 * step_ns,
                open: close - spread * 0.4,
                high: close + spread,
                low: close - spread,
                close,
                volume: 10.0 + ((i * 37) % 23) as f64,
            }
        })
        .collect()
}

fn frames() -> Vec<Vec<FeatureRow>> {
    let mut out = Vec::new();
    for which in 0..3 {
        for step in TIMEFRAMES {
            out.push(instrument(which, 1_400, step));
        }
    }
    out
}

// ── the reference implementations, transcribed from the pre-change source ────

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn sample_std(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return f64::NAN;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
}

/// `closes(f)` as it was: the window newest first.
fn window_closes(rows: &[FeatureRow], decision: usize, lookback: usize) -> Option<Vec<f64>> {
    (0..lookback).map(|b| decision.checked_sub(b).map(|i| rows[i].close)).collect()
}

fn legacy_ema(rows: &[FeatureRow], decision: usize, period: usize) -> Option<f64> {
    let lookback = period * 5;
    let c = window_closes(rows, decision, lookback)?;
    let k = 2.0 / (period as f64 + 1.0);
    let oldest = lookback - 1;
    let mut v = c[oldest];
    for back in (0..oldest).rev() {
        v = c[back] * k + v * (1.0 - k);
    }
    Some(v)
}

fn legacy_rsi(rows: &[FeatureRow], decision: usize, period: usize) -> Option<f64> {
    let lookback = period * 5 + 1;
    let c = window_closes(rows, decision, lookback)?;
    let n = period as f64;
    let oldest = lookback - 1;
    let (mut avg_gain, mut avg_loss) = (0.0, 0.0);
    let mut prev = c[oldest];
    for (i, back) in (0..oldest).rev().enumerate() {
        let x = c[back];
        let change = x - prev;
        prev = x;
        let (gain, loss) = (change.max(0.0), (-change).max(0.0));
        if i < period {
            avg_gain += gain / n;
            avg_loss += loss / n;
        } else {
            avg_gain = (avg_gain * (n - 1.0) + gain) / n;
            avg_loss = (avg_loss * (n - 1.0) + loss) / n;
        }
    }
    if avg_loss == 0.0 {
        return Some(100.0);
    }
    Some(100.0 - 100.0 / (1.0 + avg_gain / avg_loss))
}

fn legacy_rolling_mean(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    Some(mean(&window_closes(rows, decision, w)?))
}

fn legacy_rolling_std(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    Some(sample_std(&window_closes(rows, decision, w)?))
}

fn legacy_zscore(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    let c = window_closes(rows, decision, w)?;
    let s = sample_std(&c);
    Some(if s == 0.0 { f64::NAN } else { (c[0] - mean(&c)) / s })
}

fn legacy_simple_return(rows: &[FeatureRow], decision: usize, k: usize) -> Option<f64> {
    let c = window_closes(rows, decision, k + 1)?;
    Some(if c[k] == 0.0 { f64::NAN } else { c[0] / c[k] - 1.0 })
}

fn legacy_log_return_1(rows: &[FeatureRow], decision: usize) -> Option<f64> {
    let c = window_closes(rows, decision, 2)?;
    Some(if c[0] <= 0.0 || c[1] <= 0.0 { f64::NAN } else { (c[0] / c[1]).ln() })
}

fn legacy_parkinson(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    let inv_4ln2 = 1.0 / (4.0 * 2.0_f64.ln());
    let mut sum = 0.0;
    for b in 0..w {
        let r = &rows[decision.checked_sub(b)?];
        if r.low <= 0.0 || r.high <= 0.0 {
            return Some(f64::NAN);
        }
        sum += (r.high / r.low).ln().powi(2) * inv_4ln2;
    }
    Some((sum / w as f64).sqrt())
}

fn legacy_garman_klass(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    let c1 = 2.0 * 2.0_f64.ln() - 1.0;
    let mut sum = 0.0;
    for b in 0..w {
        let r = &rows[decision.checked_sub(b)?];
        if r.low <= 0.0 || r.high <= 0.0 || r.open <= 0.0 || r.close <= 0.0 {
            return Some(f64::NAN);
        }
        sum += (0.5 * (r.high / r.low).ln().powi(2) - c1 * (r.close / r.open).ln().powi(2)).max(0.0);
    }
    Some((sum / w as f64).sqrt())
}

fn legacy_rel_volume(rows: &[FeatureRow], decision: usize, w: usize) -> Option<f64> {
    let v: Vec<f64> = (0..w).map(|b| decision.checked_sub(b).map(|i| rows[i].volume)).collect::<Option<_>>()?;
    let m = mean(&v);
    Some(if m <= 0.0 { f64::NAN } else { v[0] / m })
}

fn legacy_obv(rows: &[FeatureRow], decision: usize) -> Option<f64> {
    let w = 256usize;
    let mut obv = 0.0;
    for b in 0..w - 1 {
        let now = &rows[decision.checked_sub(b)?];
        let before = &rows[decision.checked_sub(b + 1)?];
        if now.close > before.close {
            obv += now.volume;
        } else if now.close < before.close {
            obv -= now.volume;
        }
    }
    Some(obv)
}

fn legacy_calendar(rows: &[FeatureRow], decision: usize, which: &str) -> Option<f64> {
    let r = &rows[decision];
    Some(match which {
        "hour_sin" | "hour_cos" => {
            let hour = (r.ts_ns.div_euclid(3_600_000_000_000)).rem_euclid(24) as f64;
            let a = hour * std::f64::consts::TAU / 24.0;
            if which == "hour_sin" {
                a.sin()
            } else {
                a.cos()
            }
        }
        _ => {
            let dow = (r.ts_ns.div_euclid(86_400_000_000_000) + 3).rem_euclid(7) as f64;
            let a = dow * std::f64::consts::TAU / 7.0;
            if which == "dow_sin" {
                a.sin()
            } else {
                a.cos()
            }
        }
    })
}

fn legacy(name: &str, rows: &[FeatureRow], decision: usize) -> Option<f64> {
    let tail = |p: &str| name.strip_prefix(p).and_then(|s| s.parse::<usize>().ok());
    match name {
        "open" => Some(rows[decision].open),
        "high" => Some(rows[decision].high),
        "low" => Some(rows[decision].low),
        "close" => Some(rows[decision].close),
        "volume" => Some(rows[decision].volume),
        "obv" => legacy_obv(rows, decision),
        "log_returns_1" => legacy_log_return_1(rows, decision),
        "hour_sin" | "hour_cos" | "dow_sin" | "dow_cos" => legacy_calendar(rows, decision, name),
        _ => {
            if let Some(p) = tail("ema_") {
                return legacy_ema(rows, decision, p);
            }
            if let Some(p) = tail("rsi_") {
                return legacy_rsi(rows, decision, p);
            }
            if let Some(w) = tail("rolling_mean_") {
                return legacy_rolling_mean(rows, decision, w);
            }
            if let Some(w) = tail("rolling_std_") {
                return legacy_rolling_std(rows, decision, w);
            }
            if let Some(k) = tail("returns_").or_else(|| tail("momentum_")) {
                return legacy_simple_return(rows, decision, k);
            }
            if let Some(w) = tail("parkinson_vol_") {
                return legacy_parkinson(rows, decision, w);
            }
            if let Some(w) = tail("garman_klass_vol_") {
                return legacy_garman_klass(rows, decision, w);
            }
            if let Some(w) = tail("zscore_") {
                return legacy_zscore(rows, decision, w);
            }
            if let Some(w) = tail("rel_volume_") {
                return legacy_rel_volume(rows, decision, w);
            }
            None
        }
    }
}

// ── the test ─────────────────────────────────────────────────────────────────

/// Every name the platform catalogued before this phase.
const LEGACY_NAMES: &[&str] = &[
    "open",
    "high",
    "low",
    "close",
    "volume",
    "ema_7",
    "ema_14",
    "ema_21",
    "ema_50",
    "ema_200",
    "rsi_14",
    "rsi_21",
    "rolling_mean_7",
    "rolling_mean_14",
    "rolling_mean_20",
    "rolling_mean_50",
    "rolling_std_7",
    "rolling_std_14",
    "rolling_std_20",
    "returns_1",
    "returns_5",
    "returns_10",
    "returns_20",
    "log_returns_1",
    "parkinson_vol_20",
    "garman_klass_vol_20",
    "momentum_5",
    "momentum_10",
    "momentum_20",
    "zscore_20",
    "zscore_50",
    "rel_volume_20",
    "obv",
    "hour_sin",
    "hour_cos",
    "dow_sin",
    "dow_cos",
];

/// The parity DATA-006 §5 requires before any legacy path is retired: the engine
/// and the pre-change arithmetic agree **bit for bit** on every legacy name, on
/// three instruments and three timeframes, at every bar of the window's tail.
#[test]
fn every_legacy_name_computes_the_same_bits_it_did_before_the_catalogue() {
    let mut compared = 0usize;
    for rows in frames() {
        for name in LEGACY_NAMES {
            let f = feature(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            // The last 300 bars: warm and past every legacy lookback.
            for decision in rows.len() - 300..rows.len() {
                let got = value_at(f.as_ref(), &rows, decision);
                let want = legacy(name, &rows, decision).filter(|v| v.is_finite());
                assert_eq!(
                    got.map(f64::to_bits),
                    want.map(f64::to_bits),
                    "{name} at bar {decision} (ts {}): engine {got:?}, pre-change {want:?}",
                    rows[decision].ts_ns
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 90_000, "only {compared} comparisons — the fixture shrank");
}

/// Absence is part of parity: a legacy name is `None` until its declared window
/// exists, and that boundary must not have moved either. A feature that started
/// producing values one bar earlier would change every training frame's first row.
#[test]
fn the_first_bar_with_a_value_has_not_moved_for_any_legacy_name() {
    let rows = instrument(0, 1_400, TIMEFRAMES[2]);
    for name in LEGACY_NAMES {
        let f = feature(name).unwrap();
        let col = backfill_column(f.as_ref(), &rows);
        let engine_first = col.iter().position(Option::is_some);
        let legacy_first = (0..rows.len()).find(|d| legacy(name, &rows, *d).is_some_and(f64::is_finite));
        assert_eq!(engine_first, legacy_first, "{name}");
    }
}
