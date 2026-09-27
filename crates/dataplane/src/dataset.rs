//! A dataset is a hash, not a path (SPEC §3.1, INV-12).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::calendar::CalendarVersion;
use crate::corporate::AdjustmentPolicy;
use crate::defi::FinalityPolicy;
use crate::futures::ContinuousMethod;
use crate::hash::content_hash;
use crate::identity::InstrumentKey;
use crate::quality::QualityFlags;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DatasetSpec {
    pub universe_spec_id: String,
    /// Resolved at spec time and stored explicitly, so later membership revisions
    /// cannot silently change the dataset.
    pub instrument_ids: Vec<InstrumentKey>,
    pub date_range: (DateTime<Utc>, DateTime<Utc>),
    pub frequency: String,
    pub feature_set_id: String,
    pub label_spec_id: String,
    pub split_spec_id: String,
    /// THE point-in-time anchor.
    pub as_of_knowledge_time: DateTime<Utc>,
    pub quality_exclusion_mask: QualityFlags,
    pub calendar_versions: Vec<CalendarVersion>,
    pub adjustment_policy: AdjustmentPolicy,
    pub continuous_method: Option<ContinuousMethod>,
    pub finality_policy: Option<FinalityPolicy>,
    pub runtime_image_digest: String,
    /// Quarantined adjusted-history sources this dataset explicitly opts into.
    #[serde(default)]
    pub opted_in_non_reproducible_sources: Vec<i32>,
}

impl DatasetSpec {
    /// Canonicalize before hashing so equal specs hash equally.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.instrument_ids.sort();
        self.instrument_ids.dedup();
        self.calendar_versions
            .sort_by(|a, b| (&a.calendar_id, &a.version).cmp(&(&b.calendar_id, &b.version)));
        self.calendar_versions.dedup();
        self.opted_in_non_reproducible_sources.sort_unstable();
        self.opted_in_non_reproducible_sources.dedup();
        self
    }

    /// `dataset_id` over the full normalized spec.
    ///
    /// # Panics
    /// Never in practice: every field is JSON-representable.
    #[must_use]
    pub fn dataset_id(&self) -> String {
        content_hash(&self.clone().normalized()).expect("dataset spec serializes")
    }

    /// Propagates into every trial that uses the dataset (INV-08).
    #[must_use]
    pub fn non_reproducible(&self) -> bool {
        self.continuous_method
            .is_some_and(ContinuousMethod::non_reproducible)
            || !self.opted_in_non_reproducible_sources.is_empty()
    }

    /// Flags a dataset spanning backfilled knowledge times. Flagged, not blocked
    /// (CLAUDE.md §6, OQ-01).
    #[must_use]
    pub fn uses_backfilled_knowledge(
        &self,
        row_flags: impl IntoIterator<Item = QualityFlags>,
    ) -> bool {
        !self
            .quality_exclusion_mask
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
            && row_flags
                .into_iter()
                .any(|f| f.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME))
    }

    /// What share of this dataset's rows carry a derived knowledge time (Set P
    /// P-1.6).
    ///
    /// A boolean was not enough. "This dataset touches backfilled history" is true
    /// of a window with one sentinel row in a million and of one with nothing else,
    /// and those are not the same claim: the first is a footnote and the second
    /// means the dataset supplies Gate 15 with no forward evidence at all (D-P3).
    /// So the manifest states the fraction, and a reader can tell which they have.
    ///
    /// A wholly backfilled dataset is **not refused** — it is flagged, with its
    /// fraction on the record. Refusing it would delete the only history the
    /// platform has; hiding the fraction would let it be read as observed.
    #[must_use]
    pub fn backfilled_share(
        &self,
        row_flags: impl IntoIterator<Item = QualityFlags>,
    ) -> BackfilledShare {
        let mut rows = 0_u64;
        let mut backfilled = 0_u64;
        for f in row_flags {
            rows += 1;
            if f.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME) {
                backfilled += 1;
            }
        }
        BackfilledShare { rows, backfilled }
    }
}

