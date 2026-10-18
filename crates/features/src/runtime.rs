//! The single feature runtime (SPEC §3.2–§3.3; INV-13, INV-14; AT-13, AT-15).
//!
//! Every feature the platform computes — in a backtest, a training frame, a warm
//! start, or a live serve — is resolved here by name to one windowed implementation
//! and evaluated through [`dataplane::feature::EVALUATE_FN`]. There is no second
//! implementation of any feature anywhere, and none in any other language.
//!
//! A feature exists for a row only once its full declared window exists; before
//! that it is absent (NaN / `None`), never a partial-window value.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use dataplane::feature::{
    register_check, Feature, FeatureDef, FeatureError, FeatureRow, InfoClass, WindowedFrame,
    EVALUATE_FN,
};

use crate::{Ema, Rsi};

/// Bars of window for the on-balance-volume feature (cumulative within the window).
pub const OBV_WINDOW: u32 = 256;

/// Build a feature definition whose `code_hash` is the hash of the source that
/// implements it, so a served value records exactly which code produced it.
#[must_use]
pub fn definition(
    feature_id: &str,
    family: &str,
    version: u32,
    lookback_bars: u32,
    source: &str,
) -> FeatureDef {
    FeatureDef {
        feature_id: feature_id.to_string(),
        version,
        code_hash: dataplane::content_hash(&(family, version, source)).unwrap_or_default(),
        lookback_bars,
        knowledge_lag_ms: 0,
        output_dtype: "f64".into(),
        asset_classes: vec![
            "equity".into(),
            "crypto".into(),
            "futures".into(),
            "fx".into(),
        ],
        deflators: vec![],
        info_class: InfoClass::MarketPublic,
    }
}

const SOURCE: &str = include_str!("runtime.rs");
const VERSION: u32 = 2;

fn def(id: &str, family: &str, lookback: u32) -> FeatureDef {
    definition(id, family, VERSION, lookback, SOURCE)
}

#[derive(Clone, Copy)]
enum Field {
    Open,
    High,
    Low,
    Close,
    Volume,
}

struct Passthrough(FeatureDef, Field);

impl Feature for Passthrough {
    fn def(&self) -> &FeatureDef {
        &self.0
    }
    fn compute(&self, f: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let r = f.back(0)?;
        Ok(match self.1 {
            Field::Open => r.open,
            Field::High => r.high,
            Field::Low => r.low,
            Field::Close => r.close,
            Field::Volume => r.volume,
        })
    }
}

#[allow(clippy::cast_precision_loss)]
fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Sample standard deviation (ddof = 1).
#[allow(clippy::cast_precision_loss)]
fn sample_std(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return f64::NAN;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
}

/// The computation behind a catalogued family.
///
/// One variant per *algorithm*, not per name: `sma_20` and `rolling_mean_20` are
/// the same `RollingMean`, and `returns_5` and `momentum_5` the same
/// `SimpleReturn` — the registry rewrites the alias before anything is built, so
/// there is one implementation per computation (INV-14) and one name per
/// implementation in the `code_hash`.
#[derive(Clone, Copy)]
enum Kind {
    // ── rolling statistics over the close ────────────────────────────────────
    RollingMean,
    RollingStd,
    RollingVar,
    RollingMin,
    RollingMax,
    RollingSum,
    RollingMedian,
    /// Percentile of the window, `q` in whole percent, linear interpolation.
    RollingQuantile(usize),
    RollingSkew,
    RollingKurt,
    Zscore,
    /// Fraction of the window strictly below the decision bar's close, 0..1.
    PctRank,
    Ewma(usize),

    // ── returns, lags, differences ───────────────────────────────────────────
    /// `close / close[k] − 1`; `returns_k`, `momentum_k` and `ret_k` are one
    /// feature under three names and share this implementation.
    SimpleReturn(usize),
    LogReturn(usize),
    Lag(usize),
    Diff(usize),
    Roc(usize),

    // ── indicators ───────────────────────────────────────────────────────────
    /// Wilder average true range, in price units.
    Atr(usize),
    /// `ema(fast) − ema(slow)`.
    Macd {
        fast: usize,
        slow: usize,
    },
    /// `ema(signal)` of the macd series.
    MacdSignal {
        fast: usize,
        slow: usize,
        signal: usize,
    },
    /// macd minus its signal line.
    MacdHist {
        fast: usize,
        slow: usize,
        signal: usize,
    },
    /// Bollinger band, `k` sample standard deviations from the rolling mean.
    Bollinger {
        k: usize,
        part: BandPart,
    },
    /// Donchian channel over the window's highs and lows.
    Donchian(BandPart),
    /// Keltner band: `ema(n)` ± `k · atr(n)`.
    Keltner {
        n: usize,
        k: usize,
        upper: bool,
    },
    Obv,
    RelVolume,

    // ── calendar ─────────────────────────────────────────────────────────────
    HourSin,
    HourCos,
    DowSin,
    DowCos,
    /// 1 inside `[from, to)` UTC hours, else 0.
    UtcSession {
        from: i64,
        to: i64,
    },

    // ── range volatility and realised measures (F2) ──────────────────────────
    Parkinson,
    GarmanKlass,
    RogersSatchell,
    /// Root sum of squared bar log returns over the window.
    RealisedVol,
    /// Bipower variation, jump-robust, on the same scale as `RealisedVol`.
    BipowerVar,
    /// `max(0, (rv² − bv²) / rv²)` — the share of realised variance that is jump.
    JumpShare,
    Autocorr(usize),
    VarianceRatio(usize),
    EfficiencyRatio,
    // Yang-Zhang, Hurst, fractional difference and the two ensembles live in `families`.
    YangZhang,
    Hurst,
    Fracdiff(usize),
    EnsembleZscore {
        n1: usize,
        n2: usize,
        n3: usize,
    },
    EnsembleReturns {
        n1: usize,
        n2: usize,
        n3: usize,
    },
}

/// Which line of a two-sided band a feature returns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BandPart {
    Upper,
    Lower,
    Mid,
    /// Band height relative to the midpoint — a squeeze is a low value.
    Width,
    /// Where the close sits in the band: 0 at the lower line, 1 at the upper.
    PctB,
}

struct Windowed(FeatureDef, Kind);

