//! Strategy node types and expression language (v1.0 frozen; v1.1 additive AI nodes).
//!
//! # Expression grammar (v1.0, frozen)
//!
//! The expression language is deliberately minimal and explicitly defined.
//! It is a pure predicate language — no side effects, no I/O.  The
//! `strategy-validator` crate (Phase 5) parses and validates expression strings
//! against this grammar.  Here we define the AST and the raw string type used
//! in definitions.
//!
//! ## Grammar (EBNF sketch)
//!
//! ```text
//! expr        = comparison
//! comparison  = term ( ( ">" | "<" | ">=" | "<=" | "==" | "!=" ) term )?
//! term        = factor ( ( "+" | "-" ) factor )*
//! factor      = unary ( ( "*" | "/" ) unary )*
//! unary       = "-" unary | primary
//! primary     = number | feature_ref | bar_ref | "(" expr ")"
//! feature_ref = "feature" "(" "'" ident "'" ")"
//! bar_ref     = "bar" "(" "'" field "'" ")"
//! number      = [0-9]+ ( "." [0-9]+ )?   (* decimal literal, never float *)
//! ident       = [a-zA-Z_][a-zA-Z0-9_]*
//! field       = "open" | "high" | "low" | "close" | "volume"
//! ```
//!
//! ## World context functions available in expressions
//!
//! - `feature('name')` — value of a named feature from the `features.technical` lane.
//! - `bar('field')` — field of the most recent bar from `market.bars.1m`.
//!
//! Unknown functions or variable names → parse error in `strategy-validator`.
//! The validator fails closed: unknown node types are rejected, not silently ignored.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── v1.1 AI-node helpers ──────────────────────────────────────────────────────

/// Binds one field of an inference output to a named feature slot.
///
/// After each inference cycle the runtime writes `value` → `feature_slots[as_]`
/// so that downstream `Condition`, `Filter`, `Rank`, and `Sizing` nodes can
/// reference the field with the existing `feature('name')` grammar — no grammar
/// extension is needed.
///
/// **Valid `field` values** (case-sensitive):
/// `"confidence"`, `"direction"`, `"median_return"`, `"sigma"`,
/// `"q05"` / `"q10"` / `"q25"` / `"q50"` / `"q75"` / `"q90"` / `"q95"`,
/// `"var_95"`, `"var_99"`, `"es_95"`, `"skew"`, `"spread_90"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputBinding {
    /// Which field of the `InferenceOutput` to publish.
    pub field: String,
    /// Name under which the value appears in the `feature('…')` namespace.
    #[serde(rename = "as")]
    pub as_: String,
}

/// What the runtime does when an inference call abstains (sidecar down,
/// circuit-break open, or no result).
///
/// - `Flat` (default): write `NaN` to all bound output slots.  Any condition
///   referencing a `NaN` slot evaluates to `false` — the safest choice for
///   live trading (never accidentally enters a trade on a missing signal).
/// - `HoldLast`: leave the slots unchanged, so they retain the most recent
///   successful inference value.  Useful for models that run on slow cadences.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstainPolicy {
    #[default]
    Flat,
    HoldLast,
}

/// Optional boolean condition on an `Inference` node.
///
/// When present, the node can be used directly in `Signal.when` (like
/// `ModelForecast`).  When absent, the node only publishes output slots and
/// must be combined with an explicit `Condition` node that references the
/// bound feature names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchSpec {
    /// `"bullish"` | `"bearish"` | `"flat"` | `"any"`.
    pub direction: String,
    /// Minimum confidence to fire, 0.0–1.0.
    pub min_confidence: f64,
}

/// An inline ensemble: combines the outputs of multiple member models without
/// requiring a pre-registered ensemble in the model registry.
///
/// The gateway runs each member and combines them using `combiner`.
/// This is suitable for quick A/B experiments; for production ensembles with
/// calibration history, use `target_kind = "ensemble"` and a registry ensemble.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InlineEnsemble {
    /// Member model refs (ID or slug) to combine.
    pub roster: Vec<InlineRosterMember>,
    /// `"linear_opinion_pool"` (default) | `"crps_weighted"`.
    #[serde(default = "default_linear_pool")]
    pub combiner: String,
    /// Minimum weight floor per member.  `weight_floor * roster.len() ≤ 1.0`.
    #[serde(default = "default_weight_floor")]
    pub weight_floor: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InlineRosterMember {
    pub model_ref: String,
    #[serde(default = "default_production_alias")]
    pub alias: String,
}

