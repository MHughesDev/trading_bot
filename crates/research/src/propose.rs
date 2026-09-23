//! What a worker sub-run is allowed to hand back: one [`Move`], one hypothesis,
//! and the ranges to search (FEAT-003 §8.2 step 3, §8.3; AGENT-013 §8.5;
//! Set S task S-10.11).
//!
//! # Why this is a type and not a string
//!
//! The campaign's HYPOTHESIZE phase is the one place a model gets to change the
//! *structure* of what is being searched. Until now the shape of that answer was
//! prose, and prose has the failure mode this repository has hit repeatedly: a
//! free-form string that looks like a decision, is stored like a decision, and
//! cannot be replayed, validated or counted like one. A `Move` that does not
//! deserialise is refused at the boundary with the validator's reason, which is
//! the retry FEAT-003 §8.2 step 3 describes — and a `Move` that does deserialise
//! names a node, a feature or a mode that something downstream can look up.
//!
//! # One move, not a plan
//!
//! Exactly one. A generation that changed three things and improved would have
//! learned nothing about which change did it, and the hypothesis ledger would
//! record a claim no later run could act on. The campaign, not the model, decides
//! whether to spend a trial on it (FEAT-003 §8.4).
//!
//! # What this build can check, and what it cannot
//!
//! §8.3's table gives each move a machine check, and four of the eight need
//! machinery that is elsewhere: a graph validator, the feature lane, the regime
//! engine's label set, the simulator's capability list. Rather than approximate
//! them here — a validator that guesses is worse than one that abstains — each
//! move names the check it needs through [`Move::machine_check`], and
//! [`Proposal::validate`] performs the structural half and *reports* the rest as
//! [`Pending`]. The driver records what went unchecked, so "this proposal was
//! validated" never means more than it did.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::space::{Dim, DimKind};

/// How a strategy's exit is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitMode {
    Trail,
    Fixed,
    Time,
    Signal,
}

/// How position size is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizingMode {
    FixedFraction,
    VolScaled,
}

/// Which half of each parent a crossover takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombineHow {
    /// Entries from `a`, exits and filters from `b` — §8.3's stated direction.
    EntriesFromA,
}

/// One typed structural edit (FEAT-003 §8.3, vocabulary v1).
///
/// The variants are the table's eight rows and nothing else. A ninth kind of
/// edit is a spec change, not a string somebody passed through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "move", rename_all = "snake_case")]
pub enum Move {
    /// AND a new predicate into a Signal's condition.
    AddConditionFilter { node: String, expr: String },
    /// Drop a node and rewire.
    RemoveNode { node: String },
    /// Change the feature a Condition reads.
    SwapFeature {
        node: String,
        from: String,
        to: String,
    },
    /// Switch exit logic.
    ChangeExit { mode: ExitMode },
    /// Only trade when the regime is in this set.
    AddRegimeGate { regime: String },
    /// Fixed or vol-scaled sizing.
    ChangeSizing {
        mode: SizingMode,
        #[serde(default)]
        params: BTreeMap<String, f64>,
    },
    /// Remove the node whose ablation changes the objective least.
    ///
    /// No parameters: choosing *which* node is the ablation's job, and a
    /// `simplify` that named its victim would be a `remove_node` wearing a
    /// cheaper name. The ablations are Runs and count as trials (§8.3).
    Simplify,
    /// Crossover.
    Combine {
        a: String,
        b: String,
        how: CombineHow,
    },
}

/// A check §8.3 demands that this crate cannot perform alone.
///
/// Named rather than skipped, on the same principle as
/// `campaign_driver::TerminationCheck::NotComputable`: a reader of a proposal can
/// see which checks ran and which are owed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pending {
    /// The strategy graph must stay acyclic and complete.
    GraphValidator,
    /// The named feature must exist in the instrument's lane.
    FeatureLane,
    /// The regime engine must know the label.
    RegimeLabels,
    /// The simulator must support the sizing mode.
    SimulatorCapability,
    /// The expression must parse under the condition grammar.
    ConditionGrammar,
    /// The move is decided by ablation Runs, which are trials.
    AblationTrials,
}

