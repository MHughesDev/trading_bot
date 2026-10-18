//! Features that declare an external input (Set P P-3.6; DATA-006; INV-13, INV-14).
//!
//! [`crate::external_inputs`] says how a macro vintage or an EDGAR fact resolves
//! at a decision instant. This is the other half: a *registered feature* that
//! names such an input, a registration check that refuses one whose warm-up omits
//! the source's publication lag, and one evaluation routine that backtest and live
//! serve both call. Until this existed `ExternalInputs` was a finished module with
//! no feature able to hold one, which is this repository's dominant defect.
//!
//! # Names
//!
//! An external feature is resolved from its name, like the windowed families:
//!
//! * `macro_<series>` (e.g. `macro_dgs10`, `macro_t10y2y`) is the latest ALFRED
//!   observation of that series, as it stood at the decision instant. The series
//!   id is the upper-cased remainder.
//! * `fundamental_<cik>_<tag>` (e.g. `fundamental_320193_Revenues`) is the most
//!   recently **accepted** EDGAR fact of that tag. The tag keeps its XBRL case.
//!
//! They are deliberately *not* in [`crate::registry::families`]: that registry
//! builds windowed features that compute from bars alone, and an external feature
//! asked to compute without its inputs would answer with a guess. The windowed
//! runtime refuses these names with a message that points here
//! ([`is_external_name`]).
//!
//! # Bar size
//!
//! A publication lag is a duration and a lookback is a count of bars, so the
//! declared lookback depends on the bar size and an external feature is resolved
//! *for* one: [`resolve_external`] takes `bar_secs`. A daily macro series on 1h bars
//! declares a lookback of 25, on 1d bars of 2.
//!
//! # Purity
//!
//! The rows are arguments ([`ExternalInputs`]); nothing here reads a store, and
//! `cargo tree -p features` stays free of `sqlx`, `clickhouse` and `redis`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dataplane::feature::{FeatureDef, FeatureError, FeatureRow, InfoClass, WindowedFrame};
use dataplane::source::SourceDecl;
use rust_decimal::prelude::ToPrimitive;

use crate::external_inputs::{
    check_declaration, required_lookback_bars, ExternalInput, ExternalInputs, ExternalSeries,
    Resolved,
};

/// Bumped when the resolution semantics change, so a feature set that includes an
/// external feature gets a different `feature_set_id`.
const VERSION: u32 = 1;
const SOURCE: &str = include_str!("external_features.rs");

/// Why an external feature has no value at a decision.
#[derive(Debug, PartialEq)]
pub enum ExternalFeatureError {
    Feature(FeatureError),
    /// The input had nothing knowable at the decision instant (or its newest
    /// vintage reports missing). **Not zero**: the feature is absent for that bar.
    InputMissing {
        input: String,
        at: DateTime<Utc>,
    },
    NotRepresentable {
        input: String,
        value: String,
    },
}

impl std::fmt::Display for ExternalFeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Feature(e) => write!(f, "{e}"),
            Self::InputMissing { input, at } => {
                write!(f, "external input {input} has no value knowable at {at}")
            }
            Self::NotRepresentable { input, value } => write!(
                f,
                "external input {input} resolved to {value}, which is not a finite f64"
            ),
        }
    }
}

impl std::error::Error for ExternalFeatureError {}

impl From<FeatureError> for ExternalFeatureError {
    fn from(e: FeatureError) -> Self {
        Self::Feature(e)
    }
}

/// A feature with declared external inputs.
pub trait ExternalFeature: Send + Sync {
    fn def(&self) -> &FeatureDef;
    /// The inputs it reads. Every one carries its source's publication lag.
    fn inputs(&self) -> &[ExternalInput];
    /// Compute at `decision_time` from the handed-in rows.
    ///
    /// # Errors
    /// An input with no knowable value, or one that is not a finite number.
    fn compute(
        &self,
        frame: &WindowedFrame<'_>,
        external: &ExternalInputs<'_>,
        decision_time: DateTime<Utc>,
    ) -> Result<f64, ExternalFeatureError>;
}

/// The level of one external series.
struct Level {
    def: FeatureDef,
    inputs: Vec<ExternalInput>,
}