/// The window's bars, **newest first**: index `b` is `b` bars before the decision
/// bar, the same indexing `WindowedFrame::back` uses.
///
/// Newest-first and not oldest-first for one reason: the pre-existing families
/// summed their windows in this order, and a sum reordered is a different `f64`.
/// The golden parity test DATA-006 §5 requires compares old and new values with
/// tolerance zero, so the order is part of the contract, not a style choice.
struct Series {
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

impl Series {
    fn read(f: &WindowedFrame<'_>) -> Result<Self, FeatureError> {
        let n = f.lookback() as usize;
        let mut s = Self {
            open: Vec::with_capacity(n),
            high: Vec::with_capacity(n),
            low: Vec::with_capacity(n),
            close: Vec::with_capacity(n),
            volume: Vec::with_capacity(n),
        };
        for back in 0..n {
            let r = f.back(back)?;
            s.open.push(r.open);
            s.high.push(r.high);
            s.low.push(r.low);
            s.close.push(r.close);
            s.volume.push(r.volume);
        }
        Ok(s)
    }

    /// The closes oldest first — what the EMA kernel and every path-ordered
    /// measure want.
    fn closes_oldest_first(&self) -> Vec<f64> {
        self.close.iter().rev().copied().collect()
    }
}

/// Bar log returns, newest first: element `i` is `ln(close[i] / close[i+1])`.
/// `NaN` for any non-positive close, never zero.
fn log_returns(closes_newest_first: &[f64]) -> Vec<f64> {
    closes_newest_first
        .windows(2)
        .map(|w| {
            if w[0] <= 0.0 || w[1] <= 0.0 {
                f64::NAN
            } else {
                (w[0] / w[1]).ln()
            }
        })
        .collect()
}

/// The `m`-th central moment of `v`, population form.
#[allow(clippy::cast_precision_loss)]
fn central_moment(v: &[f64], m: i32) -> f64 {
    let mu = mean(v);
    v.iter().map(|x| (x - mu).powi(m)).sum::<f64>() / v.len() as f64
}

/// Linear-interpolated percentile of `v`, `q` in whole percent.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn percentile(v: &[f64], q: usize) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = (q as f64 / 100.0) * (s.len() - 1) as f64;
    let (lo, hi) = (pos.floor(), pos.ceil());
    let (a, b) = (s[lo as usize], s[hi as usize]);
    if (hi - lo).abs() < f64::EPSILON {
        a
    } else {
        a + (b - a) * (pos - lo)
    }
}

/// Bipower variation of `returns`, on the same scale as realised volatility.
///
/// `μ₁ = √(2/π)` is the expectation of the absolute value of a standard normal;
/// dividing by `μ₁²` makes bipower an unbiased estimator of integrated variance
/// under a continuous price path, so `rv² − bv²` isolates the jump component.
fn bipower(returns: &[f64]) -> f64 {
    if returns.len() < 2 {
        return f64::NAN;
    }
    let mu1_sq = 2.0 / std::f64::consts::PI;
    let acc: f64 = returns.windows(2).map(|x| x[0].abs() * x[1].abs()).sum();
    (acc / mu1_sq).sqrt()
}

impl Windowed {
    /// `ema(fast) − ema(slow)` over the `slow`-span window ending `back` bars
    /// before the decision bar. Both legs go through the one EMA kernel.
    fn macd_at(closes_oldest_first: &[f64], fast: usize, slow: usize, back: usize) -> f64 {
        let span = slow * crate::ema::EMA_LOOKBACK_PERIODS;
        let end = closes_oldest_first.len().checked_sub(back);
        let Some(end) = end.filter(|e| *e >= span) else {
            return f64::NAN;
        };
        let w = &closes_oldest_first[end - span..end];
        let fast_span = fast * crate::ema::EMA_LOOKBACK_PERIODS;
        crate::ema::ema_from_oldest(&w[span - fast_span.min(span)..], fast)
            - crate::ema::ema_from_oldest(w, slow)
    }

    /// `ema(signal)` of the macd series ending at the decision bar.
    fn macd_signal_at(closes_oldest_first: &[f64], fast: usize, slow: usize, signal: usize) -> f64 {
        let span = slow * crate::ema::EMA_LOOKBACK_PERIODS;
        let available = closes_oldest_first.len().saturating_sub(span) + 1;
        let count = (signal * crate::ema::EMA_LOOKBACK_PERIODS).min(available);
        if count == 0 || closes_oldest_first.len() < span {
            return f64::NAN;
        }
        // Oldest first: j = 0 is `count - 1` bars back, j = count - 1 is the
        // decision bar.
        let macds: Vec<f64> = (0..count)
            .map(|j| Self::macd_at(closes_oldest_first, fast, slow, count - 1 - j))
            .collect();
        crate::ema::ema_from_oldest(&macds, signal)
    }

    /// Wilder's average true range over the window, in price units. Samples are
    /// pushed oldest first, as Wilder's recurrence requires.
    fn atr_value(s: &Series, period: usize) -> f64 {
        let mut w = crate::rsi::WilderAverage::new(period);
        for i in (0..s.close.len().saturating_sub(1)).rev() {
            let prev = s.close[i + 1];
            let tr = (s.high[i] - s.low[i])
                .max((s.high[i] - prev).abs())
                .max((s.low[i] - prev).abs());
            w.push(tr);
        }
        w.value()
    }
}

