//! Macro observations and fundamentals as point-in-time feature inputs
//! (Set P P-3.6; DATA-006; INV-13, INV-14).
//!
//! The point of collecting ALFRED vintages and EDGAR filings is to *use* them, and the
//! way they are usually used is wrong: a feature reads "the CPI value" and gets the
//! revised number, or reads "revenue for Q1" and gets a figure nobody outside the
//! company had until May. This module is how the one feature engine receives them
//! correctly.
//!
//! # Arguments, never fetched
//!
//! `features` is pure: no `sqlx`, no `clickhouse`, no `redis` (`builders-purity`
//! fails the build if one enters). So nothing here reads a store. The caller —
//! the backtest runner or the live server — loads the rows with the vintage-correct
//! readers (`storage::clickhouse::alt::macro_as_of`,
//! `collectors::fundamentals::store::facts_as_of`) and **hands them in** as
//! [`ExternalInputs`]. Both environments then resolve through the same function with
//! the same decision instant, which is what makes a feature compute identically in
//! backtest and live serve: the value is a function of `(rows, decision_time)` and
//! of nothing else, and rows that became knowable after `decision_time` cannot change
//! the answer.
//!
//! # The warm-up is honest only if it includes the publication lag
//!
//! A feature over a 12-month macro window that is declared with a 12-month lookback
//! is understating its history requirement: the newest value it can see was published
//! `source_lag` after the period it describes. A feature that omits that lag from its
//! declared `lookback_bars` / `knowledge_lag_ms` leaks into the embargo computation
//! (an understated lookback is leakage there, `leakage.rs`). [`check_declaration`]
//! refuses such a feature; [`required_lookback_bars`] says what it should have
//! declared.

use std::time::Duration;

use chrono::{DateTime, Utc};
use dataplane::feature::FeatureDef;
use dataplane::fundamentals::{latest_as_of, FundamentalFact};
use dataplane::vintage::{series_as_of, MacroObservation};
use rust_decimal::Decimal;

/// What an external feature input reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalSeries {
    /// An ALFRED series, read by vintage: the latest observation period, as that
    /// period's value stood at the decision instant.
    Macro { series_id: String },
    /// An EDGAR XBRL tag, read by acceptance: the most recently *accepted* fact,
    /// not the most recent period.
    Fundamental { cik: u32, tag: String },
}

/// One declared external input of a feature: which series and the source's declared
/// publication lag (`dataplane.source.declared_vendor_lag_ms`).
///
/// The lag is **required**. There is no `Default`: an input with no lag declared is an
/// input whose warm-up nobody has thought about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalInput {
    pub name: String,
    pub series: ExternalSeries,
    pub source_lag: Duration,
}

/// What an input resolved to at a decision instant.
///
/// `Missing` is not zero. A macro series that had published nothing by then, or whose
/// newest vintage reported "." (missing), has no value, and a feature that treats the
/// absence as `0` has invented a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    Value {
        value: Decimal,
        /// When this value became knowable. Always `<=` the decision instant.
        knowledge_time: DateTime<Utc>,
    },
    Missing,
}

/// The pre-loaded rows a feature's external inputs resolve against.
///
/// Borrowed, so the engine never owns or fetches them.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExternalInputs<'a> {
    pub macro_rows: &'a [MacroObservation],
    pub facts: &'a [FundamentalFact],
}

