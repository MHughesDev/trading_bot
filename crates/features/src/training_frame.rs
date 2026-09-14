//! PURE training-frame assembly: OHLCV bars → feature columns + forward-return
//! label, with warm-up / trailing-label rows dropped.
//!
//! Every column comes from [`crate::runtime`], the single feature runtime that the
//! backtest, the warm start and the live serve also use (INV-14). There is no
//! column computed here and nowhere else.
//!
//! Purity contract (same as the rest of the crate): no I/O, no wall-clock, no
//! side effects. Identical input ⇒ identical output.

/// One OHLCV bar as plain `f64`s (statistical, not money — Set I D-4: indicator
/// math is float, monetary quantities never are).
pub use dataplane::feature::FeatureRow as OhlcvRow;

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
    /// Staleness companion, one `Vec` per `feature_names` entry: how old, in
    /// minutes, the newest observation behind that value was. Empty on a frame
    /// built from rows that were never aligned to the master clock.
    ///
    /// A value without its age is a forward-fill nobody can see (INV-11), so the
    /// aligned builder always populates this.
    pub age_minutes: Vec<Vec<i64>>,
    /// Quality companion, one `Vec` per `feature_names` entry: the union of the
    /// quality flags across that feature's declared window.
    pub quality: Vec<Vec<u32>>,
    /// Forward simple return over the label horizon, one per surviving row.
    pub label: Vec<f64>,
    /// Average-uniqueness sample weight, one per surviving row (SPEC §3.4).
    ///
    /// Labels with overlapping horizons are not independent observations: with a
    /// 60-bar horizon on 1-minute bars, sixty consecutive labels describe almost
    /// the same stretch of future, and a model that weights them equally has been
    /// told the same thing sixty times. The weight is each label's mean of
    /// `1/concurrency` over its own span.
    ///
    /// Empty on a frame built without alignment; `build_aligned_training_frame`
    /// always populates it, which is what lets the label spec declare
    /// `sample_weight_method = uniqueness` truthfully rather than `none`
    /// (ADR-P2-22).
    pub sample_weight: Vec<f64>,
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
/// `features` is the requested column set (names no implementation answers to are
/// skipped). `horizon_bars` is the forward
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

    // Every requested column from the one runtime (None == absent).
    let raw_columns: Vec<Vec<Option<f64>>> = feature_names
        .iter()
        .map(|name| match crate::runtime::feature(name) {
            Ok(f) => crate::runtime::backfill_column(f.as_ref(), bars),
            Err(_) => vec![None; n],
        })
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
        age_minutes: Vec::new(),
        quality: Vec::new(),
        label: kept_label,
        sample_weight: Vec::new(),
    }
}

/// Clamp a bar horizon into the `u32` the label module works in. A horizon that
/// does not fit is a configuration error long before it reaches here; saturating
/// keeps the weight computation total rather than panicking inside a build.
fn horizon_bars_u32(h: u64) -> u32 {
    u32::try_from(h).unwrap_or(u32::MAX)
}