impl Feature for Windowed {
    fn def(&self) -> &FeatureDef {
        &self.0
    }

    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn compute(&self, f: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        // The calendar kinds read one bar; reading the whole window for them
        // would be a lie about what they depend on as well as a waste.
        match self.1 {
            Kind::HourSin | Kind::HourCos => {
                let hour = (f.back(0)?.ts_ns.div_euclid(3_600_000_000_000)).rem_euclid(24) as f64;
                let a = hour * std::f64::consts::TAU / 24.0;
                return Ok(if matches!(self.1, Kind::HourSin) {
                    a.sin()
                } else {
                    a.cos()
                });
            }
            Kind::DowSin | Kind::DowCos => {
                // Unix day 0 was a Thursday; 0 = Monday.
                let dow =
                    (f.back(0)?.ts_ns.div_euclid(86_400_000_000_000) + 3).rem_euclid(7) as f64;
                let a = dow * std::f64::consts::TAU / 7.0;
                return Ok(if matches!(self.1, Kind::DowSin) {
                    a.sin()
                } else {
                    a.cos()
                });
            }
            Kind::UtcSession { from, to } => {
                let hour = (f.back(0)?.ts_ns.div_euclid(3_600_000_000_000)).rem_euclid(24);
                return Ok(f64::from(u8::from(hour >= from && hour < to)));
            }
            _ => {}
        }

        let w = f.lookback() as usize;
        let s = Series::read(f)?;
        let c = &s.close;
        Ok(match self.1 {
            Kind::HourSin
            | Kind::HourCos
            | Kind::DowSin
            | Kind::DowCos
            | Kind::UtcSession { .. } => unreachable!("handled above"),

            Kind::RollingMean => mean(c),
            Kind::RollingStd => sample_std(c),
            Kind::RollingVar => {
                let sd = sample_std(c);
                sd * sd
            }
            Kind::RollingMin => c.iter().copied().fold(f64::INFINITY, f64::min),
            Kind::RollingMax => c.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Kind::RollingSum => c.iter().sum(),
            Kind::RollingMedian => percentile(c, 50),
            Kind::RollingQuantile(q) => percentile(c, q),
            Kind::RollingSkew => {
                let m2 = central_moment(c, 2);
                if m2 <= 0.0 {
                    f64::NAN
                } else {
                    central_moment(c, 3) / m2.powf(1.5)
                }
            }
            Kind::RollingKurt => {
                let m2 = central_moment(c, 2);
                if m2 <= 0.0 {
                    f64::NAN
                } else {
                    central_moment(c, 4) / (m2 * m2) - 3.0
                }
            }
            Kind::Zscore => {
                let sd = sample_std(c);
                if sd == 0.0 {
                    f64::NAN
                } else {
                    (c[0] - mean(c)) / sd
                }
            }
            Kind::PctRank => {
                let below = c[1..].iter().filter(|x| **x < c[0]).count();
                below as f64 / (c.len() - 1).max(1) as f64
            }
            Kind::Ewma(halflife) => {
                // A halflife h decays by 2^(−1/h) per bar; the equivalent EMA
                // period is what the one kernel takes, so there is no second
                // exponential recurrence in the crate.
                let alpha = 1.0 - (-(2.0_f64.ln()) / halflife as f64).exp();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let p = (2.0 / alpha - 1.0).round().max(1.0) as usize;
                crate::ema::ema_from_oldest(&s.closes_oldest_first(), p)
            }

            Kind::SimpleReturn(k) => {
                let base = c[k];
                if base == 0.0 {
                    f64::NAN
                } else {
                    c[0] / base - 1.0
                }
            }
            Kind::LogReturn(k) => {
                if c[0] <= 0.0 || c[k] <= 0.0 {
                    f64::NAN
                } else {
                    (c[0] / c[k]).ln()
                }
            }
            Kind::Lag(k) => c[k],
            Kind::Diff(k) => c[0] - c[k],
            Kind::Roc(k) => {
                if c[k] == 0.0 {
                    f64::NAN
                } else {
                    (c[0] / c[k] - 1.0) * 100.0
                }
            }

            Kind::Atr(p) => Self::atr_value(&s, p),
            Kind::Macd { fast, slow } => Self::macd_at(&s.closes_oldest_first(), fast, slow, 0),
            Kind::MacdSignal { fast, slow, signal } => {
                Self::macd_signal_at(&s.closes_oldest_first(), fast, slow, signal)
            }
            Kind::MacdHist { fast, slow, signal } => {
                let o = s.closes_oldest_first();
                Self::macd_at(&o, fast, slow, 0) - Self::macd_signal_at(&o, fast, slow, signal)
            }
            Kind::Bollinger { k, part } => {
                let (m, sd) = (mean(c), sample_std(c));
                let d = k as f64 * sd;
                match part {
                    BandPart::Upper => m + d,
                    BandPart::Lower => m - d,
                    BandPart::Mid => m,
                    BandPart::Width if m == 0.0 => f64::NAN,
                    BandPart::Width => 2.0 * d / m,
                    BandPart::PctB if d == 0.0 => f64::NAN,
                    BandPart::PctB => (c[0] - (m - d)) / (2.0 * d),
                }
            }
            Kind::Donchian(part) => {
                let hi = s.high.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let lo = s.low.iter().copied().fold(f64::INFINITY, f64::min);
                let mid = (hi + lo) / 2.0;
                match part {
                    BandPart::Upper => hi,
                    BandPart::Lower => lo,
                    BandPart::Mid => mid,
                    BandPart::Width if mid == 0.0 => f64::NAN,
                    BandPart::Width => (hi - lo) / mid,
                    BandPart::PctB if hi == lo => f64::NAN,
                    BandPart::PctB => (c[0] - lo) / (hi - lo),
                }
            }
            Kind::Keltner { n, k, upper } => {
                let o = s.closes_oldest_first();
                let span = n * crate::ema::EMA_LOOKBACK_PERIODS;
                if o.len() < span {
                    f64::NAN
                } else {
                    let centre = crate::ema::ema_from_oldest(&o[o.len() - span..], n);
                    let band = k as f64 * Self::atr_value(&s, n);
                    if upper {
                        centre + band
                    } else {
                        centre - band
                    }
                }
            }
            Kind::Obv => {
                let mut obv = 0.0;
                for b in 0..w - 1 {
                    if c[b] > c[b + 1] {
                        obv += s.volume[b];
                    } else if c[b] < c[b + 1] {
                        obv -= s.volume[b];
                    }
                }
                obv
            }
            Kind::RelVolume => {
                let m = mean(&s.volume);
                if m <= 0.0 {
                    f64::NAN
                } else {
                    s.volume[0] / m
                }
            }

            Kind::Parkinson => {
                let inv_4ln2 = 1.0 / (4.0 * 2.0_f64.ln());
                let mut acc = 0.0;
                for b in 0..w {
                    if s.low[b] <= 0.0 || s.high[b] <= 0.0 {
                        return Ok(f64::NAN);
                    }
                    acc += (s.high[b] / s.low[b]).ln().powi(2) * inv_4ln2;
                }
                (acc / w as f64).sqrt()
            }
            Kind::GarmanKlass => {
                let c1 = 2.0 * 2.0_f64.ln() - 1.0;
                let mut acc = 0.0;
                for b in 0..w {
                    if s.low[b] <= 0.0 || s.high[b] <= 0.0 || s.open[b] <= 0.0 || s.close[b] <= 0.0
                    {
                        return Ok(f64::NAN);
                    }
                    acc += (0.5 * (s.high[b] / s.low[b]).ln().powi(2)
                        - c1 * (s.close[b] / s.open[b]).ln().powi(2))
                    .max(0.0);
                }
                (acc / w as f64).sqrt()
            }
            Kind::RogersSatchell => {
                let mut acc = 0.0;
                for b in 0..w {
                    if s.low[b] <= 0.0 || s.high[b] <= 0.0 || s.open[b] <= 0.0 || s.close[b] <= 0.0
                    {
                        return Ok(f64::NAN);
                    }
                    let (h, l, o, cl) = (s.high[b], s.low[b], s.open[b], s.close[b]);
                    acc += (h / cl).ln() * (h / o).ln() + (l / cl).ln() * (l / o).ln();
                }
                (acc.max(0.0) / w as f64).sqrt()
            }
            Kind::RealisedVol => log_returns(c).iter().map(|x| x * x).sum::<f64>().sqrt(),
            Kind::BipowerVar => bipower(&log_returns(c)),
            Kind::JumpShare => {
                let r = log_returns(c);
                let rv2 = r.iter().map(|x| x * x).sum::<f64>();
                let bv = bipower(&r);
                if rv2 <= 0.0 {
                    f64::NAN
                } else {
                    ((rv2 - bv * bv) / rv2).clamp(0.0, 1.0)
                }
            }
            Kind::Autocorr(lag) => {
                let r = log_returns(c);
                if r.len() <= lag + 1 {
                    f64::NAN
                } else {
                    let m = mean(&r);
                    let denom: f64 = r.iter().map(|x| (x - m) * (x - m)).sum();
                    if denom <= 0.0 {
                        f64::NAN
                    } else {
                        let num: f64 = (lag..r.len()).map(|i| (r[i] - m) * (r[i - lag] - m)).sum();
                        num / denom
                    }
                }
            }
            // Lo-MacKinlay, over **overlapping** q-windows.
            //
            // The obvious implementation aggregates with `chunks_exact(q)`,
            // which keeps every qth window and discards the rest — on 2,000
            // returns at q=5 that is 400 observations instead of 1,996. It is a
            // different and much noisier statistic, and this family's registry
            // description names Lo and MacKinlay, whose whole contribution was
            // the overlapping estimator and its variance.
            //
            // It was the non-overlapping form until the S-5.14 reconciliation
            // measured it against `detect_regime.py`, which had the overlapping
            // one: 0.4515 against 0.4266 on the same 2,011 bars.
            //
            // The divisors are the template's, deliberately, so that the two
            // implementations are the same estimator and not merely the same
            // family: `n - 1` for the one-bar variance and `(n - q + 1) · q` for
            // the aggregated one.
            Kind::VarianceRatio(q) => {
                let r = log_returns(c);
                let n = r.len();
                if n <= q || q == 0 {
                    f64::NAN
                } else {
                    let mu = mean(&r);
                    #[allow(clippy::cast_precision_loss)]
                    let var1 = r.iter().map(|x| (x - mu) * (x - mu)).sum::<f64>() / (n - 1) as f64;
                    if var1 <= 0.0 {
                        f64::NAN
                    } else {
                        // Every window of q consecutive returns, not every qth.
                        let windows = n - q + 1;
                        let q_mu = q as f64 * mu;
                        let mut acc = 0.0;
                        let mut running: f64 = r[..q].iter().sum();
                        acc += (running - q_mu) * (running - q_mu);
                        for i in q..n {
                            running += r[i] - r[i - q];
                            acc += (running - q_mu) * (running - q_mu);
                        }
                        #[allow(clippy::cast_precision_loss)]
                        let var_q = acc / (windows as f64 * q as f64);
                        var_q / var1
                    }
                }
            }
            Kind::YangZhang => {
                crate::families::yang_zhang::yang_zhang_vol(&s.open, &s.high, &s.low, &s.close)
            }
            Kind::Hurst => {
                let oldest = s.closes_oldest_first();
                let r: Vec<f64> = oldest.windows(2).map(|w| w[1].ln() - w[0].ln()).collect();
                crate::families::hurst::hurst_rs(&r)
            }
            Kind::Fracdiff(d) => {
                crate::families::fracdiff::fracdiff_log_close(&s.close, d as f64 / 100.0)
            }
            Kind::EnsembleZscore { n1, n2, n3 } => {
                crate::families::ensemble::mean_zscore(&s.close, &[n1, n2, n3])
            }
            Kind::EnsembleReturns { n1, n2, n3 } => {
                crate::families::ensemble::mean_return(&s.close, &[n1, n2, n3])
            }
            Kind::EfficiencyRatio => {
                let net = (c[0] - c[w - 1]).abs();
                let path: f64 = c.windows(2).map(|x| (x[0] - x[1]).abs()).sum();
                if path == 0.0 {
                    f64::NAN
                } else {
                    net / path
                }
            }
        })
    }
}