impl Pending {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraphValidator => "graph_validator",
            Self::FeatureLane => "feature_lane",
            Self::RegimeLabels => "regime_labels",
            Self::SimulatorCapability => "simulator_capability",
            Self::ConditionGrammar => "condition_grammar",
            Self::AblationTrials => "ablation_trials",
        }
    }
}

impl Move {
    /// The vocabulary, as names. The catalogue and the grammar both read it, so
    /// there is one list.
    pub const KINDS: &'static [&'static str] = &[
        "add_condition_filter",
        "remove_node",
        "swap_feature",
        "change_exit",
        "add_regime_gate",
        "change_sizing",
        "simplify",
        "combine",
    ];

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AddConditionFilter { .. } => "add_condition_filter",
            Self::RemoveNode { .. } => "remove_node",
            Self::SwapFeature { .. } => "swap_feature",
            Self::ChangeExit { .. } => "change_exit",
            Self::AddRegimeGate { .. } => "add_regime_gate",
            Self::ChangeSizing { .. } => "change_sizing",
            Self::Simplify => "simplify",
            Self::Combine { .. } => "combine",
        }
    }

    /// The checks this move needs from outside this crate (§8.3's third column).
    #[must_use]
    pub fn machine_check(&self) -> &'static [Pending] {
        match self {
            Self::AddConditionFilter { .. } => {
                &[Pending::ConditionGrammar, Pending::GraphValidator]
            }
            Self::RemoveNode { .. } => &[Pending::GraphValidator],
            Self::SwapFeature { .. } => &[Pending::FeatureLane],
            Self::ChangeExit { .. } => &[Pending::GraphValidator],
            Self::AddRegimeGate { .. } => &[Pending::RegimeLabels],
            Self::ChangeSizing { .. } => &[Pending::SimulatorCapability],
            Self::Simplify => &[Pending::AblationTrials],
            Self::Combine { .. } => &[Pending::GraphValidator],
        }
    }

    /// Whether this move can introduce or change a parameter.
    ///
    /// Two moves cannot: `remove_node` takes something away, and `simplify`
    /// decides by ablation which thing to take away. Ranges attached to either
    /// would be ranges over a parameter that is about to stop existing — which
    /// is not a harmless extra field, because the sweep would search it.
    #[must_use]
    pub fn may_carry_ranges(&self) -> bool {
        !matches!(self, Self::RemoveNode { .. } | Self::Simplify)
    }
}

/// Why a proposal was refused.
///
/// Every variant names the field, because this error is handed back to the model
/// as the one retry §8.2 allows, and "invalid" with no location is a retry with
/// no information.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProposalError {
    #[error(
        "the hypothesis is empty: a move with no stated claim cannot turn out wrong, and the \
         hypothesis ledger would record a generation nothing later could learn from"
    )]
    NoHypothesis,
    #[error(
        "`{0}` takes something away, so it cannot carry parameter ranges: the sweep would search \
         a parameter that is about to stop existing"
    )]
    RangesOnRemoval(&'static str),
    #[error(
        "two ranges are named `{0}`; a parameter has one range or the sweep has two answers for it"
    )]
    DuplicateRange(String),
    #[error("range `{name}` runs backwards ({detail})")]
    BackwardsRange { name: String, detail: String },
    #[error("range `{0}` offers no choices")]
    EmptyChoices(String),
    #[error("range `{name}` has a step of {step}; a step of zero or less enumerates nothing")]
    BadStep { name: String, step: i64 },
    #[error("field `{field}` is empty: {why}")]
    EmptyField {
        field: &'static str,
        why: &'static str,
    },
}

/// One generation's answer.
///
/// `serde` has no `default` on the hypothesis or the move, so a reply missing
/// either fails at the boundary rather than arriving half-formed and being
/// completed by whatever this code happened to assume.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(flatten)]
    pub r#move: Move,
    /// What the proposer thinks this move will do, stated so it could be wrong.
    pub hypothesis: String,
    /// Ranges for any new or changed parameter. Empty when the move introduces
    /// none.
    pub ranges: Vec<Dim>,
}