/// Build a training frame on the **UTC master clock** (SPEC 2, INV-11).
///
/// `obs` is the sparse bar series with its provenance; it is densified onto the
/// grid of `step_ns` ticks by the one densifier
/// ([`crate::align::densify_bars`]), features are computed over the densified
/// rows, and every feature column is emitted with its `_age_minutes` and
/// `_quality` companions. A gap -- a venue outage, a holiday, a crypto weekend
/// -- becomes a run of rows whose age says how stale they are, instead of
/// silently disappearing and making the bars on either side look adjacent.
#[must_use]
pub fn build_aligned_training_frame(
    obs: &[crate::align::BarObs],
    features: &[String],
    horizon_bars: u64,
    step_ns: i64,
) -> TrainingFrame {
    let clock = crate::align::densify_bars(obs, step_ns);
    let mut frame = build_training_frame(&clock.rows, features, horizon_bars);

    // Map each surviving row back to its grid tick to read off provenance. The
    // builder drops rows but never reorders them, so the tick is the key.
    let mut tick_index = std::collections::HashMap::with_capacity(clock.len());
    for (i, row) in clock.rows.iter().enumerate() {
        tick_index.insert(row.ts_ns, i);
    }

    // Average-uniqueness weights, computed over the *densified clock* and then
    // subset to the rows that survived the NaN drop. Computing them over the
    // surviving rows alone would understate concurrency: a dropped warm-up row's
    // label still overlaps the ones that follow it.
    let intervals = dataplane::label::horizon_intervals(clock.len(), horizon_bars_u32(horizon_bars));
    let weights = dataplane::label::uniqueness_weights(&intervals, clock.len().max(1));
    frame.sample_weight = frame
        .ts_ns
        .iter()
        .map(|ts| tick_index.get(ts).and_then(|i| weights.get(*i)).copied().unwrap_or(1.0))
        .collect();

    frame.age_minutes = Vec::with_capacity(frame.feature_names.len());
    frame.quality = Vec::with_capacity(frame.feature_names.len());
    for name in &frame.feature_names {
        let lookback = crate::runtime::lookback_bars(name).unwrap_or(1);
        let mut ages = Vec::with_capacity(frame.ts_ns.len());
        let mut quals = Vec::with_capacity(frame.ts_ns.len());
        for &ts in &frame.ts_ns {
            let (age, q) = tick_index
                .get(&ts)
                .map_or((0, dataplane::quality::QualityFlags::NONE), |&i| {
                    clock.provenance(lookback, i)
                });
            ages.push(age);
            quals.push(q.0);
        }
        frame.age_minutes.push(ages);
        frame.quality.push(quals);
    }
    frame
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
pub fn align_higher_tf(base_ts_ns: &[i64], higher_tf_bars: &[HigherTfBar]) -> Vec<Option<f64>> {
    let mut out = vec![None; base_ts_ns.len()];
    let mut htf_idx = 0usize;

    for (i, &base_ts) in base_ts_ns.iter().enumerate() {
        // Advance higher-TF pointer as far as possible without exceeding base_ts.
        while htf_idx + 1 < higher_tf_bars.len() && higher_tf_bars[htf_idx + 1].ts_ns <= base_ts {
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

fn is_known_feature(name: &str) -> bool {
    crate::runtime::is_known(name)
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
        let bars = series(&(0..100).map(|i| 100.0 + f64::from(i)).collect::<Vec<_>>());
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

    // ------------------------------------------------------------------ //
    // AT-67: sample weights exist and mean something (SPEC §3.4)
    // ------------------------------------------------------------------ //

    fn obs(minute: i64, close: f64) -> crate::align::BarObs {
        let ts = minute * 60_000_000_000;
        crate::align::BarObs {
            ts_ns: ts,
            knowledge_ns: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            quality: dataplane::quality::QualityFlags::NONE,
        }
    }

    /// A label over `h` forward bars spans `h + 1` bars, so in the interior it
    /// runs concurrently with `h` of its neighbours and its average-uniqueness
    /// weight is about `1/(h+1)`. That is the whole point: sixty consecutive
    /// 60-bar labels describe nearly the same stretch of future, and a model told
    /// to treat them as sixty independent facts will believe it.
    #[test]
    fn overlapping_labels_are_downweighted_and_disjoint_ones_are_not() {
        let bars: Vec<crate::align::BarObs> =
            (0..600).map(|i| obs(i, 100.0 + (i as f64 * 0.11).sin())).collect();
        let names = vec!["close".to_string()];

        let short = build_aligned_training_frame(&bars, &names, 1, 60_000_000_000);
        let long = build_aligned_training_frame(&bars, &names, 60, 60_000_000_000);

        assert_eq!(short.sample_weight.len(), short.row_count());
        assert_eq!(long.sample_weight.len(), long.row_count());
        assert!(short.sample_weight.iter().all(|w| *w > 0.0 && *w <= 1.0));
        assert!(long.sample_weight.iter().all(|w| *w > 0.0 && *w <= 1.0));

        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (a_short, a_long) = (avg(&short.sample_weight), avg(&long.sample_weight));
        assert!(
            (a_short - 0.5).abs() < 0.05,
            "a 1-bar label spans 2 bars, so the mean weight should be ~1/2; got {a_short:.3}"
        );
        assert!(
            (a_long - 1.0 / 61.0).abs() < 0.01,
            "a 60-bar label spans 61 bars, so the mean weight should be ~1/61; got {a_long:.4}"
        );
        assert!(
            a_long < a_short / 10.0,
            "overlap must cost weight: {a_long:.4} vs {a_short:.3}"
        );
    }

    /// An unaligned frame carries no weights, and that is reported as absence
    /// rather than as a column of ones — the label spec's declaration depends on
    /// knowing which it is.
    #[test]
    fn an_unaligned_frame_carries_no_weights() {
        let rows: Vec<OhlcvRow> = (0..200)
            .map(|i| OhlcvRow { ts_ns: i * 60_000_000_000, close: 100.0 + i as f64, ..OhlcvRow::default() })
            .collect();
        let f = build_training_frame(&rows, &["close".to_string()], 1);
        assert!(f.sample_weight.is_empty());
    }
}
