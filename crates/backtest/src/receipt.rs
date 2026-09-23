//! The reproducibility receipt (AGENT-013 §6.9, §8.4; Set S task S-10.8).
//!
//! # What a result is worth without one
//!
//! A backtest is a number produced by a pipeline. "Sharpe 1.4 on BTC-USD 1h"
//! names the number and the instrument and none of the pipeline, so two people
//! comparing two such numbers are comparing two different computations and
//! calling the difference a finding.
//!
//! The receipt is every input that can change the number: the data, the features,
//! the calendar, the engine, the strategy, the costs, the seeds, and the harness.
//! A result carrying one can be re-run; a result without one can only be
//! believed.
//!
//! # Every field is required, and that is the whole technique
//!
//! There is no `serde(default)` here. A receipt missing the calendar version is
//! not a receipt with a gap — it is a receipt that cannot detect the one class of
//! drift this platform has actually seen, which is a calendar changing under a
//! stored result. ADR-P0-08's rule, for the seventh time: the alternative fails
//! silently and in the flattering direction.
//!
//! # A reproduction is not a look
//!
//! Re-running a registered result under its own receipt spends **no trial**
//! (ADR-P2-17). It is not a new look at the data: it is the same look, performed
//! again, to find out whether the platform still computes what it computed. A
//! platform that charged for that would be a platform where nobody checked.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Every input that can change a backtest's number.
///
/// Field order is the hash's order and is deliberately stable — see
/// [`ReproSpec::digest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproSpec {
    /// The data, by content. A snapshot id would name *which* data and not
    /// *what* data, and a snapshot that was rebuilt keeps its id.
    pub dataset_hash: String,
    /// Which feature definitions were in force. A feature whose formula changed
    /// produces a different number under the same name, which is the failure
    /// this field exists for.
    pub feature_registry_version: String,
    /// Which trading calendar. The drift this platform has actually seen.
    pub calendar_version: String,
    pub engine_version: String,
    /// The strategy, by content rather than by name. `ema_cross_v3` a week later
    /// may not be `ema_cross_v3`.
    pub strategy_version_hash: String,
    pub cost_model_version: String,
    /// A **digest** of the seeds the platform used — never the seeds.
    ///
    /// AT-63 caught the first version of this field, which held them as values,
    /// and it was right to: a receipt travels to the agent, and a type carrying
    /// the seeds hands them over. A digest does everything a receipt needs — two
    /// runs under different draws produce different digests, which is the drift
    /// detection — and reveals nothing that would let a caller reproduce a
    /// favourable draw on purpose.
    ///
    /// Computed by the ledger, which is the only reader of the seeds themselves.
    pub seed_digest: String,
    pub harness_version: String,
}

/// Why a receipt was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    #[error(
        "`{field}` is empty. A receipt missing it is not a receipt with a gap — it cannot \
         detect drift in the thing it names, and a result nobody can re-run can only be \
         believed."
    )]
    Missing { field: &'static str },
}

impl ReproSpec {
    /// Check every field is present.
    ///
    /// # Errors
    /// [`ReceiptError::Missing`] naming the field, or [`ReceiptError::NoSeeds`].
    pub fn validate(&self) -> Result<(), ReceiptError> {
        for (field, value) in self.fields() {
            if value.trim().is_empty() {
                return Err(ReceiptError::Missing { field });
            }
        }
        Ok(())
    }