/// What [`Proposal::validate`] could not check here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unchecked {
    pub pending: Vec<Pending>,
}

impl Proposal {
    /// The structural checks, and the list of checks still owed.
    ///
    /// # Errors
    /// [`ProposalError`] naming the field, so the single permitted retry has
    /// something to act on.
    pub fn validate(&self) -> Result<Unchecked, ProposalError> {
        if self.hypothesis.trim().is_empty() {
            return Err(ProposalError::NoHypothesis);
        }
        self.check_move_fields()?;

        if !self.ranges.is_empty() && !self.r#move.may_carry_ranges() {
            return Err(ProposalError::RangesOnRemoval(self.r#move.kind()));
        }

        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        for dim in &self.ranges {
            if seen.insert(dim.name.as_str(), ()).is_some() {
                return Err(ProposalError::DuplicateRange(dim.name.clone()));
            }
            check_dim(dim)?;
        }

        Ok(Unchecked {
            pending: self.r#move.machine_check().to_vec(),
        })
    }

    fn check_move_fields(&self) -> Result<(), ProposalError> {
        let need = |s: &str, field: &'static str, why: &'static str| {
            if s.trim().is_empty() {
                Err(ProposalError::EmptyField { field, why })
            } else {
                Ok(())
            }
        };
        match &self.r#move {
            Move::AddConditionFilter { node, expr } => {
                need(node, "node", "which Signal's condition the predicate joins")?;
                need(expr, "expr", "the predicate itself")
            }
            Move::RemoveNode { node } => need(node, "node", "which node to drop"),
            Move::SwapFeature { node, from, to } => {
                need(node, "node", "which Condition reads the feature")?;
                need(from, "from", "the feature being replaced")?;
                need(to, "to", "the feature replacing it")
            }
            Move::AddRegimeGate { regime } => need(
                regime,
                "regime",
                "the label the regime engine must recognise",
            ),
            Move::Combine { a, b, .. } => {
                need(a, "a", "the candidate entries come from")?;
                need(b, "b", "the candidate exits and filters come from")
            }
            Move::ChangeExit { .. } | Move::ChangeSizing { .. } | Move::Simplify => Ok(()),
        }
    }

    /// The JSON schema a constrained decoder fills.
    ///
    /// Emitted from beside the type the reply deserialises into, and asserted
    /// against it in the tests below: a model that satisfies the grammar
    /// produces a value this crate accepts, and the vocabulary offered is
    /// [`Move::KINDS`] rather than a second copy of the list.
    #[must_use]
    pub fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "move": { "type": "string", "enum": Move::KINDS },
                "node": { "type": "string" },
                "expr": { "type": "string" },
                "from": { "type": "string" },
                "to": { "type": "string" },
                "mode": { "type": "string" },
                "regime": { "type": "string" },
                "params": { "type": "object" },
                "a": { "type": "string" },
                "b": { "type": "string" },
                "how": { "type": "string", "enum": ["entries_from_a"] },
                "hypothesis": {
                    "type": "string",
                    "description": "What this move will do, stated so it could be wrong."
                },
                "ranges": {
                    "type": "array",
                    "description": "Ranges for any new or changed parameter. Empty when the move introduces none.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            // Nested, not flattened. `Dim` holds its `DimKind` in a
                            // field called `kind`, and that enum is itself tagged
                            // `kind` — so the wire shape is an object inside an
                            // object with the same key. A grammar that described
                            // the flat shape would produce replies this crate
                            // refuses every time, which is a proposer failed for
                            // doing exactly what it was asked.
                            "kind": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string", "enum": ["int", "float", "enum"] },
                                    "min": { "type": "number" },
                                    "max": { "type": "number" },
                                    "step": { "type": "integer" },
                                    "log": { "type": "boolean" },
                                    "choices": { "type": "array", "items": { "type": "string" } }
                                },
                                "required": ["kind"]
                            }
                        },
                        "required": ["name", "kind"]
                    }
                }
            },
            "required": ["move", "hypothesis", "ranges"]
        })
    }
}