fn default_linear_pool() -> String {
    "linear_opinion_pool".to_string()
}

fn default_weight_floor() -> f64 {
    0.05
}

fn is_flat_policy(p: &AbstainPolicy) -> bool {
    *p == AbstainPolicy::Flat
}

/// LLM call configuration for an `LlmInference` node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmCall {
    /// Prompt template.  Supports `{{feature('name')}}` interpolation so
    /// live feature values can be injected at evaluation time.
    pub prompt: String,
    /// Arbitrary JSON params forwarded to the sidecar (temperature, max_tokens, …).
    #[serde(default)]
    pub params: serde_json::Value,
    /// How to extract an `f64` or `bool` from the LLM text response.
    pub parse: LlmParseMode,
}

/// How the LLM text response is mapped to a usable value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LlmParseMode {
    /// Boolean: true if `keyword` appears in the response (case-insensitive).
    Bool { keyword: String },
    /// Extract a single float: look for `field` key in a JSON response or regex capture.
    F64 { field: String, as_: String },
    /// Extract multiple floats from a JSON object response.
    Json { outputs: Vec<OutputBinding> },
}

fn default_production_alias() -> String {
    "production".to_string()
}

fn is_production(s: &str) -> bool {
    s == "production"
}

fn default_target_kind() -> String {
    "model".to_string()
}

fn is_model(s: &str) -> bool {
    s == "model"
}

/// Input-data contract for an AI inference node — declares *what* data and *how
/// much* of it the target receives on every evaluation (live, scanner, or
/// backtest). The strategy builder fills this from the target's declared
/// requirement so a target only runs on a window it supports.
///
/// Phase 1 carries this as part of the canonical schema; the strategy runtime
/// begins consuming it (assembling the feature window for inference) in Phase 2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInput {
    /// Named feature set the target consumes (the model's `feature_set_ref`).
    /// `None` = the target's default feature set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature_set: Option<String>,
    /// Timeframe of the bars/features fed in (e.g. `"1m"`, `"5m"`, `"1h"`).
    pub timeframe: String,
    /// Lookback window — number of bars of history provided on each run.
    pub lookback: u32,
}