impl ExternalFeature for Level {
    fn def(&self) -> &FeatureDef {
        &self.def
    }
    fn inputs(&self) -> &[ExternalInput] {
        &self.inputs
    }
    fn compute(
        &self,
        _frame: &WindowedFrame<'_>,
        external: &ExternalInputs<'_>,
        decision_time: DateTime<Utc>,
    ) -> Result<f64, ExternalFeatureError> {
        let input = &self.inputs[0];
        match external.resolve(input, decision_time) {
            Resolved::Missing => Err(ExternalFeatureError::InputMissing {
                input: input.name.clone(),
                at: decision_time,
            }),
            Resolved::Value { value, .. } => {
                value.to_f64().filter(|v| v.is_finite()).ok_or_else(|| {
                    ExternalFeatureError::NotRepresentable {
                        input: input.name.clone(),
                        value: value.to_string(),
                    }
                })
            }
        }
    }
}

const MACRO_PREFIX: &str = "macro_";
const FUNDAMENTAL_PREFIX: &str = "fundamental_";

/// Whether `name` is one this module resolves, by shape alone (no bar size needed).
#[must_use]
pub fn is_external_name(name: &str) -> bool {
    parse(name).is_some()
}

fn parse(name: &str) -> Option<ExternalSeries> {
    if let Some(id) = name.strip_prefix(MACRO_PREFIX) {
        let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric());
        return ok.then(|| ExternalSeries::Macro {
            series_id: id.to_ascii_uppercase(),
        });
    }
    if let Some(rest) = name.strip_prefix(FUNDAMENTAL_PREFIX) {
        let (cik, tag) = rest.split_once('_')?;
        let cik: u32 = cik.parse().ok()?;
        let ok = !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric());
        return ok.then(|| ExternalSeries::Fundamental {
            cik,
            tag: tag.to_owned(),
        });
    }
    None
}

/// Resolve an external feature by name, for bars of `bar_secs` seconds.
///
/// `Ok(None)` is "not an external-feature name", so a caller can fall through to
/// the windowed registry. The declaration is checked before the feature is handed
/// out: its lookback and knowledge lag include the source's lag
/// ([`check_declaration`]).
///
/// # Errors
/// A zero bar size, or a declaration the check refuses.
pub fn resolve_external(
    name: &str,
    bar_secs: u32,
) -> Result<Option<Arc<dyn ExternalFeature>>, FeatureError> {
    let Some(series) = parse(name) else {
        return Ok(None);
    };
    let lag = match &series {
        ExternalSeries::Macro { .. } => SourceDecl::alfred_observations().declared_lag,
        ExternalSeries::Fundamental { .. } => SourceDecl::sec_edgar().declared_lag,
    };
    let inputs = vec![ExternalInput {
        name: name.to_owned(),
        series,
        source_lag: lag,
    }];
    let refuse = |reason: String| FeatureError::Registration {
        feature: name.to_owned(),
        reason,
    };
    // One bar of window of its own, plus the source's lag in bars.
    let lookback =
        required_lookback_bars(1, bar_secs, &inputs).map_err(|e| refuse(e.to_string()))?;
    let def = FeatureDef {
        feature_id: name.to_owned(),
        version: VERSION,
        code_hash: dataplane::content_hash(&("external_level", VERSION, SOURCE))
            .unwrap_or_default(),
        lookback_bars: lookback,
        knowledge_lag_ms: u64::try_from(lag.as_millis()).unwrap_or(u64::MAX),
        output_dtype: "f64".into(),
        asset_classes: vec![
            "equity".into(),
            "crypto".into(),
            "futures".into(),
            "fx".into(),
        ],
        deflators: vec![],
        info_class: InfoClass::MarketPublic,
    };
    check_declaration(&def, 1, bar_secs, &inputs).map_err(|e| refuse(e.to_string()))?;
    Ok(Some(Arc::new(Level { def, inputs })))
}

