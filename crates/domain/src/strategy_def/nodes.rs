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

/// A node in the strategy computation graph.
///
/// The validator (Phase 5) parses `Node::kind` and rejects unknown types.
/// This is the **fail-closed** rule: an unknown `type` field → validation error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Unique stable node ID within this definition (e.g. `"n1"`, `"n2"`).
    pub id: String,
    /// Node type discriminant.
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// Discriminated node type.  Unknown variants are rejected by the validator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}
