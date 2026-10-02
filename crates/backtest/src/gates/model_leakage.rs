//! Gate 0's model check (Set U U-5.4; FEAT-005 §4.3; ADR-0043 decision 5).
//!
//! A run that consumed any model answer from a version trained past the
//! decision instant minus its embargo has looked at the future, whatever its
//! other numbers say. This fails it, naming the **first** offending decision,
//! because the first one is where the result stopped meaning anything.

use crate::models::ModelConsumption;
use crate::run::Flag;

/// The stable code a failing run carries.
pub const CODE: &str = "leakage.model_trained_past_decision";

/// `Some(flag)` when any consumed answer came from a version that was not
/// trained strictly before the decision instant minus its embargo — or whose
/// training end is unknown, since an unknown training end cannot be shown to be
/// in the past.
#[must_use]
pub fn check(consumed: &[ModelConsumption]) -> Option<Flag> {
    let first = consumed
        .iter()
        .filter(|c| !c.point_in_time())
        .min_by_key(|c| c.at_ns)?;
    let at = chrono::DateTime::from_timestamp_nanos(first.at_ns);
    let trained = first.trained_through_ns.map_or_else(
        || "an unknown instant".to_string(),
        |t| chrono::DateTime::from_timestamp_nanos(t).to_rfc3339(),
    );
    Some(Flag {
        code: CODE.into(),
        detail: format!(
            "first at {}: {}@{} answered a decision it was trained past (trained through {trained}, \
             embargo {}s)",
            at.to_rfc3339(),
            first.model,
            first.version,
            first.embargo_ns / 1_000_000_000
        ),
    })
}
