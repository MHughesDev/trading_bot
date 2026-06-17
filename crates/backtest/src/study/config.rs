//! [`StudyConfig`] — a deliberate set of Runs answering **one** question, along
//! **one** varying dimension (spec §1.2).
//!
//! The `question` is mandatory and logged before the Study runs — the single
//! best defense against post-hoc reinterpretation. `null_ref` is required iff
//! the kind is `PermutationNull`. The `VarySpec` is checked against the kind so
//! an ill-typed pairing cannot run.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::run::{MetricKind, ParamMap, RunConfig};

/// The ten Study kinds (spec §1.2 catalog).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyKind {
    ParameterSweep,
    WalkForward,
    Cpcv,
    NestedCv,
    PermutationNull,
    SyntheticPaths,
    CostSweep,
    TradeMonteCarlo,
    RegimeConditional,
    Neighborhood,
}

/// What dimension a Study perturbs. Each variant pairs with one or more kinds
/// (validated in [`StudyConfig::validate`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "vary", rename_all = "snake_case")]
pub enum VarySpec {
    /// `parameter_sweep`: an explicit list of param overrides applied to the base.
    Params { grid: Vec<ParamMap> },
    /// `neighborhood`: perturb one numeric param ±k steps around its center.
    Neighborhood {
        param: String,
        center: f64,
        step: f64,
        k: u32,
    },
    /// `walk_forward`: rolling IS/OOS windows over the data slice.
    DataWindows {
        windows: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    },
    /// `cpcv` / `nested_cv`: combinatorial purged train/test group assignment.
    CpcvGroups { n_groups: u32, k_test: u32 },
    /// `permutation_null` / `synthetic_paths`: vary the generator seed.
    Seeds { n: u32 },
    /// `cost_sweep`: an optimistic→pessimistic ladder of cost-model refs.
    CostLadder { cost_model_refs: Vec<String> },
    /// `trade_monte_carlo`: number of block-bootstrap resamples of the trade list.
    TradeResamples { n: u32, block: u32 },
    /// `regime_conditional`: labeled sub-windows.
    Regimes {
        windows: Vec<(DateTime<Utc>, DateTime<Utc>, String)>,
    },
}

/// How (if at all) a single config is carried forward from a Study. The rule is
/// declared **before** the Study runs and is never an argmax (INV-2 / ADR-002).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionRule {
    /// Carry nothing forward.
    None,
    /// The member whose metric is closest to the distribution median — the
    /// centroid of the stable region, not the peak.
    MedianStableCentroid,
    /// The member whose metric is closest to the worst-5% (most conservative).
    WorstCaseRobust,
}

/// Run/wall budget for a Study.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyBudget {
    pub max_runs: u32,
    pub max_wall_ms: u64,
}

impl Default for StudyBudget {
    fn default() -> Self {
        Self {
            max_runs: 100_000,
            max_wall_ms: u64::MAX,
        }
    }
}

/// A study definition (spec §1.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyConfig {
    pub study_id: String,
    pub kind: StudyKind,
    /// The "center" config being studied.
    pub base_config: RunConfig,
    pub vary: VarySpec,
    /// Which metric the distribution is taken over.
    pub metric: MetricKind,
    /// REQUIRED iff `kind == PermutationNull`.
    #[serde(default)]
    pub null_ref: Option<String>,
    #[serde(default)]
    pub budget: StudyBudget,
    /// Human-readable; logged before running. Must be non-empty.
    pub question: String,
    #[serde(default = "selection_none")]
    pub selection_rule: SelectionRule,
}

fn selection_none() -> SelectionRule {
    SelectionRule::None
}

/// A reason a `StudyConfig` is invalid.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StudyConfigError {
    #[error("study question must be non-empty (it is logged before running)")]
    EmptyQuestion,
    #[error("permutation_null study requires a null_ref (INV-3)")]
    MissingNull,
    #[error("null_ref is only meaningful for a permutation_null study")]
    UnexpectedNull,
    #[error("vary spec {vary} does not match study kind {kind:?}")]
    KindVaryMismatch { kind: StudyKind, vary: &'static str },
    #[error("cpcv requires 0 < k_test < n_groups")]
    BadCpcvGroups,
}

