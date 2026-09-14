//! Feature definitions and the single feature runtime (SPEC §3.2–§3.3, §12.5;
//! INV-13, INV-14, INV-24).

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Information class: the feature firewall's whitelist key (SPEC §7.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfoClass {
    PlatformPhysics,
    Methodology,
    MarketPublic,
    StrategyContent,
    PerformanceConditional,
    TenantOperational,
}

impl InfoClass {
    /// Only these three classes may cross a tenant boundary (INV-24).
    #[must_use]
    pub fn crosses_tenant(self) -> bool {
        matches!(self, Self::PlatformPhysics | Self::Methodology | Self::MarketPublic)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureDef {
    pub feature_id: String,
    pub version: u32,
    pub code_hash: String,
    /// DECLARED and load-bearing: enforced by the windowed view and fed to embargo.
    pub lookback_bars: u32,
    pub knowledge_lag_ms: u64,
    pub output_dtype: String,
    pub asset_classes: Vec<String>,
    pub deflators: Vec<String>,
    pub info_class: InfoClass,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeatureError {
    #[error("feature {feature} read {requested} bars back but declared lookback_bars={declared}")]
    LookbackExceeded { feature: String, declared: u32, requested: usize },
    #[error("causal access violation: read row {row} after decision row {decision}")]
    FutureRead { row: usize, decision: usize },
    #[error("insufficient history: need {need} bars, have {have}")]
    Insufficient { need: usize, have: usize },
    #[error("feature {0} is not registered")]
    Unknown(String),
    #[error("feature {feature} failed registration: {reason}")]
    Registration { feature: String, reason: String },
}

/// One input bar as features see it. `ts_ns` is the bar's timestamp in the caller's
/// declared convention; prices are already point-in-time resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeatureRow {
    pub ts_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A view that physically cannot read further back than the declared lookback, nor
/// forward of the decision bar.
pub struct WindowedFrame<'a> {
    feature: &'a str,
    data: &'a [FeatureRow],
    decision: usize,
    lookback: u32,
}

impl<'a> WindowedFrame<'a> {
    #[must_use]
    pub fn new(feature: &'a str, data: &'a [FeatureRow], decision: usize, lookback: u32) -> Self {
        Self { feature, data, decision, lookback }
    }

    /// Close `back` bars before the decision bar.
    ///
    /// # Errors
    /// See [`Self::back`].
    pub fn close(&self, back: usize) -> Result<f64, FeatureError> {
        self.back(back).map(|r| r.close)
    }

    /// `back = 0` is the decision bar; `back = lookback_bars - 1` is the oldest allowed.
    ///
    /// # Errors
    /// Reading at or beyond `lookback_bars` is a declared-lookback violation.
    pub fn back(&self, back: usize) -> Result<&'a FeatureRow, FeatureError> {
        if back >= self.lookback as usize {
            return Err(FeatureError::LookbackExceeded {
                feature: self.feature.to_string(),
                declared: self.lookback,
                requested: back + 1,
            });
        }
        let idx = self.decision.checked_sub(back).ok_or(FeatureError::Insufficient {
            need: back + 1,
            have: self.decision + 1,
        })?;
        Ok(&self.data[idx])
    }

    #[must_use]
    pub fn lookback(&self) -> u32 {
        self.lookback
    }
}

/// A feature is a pure function of a windowed frame (SPEC §3.3).
pub trait Feature: Send + Sync {
    fn def(&self) -> &FeatureDef;
    ///
    /// # Errors
    /// Any read outside the window.
    fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError>;
}

/// The ONE evaluation routine. Both [`FeatureRuntime::backfill`] and
/// [`FeatureRuntime::serve_live`] call this function; there is no second path
/// (INV-14, AT-15).
///
/// # Errors
/// Propagates window violations.
///
/// A feature is only defined once its whole declared window exists: a value computed
/// over a shorter window would differ between a backfill that starts at the dataset
/// edge and a live serve with full history.
pub fn evaluate(feature: &dyn Feature, rows: &[FeatureRow], decision: usize) -> Result<f64, FeatureError> {
    let def = feature.def();
    let need = def.lookback_bars as usize;
    if decision >= rows.len() {
        return Err(FeatureError::Insufficient { need: decision + 1, have: rows.len() });
    }
    if decision + 1 < need {
        return Err(FeatureError::Insufficient { need, have: decision + 1 });
    }
    let frame = WindowedFrame::new(&def.feature_id, rows, decision, def.lookback_bars);
    feature.compute(&frame)
}

/// Pointer to [`evaluate`], exposed so a test can assert both runtime paths resolve
/// to the same function object.
pub const EVALUATE_FN: fn(&dyn Feature, &[FeatureRow], usize) -> Result<f64, FeatureError> = evaluate;

/// Registration check (AT-13): a feature that reads beyond its declaration, or whose
/// output depends on data outside its window, fails loudly here.
///
/// # Errors
/// Returns [`FeatureError::Registration`] describing the violation.
pub fn register_check(feature: &dyn Feature) -> Result<(), FeatureError> {
    let def = feature.def();
    let lb = def.lookback_bars as usize;
    if lb == 0 {
        return Err(FeatureError::Registration { feature: def.feature_id.clone(), reason: "lookback_bars must be ≥ 1".into() });
    }
    let n = lb * 4 + 16;
    let series_a: Vec<FeatureRow> = (0..n)
        .map(|i| {
            let c = 100.0 + ((i * 7919) % 97) as f64 * 0.37;
            FeatureRow {
                ts_ns: i as i64 * 60_000_000_000,
                open: c * 0.999,
                high: c * 1.004,
                low: c * 0.995,
                close: c,
                volume: 10.0 + ((i * 31) % 17) as f64,
            }
        })
        .collect();
    // Same window, different deep history: a feature honoring its window cannot tell.
    let mut series_b = series_a.clone();
    for r in series_b.iter_mut().take(n - lb) {
        r.open = r.open * 3.0 + 1234.5;
        r.high = r.high * 3.0 + 1240.0;
        r.low = r.low * 3.0 + 1230.0;
        r.close = r.close * 3.0 + 1234.5;
        r.volume = r.volume * 7.0 + 1.0;
    }
    // Evaluated twice on identical input, a pure feature cannot differ.
    if let (Ok(x), Ok(y)) = (evaluate(feature, &series_a, n - 1), evaluate(feature, &series_a, n - 1)) {
        if x.to_bits() != y.to_bits() && !(x.is_nan() && y.is_nan()) {
            return Err(FeatureError::Registration {
                feature: def.feature_id.clone(),
                reason: "output is not a pure function of its window".into(),
            });
        }
    }
    for decision in [n - 1, n - 2, lb + 3] {
        let a = evaluate(feature, &series_a, decision).map_err(|e| FeatureError::Registration {
            feature: def.feature_id.clone(),
            reason: e.to_string(),
        })?;
        if decision == n - 1 {
            let b = evaluate(feature, &series_b, decision).map_err(|e| FeatureError::Registration {
                feature: def.feature_id.clone(),
                reason: e.to_string(),
            })?;
            if (a - b).abs() > 1e-12 && !(a.is_nan() && b.is_nan()) {
                return Err(FeatureError::Registration {
                    feature: def.feature_id.clone(),
                    reason: "output depends on data outside the declared window".into(),
                });
            }
        }
    }
    Ok(())
}

/// Causal access guard (SPEC §12.5 test 1, AT-31): wraps a full frame and refuses
/// any read past the decision row. The entire pipeline runs under it.
pub struct CausalGuard<'a> {
    rows: &'a [f64],
    decision: usize,
}

