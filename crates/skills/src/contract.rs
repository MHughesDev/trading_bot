//! What a skill promises about its input and output.
//!
//! A contract is two lists of key names and a sentence. That is less than a
//! schema and more than nothing, and the middle is deliberate: a full JSON
//! Schema would be written once, drift from the code, and be believed anyway.
//! Key names can be checked against the actual payload on every run, cheaply,
//! and a mismatch is a fact rather than an opinion.
//!
//! The check runs on **both sides**. Missing inputs are the caller's fault and
//! missing outputs are the skill's, and [`Violation`] says which — a skill that
//! gets blamed for its caller's bad payload is a skill that looks unreliable and
//! is not, which would poison the statistics in [`crate::SkillStats`] that the
//! whole "gets better over time" claim rests on.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The declared interface of a skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    /// Keys the payload must contain. Extra keys are fine: a skill that ignores
    /// what it does not need composes, and one that refuses unknown keys breaks
    /// every time a caller adds a field.
    inputs: BTreeSet<String>,
    /// Keys the result must contain.
    outputs: BTreeSet<String>,
    /// What the transformation does, for a reader choosing between skills.
    summary: String,
}

impl Contract {
    /// Declare a contract.
    ///
    /// # Errors
    /// [`ContractError`] when a key name is unusable or the output list is
    /// empty — a skill that promises nothing cannot be checked, and a skill that
    /// cannot be checked cannot accumulate a record.
    pub fn new<I, O>(inputs: I, outputs: O, summary: impl Into<String>) -> Result<Self, ContractError>
    where
        I: IntoIterator,
        I::Item: Into<String>,
        O: IntoIterator,
        O::Item: Into<String>,
    {
        let inputs: BTreeSet<String> = inputs.into_iter().map(Into::into).collect();
        let outputs: BTreeSet<String> = outputs.into_iter().map(Into::into).collect();

        for key in inputs.iter().chain(outputs.iter()) {
            if key.trim().is_empty() || key.len() > 128 {
                return Err(ContractError::BadKey(key.clone()));
            }
        }
        if outputs.is_empty() {
            return Err(ContractError::NoOutputs);
        }
        Ok(Self {
            inputs,
            outputs,
            summary: summary.into(),
        })
    }

    #[must_use]
    pub fn inputs(&self) -> &BTreeSet<String> {
        &self.inputs
    }
    #[must_use]
    pub fn outputs(&self) -> &BTreeSet<String> {
        &self.outputs
    }
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Check a payload before running.
    ///
    /// # Errors
    /// [`Violation::MissingInputs`] naming every absent key at once — reporting
    /// them one at a time costs a round trip per key.
    pub fn check_input(&self, payload: &serde_json::Value) -> Result<(), Violation> {
        let Some(obj) = payload.as_object() else {
            return Err(Violation::NotAnObject { side: Side::Input });
        };
        let missing: Vec<String> = self
            .inputs
            .iter()
            .filter(|k| !obj.contains_key(*k))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(Violation::MissingInputs { missing })
        }
    }

    /// Check a result after running.
    ///
    /// # Errors
    /// [`Violation::MissingOutputs`] naming every absent key.
    pub fn check_output(&self, result: &serde_json::Value) -> Result<(), Violation> {
        let Some(obj) = result.as_object() else {
            return Err(Violation::NotAnObject { side: Side::Output });
        };
        let missing: Vec<String> = self
            .outputs
            .iter()
            .filter(|k| !obj.contains_key(*k))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(Violation::MissingOutputs { missing })
        }
    }
}

/// Which side of the contract failed. The distinction that keeps a skill from
/// being blamed for its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "violation", rename_all = "snake_case")]
pub enum Violation {
    #[error(
        "the payload is missing {}: the skill declares these inputs and the caller did not \
         supply them. This is the caller's error, not the skill's.",
        missing.join(", ")
    )]
    MissingInputs { missing: Vec<String> },
    #[error(
        "the result is missing {}: the skill declares these outputs and did not return them. \
         This is the skill's error.",
        missing.join(", ")
    )]
    MissingOutputs { missing: Vec<String> },
    #[error("the {side:?} is not a JSON object; a skill takes one object and returns one object")]
    NotAnObject { side: Side },
}