/// Construct the implementation for `name`, unregistered.
///
/// The registry decides what exists; this decides how it is computed. Every
/// family in [`crate::registry::families`] is answered here and nothing else is
/// — `the_catalogue_and_the_constructor_agree_in_both_directions` fails the build
/// on either kind of drift, so a family cannot be listed and uncomputable, nor
/// computable and unlisted.
fn construct(name: &str) -> Option<Arc<dyn Feature>> {
    let r = crate::registry::resolve(name)?;
    let p = r.params();
    let lookback = r.lookback_bars()?;
    let family = r.family;
    let passthrough =
        |field| Some(Arc::new(Passthrough(def(name, "passthrough", 1), field)) as Arc<dyn Feature>);
    let windowed = |kind| {
        let mut d = def(name, family.name, lookback);
        d.knowledge_lag_ms = family.pit_lag_ms;
        d.output_dtype = family.output_dtype.to_string();
        Some(Arc::new(Windowed(d, kind)) as Arc<dyn Feature>)
    };
    match family.name {
        "open" => passthrough(Field::Open),
        "high" => passthrough(Field::High),
        "low" => passthrough(Field::Low),
        "close" => passthrough(Field::Close),
        "volume" => passthrough(Field::Volume),

        "ema" => Some(Arc::new(Ema::new(p[0])) as Arc<dyn Feature>),
        "rsi" => Some(Arc::new(Rsi::new(p[0])) as Arc<dyn Feature>),

        "returns" => windowed(Kind::SimpleReturn(p[0])),
        "log_returns" => windowed(Kind::LogReturn(p[0])),
        "lag" => windowed(Kind::Lag(p[0])),
        "diff" => windowed(Kind::Diff(p[0])),
        "roc" => windowed(Kind::Roc(p[0])),

        "rolling_mean" => windowed(Kind::RollingMean),
        "rolling_std" => windowed(Kind::RollingStd),
        "rolling_var" => windowed(Kind::RollingVar),
        "rolling_min" => windowed(Kind::RollingMin),
        "rolling_max" => windowed(Kind::RollingMax),
        "rolling_sum" => windowed(Kind::RollingSum),
        "rolling_median" => windowed(Kind::RollingMedian),
        "rolling_quantile" => windowed(Kind::RollingQuantile(p[1])),
        "rolling_skew" => windowed(Kind::RollingSkew),
        "rolling_kurt" => windowed(Kind::RollingKurt),
        "zscore" => windowed(Kind::Zscore),
        "pct_rank" => windowed(Kind::PctRank),
        "ewma" => windowed(Kind::Ewma(p[0])),

        "atr" => windowed(Kind::Atr(p[0])),
        "macd" => windowed(Kind::Macd {
            fast: p[0],
            slow: p[1],
        }),
        "macd_signal" => windowed(Kind::MacdSignal {
            fast: p[0],
            slow: p[1],
            signal: p[2],
        }),
        "macd_hist" => windowed(Kind::MacdHist {
            fast: p[0],
            slow: p[1],
            signal: p[2],
        }),
        "bb_upper" => windowed(Kind::Bollinger {
            k: p[1],
            part: BandPart::Upper,
        }),
        "bb_lower" => windowed(Kind::Bollinger {
            k: p[1],
            part: BandPart::Lower,
        }),
        "bb_width" => windowed(Kind::Bollinger {
            k: p[1],
            part: BandPart::Width,
        }),
        "bb_pctb" => windowed(Kind::Bollinger {
            k: p[1],
            part: BandPart::PctB,
        }),
        "donchian_upper" => windowed(Kind::Donchian(BandPart::Upper)),
        "donchian_lower" => windowed(Kind::Donchian(BandPart::Lower)),
        "donchian_mid" => windowed(Kind::Donchian(BandPart::Mid)),
        "donchian_width" => windowed(Kind::Donchian(BandPart::Width)),
        "keltner_upper" => windowed(Kind::Keltner {
            n: p[0],
            k: p[1],
            upper: true,
        }),
        "keltner_lower" => windowed(Kind::Keltner {
            n: p[0],
            k: p[1],
            upper: false,
        }),
        "obv" => windowed(Kind::Obv),
        "rel_volume" => windowed(Kind::RelVolume),

        "hour_sin" => windowed(Kind::HourSin),
        "hour_cos" => windowed(Kind::HourCos),
        "dow_sin" => windowed(Kind::DowSin),
        "dow_cos" => windowed(Kind::DowCos),
        "utc_session_asia" => windowed(Kind::UtcSession { from: 0, to: 8 }),
        "utc_session_london" => windowed(Kind::UtcSession { from: 7, to: 16 }),
        "utc_session_us" => windowed(Kind::UtcSession { from: 13, to: 22 }),

        "parkinson_vol" => windowed(Kind::Parkinson),
        "garman_klass_vol" => windowed(Kind::GarmanKlass),
        "rogers_satchell_vol" => windowed(Kind::RogersSatchell),
        "rv" => windowed(Kind::RealisedVol),
        "bv" => windowed(Kind::BipowerVar),
        "jump_share" => windowed(Kind::JumpShare),
        "autocorr" => windowed(Kind::Autocorr(p[1])),
        "variance_ratio" => windowed(Kind::VarianceRatio(p[1])),
        "efficiency_ratio" => windowed(Kind::EfficiencyRatio),
        "yang_zhang_vol" => windowed(Kind::YangZhang),
        "hurst" => windowed(Kind::Hurst),
        "fracdiff" => windowed(Kind::Fracdiff(p[1])),
        "lookback_ensemble_zscore" => windowed(Kind::EnsembleZscore {
            n1: p[0],
            n2: p[1],
            n3: p[2],
        }),
        "lookback_ensemble_returns" => windowed(Kind::EnsembleReturns {
            n1: p[0],
            n2: p[1],
            n3: p[2],
        }),

        // Unreachable while the catalogue and this match agree, which
        // `the_catalogue_and_the_constructor_agree_in_both_directions` asserts.
        _ => None,
    }
}

