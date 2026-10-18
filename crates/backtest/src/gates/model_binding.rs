//! The Gate 3 model binding (Set N N-3.6; FEAT-006 section 7, M6).
//!
//! A model or prediction series bound into a strategy that enters Gate 3 (significance) must
//! clear the hard usage rules: a return forecast needs a significant out-of-sample R-squared,
//! a volatility forecast must beat HARQ, and nothing sizes on VaR or ES without a passed ES
//! backtest. The rules and their evidence live in `crates/model-registry` (`verdict`,
//! `scoring_v2`), which depends on this crate, so this crate cannot call it. It defines the
//! seam instead: [`ModelUsagePolicy`], which the api layer implements over the registry.
//!
//! [`BindingOutcome`] has no constructor from a boolean and no public fields: the only way to
//! get one is [`BindingOutcome::judge`], which asks the policy about **every** binding. A
//! binding that names no use is refused (an unstated use is not a passed one, ADR-N-05).
//! [`super::GateRunner::with_model_bindings`] installs it, after which entering Significance or
//! the Vault returns [`super::GateError::ModelBindingRefused`] while any binding is refused.

use serde::{Deserialize, Serialize};

/// What a bound model is used for. The same three as the registry's `ForecastUse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUse {
    /// A return or direction forecast.
    Return,
    /// A volatility forecast.
    Vol,
    /// A `var_95` / `es_95` consumer.
    TailRisk,
}

/// One model version bound into the candidate strategy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBinding {
    pub model_id: String,
    pub version: i32,
    pub uses: Vec<ModelUse>,
}

impl ModelBinding {
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}@{}", self.model_id, self.version)
    }
}

/// The policy's answer for one binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingDecision {
    pub allowed: bool,
    /// Why not (and, when allowed, any notes); a refusal always has at least one.
    pub reasons: Vec<String>,
}

impl BindingDecision {
    #[must_use]
    pub fn allow() -> Self {
        Self {
            allowed: true,
            reasons: Vec::new(),
        }
    }

    #[must_use]
    pub fn refuse(reasons: Vec<String>) -> Self {
        let reasons = if reasons.is_empty() {
            vec!["refused without a reason".to_string()]
        } else {
            reasons
        };
        Self {
            allowed: false,
            reasons,
        }
    }
}

/// The registry's usage rules, as the gate sees them. Implemented in `crates/api`.
#[async_trait::async_trait]
pub trait ModelUsagePolicy: Send + Sync {
    /// Whether `binding` may size a position for the uses it names. A missing measurement is
    /// a refusal, never an allowance.
    async fn decide(&self, binding: &ModelBinding) -> BindingDecision;
}

/// Every binding judged: the gate's evidence that the models it was handed may be used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BindingOutcome {
    refusals: Vec<String>,
    judged: usize,
}

impl BindingOutcome {
    /// Ask `policy` about every binding. Nothing is short-circuited, so the report names every
    /// refused model at once.
    pub async fn judge(bindings: &[ModelBinding], policy: &dyn ModelUsagePolicy) -> Self {
        let mut refusals = Vec::new();
        for b in bindings {
            if b.uses.is_empty() {
                refusals.push(format!(
                    "{}: the binding names no use, so no rule can be checked",
                    b.label()
                ));
                continue;
            }
            let d = policy.decide(b).await;
            if !d.allowed {
                refusals.push(format!("{}: {}", b.label(), d.reasons.join("; ")));
            }
        }
        Self {
            refusals,
            judged: bindings.len(),
        }
    }

    /// How many bindings were judged.
    #[must_use]
    pub fn judged(&self) -> usize {
        self.judged
    }

    /// `Ok` when every binding was allowed; otherwise one message naming each refusal.
    ///
    /// # Errors
    /// The refusals, joined.
    pub fn satisfied(&self) -> Result<(), String> {
        if self.refusals.is_empty() {
            Ok(())
        } else {
            Err(self.refusals.join(" | "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::Experiment;
    use crate::gates::{Gate, GateError, GateRunner};

    fn experiment() -> Experiment {
        use chrono::TimeZone;
        Experiment::new(
            "exp-mb",
            "ema-family",
            crate::run::DataSlice::new(
                "u",
                chrono::Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
                chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                crate::run::EvalResolution::Day1,
            ),
            "null:block",
        )
    }

    /// A policy that allows exactly the named models.
    struct Allow(&'static [&'static str]);

    #[async_trait::async_trait]
    impl ModelUsagePolicy for Allow {
        async fn decide(&self, b: &ModelBinding) -> BindingDecision {
            if self.0.contains(&b.model_id.as_str()) {
                BindingDecision::allow()
            } else {
                BindingDecision::refuse(vec!["vol model does not beat HARQ".into()])
            }
        }
    }

    fn bind(id: &str, uses: &[ModelUse]) -> ModelBinding {
        ModelBinding {
            model_id: id.into(),
            version: 3,
            uses: uses.to_vec(),
        }
    }

    #[tokio::test]
    async fn a_passing_binding_proceeds_and_a_failing_one_names_itself() {
        let ok = BindingOutcome::judge(&[bind("good", &[ModelUse::Vol])], &Allow(&["good"])).await;
        assert!(ok.satisfied().is_ok());
        let both = BindingOutcome::judge(
            &[bind("good", &[ModelUse::Vol]), bind("bad", &[ModelUse::Vol])],
            &Allow(&["good"]),
        )
        .await;
        let e = both.satisfied().unwrap_err();
        assert!(e.contains("bad@3") && e.contains("HARQ") && !e.contains("good@3"), "{e}");
    }

    #[tokio::test]
    async fn a_binding_that_names_no_use_is_refused_without_asking_the_policy() {
        let o = BindingOutcome::judge(&[bind("good", &[])], &Allow(&["good"])).await;
        assert!(o.satisfied().unwrap_err().contains("names no use"));
    }

    #[tokio::test]
    async fn no_bindings_is_vacuously_satisfied_and_says_zero_were_judged() {
        let o = BindingOutcome::judge(&[], &Allow(&[])).await;
        assert!(o.satisfied().is_ok());
        assert_eq!(o.judged(), 0);
    }

    #[tokio::test]
    async fn the_gate_runner_refuses_significance_and_the_vault_for_a_refused_binding() {
        let refused =
            BindingOutcome::judge(&[bind("bad", &[ModelUse::Vol])], &Allow(&[])).await;
        let mut e = experiment();
        let runner = GateRunner::new(&mut e).with_model_bindings(refused);
        let err = runner.enter(Gate::Significance).unwrap_err();
        assert!(matches!(err, GateError::ModelBindingRefused(ref m) if m.contains("bad@3")), "{err}");
        assert!(matches!(
            runner.enter(Gate::Vault).unwrap_err(),
            GateError::ModelBindingRefused(_)
        ));
        // Earlier gates are not blocked by it: the refusal is about sizing, at Gate 3.
        assert!(runner.enter(Gate::Integrity).is_ok());
    }

    #[tokio::test]
    async fn the_gate_runner_lets_an_allowed_binding_reach_the_prerequisite_check() {
        let ok = BindingOutcome::judge(&[bind("good", &[ModelUse::Return])], &Allow(&["good"])).await;
        let mut e = experiment();
        let runner = GateRunner::new(&mut e).with_model_bindings(ok);
        // Not refused by the binding: it falls through to "Robustness has not passed".
        assert!(matches!(
            runner.enter(Gate::Significance).unwrap_err(),
            GateError::PrerequisiteNotPassed { .. }
        ));
    }
}
