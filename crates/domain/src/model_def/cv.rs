//! Walk-forward cross-validation specification (Set I, Phase 0).
//!
//! A `WalkForwardSpec` describes how a pinned dataset is sliced into sequential
//! **train / calibration / test** folds with **purge** and **embargo**, following
//! the López de Prado discipline. It is an *optional, additive* block on
//! [`ModelDefinition`](super::ModelDefinition): a v1.0 definition without a `cv`
//! block keeps today's behaviour (a single expanding fold).
//!
//! This module is a pure data + validation type. The actual fold geometry is
//! computed by the PURE generator in `features::walk_forward` (no I/O), which
//! turns a `WalkForwardSpec` + a dataset index into concrete index ranges.
//!
//! See ADR-0017 (walk-forward CV & leakage discipline).

use serde::{Deserialize, Serialize};

/// How the train window grows across folds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// Train window starts at the beginning and grows each fold; `train_bars`
    /// is the **minimum** length of the first fold's train window.
    #[default]
    Expanding,
    /// Train window is a fixed `train_bars` length that slides forward each fold.
    Rolling,
}

/// Declarative walk-forward CV plan. Index counts are in **bars** of the
/// model's base timeframe; the generator maps them onto a concrete dataset
/// index at materialization time.
///
/// Roles (one fold, left to right):
///
/// ```text
///  … train …│purge│ cal …│purge│ test …│ embargo │ (next fold) …
/// ```
///
/// - **train** fits the estimator.
/// - **cal** is reserved for conformal / calibration fitting (Phase 4) and is
///   never seen by HPO scoring.
/// - **test** is strictly out-of-sample.
/// - **purge** drops rows whose forward label window (horizon `H`) overlaps the
///   following role's start.
/// - **embargo** inserts a gap after `test` before the next fold's `train`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkForwardSpec {
    #[serde(default)]
    pub mode: WindowMode,
    /// Number of sequential folds (≥ 1).
    pub folds: u32,
    /// Rolling: fixed train length. Expanding: minimum first-fold train length.
    pub train_bars: u64,
    /// Calibration-role length (NEW vs Set H's train/val/test).
    pub cal_bars: u64,
    /// Out-of-sample test length per fold.
    pub test_bars: u64,
    /// Rows dropped at every role boundary whose label window overlaps it.
    #[serde(default)]
    pub purge_bars: u64,
    /// Gap after `test` before the next fold's `train` (default: label horizon).
    #[serde(default)]
    pub embargo_bars: u64,
}

impl WalkForwardSpec {
    /// Data- and horizon-independent checks: positive fold count and bar counts.
    ///
    /// Callable from the model-definition validator, which does not know the
    /// base timeframe and therefore cannot convert the label horizon to bars.
    pub fn validate_shape(&self) -> Result<(), Vec<CvValidationError>> {
        let mut errors = Vec::new();

        if self.folds < 1 {
            errors.push(CvValidationError::new("cv.folds", "must be ≥ 1"));
        }
        if self.train_bars == 0 {
            errors.push(CvValidationError::new("cv.train_bars", "must be > 0"));
        }
        if self.cal_bars == 0 {
            errors.push(CvValidationError::new("cv.cal_bars", "must be > 0"));
        }
        if self.test_bars == 0 {
            errors.push(CvValidationError::new("cv.test_bars", "must be > 0"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Full validation against the model's label `horizon_bars` (the number of
    /// base-timeframe bars the forward label looks ahead). Runs `validate_shape`
    /// plus the embargo-≥-horizon leakage guard.
    ///
    /// Bounds that depend on available history length (total span ≤ history) are
    /// checked at materialization time, not here.
    pub fn validate(&self, horizon_bars: u64) -> Result<(), Vec<CvValidationError>> {
        let mut errors = match self.validate_shape() {
            Ok(()) => Vec::new(),
            Err(e) => e,
        };

        // Embargo must cover at least the label horizon: a test row's label must
        // not bleed into the next fold's train window (ADR-0017).
        if self.embargo_bars < horizon_bars {
            errors.push(CvValidationError::new(
                "cv.embargo_bars",
                format!("must be ≥ label horizon ({horizon_bars} bars) to prevent leakage"),
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A single CV validation failure, mirroring `model_def::validate::ValidationError`.
#[derive(Debug, PartialEq, Eq)]
pub struct CvValidationError {
    pub path: String,
    pub message: String,
}

impl CvValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> WalkForwardSpec {
        WalkForwardSpec {
            mode: WindowMode::Expanding,
            folds: 5,
            train_bars: 1_000,
            cal_bars: 200,
            test_bars: 200,
            purge_bars: 12,
            embargo_bars: 12,
        }
    }

    #[test]
    fn round_trips_through_json() {
        let s = spec();
        let json = serde_json::to_string(&s).unwrap();
        let back: WalkForwardSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn mode_defaults_to_expanding_when_absent() {
        let json =
            r#"{"folds":3,"train_bars":500,"cal_bars":100,"test_bars":100,"embargo_bars":1}"#;
        let s: WalkForwardSpec = serde_json::from_str(json).unwrap();
        assert_eq!(s.mode, WindowMode::Expanding);
        assert_eq!(s.purge_bars, 0);
    }

    #[test]
    fn valid_spec_passes() {
        assert!(spec().validate(12).is_ok());
    }

    #[test]
    fn rejects_zero_folds() {
        let mut s = spec();
        s.folds = 0;
        let errs = s.validate(12).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "cv.folds"));
    }

    #[test]
    fn rejects_zero_bar_counts() {
        for mutate in [
            |s: &mut WalkForwardSpec| s.train_bars = 0,
            |s: &mut WalkForwardSpec| s.cal_bars = 0,
            |s: &mut WalkForwardSpec| s.test_bars = 0,
        ] {
            let mut s = spec();
            mutate(&mut s);
            assert!(s.validate(12).is_err());
        }
    }

    #[test]
    fn rejects_embargo_below_horizon() {
        let mut s = spec();
        s.embargo_bars = 4;
        let errs = s.validate(12).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "cv.embargo_bars"));
    }

    #[test]
    fn embargo_equal_to_horizon_passes() {
        let mut s = spec();
        s.embargo_bars = 12;
        assert!(s.validate(12).is_ok());
    }
}