impl ExternalInputs<'_> {
    /// Resolve `input` as it stood at `decision_time`.
    ///
    /// Only rows with `knowledge_time <= decision_time` are considered, by the
    /// underlying `as_of` readers; there is no argument that widens that.
    #[must_use]
    pub fn resolve(&self, input: &ExternalInput, decision_time: DateTime<Utc>) -> Resolved {
        match &input.series {
            ExternalSeries::Macro { series_id } => {
                // The latest observation period known by then, with that period's
                // latest knowable vintage. If that vintage is "." it is missing: an
                // earlier number is a different period's or an older vintage's, not
                // this one's.
                match series_as_of(self.macro_rows, series_id, decision_time).last() {
                    Some(row) => match row.value {
                        Some(value) => Resolved::Value {
                            value,
                            knowledge_time: row.knowledge_time(),
                        },
                        None => Resolved::Missing,
                    },
                    None => Resolved::Missing,
                }
            }
            ExternalSeries::Fundamental { cik, tag } => {
                match latest_as_of(self.facts, *cik, tag, decision_time) {
                    Some(f) => Resolved::Value {
                        value: f.value,
                        knowledge_time: f.accepted_at(),
                    },
                    None => Resolved::Missing,
                }
            }
        }
    }
}

/// Why a feature's declaration is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalInputError {
    /// Declared `lookback_bars` is smaller than the window plus the publication lag.
    LookbackOmitsSourceLag {
        feature: String,
        declared: u32,
        required: u32,
        source_lag_secs: u64,
    },
    /// Declared `knowledge_lag_ms` is smaller than the largest source lag.
    KnowledgeLagOmitsSourceLag {
        feature: String,
        declared_ms: u64,
        required_ms: u64,
    },
    /// The bar size is zero, so a lag cannot be expressed in bars.
    ZeroBar,
    /// An input with no name cannot be reported on.
    UnnamedInput,
}

impl std::fmt::Display for ExternalInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LookbackOmitsSourceLag {
                feature,
                declared,
                required,
                source_lag_secs,
            } => write!(
                f,
                "feature {feature} declares lookback_bars={declared} but its external inputs \
                 publish {source_lag_secs}s after the period they describe, so the warm-up needs \
                 at least {required} bars: an understated lookback is leakage in the embargo"
            ),
            Self::KnowledgeLagOmitsSourceLag {
                feature,
                declared_ms,
                required_ms,
            } => write!(
                f,
                "feature {feature} declares knowledge_lag_ms={declared_ms} but its largest \
                 external source lags {required_ms} ms; the embargo would be computed too short"
            ),
            Self::ZeroBar => write!(f, "bar size must be positive"),
            Self::UnnamedInput => write!(f, "an external input must be named"),
        }
    }
}

impl std::error::Error for ExternalInputError {}

/// The lookback, in bars, a feature over these inputs must declare: its own window
/// plus the **largest** source lag rounded up to whole bars.
///
/// # Errors
/// A zero bar size, or an unnamed input.
pub fn required_lookback_bars(
    base_window_bars: u32,
    bar_secs: u32,
    inputs: &[ExternalInput],
) -> Result<u32, ExternalInputError> {
    if bar_secs == 0 {
        return Err(ExternalInputError::ZeroBar);
    }
    if inputs.iter().any(|i| i.name.trim().is_empty()) {
        return Err(ExternalInputError::UnnamedInput);
    }
    let max_lag = inputs
        .iter()
        .map(|i| i.source_lag)
        .max()
        .unwrap_or(Duration::ZERO);
    let lag_bars = max_lag.as_secs().div_ceil(u64::from(bar_secs));
    Ok(base_window_bars.saturating_add(u32::try_from(lag_bars).unwrap_or(u32::MAX)))
}