/// The ONE evaluation routine for an external feature: backtest and live serve
/// both call it with the same `(rows, decision)` and the same [`ExternalInputs`]
/// semantics, so the value is a function of those and of nothing else.
///
/// The decision instant is `rows[decision].ts_ns`: the bar's timestamp in the
/// caller's declared convention (the same field the windowed features read), which
/// for a bar-close convention is the moment the bar's information is complete.
///
/// # Errors
/// An incomplete warm-up (which includes the publication lag), a decision beyond the
/// rows, or an input with no knowable value.
pub fn evaluate_external(
    feature: &dyn ExternalFeature,
    rows: &[FeatureRow],
    decision: usize,
    external: &ExternalInputs<'_>,
) -> Result<f64, ExternalFeatureError> {
    let def = feature.def();
    let need = def.lookback_bars as usize;
    if decision >= rows.len() {
        return Err(FeatureError::Insufficient {
            need: decision + 1,
            have: rows.len(),
        }
        .into());
    }
    if decision + 1 < need {
        return Err(FeatureError::Insufficient {
            need,
            have: decision + 1,
        }
        .into());
    }
    let frame = WindowedFrame::new(&def.feature_id, rows, decision, def.lookback_bars);
    let at = DateTime::from_timestamp_nanos(rows[decision].ts_ns);
    feature.compute(&frame, external, at)
}

/// Value at `decision`, `None` when absent for any reason.
#[must_use]
pub fn value_at_external(
    feature: &dyn ExternalFeature,
    rows: &[FeatureRow],
    decision: usize,
    external: &ExternalInputs<'_>,
) -> Option<f64> {
    evaluate_external(feature, rows, decision, external).ok()
}