/// The backfilled-knowledge-time census of one dataset, for its manifest.
///
/// Counts rather than a pre-divided float, so a reader can see the denominator. A
/// fraction of `0.0` over zero rows and a fraction of `0.0` over a million rows mean
/// entirely different things, and a bare `f64` cannot tell them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfilledShare {
    pub rows: u64,
    pub backfilled: u64,
}

impl BackfilledShare {
    /// The share of rows whose knowledge time was derived rather than observed.
    ///
    /// `None` for an empty dataset: zero out of zero is not zero per cent, and
    /// reporting it as such would describe an empty window as fully observed.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        (self.rows > 0).then(|| self.backfilled as f64 / self.rows as f64)
    }

    /// No row in this dataset was watched arrive, so it supplies no forward
    /// evidence (Set P D-P3). Flagged, never a refusal.
    #[must_use]
    pub const fn wholly_backfilled(&self) -> bool {
        self.rows > 0 && self.backfilled == self.rows
    }

    #[must_use]
    pub const fn any_backfilled(&self) -> bool {
        self.backfilled > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn spec() -> DatasetSpec {
        DatasetSpec {
            universe_spec_id: "u".into(),
            instrument_ids: vec![InstrumentKey(3), InstrumentKey(1)],
            date_range: (
                Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            ),
            frequency: "1m".into(),
            feature_set_id: "fs".into(),
            label_spec_id: "l".into(),
            split_spec_id: "s".into(),
            as_of_knowledge_time: Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
            quality_exclusion_mask: QualityFlags::SUSPECT_VOLUME,
            calendar_versions: vec![CalendarVersion::new("XNYS", "2026.1")],
            adjustment_policy: AdjustmentPolicy::SplitsOnly,
            continuous_method: None,
            finality_policy: None,
            runtime_image_digest: "sha256:img".into(),
            opted_in_non_reproducible_sources: vec![],
        }
    }

    #[test]
    fn equal_specs_hash_equally_regardless_of_order() {
        let mut b = spec();
        b.instrument_ids.reverse();
        assert_eq!(spec().dataset_id(), b.dataset_id());
    }

    /// AT-12: changing the calendar version changes every affected dataset_id.
    #[test]
    fn calendar_version_enters_the_hash() {
        let mut b = spec();
        b.calendar_versions = vec![CalendarVersion::new("XNYS", "2026.2")];
        assert_ne!(spec().dataset_id(), b.dataset_id());
    }

    #[test]
    fn exclusion_mask_and_adjustment_and_image_enter_the_hash() {
        let base = spec().dataset_id();
        let mut m = spec();
        m.quality_exclusion_mask = QualityFlags::NONE;
        let mut a = spec();
        a.adjustment_policy = AdjustmentPolicy::Unadjusted;
        let mut i = spec();
        i.runtime_image_digest = "sha256:other".into();
        let mut k = spec();
        k.as_of_knowledge_time += chrono::Duration::seconds(1);
        for other in [m, a, i, k] {
            assert_ne!(base, other.dataset_id());
        }
    }

    #[test]
    fn back_adjusted_datasets_are_non_reproducible() {
        let mut s = spec();
        assert!(!s.non_reproducible());
        s.continuous_method = Some(ContinuousMethod::BackAdjustedDifference);
        assert!(s.non_reproducible());
    }

    #[test]
    fn backfilled_knowledge_is_flagged_unless_excluded() {
        let s = spec();
        assert!(s.uses_backfilled_knowledge([QualityFlags::BACKFILLED_KNOWLEDGE_TIME]));
        let mut excl = spec();
        excl.quality_exclusion_mask |= QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert!(!excl.uses_backfilled_knowledge([QualityFlags::BACKFILLED_KNOWLEDGE_TIME]));
    }
}
