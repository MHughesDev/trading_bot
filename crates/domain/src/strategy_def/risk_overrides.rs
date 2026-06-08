//! `RiskOverrides` — per-strategy risk limits that may only **tighten** global limits.
//!
//! # Tighten-only invariant (v1.0, frozen)
//!
//! A strategy may specify overrides that are *more restrictive* than the user's
//! global risk config.  It may **never** loosen them.  This invariant is:
//!
//! 1. Documented here in the type's module docs (Phase 0 — freeze it).
//! 2. Enforced by the `strategy-validator` crate at validation time (Phase 5).
//! 3. Enforced by the `risk` crate at order-submission time (Phase 2), which
//!    always applies the **tighter** of the global limit and the strategy override.
//!
//! If a strategy definition contains an override that would loosen a global
//! limit, `strategy-validator` returns a `ValidationError::RiskOverrideTooPermissive`.
//!
//! # Fields (v1.0)
//!
//! All fields are optional.  Absent = use the global limit unchanged.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Per-strategy risk overrides (all fields are "tighten only").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskOverrides {
    /// Maximum position size for this strategy.  Must be ≤ global `max_position`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_position: Option<Decimal>,

    /// Maximum orders per minute from this strategy.  Must be ≤ global rate limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_order_rate_per_minute: Option<u32>,

    /// Maximum orders per second from this strategy.  Must be ≤ global rate limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_order_rate_per_second: Option<u32>,
}
