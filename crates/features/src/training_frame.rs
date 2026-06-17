//! PURE training-frame assembly: OHLCV bars → feature columns + forward-return
//! label, with warm-up / trailing-label rows dropped.
//!
//! This is the materialization compute core (Set I, I-0.5). It mirrors the
//! *column set* of the trainer sidecar's `apps/model-trainer/app/features.py`
//! (`fs_core_ohlcv_v3` and friends) so the columns a model trains on match what
//! the live inference path produces. Crucially, the indicator values come from
//! the **same pure primitives the live path uses** (`Ema`, `Rsi`, and the
//! rolling/return helpers below), so train and serve agree by construction
//! rather than by two independent implementations happening to match.
//!
//! Purity contract (same as the rest of the crate): no I/O, no wall-clock, no
//! side effects. Identical input ⇒ identical output, which is what lets a
//! reproduce-from-hash run (Set I Phase 3) recompute a dataset deterministically.
//!
//! Parity note: pandas seeds `ewm(min_periods=period)` RSI from the first
//! observation, whereas [`crate::Rsi`] uses Wilder's classic SMA seed. The two
//! differ by a handful of values at the very start of a series. We deliberately
//! prefer the Wilder primitive because it is the one the *live* feature path
//! emits — exact pandas parity is a separate, explicitly-scoped concern in the
//! Phase 3 reproducibility work, not this materialization.

use crate::{Ema, Rsi};

/// One OHLCV bar as plain `f64`s (statistical, not money — Set I D-4: indicator
/// math is float, monetary quantities never are).
#[derive(Clone, Copy, Debug)]
pub struct OhlcvRow {
    /// `available_time` in Unix nanoseconds (the universal PIT sort key).
    pub ts_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A columnar feature + label frame, NaN-free and aligned by row.
///
/// `ts_ns`, each column in `columns` (parallel to `feature_names`), and `label`
/// all have the same length: the number of rows that survived the NaN drop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainingFrame {
    /// Feature column names, in the order requested (unknown names skipped).
    pub feature_names: Vec<String>,
    /// `available_time` of each surviving row.
    pub ts_ns: Vec<i64>,
    /// One `Vec<f64>` per `feature_names` entry, each `ts_ns.len()` long.
    pub columns: Vec<Vec<f64>>,
    /// Forward simple return over the label horizon, one per surviving row.
    pub label: Vec<f64>,
}

impl TrainingFrame {
    pub fn row_count(&self) -> usize {
        self.ts_ns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ts_ns.is_empty()
    }
}

/// Build a [`TrainingFrame`] from one instrument's ascending OHLCV history.
///
/// `features` is the requested column set (names not computable by this crate
/// are silently skipped, matching `features.py`). `horizon_bars` is the forward
/// label window measured in bars (use [`label_horizon_bars`] to convert a token
/// like `"1h"` at a timeframe like `"5m"`). The label is the simple forward
/// return `close[i+H] / close[i] - 1`.
///
/// Any row with a NaN in *any* feature column or the label — indicator warm-up
/// at the head, the trailing `H` rows that have no future bar — is dropped, so
/// the returned frame is dense and directly trainable.
pub fn build_training_frame(
    bars: &[OhlcvRow],
    features: &[String],
    horizon_bars: u64,
) -> TrainingFrame {
    // Keep only feature names we can actually compute, preserving order.
    let feature_names: Vec<String> = features
        .iter()
        .filter(|n| is_known_feature(n))
        .cloned()
        .collect();

    if bars.is_empty() {
        return TrainingFrame {
            feature_names,
            ..Default::default()
        };
    }

    let n = bars.len();
    let close: Vec<f64> = bars.iter().map(|b| b.close).collect();

    // Compute every requested column as `Vec<Option<f64>>` (None == NaN).
    let raw_columns: Vec<Vec<Option<f64>>> = feature_names
        .iter()
        .map(|name| compute_column(name, bars, &close))
        .collect();

    // Forward-return label: None for the trailing `H` rows with no future bar.
    let h = horizon_bars as usize;
    let label: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let j = i.checked_add(h)?;
            if j < n && close[i] != 0.0 {
                let v = close[j] / close[i] - 1.0;
                finite(v)
            } else {
                None
            }
        })
        .collect();

    // Keep rows where every column and the label are present and finite.
    let mut ts_ns = Vec::new();
    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); feature_names.len()];
    let mut kept_label = Vec::new();
    for i in 0..n {
        if label[i].is_none() {
            continue;
        }
        if raw_columns.iter().any(|c| c[i].is_none()) {
            continue;
        }
        ts_ns.push(bars[i].ts_ns);
        for (c, raw) in columns.iter_mut().zip(raw_columns.iter()) {
            c.push(raw[i].expect("checked Some above"));
        }
        kept_label.push(label[i].expect("checked Some above"));
    }

    TrainingFrame {
        feature_names,
        ts_ns,
        columns,
        label: kept_label,
    }
}