/// Resolve `name` to its registered implementation. Every feature passes the
/// declared-lookback registration check before it can be evaluated (AT-13); the
/// result is cached per name.
///
/// # Errors
/// [`FeatureError::Unknown`] for a name no implementation answers to, or the
/// registration failure.
pub fn feature(name: &str) -> Result<Arc<dyn Feature>, FeatureError> {
    static REGISTERED: OnceLock<Mutex<HashMap<String, Arc<dyn Feature>>>> = OnceLock::new();
    let cache = REGISTERED.get_or_init(Default::default);
    if let Some(f) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(name)
    {
        return Ok(Arc::clone(f));
    }
    // A name that declares an external input is not a windowed feature: computing it
    // from bars alone would answer with a guess, so say where it lives (P-3.6).
    if crate::external_features::is_external_name(name) {
        return Err(FeatureError::Registration {
            feature: name.to_string(),
            reason: "declares an external input; resolve it with                      features::external_features::resolve_external and evaluate it with                      evaluate_external, handing in the point-in-time rows"
                .into(),
        });
    }
    let f = construct(name).ok_or_else(|| FeatureError::Unknown(name.to_string()))?;
    register_check(f.as_ref())?;
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string(), Arc::clone(&f));
    Ok(f)
}

/// The versioned feature-DAG hash for an ordered feature list (SPEC §3.1
/// `feature_set_id`). It covers each feature's `(feature_id, version, code_hash)`,
/// so a changed implementation yields a different id — and therefore a different
/// `dataset_id` — rather than silently reusing a snapshot built by older code.
///
/// Order is significant: the feature set's declared order is the column order.
///
/// # Errors
/// [`FeatureError::Unknown`] if any name has no implementation, or that name's
/// registration failure.
pub fn feature_set_version_hash(names: &[String]) -> Result<String, FeatureError> {
    let mut triples = Vec::with_capacity(names.len());
    for name in names {
        let d = feature(name)?;
        let d = d.def();
        triples.push((d.feature_id.clone(), d.version, d.code_hash.clone()));
    }
    dataplane::content_hash(&triples).map_err(|e| FeatureError::Registration {
        feature: "<feature_set>".to_string(),
        reason: e.to_string(),
    })
}

/// Whether `name` resolves to an implementation.
#[must_use]
pub fn is_known(name: &str) -> bool {
    construct(name).is_some()
}

/// Declared lookback of `name`, in bars.
#[must_use]
pub fn lookback_bars(name: &str) -> Option<u32> {
    construct(name).map(|f| f.def().lookback_bars)
}

/// Value of one feature at `decision`, `None` when absent (window incomplete or a
/// non-finite result).
#[must_use]
pub fn value_at(feature: &dyn Feature, rows: &[FeatureRow], decision: usize) -> Option<f64> {
    EVALUATE_FN(feature, rows, decision)
        .ok()
        .filter(|v| v.is_finite())
}

/// Batch path: one column per feature, `None` where absent.
#[must_use]
pub fn backfill_column(feature: &dyn Feature, rows: &[FeatureRow]) -> Vec<Option<f64>> {
    (0..rows.len())
        .map(|i| value_at(feature, rows, i))
        .collect()
}