    /// Every named field, in hash order.
    ///
    /// One list, used by `validate`, `digest` and `differences`. Three copies
    /// would be three chances for a field to be validated and not hashed, which
    /// is a receipt that passes and does not detect.
    fn fields(&self) -> [(&'static str, &str); 8] {
        [
            ("dataset_hash", &self.dataset_hash),
            ("feature_registry_version", &self.feature_registry_version),
            ("calendar_version", &self.calendar_version),
            ("engine_version", &self.engine_version),
            ("strategy_version_hash", &self.strategy_version_hash),
            ("cost_model_version", &self.cost_model_version),
            ("seed_digest", &self.seed_digest),
            ("harness_version", &self.harness_version),
        ]
    }

    /// The receipt, as one value.
    ///
    /// Field-separated rather than concatenated: without the separator,
    /// `("ab", "c")` and `("a", "bc")` hash the same, and two receipts that
    /// differ would report as identical.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        for (name, value) in self.fields() {
            h.update(name.as_bytes());
            h.update(b"=");
            h.update(value.as_bytes());
            h.update(b"\x1f");
        }
        format!("sha256:{}", hex::encode(h.finalize()))
    }

    /// Which fields differ, named.
    ///
    /// The point of the receipt: "the result changed" is not actionable and "the
    /// calendar version moved from 2024.3 to 2024.4" is.
    #[must_use]
    pub fn differences(&self, other: &ReproSpec) -> Vec<Difference> {
        let mut out = Vec::new();
        for ((name, mine), (_, theirs)) in self.fields().iter().zip(other.fields().iter()) {
            if mine != theirs {
                out.push(Difference {
                    field: (*name).to_string(),
                    was: (*mine).to_string(),
                    now: (*theirs).to_string(),
                });
            }
        }
        out
    }
}

/// One field that moved.
///
/// `field` is a `String` rather than a `&'static str` because a `Reproduction`
/// is serialised into the nightly drift report, and a borrowed name cannot
/// round-trip. The values still come from [`ReproSpec::fields`], which is the
/// one list, so a name here cannot be a typo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Difference {
    pub field: String,
    pub was: String,
    pub now: String,
}

/// What a reproduction found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Reproduction {
    /// Same receipt, same number, to the bit.
    BitIdentical,
    /// Same receipt, different number. **The serious one**: nothing named in the
    /// receipt moved, so the difference came from somewhere the receipt does not
    /// cover — which means the receipt is incomplete and every result carrying
    /// one is less checkable than it looked.
    DriftedUnderTheSameReceipt { was: f64, now: f64 },
    /// The receipt itself moved. Not a platform defect: something legitimately
    /// changed, and the fields say what.
    ReceiptChanged { differences: Vec<Difference> },
}

impl Reproduction {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Reproduction::BitIdentical => "bit_identical",
            Reproduction::DriftedUnderTheSameReceipt { .. } => "drifted",
            Reproduction::ReceiptChanged { .. } => "receipt_changed",
        }
    }

    /// Whether this should file a `platform_defect`.
    ///
    /// Only drift under an unchanged receipt. A changed receipt is somebody
    /// having changed something on purpose, and filing a defect for it would
    /// train everybody to ignore the defects.
    #[must_use]
    pub fn is_platform_defect(&self) -> bool {
        matches!(self, Reproduction::DriftedUnderTheSameReceipt { .. })
    }
}

/// Compare a stored result with a re-run.
///
/// `epsilon` is **not** a parameter, and that is deliberate: this asks whether
/// the platform computed the same thing, not whether it computed something close
/// enough. A tolerance here would hide exactly the small systematic drift that a
/// calendar or a cost-model change produces, which is the drift worth catching.
#[must_use]
pub fn reproduce(stored: &ReproSpec, rerun: &ReproSpec, was: f64, now: f64) -> Reproduction {
    let differences = stored.differences(rerun);
    if !differences.is_empty() {
        return Reproduction::ReceiptChanged { differences };
    }
    // Bit equality, not `==`. The question is whether the platform computed the
    // same thing, and two identical NaN bit patterns mean it did — a result that
    // was NaN and stayed NaN has not drifted, it has been broken all along, and
    // that is a different problem from the one this function detects. A number
    // that *became* NaN is drift, and the loudest kind.
    if was.to_bits() == now.to_bits() {
        Reproduction::BitIdentical
    } else {
        Reproduction::DriftedUnderTheSameReceipt { was, now }
    }
}

/// The share of stored results the nightly job re-runs (§8.4).
///
/// Two per cent of thirty days. Small enough to be free and large enough that a
/// systematic drift shows up within a week — which is the property that matters,
/// because a drift nobody notices for a quarter has been in every comparison made
/// during it.
pub const NIGHTLY_SAMPLE_SHARE: f64 = 0.02;
pub const NIGHTLY_WINDOW_DAYS: u32 = 30;

/// `repro_drift` over one nightly run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub sampled: u32,
    pub bit_identical: u32,
    pub drifted: u32,
    pub receipt_changed: u32,
}

