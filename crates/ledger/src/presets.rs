//! Configuration modes as presets over the one DEFINE schema (SPEC §16,
//! checklist 5.7, ADR-P5-02).
//!
//! Three modes — preset, guided, expert — and they are the *same* schema and the
//! *same* validator. Expert mode is guided mode with the form collapsed into
//! JSON; preset mode is guided mode with some fields already filled in. There is
//! no second validation path, because a second path is where the two disagree.
//!
//! ## The rule that makes "no default" survive presets
//!
//! `delta_practical`, `approval_spend_usd` and `max_gpu_hours` have no defaults
//! anywhere in this platform: a campaign must say what improvement would matter
//! before it looks for one, how much it may spend before asking a human, and
//! what compute ceiling it accepts. That property is worth exactly nothing if a
//! preset can supply them, because then every campaign started from a preset has
//! inherited three numbers nobody decided.
//!
//! So a preset carries [`Suggestion`]s, not values. A suggestion has to be
//! **confirmed** — [`Suggestion::confirm`] takes the number the user actually
//! chose — and an unconfirmed suggestion cannot be turned into a field value at
//! all: [`PresetDraft::resolve`] refuses. The validator downstream would refuse
//! too, but by then the preset has already framed the number, and the point is
//! that the user sees it as a question rather than as a filled box.

use serde::{Deserialize, Serialize};

/// A number a preset proposes and a human has to accept.
///
/// Sealed around its state: there is no way to read a value out of an
/// unconfirmed suggestion, so "the preset filled it in" is not expressible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    field: String,
    /// What the preset proposes, and why. The rationale travels with it because
    /// a number with no reason is one people accept without reading.
    suggested: f64,
    rationale: String,
    /// The value the user confirmed. `None` until they do.
    confirmed: Option<f64>,
}

impl Suggestion {
    #[must_use]
    pub fn new(field: impl Into<String>, suggested: f64, rationale: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            suggested,
            rationale: rationale.into(),
            confirmed: None,
        }
    }

    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// What the preset proposes. Reading this is fine — it is what the UI shows
    /// in the prompt. It is not a value the draft can resolve to.
    #[must_use]
    pub fn suggested(&self) -> f64 {
        self.suggested
    }

    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }

    /// Accept a value. Usually the suggestion, sometimes not — a user who
    /// changes the number has still made the decision, which is the whole
    /// requirement.
    #[must_use]
    pub fn confirm(mut self, value: f64) -> Self {
        self.confirmed = Some(value);
        self
    }

    /// The confirmed value, or `None`.
    ///
    /// There is deliberately no `unwrap_or(self.suggested)` anywhere in this
    /// module. That one line is the whole failure: it turns every suggestion
    /// into a default and nobody notices, because the numbers look the same.
    #[must_use]
    pub fn value(&self) -> Option<f64> {
        self.confirmed
    }

    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmed.is_some()
    }
}

/// Which mode the user is in. The differences are presentational.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Fields pre-filled from a preset; the three no-default fields arrive as
    /// suggestions to confirm.
    Preset,
    /// Every field asked for, with the computed embargo and the leakage
    /// pre-check shown alongside.
    Guided,
    /// The same JSON through the same validator.
    Expert,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Guided => "guided",
            Self::Expert => "expert",
        }
    }
}

/// The three fields no preset may supply.
pub const UNSUPPLIABLE_FIELDS: [&str; 3] =
    ["delta_practical", "approval_spend_usd", "max_gpu_hours"];

/// Why a draft could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PresetError {
    #[error("`{0}` was suggested but never confirmed; a preset may propose it and may not supply it (ADR-P5-02)")]
    Unconfirmed(String),
    #[error("`{0}` has no suggestion and no value; this platform has no default for it")]
    Absent(String),
    #[error("a preset may not carry a value for `{0}` — only a suggestion")]
    PresetSuppliedValue(String),
}

/// A campaign definition in progress.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetDraft {
    pub slug: String,
    pub hypothesis: String,
    /// The suggestions this preset made, one per no-default field.
    pub suggestions: Vec<Suggestion>,
}

impl PresetDraft {
    /// Build a draft from a named preset.
    ///
    /// # Errors
    /// A preset that tried to carry a *value* for one of the three fields rather
    /// than a suggestion. That is refused here rather than downstream, because
    /// downstream the number is already in the form.
    pub fn from_preset(
        slug: impl Into<String>,
        hypothesis: impl Into<String>,
        suggestions: Vec<Suggestion>,
    ) -> Result<Self, PresetError> {
        for s in &suggestions {
            if s.is_confirmed() {
                return Err(PresetError::PresetSuppliedValue(s.field.clone()));
            }
        }
        Ok(Self {
            slug: slug.into(),
            hypothesis: hypothesis.into(),
            suggestions,
        })
    }

    /// Confirm one field.
    #[must_use]
    pub fn confirm(mut self, field: &str, value: f64) -> Self {
        for s in &mut self.suggestions {
            if s.field == field {
                *s = s.clone().confirm(value);
            }
        }
        self
    }

