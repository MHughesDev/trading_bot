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

use dataplane::feature::{register_check, Feature, FeatureDef, FeatureError, FeatureRow, InfoClass, WindowedFrame, EVALUATE_FN};

use crate::{Ema, Rsi};

/// Bars of window for the on-balance-volume feature (cumulative within the window).
pub const OBV_WINDOW: u32 = 256;

/// Build a feature definition whose `code_hash` is the hash of the source that
/// implements it, so a served value records exactly which code produced it.
#[must_use]
pub fn definition(feature_id: &str, family: &str, version: u32, lookback_bars: u32, source: &str) -> FeatureDef {
    FeatureDef {
        feature_id: feature_id.to_string(),
        version,
        code_hash: dataplane::content_hash(&(family, version, source)).unwrap_or_default(),
        lookback_bars,
        knowledge_lag_ms: 0,
        output_dtype: "f64".into(),
        asset_classes: vec!["equity".into(), "crypto".into(), "futures".into(), "fx".into()],
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

fn closes(f: &WindowedFrame<'_>) -> Result<Vec<f64>, FeatureError> {
    (0..f.lookback() as usize).map(|b| f.close(b)).collect()
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

#[derive(Clone, Copy)]
enum Kind {
    RollingMean,
    RollingStd,
    /// `close / close[k] − 1`; `returns_k` and `momentum_k` are the same feature
    /// under two names and share this one implementation.
    SimpleReturn(usize),
    LogReturn1,
    Parkinson,
    GarmanKlass,
    Zscore,
    RelVolume,
    Obv,
    HourSin,
    HourCos,
    DowSin,
    DowCos,
}

struct Windowed(FeatureDef, Kind);

impl Feature for Windowed {
    fn def(&self) -> &FeatureDef {
        &self.0
    }

    #[allow(clippy::cast_precision_loss)]
    fn compute(&self, f: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let w = f.lookback() as usize;
        Ok(match self.1 {
            Kind::RollingMean => mean(&closes(f)?),
            Kind::RollingStd => sample_std(&closes(f)?),
            Kind::SimpleReturn(k) => {
                let base = f.close(k)?;
                if base == 0.0 {
                    f64::NAN
                } else {
                    f.close(0)? / base - 1.0
                }
            }
            Kind::LogReturn1 => {
                let (c, p) = (f.close(0)?, f.close(1)?);
                if c <= 0.0 || p <= 0.0 {
                    f64::NAN
                } else {
                    (c / p).ln()
                }
            }
            Kind::Parkinson => {
                let inv_4ln2 = 1.0 / (4.0 * 2.0_f64.ln());
                let mut sum = 0.0;
                for b in 0..w {
                    let r = f.back(b)?;
                    if r.low <= 0.0 || r.high <= 0.0 {
                        return Ok(f64::NAN);
                    }
                    sum += (r.high / r.low).ln().powi(2) * inv_4ln2;
                }
                (sum / w as f64).sqrt()
            }
            Kind::GarmanKlass => {
                let c1 = 2.0 * 2.0_f64.ln() - 1.0;
                let mut sum = 0.0;
                for b in 0..w {
                    let r = f.back(b)?;
                    if r.low <= 0.0 || r.high <= 0.0 || r.open <= 0.0 || r.close <= 0.0 {
                        return Ok(f64::NAN);
                    }
                    sum += (0.5 * (r.high / r.low).ln().powi(2) - c1 * (r.close / r.open).ln().powi(2)).max(0.0);
                }
                (sum / w as f64).sqrt()
            }
            Kind::Zscore => {
                let c = closes(f)?;
                let s = sample_std(&c);
                if s == 0.0 {
                    f64::NAN
                } else {
                    (c[0] - mean(&c)) / s
                }
            }
            Kind::RelVolume => {
                let v: Vec<f64> = (0..w).map(|b| f.back(b).map(|r| r.volume)).collect::<Result<_, _>>()?;
                let m = mean(&v);
                if m <= 0.0 {
                    f64::NAN
                } else {
                    v[0] / m
                }
            }
            Kind::Obv => {
                let mut obv = 0.0;
                for b in 0..w - 1 {
                    let (now, before) = (f.back(b)?, f.back(b + 1)?);
                    if now.close > before.close {
                        obv += now.volume;
                    } else if now.close < before.close {
                        obv -= now.volume;
                    }
                }
                obv
            }
            Kind::HourSin | Kind::HourCos => {
                let hour = (f.back(0)?.ts_ns.div_euclid(3_600_000_000_000)).rem_euclid(24) as f64;
                let a = hour * std::f64::consts::TAU / 24.0;
                if matches!(self.1, Kind::HourSin) {
                    a.sin()
                } else {
                    a.cos()
                }
            }
            Kind::DowSin | Kind::DowCos => {
                // Unix day 0 was a Thursday; 0 = Monday.
                let dow = (f.back(0)?.ts_ns.div_euclid(86_400_000_000_000) + 3).rem_euclid(7) as f64;
                let a = dow * std::f64::consts::TAU / 7.0;
                if matches!(self.1, Kind::DowSin) {
                    a.sin()
                } else {
                    a.cos()
                }
            }
        })
    }
}

fn suffix(name: &str, prefix: &str) -> Option<usize> {
    name.strip_prefix(prefix)?.parse().ok().filter(|n| *n > 0)
}

fn lb(n: usize) -> Option<u32> {
    u32::try_from(n).ok()
}

/// Construct the implementation for `name`, unregistered.
fn construct(name: &str) -> Option<Arc<dyn Feature>> {
    let passthrough = |field| Some(Arc::new(Passthrough(def(name, "passthrough", 1), field)) as Arc<dyn Feature>);
    let windowed = |family: &str, lookback: u32, kind| Some(Arc::new(Windowed(def(name, family, lookback), kind)) as Arc<dyn Feature>);
    match name {
        "open" => passthrough(Field::Open),
        "high" => passthrough(Field::High),
        "low" => passthrough(Field::Low),
        "close" => passthrough(Field::Close),
        "volume" => passthrough(Field::Volume),
        "log_returns_1" => windowed("log_return", 2, Kind::LogReturn1),
        "obv" => windowed("obv", OBV_WINDOW, Kind::Obv),
        "hour_sin" => windowed("calendar", 1, Kind::HourSin),
        "hour_cos" => windowed("calendar", 1, Kind::HourCos),
        "dow_sin" => windowed("calendar", 1, Kind::DowSin),
        "dow_cos" => windowed("calendar", 1, Kind::DowCos),
        _ => {
            if let Some(p) = suffix(name, "ema_") {
                return (p <= 100_000).then(|| Arc::new(Ema::new(p)) as Arc<dyn Feature>);
            }
            if let Some(p) = suffix(name, "rsi_") {
                return (2..=100_000).contains(&p).then(|| Arc::new(Rsi::new(p)) as Arc<dyn Feature>);
            }
            if let Some(w) = suffix(name, "rolling_mean_") {
                return windowed("rolling_mean", lb(w)?, Kind::RollingMean);
            }
            if let Some(w) = suffix(name, "rolling_std_").filter(|w| *w >= 2) {
                return windowed("rolling_std", lb(w)?, Kind::RollingStd);
            }
            if let Some(k) = suffix(name, "returns_").or_else(|| suffix(name, "momentum_")) {
                return windowed("simple_return", lb(k + 1)?, Kind::SimpleReturn(k));
            }
            if let Some(w) = suffix(name, "parkinson_vol_") {
                return windowed("parkinson_vol", lb(w)?, Kind::Parkinson);
            }
            if let Some(w) = suffix(name, "garman_klass_vol_") {
                return windowed("garman_klass_vol", lb(w)?, Kind::GarmanKlass);
            }
            if let Some(w) = suffix(name, "zscore_").filter(|w| *w >= 2) {
                return windowed("zscore", lb(w)?, Kind::Zscore);
            }
            if let Some(w) = suffix(name, "rel_volume_") {
                return windowed("rel_volume", lb(w)?, Kind::RelVolume);
            }
            None
        }
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
    if let Some(f) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(name) {
        return Ok(Arc::clone(f));
    }
    let f = construct(name).ok_or_else(|| FeatureError::Unknown(name.to_string()))?;
    register_check(f.as_ref())?;
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(name.to_string(), Arc::clone(&f));
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
    EVALUATE_FN(feature, rows, decision).ok().filter(|v| v.is_finite())
}

/// Batch path: one column per feature, `None` where absent.
#[must_use]
pub fn backfill_column(feature: &dyn Feature, rows: &[FeatureRow]) -> Vec<Option<f64>> {
    (0..rows.len()).map(|i| value_at(feature, rows, i)).collect()
}

/// The latest value of each named feature over `rows`.
///
/// # Errors
/// Unknown or unregistrable names.
pub fn latest(names: &[String], rows: &[FeatureRow]) -> Result<BTreeMap<String, Option<f64>>, FeatureError> {
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
            .map(|(i, &c)| FeatureRow { ts_ns: i as i64 * 3_600_000_000_000, open: c, high: c * 1.01, low: c * 0.99, close: c, volume: 1.0 + (i % 3) as f64 })
            .collect()
    }

    #[test]
    fn every_catalogued_feature_registers() {
        for name in crate::feature_sets::known_features() {
            feature(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn unknown_and_degenerate_names_do_not_resolve() {
        for bad in ["ema_0", "rsi_1", "rolling_std_1", "returns_", "nope", "ema_x"] {
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
        let closes: Vec<f64> = (0..60).map(|i| (f64::from(i) * 0.3).sin() * 5.0 + 100.0).collect();
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
        assert_eq!(value_at(feature("rsi_3").unwrap().as_ref(), &r, 29), Some(100.0));
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
        assert_eq!(feature("ema_7").unwrap().def().code_hash, feature("ema_9").unwrap().def().code_hash);
    }

    /// The feature-set id is a version hash, not a name: a set with a different
    /// member, or the same members in a different order, is a different set.
    #[test]
    fn feature_set_version_hash_covers_membership_and_order() {
        let a = feature_set_version_hash(&["ema_7".into(), "rsi_14".into()]).unwrap();
        let same = feature_set_version_hash(&["ema_7".into(), "rsi_14".into()]).unwrap();
        let reordered = feature_set_version_hash(&["rsi_14".into(), "ema_7".into()]).unwrap();
        let extra = feature_set_version_hash(&["ema_7".into(), "rsi_14".into(), "close".into()]).unwrap();
        assert_eq!(a, same);
        assert_ne!(a, reordered, "column order is part of the set");
        assert_ne!(a, extra);
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn feature_set_version_hash_refuses_an_unknown_member() {
        assert!(feature_set_version_hash(&["ema_7".into(), "not_a_feature".into()]).is_err());
    }
}
