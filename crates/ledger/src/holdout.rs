//! The sealed holdout (SPEC §12.7, AT-33): one evaluation per strategy lineage,
//! ever, enforced by the ledger rather than by any caller.
//!
//! A request first **claims** the lineage — the claim is logged before any holdout
//! data is read, and a database index lets exactly one first claim exist. A later
//! request is logged too and handed the first result instead of a new evaluation.
//! A claim whose evaluation never recorded a result leaves the lineage spent.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

/// The ledger's answer to a sealed-holdout request.
#[derive(Clone, Debug, PartialEq)]
pub enum HoldoutClaim {
    /// This request is the lineage's one evaluation; it is claimed and logged.
    First,
    /// The lineage was already evaluated. This is that result; the request is logged.
    Repeat { first_trial: Uuid, called_at: DateTime<Utc>, result: Value },
}

#[derive(Debug, Default)]
pub(crate) struct MemHoldout {
    pub claimed: bool,
    pub call: Option<(Uuid, DateTime<Utc>, Value)>,
    pub attempts: usize,
}

/// The refusal for a lineage claimed without a recorded result.
pub(crate) fn claimed_without_result(lineage: &str) -> String {
    format!("the sealed holdout for lineage '{lineage}' has already been claimed and has no recorded result; it can never be evaluated again")
}
