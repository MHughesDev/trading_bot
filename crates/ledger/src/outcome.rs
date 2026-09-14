//! The outcome is a typed vector (SPEC §4.3, ADR-012, AT-34). There is no score.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutcomeVector {
    // predictive
    pub auc: Option<f64>,
    pub logloss: Option<f64>,
    pub brier: Option<f64>,
    pub ece: Option<f64>,
    pub ic: Option<f64>,
    pub ic_ir: Option<f64>,
    // portfolio, net of costs
    pub sharpe_net: Option<f64>,
    pub sortino_net: Option<f64>,
    pub calmar: Option<f64>,
    pub psr: Option<f64>,
    pub dsr: Option<f64>,
    pub pbo: Option<f64>,
    pub max_dd: Option<f64>,
    pub dd_duration_days: Option<i32>,
    pub turnover_annual: Option<f64>,
    pub capacity_usd: Option<f64>,
    pub breakeven_cost_multiple: Option<f64>,
    // robustness
    pub seed_sharpe_std: Option<f64>,
    pub regime_pnl_hhi: Option<f64>,
    pub cpcv_p05_sharpe: Option<f64>,
    pub bootstrap_p05_sharpe: Option<f64>,
    pub param_cliff_score: Option<f64>,
    // attribution
    pub alpha_t_stat: Option<f64>,
    pub factor_r2: Option<f64>,
    // operational
    pub inference_latency_p99_ms: Option<f64>,
    pub model_bytes: Option<i64>,
    pub train_gpu_seconds: Option<f64>,
}

/// Field names, in the column order of `mlops.outcome_vector`.
pub const OUTCOME_FIELDS: &[&str] = &[
    "auc", "logloss", "brier", "ece", "ic", "ic_ir", "sharpe_net", "sortino_net", "calmar", "psr", "dsr", "pbo",
    "max_dd", "dd_duration_days", "turnover_annual", "capacity_usd", "breakeven_cost_multiple", "seed_sharpe_std",
    "regime_pnl_hhi", "cpcv_p05_sharpe", "bootstrap_p05_sharpe", "param_cliff_score", "alpha_t_stat", "factor_r2",
    "inference_latency_p99_ms", "model_bytes", "train_gpu_seconds",
];

/// Names that would reintroduce a scalar "this is optimal" column (AT-34).
pub const FORBIDDEN_SCALAR_FIELDS: &[&str] = &["score", "fitness", "objective_value", "reward", "rank"];

impl OutcomeVector {
    /// Digest chained into `mlops.trial_event.outcome_digest`. Canonical JSON over
    /// the exact f64 bits serde renders, so a value read back from Postgres
    /// reproduces the digest byte for byte.
    ///
    /// # Panics
    /// Never: every field is JSON-representable (non-finite values serialize as null).
    #[must_use]
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(&serde_json::to_value(self).expect("outcome serializes")).expect("outcome serializes");
        format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
    }

    /// Value of a named metric, for objective evaluation.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f64> {
        let v = serde_json::to_value(self).ok()?;
        v.get(name)?.as_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_list_matches_the_struct_and_has_no_scalar_score() {
        let v = serde_json::to_value(OutcomeVector::default()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for f in OUTCOME_FIELDS {
            assert!(keys.contains(f), "{f} missing from OutcomeVector");
        }
        assert_eq!(keys.len(), OUTCOME_FIELDS.len());
        for bad in FORBIDDEN_SCALAR_FIELDS {
            assert!(!keys.contains(bad), "{bad} would scalarize the outcome (AT-34)");
        }
    }

    #[test]
    fn digest_is_stable_and_sensitive() {
        let a = OutcomeVector { sharpe_net: Some(1.25), ..Default::default() };
        let mut b = a;
        assert_eq!(a.digest(), b.digest());
        b.sharpe_net = Some(1.250_000_000_000_000_2);
        assert_ne!(a.digest(), b.digest());
    }
}
