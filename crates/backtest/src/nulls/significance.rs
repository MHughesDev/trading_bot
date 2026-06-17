//! INV-3: a significance result is never naked (spec §0).
//!
//! A [`SignificanceResult`] is invalid unless it carries (a) the null it was
//! tested against and (b) the trial count at the moment it was computed. There is
//! **no constructor that omits either** — the only way to make one is [`SignificanceResult::new`],
//! which requires all three values. Gate 3 (Phase 4) produces it; the workbench
//! (Phase 5) renders the p-value, the null, and the trial count inseparably or
//! renders nothing.

use serde::{Deserialize, Serialize};

use super::NullId;

/// A significance verdict bound to its null and the trial count at evaluation.
/// Fields are private so the type cannot be constructed with a bare p-value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignificanceResult {
    p_value: f64,
    null_ref: NullId,
    trial_count_at_eval: i64,
}

impl SignificanceResult {
    /// The only constructor — all three are mandatory (INV-3).
    #[must_use]
    pub fn new(p_value: f64, null_ref: NullId, trial_count_at_eval: i64) -> Self {
        Self {
            p_value,
            null_ref,
            trial_count_at_eval,
        }
    }

    #[must_use]
    pub fn p_value(&self) -> f64 {
        self.p_value
    }

    #[must_use]
    pub fn null_ref(&self) -> &NullId {
        &self.null_ref
    }

    #[must_use]
    pub fn trial_count_at_eval(&self) -> i64 {
        self.trial_count_at_eval
    }

    /// Render the inseparable triple for a report. Returns `None` only never —
    /// the type guarantees all three are present; this is the canonical display.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "p={:.4} vs {} @ {} trials",
            self.p_value, self.null_ref, self.trial_count_at_eval
        )
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::nulls::{Null, NullKind, NullParams};

    #[test]
    fn carries_null_and_trial_count_together() {
        let null = Null::new(NullKind::BlockPermutation, NullParams::default()).unwrap();
        let sig = SignificanceResult::new(0.012, null.null_id.clone(), 1320);
        assert_eq!(sig.p_value(), 0.012);
        assert_eq!(sig.trial_count_at_eval(), 1320);
        assert_eq!(sig.null_ref(), &null.null_id);
        assert!(sig.render().contains("1320"));
        assert!(sig.render().contains("p=0.0120"));
    }

    #[test]
    fn round_trips_serde_with_all_fields() {
        let null = Null::new(NullKind::SignalReturnDecouple, NullParams::default()).unwrap();
        let sig = SignificanceResult::new(0.05, null.null_id, 7);
        let back: SignificanceResult =
            serde_json::from_str(&serde_json::to_string(&sig).unwrap()).unwrap();
        assert_eq!(sig, back);
    }
}
