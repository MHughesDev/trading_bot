//! Strategy node types and expression language (frozen at v1.0).
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

use serde::{Deserialize, Serialize};

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
        #[serde(default = "default_production_alias", skip_serializing_if = "is_production")]
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
        assert!(!s.contains("target_kind"), "default target_kind omitted: {s}");
        assert!(!s.contains("alias"), "default alias omitted: {s}");
        assert!(!s.contains("input"), "absent input omitted: {s}");
    }
}