fn check_dim(dim: &Dim) -> Result<(), ProposalError> {
    match &dim.kind {
        DimKind::Int { min, max, step } => {
            if min > max {
                return Err(ProposalError::BackwardsRange {
                    name: dim.name.clone(),
                    detail: format!("min {min} is above max {max}"),
                });
            }
            if *step <= 0 {
                return Err(ProposalError::BadStep {
                    name: dim.name.clone(),
                    step: *step,
                });
            }
            Ok(())
        }
        DimKind::Float { min, max, log } => {
            if !min.is_finite() || !max.is_finite() {
                return Err(ProposalError::BackwardsRange {
                    name: dim.name.clone(),
                    detail: "a bound is not finite".to_string(),
                });
            }
            if min > max {
                return Err(ProposalError::BackwardsRange {
                    name: dim.name.clone(),
                    detail: format!("min {min} is above max {max}"),
                });
            }
            // A log scale through or at zero has no logarithm. Refused here
            // rather than producing NaN coordinates the sampler would search.
            if *log && *min <= 0.0 {
                return Err(ProposalError::BackwardsRange {
                    name: dim.name.clone(),
                    detail: "a log-scaled range must start above zero".to_string(),
                });
            }
            Ok(())
        }
        DimKind::Enum { choices } => {
            if choices.is_empty() {
                return Err(ProposalError::EmptyChoices(dim.name.clone()));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim_int(name: &str) -> Dim {
        Dim {
            name: name.to_string(),
            kind: DimKind::Int {
                min: 5,
                max: 50,
                step: 1,
            },
        }
    }

    fn every_variant() -> Vec<Move> {
        vec![
            Move::AddConditionFilter {
                node: "signal_1".into(),
                expr: "atr_14 > 0.01".into(),
            },
            Move::RemoveNode {
                node: "filter_2".into(),
            },
            Move::SwapFeature {
                node: "cond_1".into(),
                from: "ema_20".into(),
                to: "ema_50".into(),
            },
            Move::ChangeExit {
                mode: ExitMode::Trail,
            },
            Move::AddRegimeGate {
                regime: "trending".into(),
            },
            Move::ChangeSizing {
                mode: SizingMode::VolScaled,
                params: BTreeMap::new(),
            },
            Move::Simplify,
            Move::Combine {
                a: "cand_a".into(),
                b: "cand_b".into(),
                how: CombineHow::EntriesFromA,
            },
        ]
    }

    fn proposal(m: Move) -> Proposal {
        Proposal {
            r#move: m,
            hypothesis: "a volatility filter removes the losing trades in chop".to_string(),
            ranges: vec![],
        }
    }

    #[test]
    fn a_proposal_round_trips_through_the_wire_shape() {
        let p = Proposal {
            r#move: Move::SwapFeature {
                node: "cond_1".into(),
                from: "ema_20".into(),
                to: "ema_50".into(),
            },
            hypothesis: "the slower average survives the whipsaws".into(),
            ranges: vec![dim_int("lookback")],
        };
        let wire = serde_json::to_value(&p).expect("serialise");
        assert_eq!(
            wire["move"], "swap_feature",
            "the tag is flattened alongside the fields"
        );
        let back: Proposal = serde_json::from_value(wire).expect("deserialise");
        assert_eq!(back, p);
    }

    /// The field the whole object exists for has no default, so a reply that
    /// omits it fails at the boundary instead of arriving with an empty claim.
    #[test]
    fn a_reply_without_a_hypothesis_does_not_deserialise() {
        let wire = json!({ "move": "simplify", "ranges": [] });
        let err = serde_json::from_value::<Proposal>(wire)
            .unwrap_err()
            .to_string();
        assert!(err.contains("hypothesis"), "{err}");
    }

    /// An unknown move is refused rather than mapped to the nearest one. The
    /// vocabulary is v1 of a spec, not a suggestion.
    #[test]
    fn an_invented_move_does_not_deserialise() {
        let wire = json!({
            "move": "rewrite_everything",
            "hypothesis": "trust me",
            "ranges": []
        });
        assert!(serde_json::from_value::<Proposal>(wire).is_err());
    }

    #[test]
    fn an_empty_hypothesis_is_refused() {
        let mut p = proposal(Move::Simplify);
        p.hypothesis = "   ".into();
        assert_eq!(p.validate(), Err(ProposalError::NoHypothesis));
    }

    /// `simplify` and `remove_node` take something away. Ranges on either would
    /// be searched, and what they range over is about to stop existing.
    #[test]
    fn a_removal_cannot_carry_ranges() {
        for m in [
            Move::Simplify,
            Move::RemoveNode {
                node: "filter_2".into(),
            },
        ] {
            let kind = m.kind();
            let mut p = proposal(m);
            p.ranges = vec![dim_int("threshold")];
            assert_eq!(p.validate(), Err(ProposalError::RangesOnRemoval(kind)));
        }
    }

    #[test]
    fn two_ranges_for_one_parameter_are_refused() {
        let mut p = proposal(Move::ChangeExit {
            mode: ExitMode::Trail,
        });
        p.ranges = vec![dim_int("stop_atr"), dim_int("stop_atr")];
        assert_eq!(
            p.validate(),
            Err(ProposalError::DuplicateRange("stop_atr".into()))
        );
    }

    #[test]
    fn a_backwards_or_stepless_range_is_refused() {
        let mut p = proposal(Move::ChangeExit {
            mode: ExitMode::Fixed,
        });
        p.ranges = vec![Dim {
            name: "bars".into(),
            kind: DimKind::Int {
                min: 9,
                max: 2,
                step: 1,
            },
        }];
        assert!(matches!(
            p.validate(),
            Err(ProposalError::BackwardsRange { .. })
        ));

        p.ranges = vec![Dim {
            name: "bars".into(),
            kind: DimKind::Int {
                min: 2,
                max: 9,
                step: 0,
            },
        }];
        assert_eq!(
            p.validate(),
            Err(ProposalError::BadStep {
                name: "bars".into(),
                step: 0
            })
        );
    }

    /// A log-scaled range that starts at or below zero has no logarithm. The
    /// sampler would encode it as NaN and search coordinates that mean nothing.
    #[test]
    fn a_log_range_through_zero_is_refused() {
        let mut p = proposal(Move::ChangeSizing {
            mode: SizingMode::VolScaled,
            params: BTreeMap::new(),
        });
        p.ranges = vec![Dim {
            name: "risk_frac".into(),
            kind: DimKind::Float {
                min: 0.0,
                max: 0.05,
                log: true,
            },
        }];
        assert!(matches!(
            p.validate(),
            Err(ProposalError::BackwardsRange { .. })
        ));
    }

    #[test]
    fn a_move_with_an_empty_required_field_is_refused() {
        let p = proposal(Move::AddRegimeGate {
            regime: "  ".into(),
        });
        assert_eq!(
            p.validate(),
            Err(ProposalError::EmptyField {
                field: "regime",
                why: "the label the regime engine must recognise"
            })
        );
    }

    /// Every move names the checks it still owes, and validating never returns
    /// an empty list — which is the point: "validated" here never means "fully
    /// checked", and the caller is handed the difference rather than left to
    /// assume it away.
    #[test]
    fn every_move_declares_a_check_this_crate_cannot_perform() {
        for m in every_variant() {
            let kind = m.kind();
            let unchecked = proposal(m).validate().expect("structurally valid");
            assert!(
                !unchecked.pending.is_empty(),
                "{kind} claims to be fully checked here, and none of them are"
            );
        }
    }

    /// The vocabulary list and the variants are the same list, in the same
    /// order. `KINDS` is read by the grammar and by the catalogue, so a variant
    /// missing from it would be a move no model is ever offered.
    #[test]
    fn the_kind_list_is_the_variant_list() {
        let named: Vec<&str> = every_variant().iter().map(Move::kind).collect();
        assert_eq!(named, Move::KINDS);
    }

    /// The grammar offers exactly the vocabulary, and demands the two fields a
    /// proposal is worthless without.
    #[test]
    fn the_schema_offers_the_vocabulary_and_demands_the_claim() {
        let s = Proposal::schema();
        let enumerated: Vec<&str> = s["properties"]["move"]["enum"]
            .as_array()
            .expect("an enum")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(enumerated, Move::KINDS);
        let required: Vec<&str> = s["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(required.contains(&"hypothesis"));
        assert!(required.contains(&"move"));
    }

    /// Every field the grammar offers is one the type can actually receive.
    ///
    /// A schema property with no home would be a field a constrained decoder is
    /// invited to fill and `serde` then rejects the whole reply over — the model
    /// would be failed for doing exactly what it was asked.
    #[test]
    fn every_schema_property_is_a_field_some_variant_accepts() {
        let s = Proposal::schema();
        let offered: Vec<String> = s["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .cloned()
            .collect();
        let mut accepted: BTreeMap<String, ()> = BTreeMap::new();
        accepted.insert("move".into(), ());
        accepted.insert("hypothesis".into(), ());
        accepted.insert("ranges".into(), ());
        for m in every_variant() {
            let wire = serde_json::to_value(proposal(m)).expect("serialise");
            for k in wire.as_object().expect("an object").keys() {
                accepted.insert(k.clone(), ());
            }
        }
        for field in offered {
            assert!(
                accepted.contains_key(&field),
                "the grammar offers `{field}`, which no variant of Move accepts"
            );
        }
    }

    /// The `ranges` grammar describes the shape `Dim` actually deserialises.
    ///
    /// This is the test that caught the first version. `Dim` holds its `DimKind`
    /// in a field named `kind`, and `DimKind` is itself internally tagged on
    /// `kind` — so the wire shape nests an object under that key. The schema
    /// described the flat shape, which reads perfectly well and produces a reply
    /// `serde` rejects every single time: the proposer would have been failed,
    /// twice, for following the grammar it was given.
    #[test]
    fn a_range_built_to_the_grammar_is_a_range_this_crate_accepts() {
        let items = &Proposal::schema()["properties"]["ranges"]["items"];

        // Build a value using only what the schema offers, then deserialise it.
        for built in [
            json!({ "name": "lookback", "kind": { "kind": "int", "min": 5, "max": 50, "step": 1 } }),
            json!({ "name": "risk", "kind": { "kind": "float", "min": 0.001, "max": 0.05, "log": true } }),
            json!({ "name": "mode", "kind": { "kind": "enum", "choices": ["a", "b"] } }),
        ] {
            let dim: Dim = serde_json::from_value(built.clone()).unwrap_or_else(|e| {
                panic!("the grammar's shape is not Dim's shape: {e} for {built}")
            });
            // And the reverse: what `Dim` emits uses only keys the schema names.
            let wire = serde_json::to_value(&dim).expect("serialise");
            let offered = items["properties"].as_object().expect("properties");
            for key in wire.as_object().expect("an object").keys() {
                assert!(
                    offered.contains_key(key),
                    "`Dim` emits `{key}`, which the grammar never mentions"
                );
            }
            let inner_offered = offered["kind"]["properties"]
                .as_object()
                .expect("the nested kind object");
            for key in wire["kind"].as_object().expect("nested").keys() {
                assert!(
                    inner_offered.contains_key(key),
                    "`DimKind` emits `{key}`, which the grammar never mentions"
                );
            }
        }
    }

    /// And a whole proposal built to the grammar deserialises, ranges included.
    #[test]
    fn a_proposal_built_to_the_grammar_deserialises() {
        let wire = json!({
            "move": "add_condition_filter",
            "node": "signal_1",
            "expr": "atr_14 > 0.01",
            "hypothesis": "a volatility floor removes the chop trades",
            "ranges": [
                { "name": "atr_floor", "kind": { "kind": "float", "min": 0.001, "max": 0.05, "log": false } }
            ]
        });
        let p: Proposal = serde_json::from_value(wire).expect("the grammar's shape parses");
        p.validate().expect("and validates");
    }
}
