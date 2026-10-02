//! What every engine step leaves behind (ADR-0047, U-1.9).
//!
//! A trace names the decision instant, every node's value, every model call and
//! its answer, every rule's outcome, the resolved intents, the planned orders and
//! the refusals. It is serialised with ordered maps only, so two runs over the
//! same inputs produce **byte-identical** traces — which is what lets the nightly
//! parity job compare a paper session against its backtest replay byte for byte.

use std::collections::BTreeMap;

use domain::strategy_graph::Value;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::decide::RuleOutcome;
use crate::models::{ModelRequest, ModelResponse};
use crate::planner::{OrderAction, Refusal, ResolvedIntent};

/// What caused this step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cause {
    /// A bar of the decision timeframe closed.
    BarClose,
    /// One of this strategy's orders filled.
    Fill { tag: String },
    /// One of this strategy's orders was cancelled, rejected or expired.
    OrderClosed { tag: String },
    /// A scheduled trigger.
    Schedule,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub at_ns: i64,
    /// Decision ticks since the engine started (warm-up included).
    pub tick: u64,
    pub cause: Cause,
    /// Every node's value this tick, by node id.
    pub values: BTreeMap<String, Value>,
    pub model_requests: Vec<ModelRequest>,
    pub model_responses: Vec<ModelResponse>,
    /// Rule index → what it did.
    pub rules: BTreeMap<usize, RuleOutcome>,
    /// (rule index, intent) for every rule that fired.
    pub intents: Vec<(usize, ResolvedIntent)>,
    pub actions: Vec<OrderAction>,
    pub refusals: Vec<Refusal>,
}

impl DecisionTrace {
    /// SHA-256 of the canonical serialisation — the value the parity job
    /// compares, and what a journal stores when it cannot keep the whole trace.
    #[must_use]
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(&bytes))
    }
}