impl<'a> CausalGuard<'a> {
    #[must_use]
    pub fn new(rows: &'a [f64], decision: usize) -> Self {
        Self { rows, decision }
    }

    ///
    /// # Errors
    /// A read past the decision row.
    pub fn at(&self, row: usize) -> Result<f64, FeatureError> {
        if row > self.decision {
            return Err(FeatureError::FutureRead { row, decision: self.decision });
        }
        self.rows.get(row).copied().ok_or(FeatureError::Insufficient { need: row + 1, have: self.rows.len() })
    }

    #[must_use]
    pub fn decision(&self) -> usize {
        self.decision
    }
}

/// One served feature vector (SPEC §3.3 `feature_serving_log`), written on EVERY
/// live serve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServeRecord {
    pub serve_id: Uuid,
    pub tenant: String,
    pub instrument_id: i64,
    pub event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub feature_set_id: String,
    /// `feature_id -> code_hash` of the implementation that produced each value.
    pub code_hashes: BTreeMap<String, String>,
    pub values: BTreeMap<String, f64>,
    pub served_at: DateTime<Utc>,
}

pub trait ServeLog: Send + Sync {
    fn write(&self, rec: ServeRecord);
}

#[derive(Default)]
pub struct MemoryServeLog(pub std::sync::Mutex<Vec<ServeRecord>>);

impl ServeLog for MemoryServeLog {
    fn write(&self, rec: ServeRecord) {
        self.0.lock().expect("serve log poisoned").push(rec);
    }
}

pub struct FeatureRuntime {
    features: BTreeMap<String, Arc<dyn Feature>>,
    feature_set_id: String,
    log: Arc<dyn ServeLog>,
}

impl FeatureRuntime {
    /// Every feature passes [`register_check`] or the runtime is not built.
    ///
    /// # Errors
    /// The first registration failure.
    pub fn new(feature_set_id: impl Into<String>, features: Vec<Arc<dyn Feature>>, log: Arc<dyn ServeLog>) -> Result<Self, FeatureError> {
        let mut map = BTreeMap::new();
        for f in features {
            register_check(f.as_ref())?;
            map.insert(f.def().feature_id.clone(), f);
        }
        Ok(Self { features: map, feature_set_id: feature_set_id.into(), log })
    }