/// The latest value of each named feature over `rows`.
///
/// # Errors
/// Unknown or unregistrable names.
pub fn latest(
    names: &[String],
    rows: &[FeatureRow],
) -> Result<BTreeMap<String, Option<f64>>, FeatureError> {
    let last = rows.len().checked_sub(1);
    names
        .iter()
        .map(|n| {
            let f = feature(n)?;
            Ok((n.clone(), last.and_then(|d| value_at(f.as_ref(), rows, d))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(closes: &[f64]) -> Vec<FeatureRow> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| FeatureRow {
                ts_ns: i as i64 * 3_600_000_000_000,
                open: c,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                volume: 1.0 + (i % 3) as f64,
            })
            .collect()
    }

    /// The variance ratio aggregates over **overlapping** q-windows.
    ///
    /// Pinned here as well as in the cross-language reconciliation
    /// (`crates/mcp-server/tests/volatility_reconciliation.rs`) because that one
    /// needs a Python interpreter, and a developer without one would otherwise
    /// lose the only guard on this.
    ///
    /// Both aggregations are written out below. That is a formula restated in a
    /// test, which is usually a smell, and here it is the point: the two differ
    /// only in which windows they keep, the family was the non-overlapping one
    /// until S-5.14 measured it, and a test that could not tell them apart is
    /// what let that stand.
    #[test]
    fn the_variance_ratio_aggregates_over_overlapping_windows() {
        // Alternating returns: strongly mean-reverting, which is the case the
        // statistic exists to detect and where the two aggregations differ most.
        let mut closes = vec![100.0];
        for i in 0..80 {
            let step = if i % 2 == 0 { 1.03 } else { 1.0 / 1.02 };
            closes.push(closes[closes.len() - 1] * step);
        }
        let (n_param, q) = (50usize, 5usize);
        let f = feature(&format!("variance_ratio_{n_param}_{q}")).unwrap();
        let w = f.def().lookback_bars as usize;
        let r = rows(&closes);
        let got = value_at(f.as_ref(), &r, r.len() - 1).expect("a value");

        // The same window the engine read: the last `w` rows, newest first.
        let window: Vec<f64> = r[r.len() - w..].iter().rev().map(|x| x.close).collect();
        let ret = log_returns(&window);
        let n = ret.len();
        let mu = mean(&ret);
        let var1 = ret.iter().map(|x| (x - mu) * (x - mu)).sum::<f64>() / (n - 1) as f64;

        let overlapping = {
            let sums: Vec<f64> = (0..=n - q).map(|i| ret[i..i + q].iter().sum()).collect();
            let acc: f64 = sums
                .iter()
                .map(|s| (s - q as f64 * mu) * (s - q as f64 * mu))
                .sum();
            acc / (sums.len() as f64 * q as f64) / var1
        };
        let non_overlapping = {
            let sums: Vec<f64> = ret.chunks_exact(q).map(|x| x.iter().sum()).collect();
            let m = mean(&sums);
            let acc: f64 = sums.iter().map(|s| (s - m) * (s - m)).sum();
            acc / sums.len() as f64 / (q as f64 * central_moment(&ret, 2))
        };

        assert!(
            (got - overlapping).abs() < 1e-12,
            "the engine says {got} and the overlapping Lo-MacKinlay estimator says {overlapping}"
        );
        assert!(
            (got - non_overlapping).abs() > 1e-6,
            "the engine's {got} is indistinguishable from the non-overlapping aggregation \
             ({non_overlapping}) on a fixture chosen to separate them, so this test would not \
             have caught the bug it exists for"
        );
    }

    #[test]
    fn every_catalogued_feature_registers() {
        for name in crate::feature_sets::known_features() {
            feature(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn unknown_and_degenerate_names_do_not_resolve() {
        for bad in [
            "ema_0",
            "rsi_1",
            "rolling_std_1",
            "returns_",
            "nope",
            "ema_x",
        ] {
            assert!(feature(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn values_are_absent_until_the_whole_window_exists() {
        let r = rows(&(0..40).map(|i| 100.0 + f64::from(i)).collect::<Vec<_>>());
        let ema = feature("ema_3").unwrap();
        let col = backfill_column(ema.as_ref(), &r);
        assert!(col[..14].iter().all(Option::is_none));
        assert!(col[14..].iter().all(Option::is_some));
        let rsi = feature("rsi_2").unwrap();
        assert!(backfill_column(rsi.as_ref(), &r)[10].is_some());
        assert!(backfill_column(rsi.as_ref(), &r)[9].is_none());
    }

    #[test]
    fn windowed_ema_matches_the_recursive_definition_on_its_window() {
        let closes: Vec<f64> = (0..60)
            .map(|i| (f64::from(i) * 0.3).sin() * 5.0 + 100.0)
            .collect();
        let r = rows(&closes);
        let got = value_at(feature("ema_4").unwrap().as_ref(), &r, 59).unwrap();
        let k = 2.0 / 5.0;
        let mut v = closes[40];
        for c in &closes[41..] {
            v = c * k + v * (1.0 - k);
        }
        assert_eq!(got.to_bits(), v.to_bits());
    }

    #[test]
    fn rsi_of_a_monotone_rise_is_100() {
        let r = rows(&(0..30).map(|i| 10.0 + f64::from(i)).collect::<Vec<_>>());
        assert_eq!(
            value_at(feature("rsi_3").unwrap().as_ref(), &r, 29),
            Some(100.0)
        );
    }

    #[test]
    fn returns_and_momentum_are_one_feature() {
        let r = rows(&[1.0, 2.0, 4.0, 8.0]);
        let a = value_at(feature("returns_2").unwrap().as_ref(), &r, 3);
        let b = value_at(feature("momentum_2").unwrap().as_ref(), &r, 3);
        assert_eq!(a, Some(3.0));
        assert_eq!(a, b);
    }

    #[test]
    fn code_hash_names_the_implementation() {
        let e = feature("ema_7").unwrap();
        let r = feature("rsi_14").unwrap();
        assert!(e.def().code_hash.starts_with("sha256:"));
        assert_ne!(e.def().code_hash, r.def().code_hash);
        assert_eq!(
            feature("ema_7").unwrap().def().code_hash,
            feature("ema_9").unwrap().def().code_hash
        );
    }

    /// The feature-set id is a version hash, not a name: a set with a different
    /// member, or the same members in a different order, is a different set.
    #[test]
    fn feature_set_version_hash_covers_membership_and_order() {
        let a = feature_set_version_hash(&["ema_7".into(), "rsi_14".into()]).unwrap();
        let same = feature_set_version_hash(&["ema_7".into(), "rsi_14".into()]).unwrap();
        let reordered = feature_set_version_hash(&["rsi_14".into(), "ema_7".into()]).unwrap();
        let extra =
            feature_set_version_hash(&["ema_7".into(), "rsi_14".into(), "close".into()]).unwrap();
        assert_eq!(a, same);
        assert_ne!(a, reordered, "column order is part of the set");
        assert_ne!(a, extra);
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn feature_set_version_hash_refuses_an_unknown_member() {
        assert!(feature_set_version_hash(&["ema_7".into(), "not_a_feature".into()]).is_err());
    }

    // ── the catalogue and the engine are one thing ───────────────────────────

    /// The match arms of `construct`, read out of its own source.
    fn constructor_arms() -> std::collections::BTreeSet<&'static str> {
        let body = SOURCE
            .split("fn construct(name: &str)")
            .nth(1)
            .expect("construct is in this file");
        let mut arms = std::collections::BTreeSet::new();
        for piece in body
            .split('"')
            .skip(1)
            .step_by(2)
            .zip(body.split('"').skip(2).step_by(2))
        {
            let (name, after) = piece;
            if after.trim_start().starts_with("=>") {
                arms.insert(name);
            }
        }
        arms
    }

    /// The registry says what exists and `construct` says how it is computed.
    /// Either can drift from the other, and both directions are silent failures
    /// with opposite symptoms: a listed-but-unbuildable family is a feature the
    /// model is told about and cannot use; a buildable-but-unlisted one is a
    /// feature that works and that nothing can discover, which is how the
    /// platform came to hold three disagreeing answers to "what features exist"
    /// (AGENT-011 §8.1).
    #[test]
    fn the_catalogue_and_the_constructor_agree_in_both_directions() {
        let mut unbuildable = Vec::new();
        for f in crate::registry::families() {
            let name = f.example();
            if construct(&name).is_none() {
                unbuildable.push(name);
            }
        }
        assert!(
            unbuildable.is_empty(),
            "catalogued but not constructible: {unbuildable:?}"
        );

        let arms = constructor_arms();
        assert!(
            arms.len() > 40,
            "the arm scan found only {}; it has drifted",
            arms.len()
        );
        let known: std::collections::BTreeSet<&str> = crate::registry::families()
            .iter()
            .filter(|f| f.alias_of.is_none())
            .map(|f| f.name)
            .collect();
        let orphan: Vec<&&str> = arms.difference(&known).collect();
        assert!(
            orphan.is_empty(),
            "constructor arms with no catalogue row: {orphan:?}"
        );
        let unhandled: Vec<&&str> = known.difference(&arms).collect();
        assert!(
            unhandled.is_empty(),
            "catalogue rows with no constructor arm: {unhandled:?}"
        );
    }

    /// Every family, at its example parameters, passes the declared-lookback
    /// registration check (AT-13) — so no catalogued feature can serve a value
    /// computed from bars outside the window it declares (INV-13).
    #[test]
    fn every_catalogued_family_passes_the_declared_lookback_check() {
        for f in crate::registry::families() {
            let name = f.example();
            feature(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    /// Adversarial (EXECUTION-BOARD §3.3): a family that reads further back than
    /// it declares must be **refused**, not merely reported. Without this the
    /// declaration is a comment — and a feature quietly reading three times its
    /// declared window is exactly what makes a warm-up requirement wrong, because
    /// the executor would load a lead-in that does not cover it.
    #[test]
    fn a_feature_that_reads_past_its_declared_lookback_is_refused() {
        struct UnderDeclared(FeatureDef);
        impl Feature for UnderDeclared {
            fn def(&self) -> &FeatureDef {
                &self.0
            }
            fn compute(&self, f: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
                let mut acc = 0.0;
                for b in 0..12 {
                    acc += f.close(b)?;
                }
                Ok(acc)
            }
        }
        let under = UnderDeclared(def("liar_4", "liar", 4));
        let err = register_check(&under)
            .expect_err("a feature reading past its window must not register");
        assert!(
            matches!(
                err,
                FeatureError::Registration { .. } | FeatureError::LookbackExceeded { .. }
            ),
            "unexpected error: {err}"
        );
    }

    /// A zero lookback is refused too. It reads like "no history needed" and is
    /// really "no window at all", which `WindowedFrame` cannot serve a bar from.
    #[test]
    fn a_feature_declaring_no_window_is_refused() {
        struct NoWindow(FeatureDef);
        impl Feature for NoWindow {
            fn def(&self) -> &FeatureDef {
                &self.0
            }
            fn compute(&self, _: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
                Ok(0.0)
            }
        }
        assert!(register_check(&NoWindow(def("void", "void", 0))).is_err());
    }

    // ── the families themselves ─────────────────────────────────────────────

    /// The ceiling this phase existed to remove: the platform used to accept
    /// `ema_N` and `rsi_N` and nothing else on the backtest path. These are the
    /// families a strategy can now name, and each produces a finite value.
    #[test]
    fn the_whole_catalogue_computes_a_finite_value_on_ordinary_bars() {
        let r = rows(
            &(0..4_000)
                .map(|i| 100.0 + (f64::from(i) * 0.013).sin() * 8.0 + f64::from(i) * 0.002)
                .collect::<Vec<_>>(),
        );
        let mut absent = Vec::new();
        for f in crate::registry::families() {
            let name = f.example();
            let feat = feature(&name).unwrap();
            match value_at(feat.as_ref(), &r, r.len() - 1) {
                Some(v) => assert!(v.is_finite(), "{name} = {v}"),
                None => absent.push(name),
            }
        }
        assert!(
            absent.is_empty(),
            "no value on 4,000 clean bars: {absent:?}"
        );
    }

    #[test]
    fn atr_is_the_wilder_average_of_the_true_range() {
        // Constant 2.0-wide bars with no gaps: every true range is 2.0, so any
        // weighting of them averages to 2.0.
        let r: Vec<FeatureRow> = (0..200)
            .map(|i| FeatureRow {
                ts_ns: i as i64 * 3_600_000_000_000,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1.0,
            })
            .collect();
        let v = value_at(feature("atr_14").unwrap().as_ref(), &r, 199).unwrap();
        assert!((v - 2.0).abs() < 1e-12, "atr_14 = {v}");
    }

    #[test]
    fn macd_is_the_difference_of_the_two_emas_it_names() {
        let closes: Vec<f64> = (0..1_000)
            .map(|i| 100.0 + (f64::from(i) * 0.07).sin() * 6.0)
            .collect();
        let r = rows(&closes);
        let d = r.len() - 1;
        let macd = value_at(feature("macd_12_26").unwrap().as_ref(), &r, d).unwrap();
        let fast = value_at(feature("ema_12").unwrap().as_ref(), &r, d).unwrap();
        let slow = value_at(feature("ema_26").unwrap().as_ref(), &r, d).unwrap();
        assert_eq!(
            macd.to_bits(),
            (fast - slow).to_bits(),
            "macd must be exactly its two legs"
        );
    }

    #[test]
    fn macd_hist_is_macd_minus_its_signal() {
        let closes: Vec<f64> = (0..1_500)
            .map(|i| 100.0 + (f64::from(i) * 0.05).cos() * 4.0)
            .collect();
        let r = rows(&closes);
        let d = r.len() - 1;
        let hist = value_at(feature("macd_hist_12_26_9").unwrap().as_ref(), &r, d).unwrap();
        let macd = value_at(feature("macd_12_26").unwrap().as_ref(), &r, d).unwrap();
        let sig = value_at(feature("macd_signal_12_26_9").unwrap().as_ref(), &r, d).unwrap();
        assert!(
            (hist - (macd - sig)).abs() < 1e-9,
            "{hist} vs {}",
            macd - sig
        );
    }

    #[test]
    fn a_bollinger_band_brackets_its_own_middle() {
        let r = rows(
            &(0..200)
                .map(|i| 100.0 + (f64::from(i) * 0.3).sin() * 5.0)
                .collect::<Vec<_>>(),
        );
        let d = 199;
        let up = value_at(feature("bb_upper_20_2").unwrap().as_ref(), &r, d).unwrap();
        let lo = value_at(feature("bb_lower_20_2").unwrap().as_ref(), &r, d).unwrap();
        let mid = value_at(feature("rolling_mean_20").unwrap().as_ref(), &r, d).unwrap();
        let pctb = value_at(feature("bb_pctb_20_2").unwrap().as_ref(), &r, d).unwrap();
        assert!(lo < mid && mid < up, "{lo} {mid} {up}");
        let close = r[d].close;
        assert!((pctb - (close - lo) / (up - lo)).abs() < 1e-12);
    }

    #[test]
    fn a_donchian_channel_contains_every_bar_of_its_window() {
        let r = rows(
            &(0..120)
                .map(|i| 100.0 + f64::from(i % 17))
                .collect::<Vec<_>>(),
        );
        let d = 119;
        let up = value_at(feature("donchian_upper_20").unwrap().as_ref(), &r, d).unwrap();
        let lo = value_at(feature("donchian_lower_20").unwrap().as_ref(), &r, d).unwrap();
        for row in &r[d - 19..=d] {
            assert!(row.high <= up + 1e-12 && row.low >= lo - 1e-12);
        }
    }

    #[test]
    fn pct_rank_is_one_at_a_window_high_and_zero_at_a_window_low() {
        let rising = rows(&(0..60).map(|i| 100.0 + f64::from(i)).collect::<Vec<_>>());
        assert_eq!(
            value_at(feature("pct_rank_20").unwrap().as_ref(), &rising, 59),
            Some(1.0)
        );
        let falling = rows(&(0..60).map(|i| 100.0 - f64::from(i)).collect::<Vec<_>>());
        assert_eq!(
            value_at(feature("pct_rank_20").unwrap().as_ref(), &falling, 59),
            Some(0.0)
        );
    }

    #[test]
    fn efficiency_ratio_is_one_on_a_straight_line_and_near_zero_on_a_saw() {
        let straight = rows(&(0..80).map(|i| 100.0 + f64::from(i)).collect::<Vec<_>>());
        let v = value_at(
            feature("efficiency_ratio_20").unwrap().as_ref(),
            &straight,
            79,
        )
        .unwrap();
        assert!((v - 1.0).abs() < 1e-12, "{v}");
        let saw = rows(
            &(0..80)
                .map(|i| 100.0 + f64::from(i % 2))
                .collect::<Vec<_>>(),
        );
        let v = value_at(feature("efficiency_ratio_20").unwrap().as_ref(), &saw, 79).unwrap();
        assert!(v < 0.1, "{v}");
    }

    #[test]
    fn a_utc_session_is_one_inside_its_window_and_zero_outside() {
        let r: Vec<FeatureRow> = (0..24)
            .map(|h| FeatureRow {
                ts_ns: i64::from(h) * 3_600_000_000_000,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect();
        let f = feature("utc_session_london").unwrap();
        assert_eq!(value_at(f.as_ref(), &r, 7), Some(1.0));
        assert_eq!(value_at(f.as_ref(), &r, 15), Some(1.0));
        assert_eq!(value_at(f.as_ref(), &r, 16), Some(0.0));
        assert_eq!(value_at(f.as_ref(), &r, 6), Some(0.0));
    }

    /// An alias is a name, not a second implementation: it must produce the same
    /// `code_hash` as its canonical family, or two paths could compute the same
    /// number and record different provenance for it (INV-14).
    #[test]
    fn an_alias_and_its_canonical_family_are_the_same_implementation() {
        for (alias, canonical) in [
            ("sma_20", "rolling_mean_20"),
            ("momentum_5", "returns_5"),
            ("ret_5", "returns_5"),
            ("logret_1", "log_returns_1"),
        ] {
            let a = feature(alias).unwrap();
            let c = feature(canonical).unwrap();
            assert_eq!(
                a.def().code_hash,
                c.def().code_hash,
                "{alias} vs {canonical}"
            );
            assert_eq!(a.def().lookback_bars, c.def().lookback_bars);
        }
        let r = rows(
            &(0..200)
                .map(|i| 100.0 + (f64::from(i) * 0.21).sin() * 3.0)
                .collect::<Vec<_>>(),
        );
        for (alias, canonical) in [("sma_20", "rolling_mean_20"), ("momentum_5", "returns_5")] {
            let a = value_at(feature(alias).unwrap().as_ref(), &r, 199);
            let c = value_at(feature(canonical).unwrap().as_ref(), &r, 199);
            assert_eq!(
                a.map(f64::to_bits),
                c.map(f64::to_bits),
                "{alias} vs {canonical}"
            );
        }
    }

    /// Every family declares the bar-derived lag, and it travels onto the
    /// definition. A family over a lagged source that forgot to set it would be
    /// claiming the source arrives instantly.
    #[test]
    fn the_declared_pit_lag_reaches_the_feature_definition() {
        for f in crate::registry::families() {
            let name = f.example();
            let built = feature(&name).unwrap();
            assert_eq!(built.def().knowledge_lag_ms, f.pit_lag_ms, "{name}");
        }
    }
}
