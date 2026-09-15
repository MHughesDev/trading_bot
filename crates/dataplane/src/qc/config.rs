//! Thresholds for the check battery, and the version stamp that makes a grade
//! reproducible (DATA-005 §7, P-2.1).
//!
//! Every threshold lives here rather than in the check that uses it, for one
//! reason: **a grade has to be reproducible, and a grade is only reproducible if
//! you can say what it was computed against.** A constant inlined in a check is
//! a threshold nobody can version.
//!
//! So every report carries [`QcConfig::version`] — a content hash of the whole
//! configuration. Two reports with the same version were graded by the same
//! rules; two with different versions were not, whatever their letters say.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The full threshold set.
///
/// `Deserialize` from `config/data_qc.toml`, with the spec's defaults compiled
/// in so a missing file degrades to the documented behaviour rather than to
/// nothing. A quality checker that silently does no checking is worse than one
/// that refuses to start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QcConfig {
    pub coverage: CoverageThresholds,
    pub bad_ticks: BadTickThresholds,
    pub stale: StaleThresholds,
    pub volume: VolumeThresholds,
    pub cross_source: CrossSourceThresholds,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CoverageThresholds {
    /// Fraction of expected bars that may be missing before grade B is lost.
    pub b_max_missing: f64,
    /// Fraction at or above which the series is grade D.
    pub d_min_missing: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BadTickThresholds {
    /// How many rolling standard deviations a log return must exceed.
    pub sigma_multiple: f64,
    /// How much of that move must reverse on the next bar, as a fraction.
    pub reversal_fraction: f64,
    /// Window for the rolling standard deviation, in bars.
    pub sigma_window: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StaleThresholds {
    /// Consecutive identical closes with zero volume before a run is flagged.
    /// Per the spec this is per timeframe; the map is keyed by timeframe label
    /// with `default` as the fallback.
    pub runs: std::collections::BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VolumeThresholds {
    /// Share of zero-volume bars above which the check fires.
    pub max_zero_share: f64,
    /// Multiple of the rolling median that counts as a spike.
    pub spike_multiple: f64,
    pub median_window: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CrossSourceThresholds {
    /// Median absolute deviation, in basis points, above which two venues are
    /// deemed to disagree.
    pub max_mad_bps: f64,
}

impl Default for CoverageThresholds {
    fn default() -> Self {
        // DATA-005 §7: B keeps gaps under 0.5%, D begins at 5%.
        Self { b_max_missing: 0.005, d_min_missing: 0.05 }
    }
}

impl Default for BadTickThresholds {
    fn default() -> Self {
        // `abs(log return) > 8·σ_rolling` reversing ≥ 80% on the next bar.
        Self { sigma_multiple: 8.0, reversal_fraction: 0.8, sigma_window: 50 }
    }
}

impl Default for StaleThresholds {
    fn default() -> Self {
        // Shorter timeframes tolerate fewer identical closes: four flat hours is
        // a broken feed, four flat minutes is a quiet market.
        let runs = [
            ("1m", 30usize),
            ("5m", 12),
            ("15m", 8),
            ("1h", 4),
            ("1d", 3),
            ("default", 10),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Self { runs }
    }
}

impl Default for VolumeThresholds {
    fn default() -> Self {
        Self { max_zero_share: 0.10, spike_multiple: 20.0, median_window: 50 }
    }
}

impl Default for CrossSourceThresholds {
    fn default() -> Self {
        Self { max_mad_bps: 50.0 }
    }
}

impl QcConfig {
    /// Load from TOML.
    ///
    /// # Errors
    /// A parse failure, or an unknown key — `deny_unknown_fields` is deliberate.
    /// A misspelled threshold that is silently ignored is a threshold that
    /// quietly reverts to its default, and the report would still claim the
    /// config version it was asked for.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The content hash of this configuration: 16 hex characters.
    ///
    /// Recorded on every report. Two reports with the same version were graded
    /// by the same rules; two with different versions were not, whatever their
    /// letters say.
    ///
    /// Computed from the canonical serialization rather than the file bytes, so
    /// reformatting the TOML does not invalidate every historical grade while
    /// changing a threshold always does.
    #[must_use]
    pub fn version(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        hex::encode(&Sha256::digest(canonical.as_bytes())[..8])
    }

    /// The stale-run threshold for a timeframe, falling back to `default`.
    #[must_use]
    pub fn stale_run_for(&self, timeframe: &str) -> usize {
        self.stale
            .runs
            .get(timeframe)
            .or_else(|| self.stale.runs.get("default"))
            .copied()
            .unwrap_or(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_specs_numbers() {
        let c = QcConfig::default();
        assert!((c.coverage.b_max_missing - 0.005).abs() < f64::EPSILON, "B keeps gaps under 0.5%");
        assert!((c.coverage.d_min_missing - 0.05).abs() < f64::EPSILON, "D begins at 5%");
        assert!((c.bad_ticks.sigma_multiple - 8.0).abs() < f64::EPSILON);
        assert!((c.bad_ticks.reversal_fraction - 0.8).abs() < f64::EPSILON);
        assert!((c.volume.spike_multiple - 20.0).abs() < f64::EPSILON);
    }

    /// The property the whole version stamp exists for.
    #[test]
    fn changing_any_threshold_changes_the_version() {
        let base = QcConfig::default();
        let v = base.version();

        let mut changed = QcConfig::default();
        changed.coverage.b_max_missing = 0.006;
        assert_ne!(changed.version(), v);

        let mut changed = QcConfig::default();
        changed.bad_ticks.sigma_multiple = 7.9;
        assert_ne!(changed.version(), v);

        let mut changed = QcConfig::default();
        changed.stale.runs.insert("1m".into(), 31);
        assert_ne!(changed.version(), v);
    }

    #[test]
    fn the_same_configuration_always_hashes_the_same_way() {
        assert_eq!(QcConfig::default().version(), QcConfig::default().version());
        assert_eq!(QcConfig::default().version().len(), 16);
    }

    /// Reformatting the file must not invalidate every historical grade.
    #[test]
    fn whitespace_in_the_toml_does_not_change_the_version() {
        let a = QcConfig::from_toml("[coverage]\nb_max_missing = 0.005\n").unwrap();
        let b = QcConfig::from_toml("\n\n[coverage]\n\nb_max_missing   =   0.005\n\n").unwrap();
        assert_eq!(a.version(), b.version());
    }

    /// A misspelled threshold that is silently ignored reverts to its default
    /// while the report still claims the version that was asked for.
    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let err = QcConfig::from_toml("[coverage]\nb_max_missng = 0.005\n").unwrap_err();
        assert!(err.contains("b_max_missng") || err.contains("unknown"), "{err}");
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_everything_it_omits() {
        let c = QcConfig::from_toml("[volume]\nspike_multiple = 15.0\n").unwrap();
        assert!((c.volume.spike_multiple - 15.0).abs() < f64::EPSILON);
        assert!(
            (c.coverage.d_min_missing - 0.05).abs() < f64::EPSILON,
            "an omitted section keeps the spec default"
        );
    }

    /// Four flat hours is a broken feed; four flat minutes is a quiet market.
    #[test]
    fn the_stale_threshold_is_looser_on_shorter_timeframes() {
        let c = QcConfig::default();
        assert!(c.stale_run_for("1m") > c.stale_run_for("1h"));
        assert!(c.stale_run_for("1h") > c.stale_run_for("1d"));
    }

    #[test]
    fn an_unknown_timeframe_falls_back_to_the_default_rather_than_to_zero() {
        let c = QcConfig::default();
        assert_eq!(c.stale_run_for("3m"), c.stale_run_for("default"));
        assert!(c.stale_run_for("3m") > 0, "zero would flag every bar");
    }
}