impl DriftReport {
    pub fn record(&mut self, outcome: &Reproduction) {
        self.sampled += 1;
        match outcome {
            Reproduction::BitIdentical => self.bit_identical += 1,
            Reproduction::DriftedUnderTheSameReceipt { .. } => self.drifted += 1,
            Reproduction::ReceiptChanged { .. } => self.receipt_changed += 1,
        }
    }

    /// `repro_drift`: the share that changed under an unchanged receipt.
    ///
    /// `None` on an empty sample, rather than zero. "0% drift over nothing
    /// sampled" reads as a healthy platform and is the absence of a measurement.
    #[must_use]
    pub fn drift_rate(&self) -> Option<f64> {
        (self.sampled > 0).then(|| f64::from(self.drifted) / f64::from(self.sampled))
    }

    /// Whether this run should file a defect.
    #[must_use]
    pub fn files_a_defect(&self) -> bool {
        self.drifted > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> ReproSpec {
        ReproSpec {
            dataset_hash: "sha256:data".into(),
            feature_registry_version: "features_v7".into(),
            calendar_version: "2024.3".into(),
            engine_version: "engine_v2".into(),
            strategy_version_hash: "sha256:strategy".into(),
            cost_model_version: "costs_v3".into(),
            seed_digest: "sha256:seeds".into(),
            harness_version: "0.1.0".into(),
        }
    }

    #[test]
    fn a_complete_receipt_validates() {
        receipt().validate().expect("complete");
    }

    /// Every field is required, and the error says why that one matters.
    #[test]
    fn a_receipt_missing_any_field_is_refused_by_name() {
        let setters: Vec<(&str, fn(&mut ReproSpec))> = vec![
            ("dataset_hash", |r| r.dataset_hash = String::new()),
            ("feature_registry_version", |r| {
                r.feature_registry_version = "  ".into();
            }),
            ("calendar_version", |r| r.calendar_version = String::new()),
            ("engine_version", |r| r.engine_version = String::new()),
            ("strategy_version_hash", |r| {
                r.strategy_version_hash = String::new();
            }),
            ("cost_model_version", |r| {
                r.cost_model_version = String::new()
            }),
            ("seed_digest", |r| r.seed_digest = String::new()),
            ("harness_version", |r| r.harness_version = String::new()),
        ];
        for (field, break_it) in setters {
            let mut r = receipt();
            break_it(&mut r);
            assert_eq!(
                r.validate(),
                Err(ReceiptError::Missing { field }),
                "{field} was not required"
            );
        }
    }

    /// The digest separates its fields.
    ///
    /// Without the separator `("ab", "c")` and `("a", "bc")` hash the same, and
    /// two receipts that differ would report as identical — which is the one
    /// failure a receipt must not have.
    #[test]
    fn two_receipts_that_differ_do_not_share_a_digest() {
        let mut a = receipt();
        a.calendar_version = "20".into();
        a.engine_version = "24.3".into();
        let mut b = receipt();
        b.calendar_version = "2".into();
        b.engine_version = "024.3".into();
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn the_same_receipt_digests_the_same() {
        assert_eq!(receipt().digest(), receipt().digest());
    }

    /// A different draw changes the receipt — through the seed *digest*, never
    /// the seeds.
    ///
    /// AT-63 caught the first version of this field holding the values, and it
    /// was right to: a receipt travels to the agent, and a digest detects the
    /// drift without handing over what would let a caller reproduce a
    /// favourable draw.
    #[test]
    fn a_different_seed_draw_changes_the_receipt() {
        let mut other = receipt();
        other.seed_digest = "sha256:other".into();
        assert_ne!(receipt().digest(), other.digest());
        let diffs = receipt().differences(&other);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].field, "seed_digest");
    }

    /// Same receipt, same number.
    #[test]
    fn a_reproduction_under_an_unchanged_receipt_is_bit_identical() {
        let r = reproduce(&receipt(), &receipt(), 1.4, 1.4);
        assert_eq!(r, Reproduction::BitIdentical);
        assert!(!r.is_platform_defect());
    }