/// Refuse a feature whose declared lookback or knowledge lag omits its external
/// sources' publication lag.
///
/// # Errors
/// [`ExternalInputError::LookbackOmitsSourceLag`] or
/// [`ExternalInputError::KnowledgeLagOmitsSourceLag`].
pub fn check_declaration(
    def: &FeatureDef,
    base_window_bars: u32,
    bar_secs: u32,
    inputs: &[ExternalInput],
) -> Result<(), ExternalInputError> {
    let required = required_lookback_bars(base_window_bars, bar_secs, inputs)?;
    let max_lag = inputs
        .iter()
        .map(|i| i.source_lag)
        .max()
        .unwrap_or(Duration::ZERO);
    if def.lookback_bars < required {
        return Err(ExternalInputError::LookbackOmitsSourceLag {
            feature: def.feature_id.clone(),
            declared: def.lookback_bars,
            required,
            source_lag_secs: max_lag.as_secs(),
        });
    }
    let required_ms = u64::try_from(max_lag.as_millis()).unwrap_or(u64::MAX);
    if def.knowledge_lag_ms < required_ms {
        return Err(ExternalInputError::KnowledgeLagOmitsSourceLag {
            feature: def.feature_id.clone(),
            declared_ms: def.knowledge_lag_ms,
            required_ms,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};
    use dataplane::feature::InfoClass;
    use dataplane::source::Knowledge;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn t(y: i32, m: u32, day: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, day, h, 0, 0).unwrap()
    }

    fn cpi_rows() -> Vec<MacroObservation> {
        vec![
            // April's CPI, first published 10 May at 300, revised 12 June to 305.
            MacroObservation::derived("CPIAUCSL", d(2025, 4, 1), d(2025, 5, 10), Some(dec!(300))),
            MacroObservation::derived("CPIAUCSL", d(2025, 4, 1), d(2025, 6, 12), Some(dec!(305))),
            // May's CPI, first published 12 June, then a vintage reporting missing.
            MacroObservation::derived("CPIAUCSL", d(2025, 5, 1), d(2025, 6, 12), Some(dec!(310))),
            MacroObservation::derived("CPIAUCSL", d(2025, 5, 1), d(2025, 7, 15), None),
        ]
    }

    fn cpi() -> ExternalInput {
        ExternalInput {
            name: "cpi".into(),
            series: ExternalSeries::Macro {
                series_id: "CPIAUCSL".into(),
            },
            source_lag: Duration::from_secs(86_399),
        }
    }

    fn feature(lookback: u32, lag_ms: u64) -> FeatureDef {
        FeatureDef {
            feature_id: "cpi_yoy".into(),
            version: 1,
            code_hash: "h".into(),
            lookback_bars: lookback,
            knowledge_lag_ms: lag_ms,
            output_dtype: "f64".into(),
            asset_classes: vec!["crypto".into()],
            deflators: vec![],
            info_class: InfoClass::MarketPublic,
        }
    }

    #[test]
    fn a_macro_input_reads_the_vintage_current_at_the_decision_instant() {
        let rows = cpi_rows();
        let ins = ExternalInputs {
            macro_rows: &rows,
            facts: &[],
        };
        // Before May's CPI is out: April, original vintage.
        match ins.resolve(&cpi(), t(2025, 6, 1, 0)) {
            Resolved::Value { value, .. } => assert_eq!(value, dec!(300)),
            Resolved::Missing => panic!("April was published"),
        }
        // 13 June: May is out (310) and April has been revised.
        match ins.resolve(&cpi(), t(2025, 6, 13, 0)) {
            Resolved::Value { value, .. } => assert_eq!(value, dec!(310)),
            Resolved::Missing => panic!(),
        }
        // Before anything was published.
        assert_eq!(ins.resolve(&cpi(), t(2025, 5, 1, 0)), Resolved::Missing);
        // After the "." vintage for May: missing, not April's number and not zero.
        assert_eq!(ins.resolve(&cpi(), t(2025, 7, 20, 0)), Resolved::Missing);
    }

    /// The headline property: a feature computes identically in backtest and live
    /// serve. The backtest holds the whole table; the live server holds only what had
    /// arrived by the decision instant. Same function, same instant, same answer.
    #[test]
    fn backtest_and_live_resolve_identically_at_every_decision_instant() {
        let all = cpi_rows();
        for decision in [
            t(2025, 5, 11, 0),
            t(2025, 6, 11, 0),
            t(2025, 6, 13, 0),
            t(2025, 7, 20, 0),
        ] {
            let live: Vec<MacroObservation> = all
                .iter()
                .filter(|r| r.knowledge_time() <= decision)
                .cloned()
                .collect();
            let backtest = ExternalInputs {
                macro_rows: &all,
                facts: &[],
            }
            .resolve(&cpi(), decision);
            let served = ExternalInputs {
                macro_rows: &live,
                facts: &[],
            }
            .resolve(&cpi(), decision);
            assert_eq!(backtest, served, "at {decision}");
        }
    }

    #[test]
    fn a_fundamental_is_read_by_acceptance_not_by_period() {
        let fact = |accession: &str, accepted: DateTime<Utc>, value: Decimal| FundamentalFact {
            cik: 320_193,
            tag: "Revenues".into(),
            unit: "USD".into(),
            period_start: None,
            period_end: d(2025, 3, 31),
            fiscal_year: Some(2025),
            fiscal_period: Some("Q1".into()),
            form: "10-Q".into(),
            accession: accession.into(),
            value,
            knowledge: Knowledge::Observed { at: accepted },
        };
        let facts = vec![
            fact("a", t(2025, 5, 2, 20), dec!(100)),
            fact("b", t(2025, 8, 1, 21), dec!(110)),
        ];
        let ins = ExternalInputs {
            macro_rows: &[],
            facts: &facts,
        };
        let rev = ExternalInput {
            name: "revenue".into(),
            series: ExternalSeries::Fundamental {
                cik: 320_193,
                tag: "Revenues".into(),
            },
            source_lag: Duration::from_secs(86_399),
        };
        // The period ended 31 March; on 15 April nothing had been filed.
        assert_eq!(ins.resolve(&rev, t(2025, 4, 15, 0)), Resolved::Missing);
        assert!(
            matches!(ins.resolve(&rev, t(2025, 5, 3, 0)), Resolved::Value { value, .. } if value == dec!(100))
        );
        assert!(
            matches!(ins.resolve(&rev, t(2025, 8, 2, 0)), Resolved::Value { value, .. } if value == dec!(110))
        );
    }

    /// P-3.6 clause 3: the declared lookback includes the source's lag, and a feature
    /// that omits it is refused.
    #[test]
    fn a_feature_that_omits_the_source_lag_is_refused() {
        // Daily bars; a 12-bar window over a source that publishes ~1 day late.
        let inputs = [cpi()];
        let required = required_lookback_bars(12, 86_400, &inputs).unwrap();
        assert_eq!(
            required, 13,
            "12 bars of window plus one bar of publication lag"
        );

        assert!(check_declaration(&feature(13, 86_399_000), 12, 86_400, &inputs).is_ok());
        assert!(matches!(
            check_declaration(&feature(12, 86_399_000), 12, 86_400, &inputs),
            Err(ExternalInputError::LookbackOmitsSourceLag {
                declared: 12,
                required: 13,
                ..
            })
        ));
        assert!(matches!(
            check_declaration(&feature(13, 0), 12, 86_400, &inputs),
            Err(ExternalInputError::KnowledgeLagOmitsSourceLag { .. })
        ));
    }

    #[test]
    fn the_largest_source_lag_governs_and_degenerate_declarations_are_refused() {
        let slow = ExternalInput {
            name: "10k".into(),
            series: ExternalSeries::Fundamental {
                cik: 1,
                tag: "Assets".into(),
            },
            source_lag: Duration::from_secs(3 * 86_400),
        };
        assert_eq!(
            required_lookback_bars(5, 86_400, &[cpi(), slow]).unwrap(),
            8
        );
        assert_eq!(
            required_lookback_bars(5, 0, &[cpi()]),
            Err(ExternalInputError::ZeroBar)
        );
        let mut unnamed = cpi();
        unnamed.name = " ".into();
        assert_eq!(
            required_lookback_bars(5, 60, &[unnamed]),
            Err(ExternalInputError::UnnamedInput)
        );
        // No external input, no added lag.
        assert_eq!(required_lookback_bars(5, 60, &[]).unwrap(), 5);
    }
}