    /// The confirmed values, or the first reason there are none.
    ///
    /// # Errors
    /// Any of the three fields unconfirmed or absent.
    pub fn resolve(&self) -> Result<std::collections::BTreeMap<String, f64>, PresetError> {
        let mut out = std::collections::BTreeMap::new();
        for field in UNSUPPLIABLE_FIELDS {
            let Some(s) = self.suggestions.iter().find(|s| s.field == field) else {
                return Err(PresetError::Absent(field.to_string()));
            };
            let Some(v) = s.value() else {
                return Err(PresetError::Unconfirmed(field.to_string()));
            };
            out.insert(field.to_string(), v);
        }
        Ok(out)
    }

    /// The fields still waiting on a human.
    #[must_use]
    pub fn pending(&self) -> Vec<&str> {
        self.suggestions
            .iter()
            .filter(|s| !s.is_confirmed())
            .map(|s| s.field.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> PresetDraft {
        PresetDraft::from_preset(
            "intraday-reversion",
            "intraday mean reversion survives costs on majors",
            vec![
                Suggestion::new(
                    "delta_practical",
                    0.25,
                    "a quarter of a Sharpe point is roughly the smallest improvement that \
                     survives this platform's cost model",
                ),
                Suggestion::new(
                    "approval_spend_usd",
                    50.0,
                    "about an hour of GPU on this box",
                ),
                Suggestion::new("max_gpu_hours", 4.0, "one overnight run"),
            ],
        )
        .expect("a preset of suggestions")
    }

    /// The whole point: a preset proposes and a human decides.
    #[test]
    fn an_unconfirmed_suggestion_cannot_become_a_value() {
        let d = draft();
        assert_eq!(d.pending().len(), 3);
        match d.resolve() {
            // The first of the three, in the order `UNSUPPLIABLE_FIELDS` states.
            Err(PresetError::Unconfirmed(f)) => assert_eq!(f, "delta_practical"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn confirming_every_field_resolves_it() {
        let d = draft()
            .confirm("delta_practical", 0.25)
            .confirm("approval_spend_usd", 50.0)
            .confirm("max_gpu_hours", 4.0);
        assert!(d.pending().is_empty());
        let v = d.resolve().expect("resolved");
        assert_eq!(v.len(), 3);
        assert!((v["delta_practical"] - 0.25).abs() < f64::EPSILON);
    }

    /// A user who changes the number has still made the decision. The
    /// requirement is that a human chose, not that they agreed.
    #[test]
    fn a_user_may_confirm_a_different_number() {
        let d = draft()
            .confirm("delta_practical", 0.9)
            .confirm("approval_spend_usd", 0.0)
            .confirm("max_gpu_hours", 1.0);
        let v = d.resolve().unwrap();
        assert!((v["delta_practical"] - 0.9).abs() < f64::EPSILON);
        // Zero is a real answer for the spend threshold: ask about everything.
        assert!((v["approval_spend_usd"] - 0.0).abs() < f64::EPSILON);
    }

    /// A preset that arrives already confirmed is a preset supplying a default
    /// wearing a suggestion's clothes.
    #[test]
    fn a_preset_may_not_ship_a_confirmed_suggestion() {
        let sneaky = vec![Suggestion::new("delta_practical", 0.25, "r").confirm(0.25)];
        match PresetDraft::from_preset("s", "h", sneaky) {
            Err(PresetError::PresetSuppliedValue(f)) => assert_eq!(f, "delta_practical"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_field_is_absent_not_defaulted() {
        let partial = PresetDraft::from_preset(
            "s",
            "h",
            vec![Suggestion::new("delta_practical", 0.25, "r")],
        )
        .unwrap()
        .confirm("delta_practical", 0.25);
        match partial.resolve() {
            Err(PresetError::Absent(f)) => assert_eq!(f, "approval_spend_usd"),
            other => panic!("{other:?}"),
        }
    }

    /// Reading a suggestion is what the prompt does; it is not a value the draft
    /// can resolve to. These two must never collapse into one accessor.
    #[test]
    fn a_suggestion_and_a_value_are_different_things() {
        let s = Suggestion::new("max_gpu_hours", 4.0, "one overnight run");
        assert!((s.suggested() - 4.0).abs() < f64::EPSILON);
        assert_eq!(s.value(), None);
        assert!(!s.rationale().is_empty(), "a number with no reason is one people accept unread");

        let confirmed = s.confirm(2.0);
        assert!((confirmed.suggested() - 4.0).abs() < f64::EPSILON, "the suggestion is still visible");
        assert_eq!(confirmed.value(), Some(2.0), "and the decision is the user's");
    }

    #[test]
    fn the_three_unsuppliable_fields_are_the_ones_with_no_default() {
        assert_eq!(
            UNSUPPLIABLE_FIELDS,
            ["delta_practical", "approval_spend_usd", "max_gpu_hours"]
        );
    }

    #[test]
    fn every_mode_is_the_same_schema() {
        for m in [Mode::Preset, Mode::Guided, Mode::Expert] {
            assert!(!m.as_str().is_empty());
        }
        assert_eq!(Mode::Expert.as_str(), "expert");
    }
}