    /// Same receipt, different number: the serious one.
    #[test]
    fn drift_under_an_unchanged_receipt_files_a_defect() {
        let r = reproduce(&receipt(), &receipt(), 1.4, 1.400_000_1);
        assert!(
            r.is_platform_defect(),
            "nothing named in the receipt moved, so the difference came from somewhere \
             the receipt does not cover"
        );
        assert_eq!(r.as_str(), "drifted");
    }

    /// There is no tolerance, deliberately.
    ///
    /// A tolerance here would hide exactly the small systematic drift a calendar
    /// or cost-model change produces, which is the drift worth catching.
    #[test]
    fn a_difference_in_the_last_bit_is_still_a_difference() {
        let now = f64::from_bits(1.4_f64.to_bits() + 1);
        assert!(reproduce(&receipt(), &receipt(), 1.4, now).is_platform_defect());
    }

    /// A result that became NaN is the loudest possible drift.
    ///
    /// And one that was NaN and stayed NaN is **not** drift: the platform
    /// computed the same thing both times. It has been broken all along, which
    /// is a different problem and one this function is not the detector for —
    /// reporting it here would file a drift defect every night for a result
    /// nothing changed.
    #[test]
    fn a_number_that_became_nan_is_drift_and_one_that_always_was_is_not() {
        assert!(reproduce(&receipt(), &receipt(), 1.4, f64::NAN).is_platform_defect());
        assert_eq!(
            reproduce(&receipt(), &receipt(), f64::NAN, f64::NAN),
            Reproduction::BitIdentical
        );
    }

    /// A changed receipt is not a defect, and it says what moved.
    #[test]
    fn a_deliberate_calendar_bump_is_reported_as_a_receipt_change() {
        let mut after = receipt();
        after.calendar_version = "2024.4".into();
        let r = reproduce(&receipt(), &after, 1.4, 1.39);
        match &r {
            Reproduction::ReceiptChanged { differences } => {
                assert_eq!(differences.len(), 1);
                assert_eq!(differences[0].field, "calendar_version");
                assert_eq!(differences[0].was, "2024.3");
                assert_eq!(differences[0].now, "2024.4");
            }
            other => panic!("expected a receipt change, got {other:?}"),
        }
        assert!(
            !r.is_platform_defect(),
            "filing a defect for a deliberate change trains everybody to ignore the defects"
        );
    }

    /// "The result changed" is not actionable; the field name is.
    #[test]
    fn a_difference_names_the_field_that_moved() {
        let mut after = receipt();
        after.cost_model_version = "costs_v4".into();
        after.engine_version = "engine_v3".into();
        let diffs = receipt().differences(&after);
        let fields: Vec<&str> = diffs.iter().map(|d| d.field.as_str()).collect();
        assert!(fields.contains(&"cost_model_version"));
        assert!(fields.contains(&"engine_version"));
        assert_eq!(fields.len(), 2);
    }

    /// The drift rate over an empty sample is `None`.
    #[test]
    fn no_sample_is_not_a_healthy_platform() {
        let empty = DriftReport::default();
        assert_eq!(
            empty.drift_rate(),
            None,
            "0% over nothing sampled reads as healthy and is the absence of a measurement"
        );
        assert!(!empty.files_a_defect());
    }

    /// One drift in the sample files a defect.
    #[test]
    fn any_drift_at_all_files_a_defect() {
        let mut report = DriftReport::default();
        for _ in 0..49 {
            report.record(&Reproduction::BitIdentical);
        }
        assert!(!report.files_a_defect());
        report.record(&Reproduction::DriftedUnderTheSameReceipt { was: 1.4, now: 1.3 });
        assert!(report.files_a_defect());
        assert!((report.drift_rate().unwrap() - 0.02).abs() < 1e-9);
    }

    /// A receipt change does not count toward drift.
    #[test]
    fn the_drift_rate_counts_only_unexplained_changes() {
        let mut report = DriftReport::default();
        report.record(&Reproduction::BitIdentical);
        report.record(&Reproduction::ReceiptChanged {
            differences: vec![],
        });
        assert_eq!(report.sampled, 2);
        assert_eq!(report.receipt_changed, 1);
        assert!((report.drift_rate().unwrap()).abs() < f64::EPSILON);
        assert!(!report.files_a_defect());
    }
}