/// Batch path: one value per bar, `None` where absent.
#[must_use]
pub fn backfill_column_external(
    feature: &dyn ExternalFeature,
    rows: &[FeatureRow],
    external: &ExternalInputs<'_>,
) -> Vec<Option<f64>> {
    (0..rows.len())
        .map(|i| value_at_external(feature, rows, i, external))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};
    use dataplane::vintage::MacroObservation;
    use rust_decimal_macros::dec;

    const HOUR_NS: i64 = 3_600 * 1_000_000_000;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn bars(start: DateTime<Utc>, n: usize) -> Vec<FeatureRow> {
        (0..n)
            .map(|i| FeatureRow {
                ts_ns: start.timestamp_nanos_opt().unwrap() + i as i64 * HOUR_NS,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect()
    }

    /// DGS10: 4.0 published 10 June (end of that day it is knowable), revised to 4.2
    /// on 12 June.
    fn dgs10() -> Vec<MacroObservation> {
        vec![
            MacroObservation::derived("DGS10", day(2025, 6, 9), day(2025, 6, 10), Some(dec!(4.0))),
            MacroObservation::derived("DGS10", day(2025, 6, 9), day(2025, 6, 12), Some(dec!(4.2))),
        ]
    }

    fn feat(name: &str, bar_secs: u32) -> Arc<dyn ExternalFeature> {
        resolve_external(name, bar_secs)
            .expect("declaration")
            .expect("an external name")
    }

    #[test]
    fn names_resolve_by_shape_and_anything_else_falls_through() {
        assert!(is_external_name("macro_dgs10"));
        assert!(is_external_name("fundamental_320193_Revenues"));
        assert!(!is_external_name("rsi_14"));
        assert!(!is_external_name("macro_"));
        assert!(!is_external_name("fundamental_notacik_Revenues"));
        assert!(!is_external_name("fundamental_320193_"));
        assert!(resolve_external("rsi_14", 3600).unwrap().is_none());
        // The series id is the upper-cased remainder.
        let f = feat("macro_dgs10", 3600);
        assert_eq!(
            f.inputs()[0].series,
            ExternalSeries::Macro {
                series_id: "DGS10".into()
            }
        );
    }

    /// A registered feature declares its input, and the declared lookback includes the
    /// source's lag in whole bars: one day less a second on 1h bars is 24 bars, plus
    /// the feature's own bar.
    #[test]
    fn the_declared_lookback_includes_the_publication_lag_at_each_bar_size() {
        assert_eq!(feat("macro_dgs10", 3600).def().lookback_bars, 25);
        assert_eq!(feat("macro_dgs10", 86_400).def().lookback_bars, 2);
        assert_eq!(feat("macro_dgs10", 3600).def().knowledge_lag_ms, 86_399_000);
        assert!(
            resolve_external("macro_dgs10", 0).is_err(),
            "a zero bar cannot carry a lag"
        );
    }

    /// The registration check the module exists for: a feature that omits the lag is
    /// refused (the check itself is `external_inputs::check_declaration`).
    #[test]
    fn a_feature_that_omits_the_source_lag_is_refused() {
        let f = feat("macro_dgs10", 3600);
        let mut short = f.def().clone();
        short.lookback_bars = 1;
        assert!(check_declaration(&short, 1, 3600, f.inputs()).is_err());
        let mut no_lag = f.def().clone();
        no_lag.knowledge_lag_ms = 0;
        assert!(check_declaration(&no_lag, 1, 3600, f.inputs()).is_err());
        assert!(check_declaration(f.def(), 1, 3600, f.inputs()).is_ok());
    }

    #[test]
    fn the_value_is_the_vintage_current_at_each_bars_decision_instant() {
        let rows_macro = dgs10();
        let ext = ExternalInputs {
            macro_rows: &rows_macro,
            facts: &[],
        };
        let f = feat("macro_dgs10", 3600);
        // Bars from 11 June 00:00 for 3 days: before 12 June's revision is knowable
        // (end of the 12th) the original vintage, after it the revised one.
        let start = Utc.with_ymd_and_hms(2025, 6, 10, 0, 0, 0).unwrap();
        let rows = bars(start, 24 * 4);
        let col = backfill_column_external(f.as_ref(), &rows, &ext);
        // Warm-up: the first 24 bars are absent, not guessed.
        assert!(col[..24].iter().all(Option::is_none), "{:?}", &col[..24]);
        // 11 June 00:00 (bar 24): the 10th has ended, 4.0 is knowable.
        assert_eq!(col[24], Some(4.0));
        // 12 June 12:00 (bar 60): the 12th is not over, so still 4.0.
        assert_eq!(col[60], Some(4.0));
        // 13 June 00:00 (bar 72): 12 June ended, the revision is knowable.
        assert_eq!(col[72], Some(4.2));
    }

    /// INV-14: a feature over a macro series computes identically in backtest and in
    /// live serve. The backtest holds the whole table; the live server only the rows
    /// that had arrived by the decision instant.
    #[test]
    fn backtest_and_live_agree_at_every_bar() {
        let all = dgs10();
        let f = feat("macro_dgs10", 3600);
        let start = Utc.with_ymd_and_hms(2025, 6, 10, 0, 0, 0).unwrap();
        let rows = bars(start, 24 * 5);
        let backtest = backfill_column_external(
            f.as_ref(),
            &rows,
            &ExternalInputs {
                macro_rows: &all,
                facts: &[],
            },
        );
        for (i, expected) in backtest.iter().enumerate() {
            let at = DateTime::from_timestamp_nanos(rows[i].ts_ns);
            let live_rows: Vec<MacroObservation> = all
                .iter()
                .filter(|r| r.knowledge_time() <= at)
                .cloned()
                .collect();
            let live = value_at_external(
                f.as_ref(),
                &rows,
                i,
                &ExternalInputs {
                    macro_rows: &live_rows,
                    facts: &[],
                },
            );
            assert_eq!(live, *expected, "bar {i}");
        }
    }

    /// The windowed runtime must not compute an external feature from bars alone.
    #[test]
    fn the_windowed_runtime_refuses_an_external_name_and_says_where_it_lives() {
        let err = crate::runtime::feature("macro_dgs10")
            .err()
            .expect("refused");
        let msg = err.to_string();
        assert!(msg.contains("external input"), "{msg}");
        assert!(msg.contains("resolve_external"), "{msg}");
        assert!(
            !matches!(err, FeatureError::Unknown(_)),
            "it is known, just not windowed"
        );
    }

    /// Missing is absent, never zero.
    #[test]
    fn a_series_with_nothing_published_is_absent_not_zero() {
        let f = feat("macro_dgs10", 3600);
        let rows = bars(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), 48);
        let err = evaluate_external(f.as_ref(), &rows, 40, &ExternalInputs::default()).unwrap_err();
        assert!(
            matches!(err, ExternalFeatureError::InputMissing { .. }),
            "{err:?}"
        );
    }

    /// The warm-up refusal is the same shape as a windowed feature's.
    #[test]
    fn an_incomplete_warm_up_is_refused() {
        let rows_macro = dgs10();
        let f = feat("macro_dgs10", 3600);
        let rows = bars(Utc.with_ymd_and_hms(2025, 6, 12, 0, 0, 0).unwrap(), 48);
        let err = evaluate_external(
            f.as_ref(),
            &rows,
            10,
            &ExternalInputs {
                macro_rows: &rows_macro,
                facts: &[],
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ExternalFeatureError::Feature(FeatureError::Insufficient { .. })
        ));
    }
}