/// A node in the strategy computation graph.
///
/// The validator (Phase 5) parses `Node::kind` and rejects unknown types.
/// This is the **fail-closed** rule: an unknown `type` field → validation error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Unique stable node ID within this definition (e.g. `"n1"`, `"n2"`).
    pub id: String,
    /// Node type discriminant.
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// Discriminated node type.  Unknown variants are rejected by the validator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// Evaluates a predicate expression over the current `WorldContext`.
    /// Returns a boolean; referenced by `signal` nodes.
    Condition {
        /// Expression string conforming to the grammar above.
        expr: String,
    },
    /// Emits a named signal when a referenced condition is true.
    Signal {
        /// ID of the condition node to watch.
        when: String,
        /// Named signal consumed by `actions` (e.g. `"long"`, `"exit"`).
        emit: String,
    },
    // ── v1.5 universe/pipeline nodes ─────────────────────────────────────────
    /// Seeds the universe pipeline with the provided instrument set.
    DataSource {
        /// DataType key for the lane this source consumes (e.g. `"market.ohlcv"`).
        data_type: String,
    },
    /// Ranks universe entries by a named feature value.
    Rank {
        /// ID of the upstream node supplying the universe.
        input: String,
        /// Feature name to rank by.
        feature: String,
        /// `true` = ascending (lowest first), `false` = descending.
        ascending: bool,
    },
    /// Filters universe entries using a predicate expression over feature values.
    Filter {
        /// ID of the upstream node supplying the universe.
        input: String,
        /// Predicate expression (same grammar as `Condition`).
        expr: String,
    },
    /// Keeps only the top N entries from the upstream universe.
    TakeTopN {
        /// ID of the upstream node supplying the universe.
        input: String,
        /// Maximum number of entries to keep.
        n: usize,
    },
    /// Terminal node — marks the instruments to surface in scanner results.
    SurfaceAction {
        /// ID of the upstream node supplying the final universe.
        input: String,
    },
    /// v1.1: AI inference condition (model, ensemble, or pipeline).
    ///
    /// `model_ref` + `target_kind` identify the inference target; all three
    /// target kinds resolve to a single forecast through the inference gateway,
    /// so the strategy schema is agnostic to which kind it is. The condition is
    /// true when the resolved forecast's direction matches `direction` and its
    /// confidence ≥ `min_confidence`. Returns false when inference abstains
    /// (sidecar down, circuit-break open, or target not found).
    ModelForecast {
        /// Inference target reference — a model, ensemble, or pipeline ID/slug
        /// (see `target_kind`). Named `model_ref` for backward compatibility.
        model_ref: String,
        /// Target kind: `"model"` (default) | `"ensemble"` | `"pipeline"`.
        #[serde(default = "default_target_kind", skip_serializing_if = "is_model")]
        target_kind: String,
        /// Alias to resolve (default: "production").
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        /// Expected forecast direction: "bullish" | "bearish" | "any".
        direction: String,
        /// Minimum confidence threshold (0.0–1.0).
        min_confidence: f64,
        /// Input-data contract: what data and how much the target receives.
        /// `None` = let the target use its declared default window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<ModelInput>,
    },

    // ── v1.1 AI blocks (requires definition_version "1.1") ───────────────────
    /// Value-producing AI inference node.
    ///
    /// Calls a `Forecaster` (or an `ensemble`/`pipeline`) and publishes named
    /// output fields into the feature slot namespace so downstream `Condition`,
    /// `Filter`, `Rank`, and `Sizing` nodes can reference them with the
    /// standard `feature('name')` grammar.
    ///
    /// Optionally also evaluates as a boolean condition (via `condition`) so
    /// it can be used directly in `Signal.when` without a separate `Condition`
    /// node.
    Inference {
        /// Forecaster model reference (ID or slug).
        model_ref: String,
        /// `"model"` (default) | `"ensemble"` | `"pipeline"`.
        #[serde(default = "default_target_kind", skip_serializing_if = "is_model")]
        target_kind: String,
        /// Version alias to resolve (default: `"production"`).
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        /// Input-data contract.  `None` = use the model's declared default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<ModelInput>,
        /// Fields to publish into the feature slot namespace.  Each binding
        /// maps one output field to a feature name the strategy graph can read.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        outputs: Vec<OutputBinding>,
        /// What to do when inference abstains (default: `flat` — write NaN).
        #[serde(default, skip_serializing_if = "is_flat_policy")]
        abstain: AbstainPolicy,
        /// Optional inline ensemble of member models, evaluated client-side
        /// without requiring a registry ensemble.  Mutually exclusive with
        /// `target_kind = "ensemble"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ensemble: Option<InlineEnsemble>,
        /// Optional boolean condition.  When `Some`, the node can be used in
        /// `Signal.when` directly.  When `None`, it only publishes slots.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<MatchSpec>,
    },

    /// AI-driven position sizing node (requires a `RiskSizing` model).
    ///
    /// The model returns a `size_fraction` (decimal string) which `PlaceOrder`
    /// actions reference via `size_mode: model, node_ref: "<this node's id>"`.
    /// The fraction is converted to `Decimal` at the action boundary (ADR-0002).
    Sizing {
        /// `RiskSizing` model reference.
        model_ref: String,
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<ModelInput>,
        /// Clamp the returned fraction to `[min, max]`.
        /// Values are decimal strings (ADR-0002).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clamp: Option<[String; 2]>,
        /// Fallback fixed size (decimal string) when the model abstains.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<String>,
        #[serde(default, skip_serializing_if = "is_flat_policy")]
        abstain: AbstainPolicy,
    },

    /// AI decision node (requires a `TradeDecision` model).
    ///
    /// Maps the model's predicted action class to a named signal via `class_map`.
    /// Acts like a `Signal` node — fires when the model's class matches a key in
    /// `class_map` and confidence ≥ `min_confidence`.
    Decision {
        /// `TradeDecision` model reference.
        model_ref: String,
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<ModelInput>,
        /// Maps predicted class name → emitted signal name.
        /// E.g. `{"long": "enter_long", "short": "enter_short", "flat": "exit"}`.
        class_map: HashMap<String, String>,
        /// Minimum confidence to emit a signal (0.0–1.0).
        #[serde(default)]
        min_confidence: f64,
        #[serde(default, skip_serializing_if = "is_flat_policy")]
        abstain: AbstainPolicy,
    },

    /// Universe-pipeline ranking by `SignalRanker` model score.
    ///
    /// Replaces / augments `Rank` when the ranking criterion comes from an AI
    /// model rather than a named feature.  The model's confidence score is
    /// used as the rank key (descending by default).
    ModelRank {
        /// ID of the upstream universe node.
        input: String,
        /// `SignalRanker` model reference.
        model_ref: String,
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        /// `true` = ascending (lowest score first).
        #[serde(default)]
        ascending: bool,
    },

    /// LLM / external-adapter inference node.
    ///
    /// Calls an `ExternalLlmAdapter` model, renders the prompt with live
    /// feature values, and parses the response into the feature namespace
    /// and/or a boolean condition.
    LlmInference {
        /// `ExternalLlmAdapter` model reference.
        model_ref: String,
        #[serde(
            default = "default_production_alias",
            skip_serializing_if = "is_production"
        )]
        alias: String,
        /// The call specification: prompt template, model params, and parse mode.
        call: LlmCall,
        /// Cache LLM responses for this many seconds (default 0 = no cache).
        #[serde(default)]
        cache_ttl_s: u32,
        /// Maximum per-call cost in USD (decimal string).  The node abstains
        /// rather than exceeding this limit.  `None` = unlimited.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_cost_usd: Option<String>,
        #[serde(default, skip_serializing_if = "is_flat_policy")]
        abstain: AbstainPolicy,
        /// Optional boolean condition (same semantics as `Inference.condition`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<MatchSpec>,
    },

    /// Derives a new feature from two or more bound `Inference` output slots.
    ///
    /// Useful for in-graph aggregation (e.g. average of two models' `median_return`)
    /// without pre-registering a registry ensemble.
    Combine {
        /// Feature names (bound via `Inference.outputs`) to combine.
        inputs: Vec<String>,
        /// Combination operation.
        op: CombineOp,
        /// Name under which the combined value is written to the feature namespace.
        #[serde(rename = "as")]
        as_: String,
    },
}

