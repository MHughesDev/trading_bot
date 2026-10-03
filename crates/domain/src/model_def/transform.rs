//! A `transform` model's code (Set U U-5.7; ADR-0040 decision 5).
//!
//! Anything the strategy operator library cannot express is a model: a custom
//! function registered with a declared contract and no training, called exactly
//! like a trained model. The inference sidecar runs it per row with the row's
//! named inputs and expects a dict of the contract's outputs back.
//!
//! Its trust is that of every model artifact the sidecar loads — owner-
//! registered code. The sidecar runs it with restricted builtins and no
//! imports, which stops accidents, not an adversary.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSpec {
    /// Only `python` today.
    pub language: String,
    /// The function the sidecar calls: `fn(inputs: dict) -> dict`.
    pub entrypoint: String,
    /// Its source.
    pub source: String,
}