// ---------------------------------------------------------------------------
// I-3.3  Multi-resolution feature assembly
// ---------------------------------------------------------------------------

/// A higher-timeframe bar, aligned to base-timeframe rows.
///
/// For each base row at `ts_ns`, we attach the value of the *last settled*
/// higher-timeframe bar whose close `ts_ns` ≤ base row's `ts_ns`.  This is
/// forming-bar-safe: we never peek into the bar that is still forming.
#[derive(Clone, Copy, Debug)]
pub struct HigherTfBar {
    /// Close timestamp of this bar in Unix nanoseconds.
    pub ts_ns: i64,
    pub value: f64,
}

/// Align a pre-computed higher-timeframe feature series to the base grid.
///
/// For each base row at `base_ts_ns[i]`, returns the `value` of the last
/// `HigherTfBar` whose `ts_ns ≤ base_ts_ns[i]`.  Returns `None` until at
/// least one higher-TF bar has settled.
///
/// Both slices must be sorted ascending by `ts_ns`.
pub fn align_higher_tf(
    base_ts_ns: &[i64],
    higher_tf_bars: &[HigherTfBar],
) -> Vec<Option<f64>> {
    let mut out = vec![None; base_ts_ns.len()];
    let mut htf_idx = 0usize;

    for (i, &base_ts) in base_ts_ns.iter().enumerate() {
        // Advance higher-TF pointer as far as possible without exceeding base_ts.
        while htf_idx + 1 < higher_tf_bars.len()
            && higher_tf_bars[htf_idx + 1].ts_ns <= base_ts
        {
            htf_idx += 1;
        }
        // The settled bar must have ts_ns ≤ base_ts.
        if !higher_tf_bars.is_empty() && higher_tf_bars[htf_idx].ts_ns <= base_ts {
            out[i] = finite(higher_tf_bars[htf_idx].value);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// I-3.4  Devolatization feature op
// ---------------------------------------------------------------------------

/// Divide each value by `sigma` (σ fitted on train only, from Phase 1).
///
/// Returns the devolatized series; `sigma` must be positive. Caller is
/// responsible for using the same σ persisted in the bundle at serve time so
/// train/serve parity holds.
pub fn devol(values: &[f64], sigma: f64) -> Vec<f64> {
    assert!(sigma > 0.0, "sigma must be positive");
    values.iter().map(|v| v / sigma).collect()
}

/// Fit the σ scaler on a training slice: realized std of `values` clipped to
/// the given `[lo, hi]` percentile to avoid outlier contamination.
///
/// Returns `(sigma, mean)` where `mean` is the training mean (subtracted
/// before σ-scaling when centering is desired).
#[allow(clippy::cast_precision_loss)]
pub fn fit_sigma(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (1.0, 0.0);
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let sigma = var.sqrt().max(1e-12);
    (sigma, mean)
}

/// Convert a horizon token (`"90s"`, `"15m"`, `"1h"`, `"1d"`) at a `timeframe`
/// token into a whole number of bars, mirroring `features.py.horizon_in_bars`
/// (round to nearest, floor of 1). Returns `None` only if either token is
/// unparseable.
pub fn label_horizon_bars(horizon: &str, timeframe: &str) -> Option<u64> {
    let h = token_to_minutes(horizon)?;
    let tf = token_to_minutes(timeframe)?.max(1e-9);
    let bars = (h / tf).round();
    Some((bars as i64).max(1) as u64)
}

/// Timeframe / horizon token → minutes (fractional for sub-minute units), e.g.
/// `"30s"` → 0.5, `"5m"` → 5, `"4h"` → 240, `"1d"` → 1440. Mirrors the unit map
/// in `features.py`.
fn token_to_minutes(token: &str) -> Option<f64> {
    let token = token.trim().to_ascii_lowercase();
    let unit = token.chars().last()?;
    let value: f64 = token[..token.len() - 1].parse().ok()?;
    let mult = match unit {
        's' => 1.0 / 60.0,
        'm' => 1.0,
        'h' => 60.0,
        'd' => 1440.0,
        _ => return None,
    };
    Some(value * mult)
}

/// Whether `compute_column` can produce a value for this name.
fn is_known_feature(name: &str) -> bool {
    crate::feature_sets::is_known(name)
}

/// Compute a single named column over the series, NaN (`None`) during warm-up.
#[allow(clippy::too_many_lines)]
fn compute_column(name: &str, bars: &[OhlcvRow], close: &[f64]) -> Vec<Option<f64>> {
    let n = bars.len();
    match name {
        // ── Passthrough ──────────────────────────────────────────────────
        "open" => bars.iter().map(|b| finite(b.open)).collect(),
        "high" => bars.iter().map(|b| finite(b.high)).collect(),
        "low" => bars.iter().map(|b| finite(b.low)).collect(),
        "close" => bars.iter().map(|b| finite(b.close)).collect(),
        "volume" => bars.iter().map(|b| finite(b.volume)).collect(),

        // ── EMA family ───────────────────────────────────────────────────
        _ if name.starts_with("ema_") => {
            let period = suffix_usize(name, "ema_").unwrap_or(0);
            if period == 0 {
                return vec![None; n];
            }
            let mut ema = Ema::new(period);
            close.iter().map(|&c| finite(ema.update(c))).collect()
        }

        // ── RSI family ───────────────────────────────────────────────────
        _ if name.starts_with("rsi_") => {
            let period = suffix_usize(name, "rsi_").unwrap_or(0);
            if period < 2 {
                return vec![None; n];
            }
            let mut rsi = Rsi::new(period);
            close
                .iter()
                .map(|&c| rsi.update(c).and_then(finite))
                .collect()
        }

        // ── Rolling moments ───────────────────────────────────────────────
        _ if name.starts_with("rolling_mean_") => {
            let w = suffix_usize(name, "rolling_mean_").unwrap_or(0);
            rolling(close, w, rolling_mean)
        }
        _ if name.starts_with("rolling_std_") => {
            let w = suffix_usize(name, "rolling_std_").unwrap_or(0);
            rolling(close, w, rolling_std)
        }

        // ── Returns / lags (I-3.2) ────────────────────────────────────────
        _ if name.starts_with("returns_") => {
            let k = suffix_usize(name, "returns_").unwrap_or(0);
            (0..n)
                .map(|i| {
                    if k == 0 || i < k || close[i - k] == 0.0 {
                        None
                    } else {
                        finite(close[i] / close[i - k] - 1.0)
                    }
                })
                .collect()
        }
        "log_returns_1" => (0..n)
            .map(|i| {
                if i == 0 || close[i - 1] <= 0.0 || close[i] <= 0.0 {
                    None
                } else {
                    finite((close[i] / close[i - 1]).ln())
                }
            })
            .collect(),

        // ── Momentum (I-3.2): close[i]/close[i-k] - 1 ───────────────────
        _ if name.starts_with("momentum_") => {
            let k = suffix_usize(name, "momentum_").unwrap_or(0);
            (0..n)
                .map(|i| {
                    if k == 0 || i < k || close[i - k] == 0.0 {
                        None
                    } else {
                        finite(close[i] / close[i - k] - 1.0)
                    }
                })
                .collect()
        }

        // ── Volatility estimators (I-3.2) ─────────────────────────────────
        // Parkinson: σ² = 1/(4·ln2·N) · Σ (ln(H/L))²
        _ if name.starts_with("parkinson_vol_") => {
            let w = suffix_usize(name, "parkinson_vol_").unwrap_or(0);
            let inv_4ln2 = 1.0 / (4.0 * 2.0_f64.ln());
            let sq: Vec<Option<f64>> = bars
                .iter()
                .map(|b| {
                    if b.low <= 0.0 || b.high <= 0.0 {
                        None
                    } else {
                        finite((b.high / b.low).ln().powi(2) * inv_4ln2)
                    }
                })
                .collect();
            // Rolling mean of sq over w bars, then sqrt
            #[allow(clippy::cast_precision_loss)]
            rolling_option(&sq, w, |win| {
                let sum: f64 = win.iter().sum();
                finite((sum / win.len() as f64).sqrt())
            })
        }
        // Garman–Klass: σ² = 0.5·(ln H/L)² - (2·ln2-1)·(ln C/O)²
        _ if name.starts_with("garman_klass_vol_") => {
            let w = suffix_usize(name, "garman_klass_vol_").unwrap_or(0);
            let c1 = 2.0 * 2.0_f64.ln() - 1.0;
            let sq: Vec<Option<f64>> = bars
                .iter()
                .map(|b| {
                    if b.low <= 0.0 || b.high <= 0.0 || b.open <= 0.0 || b.close <= 0.0 {
                        None
                    } else {
                        let hl = 0.5 * (b.high / b.low).ln().powi(2);
                        let co = c1 * (b.close / b.open).ln().powi(2);
                        finite(hl - co)
                    }
                })
                .collect();
            #[allow(clippy::cast_precision_loss)]
            rolling_option(&sq, w, |win| {
                let sum: f64 = win.iter().map(|v| v.max(0.0)).sum();
                finite((sum / win.len() as f64).sqrt())
            })
        }

        // ── Z-score (I-3.2): (close - mean_w) / std_w ────────────────────
        _ if name.starts_with("zscore_") => {
            let w = suffix_usize(name, "zscore_").unwrap_or(0);
            (0..n)
                .map(|i| {
                    if w == 0 || i + 1 < w {
                        None
                    } else {
                        let win = &close[i + 1 - w..=i];
                        let mean = rolling_mean(win)?;
                        let std = rolling_std(win)?;
                        if std == 0.0 {
                            None
                        } else {
                            finite((close[i] - mean) / std)
                        }
                    }
                })
                .collect()
        }

        // ── Relative volume (I-3.2): volume / rolling_mean(volume, w) ─────
        _ if name.starts_with("rel_volume_") => {
            let w = suffix_usize(name, "rel_volume_").unwrap_or(0);
            let vol: Vec<f64> = bars.iter().map(|b| b.volume).collect();
            let mean_vol = rolling(&vol, w, rolling_mean);
            (0..n)
                .map(|i| {
                    let mv = mean_vol[i]?;
                    if mv <= 0.0 {
                        None
                    } else {
                        finite(vol[i] / mv)
                    }
                })
                .collect()
        }

        // ── On-balance volume (I-3.2) ─────────────────────────────────────
        "obv" => {
            let mut obv = 0.0f64;
            let mut out = vec![None; n];
            for i in 0..n {
                if i > 0 {
                    let sign = if close[i] > close[i - 1] {
                        1.0
                    } else if close[i] < close[i - 1] {
                        -1.0
                    } else {
                        0.0
                    };
                    obv += sign * bars[i].volume;
                }
                out[i] = finite(obv);
            }
            out
        }

        // ── Calendar / session (I-3.2): sin/cos encodings ─────────────────
        // ts_ns is Unix nanoseconds; hour = (ts_ns / 1e9 / 3600) % 24
        "hour_sin" => bars
            .iter()
            .map(|b| {
                let hour = ((b.ts_ns / 1_000_000_000 / 3600) % 24) as f64;
                finite((hour * std::f64::consts::TAU / 24.0).sin())
            })
            .collect(),
        "hour_cos" => bars
            .iter()
            .map(|b| {
                let hour = ((b.ts_ns / 1_000_000_000 / 3600) % 24) as f64;
                finite((hour * std::f64::consts::TAU / 24.0).cos())
            })
            .collect(),
        "dow_sin" => bars
            .iter()
            .map(|b| {
                // Unix epoch = Thursday (day 4 of week, 0=Mon in ISO).
                let days = b.ts_ns / 1_000_000_000 / 86400;
                let dow = ((days + 3) % 7) as f64; // 0=Mon
                finite((dow * std::f64::consts::TAU / 7.0).sin())
            })
            .collect(),
        "dow_cos" => bars
            .iter()
            .map(|b| {
                let days = b.ts_ns / 1_000_000_000 / 86400;
                let dow = ((days + 3) % 7) as f64;
                finite((dow * std::f64::consts::TAU / 7.0).cos())
            })
            .collect(),

        _ => vec![None; n],
    }
}

/// Rolling window reducer over a `Vec<Option<f64>>` (None propagates as None in window).
fn rolling_option<F>(values: &[Option<f64>], w: usize, f: F) -> Vec<Option<f64>>
where
    F: Fn(&Vec<f64>) -> Option<f64>,
{
    let n = values.len();
    (0..n)
        .map(|i| {
            if w == 0 || i + 1 < w {
                return None;
            }
            let win: Vec<f64> = values[i + 1 - w..=i]
                .iter()
                .filter_map(|v| *v)
                .collect();
            if win.len() < w {
                None
            } else {
                f(&win)
            }
        })
        .collect()
}

/// Apply a window reducer over the trailing `w` closes; `None` until `w` samples
/// exist (pandas `rolling(w)` semantics).
fn rolling(close: &[f64], w: usize, f: fn(&[f64]) -> Option<f64>) -> Vec<Option<f64>> {
    let n = close.len();
    (0..n)
        .map(|i| {
            if w == 0 || i + 1 < w {
                None
            } else {
                f(&close[i + 1 - w..=i])
            }
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn rolling_mean(win: &[f64]) -> Option<f64> {
    let n = win.len();
    if n == 0 {
        return None;
    }
    finite(win.iter().sum::<f64>() / n as f64)
}

/// Sample standard deviation (ddof = 1), matching pandas `rolling.std()`.
#[allow(clippy::cast_precision_loss)]
fn rolling_std(win: &[f64]) -> Option<f64> {
    let n = win.len();
    if n < 2 {
        return None;
    }
    let mean = win.iter().sum::<f64>() / n as f64;
    let var = win.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    finite(var.sqrt())
}

fn suffix_usize(name: &str, prefix: &str) -> Option<usize> {
    name.strip_prefix(prefix)?.split('_').next()?.parse().ok()
}

/// Pass through only finite values; NaN/±∞ become `None` so they are dropped.
fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: i64, close: f64) -> OhlcvRow {
        OhlcvRow {
            ts_ns: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    fn series(closes: &[f64]) -> Vec<OhlcvRow> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| row(i as i64 * 60_000_000_000, c))
            .collect()
    }

    #[test]
    fn label_horizon_bars_mirrors_python() {
        assert_eq!(label_horizon_bars("1h", "5m"), Some(12));
        assert_eq!(label_horizon_bars("15m", "15m"), Some(1));
        assert_eq!(label_horizon_bars("1d", "1m"), Some(1440));
        // Sub-bar horizon floors to 1, never 0.
        assert_eq!(label_horizon_bars("30s", "5m"), Some(1));
        assert_eq!(label_horizon_bars("bad", "5m"), None);
    }

    #[test]
    fn forward_label_and_warmup_drop() {
        // close[i+2]/close[i]-1, horizon = 2 bars.
        let bars = series(&[10.0, 11.0, 12.0, 13.0, 14.0]);
        let frame = build_training_frame(&bars, &["close".to_string()], 2);
        // n=5, horizon=2 ⇒ rows 0..=2 have a forward label (3,4 trail off).
        assert_eq!(frame.row_count(), 3);
        assert_eq!(frame.feature_names, vec!["close".to_string()]);
        // label[0] = 12/10 - 1 = 0.2
        assert!((frame.label[0] - 0.2).abs() < 1e-12);
        // close column passes through the price.
        assert!((frame.columns[0][0] - 10.0).abs() < 1e-12);
        assert_eq!(frame.ts_ns.len(), frame.label.len());
    }

    #[test]
    fn rsi_warmup_rows_are_dropped() {
        // 20 strictly increasing closes; rsi_14 is None until enough changes,
        // and the trailing horizon row is dropped too. Every surviving row must
        // be dense (no NaN leaked through).
        let bars = series(&(0..20).map(|i| 100.0 + f64::from(i)).collect::<Vec<_>>());
        let feats = vec!["rsi_14".to_string(), "close".to_string()];
        let frame = build_training_frame(&bars, &feats, 1);
        assert!(frame.row_count() > 0, "some rows survive");
        for col in &frame.columns {
            assert_eq!(col.len(), frame.row_count());
            assert!(col.iter().all(|v| v.is_finite()));
        }
        assert!(frame.label.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn rolling_std_is_sample_ddof1() {
        // closes 1,2,3,4: rolling_std_2 at i=1 is std([1,2]) ddof1 = 0.7071…
        let bars = series(&[1.0, 2.0, 3.0, 4.0]);
        let frame = build_training_frame(&bars, &["rolling_std_2".to_string()], 1);
        // Surviving rows: need rolling_std (i>=1) and a forward label (i<=2):
        // rows i=1,2. First kept row is i=1.
        let expected = (0.5f64).sqrt(); // sample std of [1,2]
        assert!((frame.columns[0][0] - expected).abs() < 1e-12);
    }

    #[test]
    fn unknown_features_are_skipped() {
        let bars = series(&[1.0, 2.0, 3.0]);
        let feats = vec!["close".to_string(), "not_a_feature".to_string()];
        let frame = build_training_frame(&bars, &feats, 1);
        assert_eq!(frame.feature_names, vec!["close".to_string()]);
        assert_eq!(frame.columns.len(), 1);
    }

    #[test]
    fn empty_bars_yield_empty_frame() {
        let frame = build_training_frame(&[], &["close".to_string()], 1);
        assert!(frame.is_empty());
        assert_eq!(frame.feature_names, vec!["close".to_string()]);
        assert_eq!(frame.row_count(), 0);
    }
}