/// How a `Combine` node aggregates its input slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CombineOp {
    /// Weighted average.  `weights` must have the same length as `inputs`.
    WeightedAverage { weights: Vec<f64> },
    /// Element-wise maximum.
    Max,
    /// Element-wise minimum.
    Min,
    /// Sum of all inputs.
    Sum,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimal AI node the builder emits for a single model defaults
    /// `target_kind` to "model" and carries no `input` when omitted.
    #[test]
    fn model_forecast_minimal_defaults() {
        let json = r#"{
            "id": "mf1",
            "type": "model_forecast",
            "model_ref": "price_forecaster",
            "direction": "bullish",
            "min_confidence": 0.6
        }"#;
        let node: Node = serde_json::from_str(json).unwrap();
        match node.kind {
            NodeKind::ModelForecast {
                model_ref,
                target_kind,
                alias,
                direction,
                min_confidence,
                input,
            } => {
                assert_eq!(model_ref, "price_forecaster");
                assert_eq!(target_kind, "model");
                assert_eq!(alias, "production");
                assert_eq!(direction, "bullish");
                assert_eq!(min_confidence, 0.6);
                assert!(input.is_none());
            }
            other => panic!("expected ModelForecast, got {other:?}"),
        }
    }

    /// The full AI node the builder emits for an ensemble with an input window
    /// deserializes every field, and re-serializes without the default
    /// `target_kind`/`alias` keys (skip_serializing_if).
    #[test]
    fn model_forecast_full_roundtrip() {
        let json = r#"{
            "id": "mf1",
            "type": "model_forecast",
            "model_ref": "vol_ensemble",
            "target_kind": "ensemble",
            "alias": "candidate",
            "direction": "any",
            "min_confidence": 0.7,
            "input": { "feature_set": "fs_core_v3", "timeframe": "5m", "lookback": 256 }
        }"#;
        let node: Node = serde_json::from_str(json).unwrap();
        match &node.kind {
            NodeKind::ModelForecast {
                target_kind, input, ..
            } => {
                assert_eq!(target_kind, "ensemble");
                let input = input.as_ref().expect("input present");
                assert_eq!(input.feature_set.as_deref(), Some("fs_core_v3"));
                assert_eq!(input.timeframe, "5m");
                assert_eq!(input.lookback, 256);
            }
            other => panic!("expected ModelForecast, got {other:?}"),
        }

        // Round-trips: a model-kind node with production alias omits both keys.
        let model_node = Node {
            id: "mf2".into(),
            kind: NodeKind::ModelForecast {
                model_ref: "m".into(),
                target_kind: "model".into(),
                alias: "production".into(),
                direction: "bearish".into(),
                min_confidence: 0.5,
                input: None,
            },
        };
        let s = serde_json::to_string(&model_node).unwrap();
        assert!(
            !s.contains("target_kind"),
            "default target_kind omitted: {s}"
        );
        assert!(!s.contains("alias"), "default alias omitted: {s}");
        assert!(!s.contains("input"), "absent input omitted: {s}");
    }
}