    #[must_use]
    pub fn defs(&self) -> Vec<&FeatureDef> {
        self.features.values().map(|f| f.def()).collect()
    }

    #[must_use]
    pub fn feature_set_id(&self) -> &str {
        &self.feature_set_id
    }

    #[must_use]
    pub fn feature(&self, id: &str) -> Option<&Arc<dyn Feature>> {
        self.features.get(id)
    }

    #[must_use]
    pub fn code_hashes(&self) -> BTreeMap<String, String> {
        self.features.iter().map(|(id, f)| (id.clone(), f.def().code_hash.clone())).collect()
    }

    #[must_use]
    pub fn max_lookback(&self) -> u32 {
        self.features.values().map(|f| f.def().lookback_bars).max().unwrap_or(0)
    }

    #[must_use]
    pub fn max_knowledge_lag_ms(&self) -> u64 {
        self.features.values().map(|f| f.def().knowledge_lag_ms).max().unwrap_or(0)
    }

    /// Batch path. Rows lacking their full window are NaN.
    #[must_use]
    pub fn backfill(&self, series: &[FeatureRow]) -> BTreeMap<String, Vec<f64>> {
        self.features
            .iter()
            .map(|(id, f)| {
                let col = (0..series.len())
                    .map(|i| EVALUATE_FN(f.as_ref(), series, i).unwrap_or(f64::NAN))
                    .collect();
                (id.clone(), col)
            })
            .collect()
    }