impl StudyConfig {
    /// Validate the kind/vary pairing, the question, and the null requirement.
    ///
    /// # Errors
    /// Returns a [`StudyConfigError`] describing the first violation.
    pub fn validate(&self) -> Result<(), StudyConfigError> {
        if self.question.trim().is_empty() {
            return Err(StudyConfigError::EmptyQuestion);
        }
        match self.kind {
            StudyKind::PermutationNull => {
                if self.null_ref.is_none() {
                    return Err(StudyConfigError::MissingNull);
                }
            }
            _ => {
                if self.null_ref.is_some() {
                    return Err(StudyConfigError::UnexpectedNull);
                }
            }
        }
        let ok = matches!(
            (self.kind, &self.vary),
            (StudyKind::ParameterSweep, VarySpec::Params { .. })
                | (StudyKind::Neighborhood, VarySpec::Neighborhood { .. })
                | (StudyKind::WalkForward, VarySpec::DataWindows { .. })
                | (
                    StudyKind::Cpcv | StudyKind::NestedCv,
                    VarySpec::CpcvGroups { .. }
                )
                | (
                    StudyKind::PermutationNull | StudyKind::SyntheticPaths,
                    VarySpec::Seeds { .. }
                )
                | (StudyKind::CostSweep, VarySpec::CostLadder { .. })
                | (StudyKind::TradeMonteCarlo, VarySpec::TradeResamples { .. })
                | (StudyKind::RegimeConditional, VarySpec::Regimes { .. })
        );
        if !ok {
            return Err(StudyConfigError::KindVaryMismatch {
                kind: self.kind,
                vary: self.vary.tag(),
            });
        }
        if let VarySpec::CpcvGroups { n_groups, k_test } = &self.vary {
            if *k_test == 0 || *k_test >= *n_groups {
                return Err(StudyConfigError::BadCpcvGroups);
            }
        }
        Ok(())
    }
}

impl VarySpec {
    fn tag(&self) -> &'static str {
        match self {
            VarySpec::Params { .. } => "params",
            VarySpec::Neighborhood { .. } => "neighborhood",
            VarySpec::DataWindows { .. } => "data_windows",
            VarySpec::CpcvGroups { .. } => "cpcv_groups",
            VarySpec::Seeds { .. } => "seeds",
            VarySpec::CostLadder { .. } => "cost_ladder",
            VarySpec::TradeResamples { .. } => "trade_resamples",
            VarySpec::Regimes { .. } => "regimes",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{DataSlice, EvalResolution, RunConfigBuilder};
    use chrono::TimeZone;

    fn base() -> RunConfig {
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        RunConfigBuilder::new("s", "v", s, "c", "z", "snap").build()
    }

    fn cfg(
        kind: StudyKind,
        vary: VarySpec,
        question: &str,
        null_ref: Option<String>,
    ) -> StudyConfig {
        StudyConfig {
            study_id: "study-1".into(),
            kind,
            base_config: base(),
            vary,
            metric: MetricKind::Sharpe,
            null_ref,
            budget: StudyBudget::default(),
            question: question.into(),
            selection_rule: SelectionRule::None,
        }
    }

    #[test]
    fn valid_sweep_validates() {
        let c = cfg(
            StudyKind::ParameterSweep,
            VarySpec::Params { grid: vec![] },
            "how does perf vary across params?",
            None,
        );
        assert!(c.validate().is_ok());
    }

    #[test]
    fn empty_question_rejected() {
        let c = cfg(
            StudyKind::ParameterSweep,
            VarySpec::Params { grid: vec![] },
            "  ",
            None,
        );
        assert_eq!(c.validate(), Err(StudyConfigError::EmptyQuestion));
    }

    #[test]
    fn permutation_requires_null() {
        let c = cfg(
            StudyKind::PermutationNull,
            VarySpec::Seeds { n: 100 },
            "is it real?",
            None,
        );
        assert_eq!(c.validate(), Err(StudyConfigError::MissingNull));
        let ok = cfg(
            StudyKind::PermutationNull,
            VarySpec::Seeds { n: 100 },
            "is it real?",
            Some("null:block".into()),
        );
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn non_permutation_rejects_null() {
        let c = cfg(
            StudyKind::ParameterSweep,
            VarySpec::Params { grid: vec![] },
            "q",
            Some("null:x".into()),
        );
        assert_eq!(c.validate(), Err(StudyConfigError::UnexpectedNull));
    }

    #[test]
    fn kind_vary_mismatch_rejected() {
        let c = cfg(
            StudyKind::ParameterSweep,
            VarySpec::Seeds { n: 10 },
            "q",
            None,
        );
        assert!(matches!(
            c.validate(),
            Err(StudyConfigError::KindVaryMismatch { .. })
        ));
    }

    #[test]
    fn cpcv_group_bounds_checked() {
        let c = cfg(
            StudyKind::Cpcv,
            VarySpec::CpcvGroups {
                n_groups: 5,
                k_test: 5,
            },
            "q",
            None,
        );
        assert_eq!(c.validate(), Err(StudyConfigError::BadCpcvGroups));
    }

    #[test]
    fn round_trips_serde() {
        let c = cfg(
            StudyKind::Neighborhood,
            VarySpec::Neighborhood {
                param: "fast".into(),
                center: 12.0,
                step: 1.0,
                k: 3,
            },
            "is it a plateau?",
            None,
        );
        let back: StudyConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