impl Violation {
    /// Whose fault this is. [`crate::SkillStats`] counts the two separately,
    /// because a skill blamed for its caller's bad payload looks unreliable and
    /// is not.
    #[must_use]
    pub const fn blames(&self) -> Side {
        match self {
            Self::MissingInputs { .. } => Side::Input,
            Self::MissingOutputs { .. } => Side::Output,
            Self::NotAnObject { side } => *side,
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("{0:?} is not a usable key name")]
    BadKey(String),
    #[error(
        "a contract must declare at least one output: a skill that promises nothing cannot be \
         checked, and one that cannot be checked cannot accumulate a record"
    )]
    NoOutputs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c() -> Contract {
        Contract::new(["returns", "window"], ["sharpe"], "Rolling Sharpe.").unwrap()
    }

    #[test]
    fn a_matching_payload_passes_both_sides() {
        assert!(c().check_input(&json!({"returns": [1], "window": 20})).is_ok());
        assert!(c().check_output(&json!({"sharpe": 1.2})).is_ok());
    }

    /// A skill that ignores what it does not need composes; one that refuses
    /// unknown keys breaks every time a caller adds a field.
    #[test]
    fn extra_keys_are_allowed_on_both_sides() {
        assert!(c()
            .check_input(&json!({"returns": [1], "window": 20, "note": "x"}))
            .is_ok());
        assert!(c().check_output(&json!({"sharpe": 1.2, "n": 250})).is_ok());
    }

    /// Reporting missing keys one at a time costs a round trip per key.
    #[test]
    fn every_missing_key_is_named_at_once() {
        let err = c().check_input(&json!({})).unwrap_err();
        match &err {
            Violation::MissingInputs { missing } => {
                assert_eq!(missing, &vec!["returns".to_string(), "window".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        assert!(err.to_string().contains("returns, window"), "{err}");
    }

    /// The distinction the statistics depend on: a skill blamed for its
    /// caller's bad payload looks unreliable and is not.
    #[test]
    fn a_violation_says_whose_fault_it_is() {
        assert_eq!(c().check_input(&json!({})).unwrap_err().blames(), Side::Input);
        assert_eq!(c().check_output(&json!({})).unwrap_err().blames(), Side::Output);

        let input_err = c().check_input(&json!({})).unwrap_err().to_string();
        assert!(input_err.contains("caller's error"), "{input_err}");
        let output_err = c().check_output(&json!({})).unwrap_err().to_string();
        assert!(output_err.contains("skill's error"), "{output_err}");
    }

    #[test]
    fn a_non_object_is_refused_on_the_side_it_arrived_on() {
        assert_eq!(
            c().check_input(&json!([1, 2])).unwrap_err(),
            Violation::NotAnObject { side: Side::Input }
        );
        assert_eq!(
            c().check_output(&json!("done")).unwrap_err(),
            Violation::NotAnObject { side: Side::Output }
        );
    }

    #[test]
    fn a_contract_with_no_outputs_is_refused() {
        let empty: [&str; 0] = [];
        assert_eq!(
            Contract::new(["a"], empty, "x").unwrap_err(),
            ContractError::NoOutputs
        );
    }

    #[test]
    fn a_contract_with_no_inputs_is_fine() {
        let empty: [&str; 0] = [];
        let c = Contract::new(empty, ["now"], "Current time.").unwrap();
        assert!(c.check_input(&json!({})).is_ok());
    }

    #[test]
    fn an_unusable_key_name_is_refused() {
        assert!(Contract::new([" "], ["out"], "x").is_err());
        assert!(Contract::new(["a"], ["x".repeat(129)], "x").is_err());
    }

    #[test]
    fn a_contract_round_trips_through_json() {
        let j = serde_json::to_string(&c()).unwrap();
        assert_eq!(serde_json::from_str::<Contract>(&j).unwrap(), c());
    }
}