    /// Live path: same evaluation function, incremental window; logged every time.
    ///
    /// # Errors
    /// Any feature error; nothing is logged for a failed serve.
    pub fn serve_live(
        &self,
        tenant: &str,
        instrument_id: i64,
        window: &[FeatureRow],
        event_time: DateTime<Utc>,
        knowledge_time: DateTime<Utc>,
    ) -> Result<BTreeMap<String, f64>, FeatureError> {
        let decision = window.len().checked_sub(1).ok_or(FeatureError::Insufficient { need: 1, have: 0 })?;
        let mut values = BTreeMap::new();
        for (id, f) in &self.features {
            values.insert(id.clone(), EVALUATE_FN(f.as_ref(), window, decision)?);
        }
        self.log.write(ServeRecord {
            serve_id: Uuid::new_v4(),
            tenant: tenant.into(),
            instrument_id,
            event_time,
            knowledge_time,
            feature_set_id: self.feature_set_id.clone(),
            code_hashes: self.code_hashes(),
            values: values.clone(),
            served_at: Utc::now(),
        });
        Ok(values)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Diagnosis {
    Match,
    LateArrival,
    CodeDrift,
    Nondeterminism,
    Precision,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsistencyDiff {
    pub serve_id: Uuid,
    pub feature_id: String,
    pub served_value: f64,
    pub recomputed_value: f64,
    pub abs_diff: f64,
    pub served_knowledge_time: DateTime<Utc>,
    pub recomputed_knowledge_time: DateTime<Utc>,
    pub diagnosis: Diagnosis,
}

/// Classify one served-vs-recomputed pair. Carrying both knowledge times is what
/// turns "the numbers differ" into "the backfill saw data that arrived late".
#[must_use]
pub fn diagnose(
    served: f64,
    recomputed: f64,
    served_kt: DateTime<Utc>,
    recomputed_kt: DateTime<Utc>,
    served_code_hash: &str,
    current_code_hash: &str,
    rerun_agrees: bool,
) -> Diagnosis {
    let diff = (served - recomputed).abs();
    let rel = diff / served.abs().max(recomputed.abs()).max(1e-300);
    if diff == 0.0 || (served.is_nan() && recomputed.is_nan()) {
        return Diagnosis::Match;
    }
    if served_code_hash != current_code_hash {
        return Diagnosis::CodeDrift;
    }
    if recomputed_kt > served_kt {
        return Diagnosis::LateArrival;
    }
    if !rerun_agrees {
        return Diagnosis::Nondeterminism;
    }
    if rel < 1e-9 {
        return Diagnosis::Precision;
    }
    Diagnosis::CodeDrift
}

/// p99 absolute relative diff over deterministic features (SLO < 1e-9).
#[must_use]
pub fn p99_relative_diff(diffs: &[ConsistencyDiff]) -> f64 {
    let mut rel: Vec<f64> = diffs
        .iter()
        .map(|d| d.abs_diff / d.served_value.abs().max(d.recomputed_value.abs()).max(1e-300))
        .filter(|r| r.is_finite())
        .collect();
    if rel.is_empty() {
        return 0.0;
    }
    rel.sort_by(f64::total_cmp);
    let idx = ((rel.len() as f64) * 0.99).ceil() as usize;
    rel[idx.saturating_sub(1).min(rel.len() - 1)]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Duration;

    pub struct Sma {
        pub def: FeatureDef,
        /// How many bars the code actually reads (may lie about its declaration).
        pub reads: usize,
    }

    pub fn def(id: &str, lookback: u32) -> FeatureDef {
        FeatureDef {
            feature_id: id.into(),
            version: 1,
            code_hash: "sha256:x".into(),
            lookback_bars: lookback,
            knowledge_lag_ms: 0,
            output_dtype: "f64".into(),
            asset_classes: vec!["crypto".into()],
            deflators: vec![],
            info_class: InfoClass::MarketPublic,
        }
    }

    impl Feature for Sma {
        fn def(&self) -> &FeatureDef {
            &self.def
        }
        fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
            let mut s = 0.0;
            for b in 0..self.reads {
                s += frame.close(b)?;
            }
            Ok(s / self.reads as f64)
        }
    }

    /// AT-13: declares 20, reads 50 → registration fails.
    #[test]
    fn a_lying_feature_fails_registration() {
        let liar = Sma { def: def("sma_liar", 20), reads: 50 };
        assert!(matches!(register_check(&liar), Err(FeatureError::Registration { .. })));
        let honest = Sma { def: def("sma_20", 20), reads: 20 };
        assert_eq!(register_check(&honest), Ok(()));
    }

    /// A feature that smuggles history through interior state — the window cannot
    /// stop that — is caught by the window-independence probe.
    #[test]
    fn stateful_history_smuggling_fails_registration() {
        struct Smuggler(FeatureDef, std::sync::Mutex<f64>);
        impl Feature for Smuggler {
            fn def(&self) -> &FeatureDef {
                &self.0
            }
            fn compute(&self, f: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
                let mut acc = self.1.lock().unwrap();
                *acc += f.close(0)?;
                Ok(*acc)
            }
        }
        assert!(matches!(
            register_check(&Smuggler(def("cumsum_liar", 5), std::sync::Mutex::new(0.0))),
            Err(FeatureError::Registration { .. })
        ));
    }

    /// AT-31: a deliberate future read under the causal guard raises.
    #[test]
    fn causal_guard_raises_on_future_read() {
        let rows: Vec<f64> = (0..10).map(f64::from).collect();
        let g = CausalGuard::new(&rows, 4);
        assert_eq!(g.at(4), Ok(4.0));
        assert_eq!(g.at(5), Err(FeatureError::FutureRead { row: 5, decision: 4 }));
    }

    /// AT-15: backfill and live resolve to the same function object and agree.
    #[test]
    fn backfill_and_live_share_one_implementation() {
        assert!(std::ptr::fn_addr_eq(EVALUATE_FN, evaluate as fn(&dyn Feature, &[FeatureRow], usize) -> Result<f64, FeatureError>));
        let log = Arc::new(MemoryServeLog::default());
        let rt = FeatureRuntime::new("fs1", vec![Arc::new(Sma { def: def("sma_5", 5), reads: 5 })], log.clone()).unwrap();
        let series: Vec<FeatureRow> =
            (0..50).map(|i| FeatureRow { close: f64::from(i).sin() * 10.0 + 100.0, ..FeatureRow::default() }).collect();
        let batch = rt.backfill(&series);
        let now = Utc::now();
        for i in 10..50 {
            let live = rt.serve_live("t", 1, &series[..=i], now, now).unwrap();
            assert_eq!(live["sma_5"].to_bits(), batch["sma_5"][i].to_bits(), "bit-identical at {i}");
        }
        assert_eq!(log.0.lock().unwrap().len(), 40, "every live serve is logged");
    }

    #[test]
    fn diagnosis_uses_both_knowledge_times() {
        let t = Utc::now();
        assert_eq!(diagnose(1.0, 1.0, t, t, "a", "a", true), Diagnosis::Match);
        assert_eq!(diagnose(1.0, 2.0, t, t + Duration::minutes(40), "a", "a", true), Diagnosis::LateArrival);
        assert_eq!(diagnose(1.0, 2.0, t, t, "a", "b", true), Diagnosis::CodeDrift);
        assert_eq!(diagnose(1.0, 2.0, t, t, "a", "a", false), Diagnosis::Nondeterminism);
        assert_eq!(diagnose(1.0, 1.0 + 1e-12, t, t, "a", "a", true), Diagnosis::Precision);
    }

    #[test]
    fn firewall_classes() {
        assert!(InfoClass::MarketPublic.crosses_tenant());
        assert!(!InfoClass::StrategyContent.crosses_tenant());
        assert!(!InfoClass::PerformanceConditional.crosses_tenant());
    }
}
