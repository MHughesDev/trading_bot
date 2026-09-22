//! The comparison protocol's evidence and its verdict, as the ledger stores
//! them (SPEC §11.5, §10; migration 0113; ADR-P2-34, ADR-P2-35).
//!
//! Two record types, and the asymmetry between them is deliberate.
//!
//! [`ReplicateMeasurement`] is evidence: one arm, one replicate of the nuisance
//! grid, one number, and — required, not optional — the counted trial that
//! produced it. The ledger will not accept a measurement that does not name a
//! trial, so "a comparison built from numbers nobody paid a look for" is not
//! something a caller can express (INV-16, read from the evidence end).
//!
//! [`ComparisonRecord`] is a verdict, and it is deliberately *not* a type this
//! crate can construct from parts. It carries every field of
//! `backtest::stats::compare::Comparison`, which is sealed and can only be
//! produced by `ComparisonPlan::judge`; the conversion lives in the crate that
//! owns the sealed type, so there is no second door into this table that skips
//! the protocol. What this crate owns is the storage, the idempotency and the
//! reading — never the judging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One draw of the §11.5 nuisance parameters: which window of history, which
/// cost perturbation, which stochastic-fill seed.
///
/// The same key is applied to both arms — that is what makes the difference
/// paired, and an unpaired difference estimates a different and much noisier
/// quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReplicateKey {
    pub window_index: i32,
    /// Cost-model perturbation in hundredths of a basis point of the modelled
    /// cost, kept integral so the key is hashable and comparable.
    pub cost_bp: i32,
    pub seed: i64,
}

/// One arm's measurement on one replicate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplicateMeasurement {
    /// Which side of the comparison. `"incumbent"` is the campaign's declared
    /// benchmark; a challenger names the candidate it came from.
    pub arm: String,
    pub key: ReplicateKey,
    /// The declared objective this number is a value of.
    pub metric: String,
    pub value: f64,
    /// The counted look that produced it (INV-16).
    pub trial_id: Uuid,
}

/// The name the campaign's benchmark arm is recorded under.
///
/// A constant rather than a caller's string: the COMPARE phase reads the two
/// arms back by name, and a typo on one side would silently produce an
/// unpairable comparison rather than an error.
pub const INCUMBENT_ARM: &str = "incumbent";

/// Everything a judged pair carries into the table.
///
/// Every field is required for the same reason the sealed `Comparison` has no
/// public constructor: a verdict that travels without its profile, its
/// effective trial count, its declared effect size and its flags is a bare
/// number, and a bare number is what gets deployed on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonRecord {
    /// The fold position of the COMPARE phase that produced it. The ledger is
    /// unique on `(campaign_id, campaign_seq)`, so a re-queued worker at the
    /// same position writes nothing new.
    pub campaign_seq: i64,
    pub arm_a: String,
    pub arm_b: String,
    pub metric: String,
    pub verdict: String,
    pub cs_centre: f64,
    pub cs_lower: f64,
    pub cs_upper: f64,
    pub alpha: f64,
    pub mean_difference: f64,
    pub p_a_better: f64,
    pub p_a_better_lower: f64,
    pub achieved_k: i32,
    pub wins: i32,
    pub ties: i32,
    pub losses: i32,
    pub profile_id: String,
    pub n_eff: f64,
    pub delta_practical: f64,
    pub trials_a: i64,
    pub trials_b: i64,
    pub flags: serde_json::Value,
    pub headline: String,
}

/// A stored comparison, with the time it was recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredComparison {
    pub record: ComparisonRecord,
    pub recorded_at: DateTime<Utc>,
}

impl StoredComparison {
    /// Whether this comparison's anytime-valid upper bound rules out an
    /// improvement of at least `delta_practical`.
    ///
    /// This is the arithmetic behind §10's DIMINISHING_RETURNS rule, and it is
    /// computed from the recorded interval rather than read off the verdict
    /// string: the verdict is a summary, the bound is the measurement.
    ///
    /// The interval is anytime-valid at level `alpha`, so an upper bound below
    /// `δ` means `P(true difference > δ) ≤ alpha` — which is exactly the
    /// probability §10 asks about, and the reason it can be asked after an
    /// unbounded number of peeks.
    #[must_use]
    pub fn rules_out_delta(&self) -> bool {
        self.record.cs_upper < self.record.delta_practical
    }

    /// Whether this comparison moved the incumbent. Only a promotion does.
    #[must_use]
    pub fn promoted(&self) -> bool {
        self.record.verdict == "promote"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(cs_upper: f64, verdict: &str) -> StoredComparison {
        StoredComparison {
            record: ComparisonRecord {
                campaign_seq: 7,
                arm_a: "challenger".into(),
                arm_b: INCUMBENT_ARM.into(),
                metric: "sharpe_net".into(),
                verdict: verdict.into(),
                cs_centre: 0.0,
                cs_lower: -1.0,
                cs_upper,
                alpha: 0.05,
                mean_difference: 0.0,
                p_a_better: 0.5,
                p_a_better_lower: 0.1,
                achieved_k: 9,
                wins: 4,
                ties: 1,
                losses: 4,
                profile_id: "strict_v1".into(),
                n_eff: 12.0,
                delta_practical: 0.25,
                trials_a: 9,
                trials_b: 9,
                flags: serde_json::json!({}),
                headline: "reject — ...".into(),
            },
            recorded_at: Utc::now(),
        }
    }

    /// The diminishing-returns test reads the bound, not the label. A verdict
    /// of `keep_sampling` whose interval has nevertheless closed below δ still
    /// rules δ out, and a `reject` whose bound sits above δ does not.
    #[test]
    fn the_delta_rule_reads_the_interval_rather_than_the_verdict_string() {
        assert!(record(0.10, "keep_sampling").rules_out_delta());
        assert!(!record(0.90, "reject").rules_out_delta());
        // Exactly at δ is not below it: the interval still spans the effect
        // size the campaign said would matter.
        assert!(!record(0.25, "reject").rules_out_delta());
    }

    #[test]
    fn only_a_promotion_moves_the_incumbent() {
        assert!(record(0.1, "promote").promoted());
        for other in ["reject", "keep_sampling", "practically_equivalent"] {
            assert!(!record(0.1, other).promoted(), "{other}");
        }
    }
}
