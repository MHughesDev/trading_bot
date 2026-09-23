//! Control limits, declared before the drawdown (COMP-007 §3, P2; S-7.9).
//!
//! A limit chosen while the loss is happening is a limit chosen by the loss. So
//! limits are declared at promotion from a named backtest or paper run, versioned,
//! and never edited in place; changing one *during* a breach takes the two-step
//! pattern and is logged with the breach it overrode.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::family::Family;

/// Where a limit's numbers came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitMethod {
    /// Shadow/paper baseline ± 3σ on an EWMA.
    PaperEwma3Sigma,
    /// Backtest baseline ± 3σ.
    Backtest3Sigma,
    /// The venue's own rate ceiling (AGENT-006 `get_venue_constraints`).
    VenueRateCeiling,
    /// The risk gate's limit, re-checked here independently.
    RiskGateMirror,
    /// A number a human declared outright, with the run that justified it.
    DeclaredAbsolute,
    /// The backtest's 95th-percentile Monte Carlo drawdown.
    McP95Drawdown,
    /// A CUSUM decision interval.
    Cusum,
}

impl LimitMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PaperEwma3Sigma => "paper_ewma_3sigma",
            Self::Backtest3Sigma => "backtest_3sigma",
            Self::VenueRateCeiling => "venue_rate_ceiling",
            Self::RiskGateMirror => "risk_gate_mirror",
            Self::DeclaredAbsolute => "declared_absolute",
            Self::McP95Drawdown => "mc_p95_drawdown",
            Self::Cusum => "cusum",
        }
    }
}

/// Whether crossing a limit records or acts.
///
/// Declared with the limit rather than derived from the metric's name. "Which
/// metrics are only advisory" is a decision about a particular strategy, and a
/// convention buried in a match arm is a decision nobody can see from the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSeverity {
    Warning,
    Breach,
}

impl LimitSeverity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Breach => "breach",
        }
    }
}

impl std::str::FromStr for LimitSeverity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "warning" => Ok(Self::Warning),
            "breach" => Ok(Self::Breach),
            other => Err(format!("unknown limit severity {other:?}")),
        }
    }
}

/// One declared limit on one metric of one strategy.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlLimit {
    pub strategy_id: String,
    pub metric: String,
    pub family: Family,
    pub severity: LimitSeverity,
    pub version: i32,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub method: LimitMethod,
    /// The backtest or paper run the baseline came from. Never empty — a baseline
    /// whose provenance is unrecorded is a number somebody typed, and the whole
    /// difference between a control limit and a guess is that one of them can be
    /// traced to a measurement.
    pub declared_from: String,
    pub declared_at: DateTime<Utc>,
    pub declared_by: String,
}

impl ControlLimit {
    /// Whether a reading sits outside this limit.
    #[must_use]
    pub fn breached_by(&self, value: f64) -> bool {
        if !value.is_finite() {
            // A NaN reading is not "inside". It is a measurement that failed, and
            // treating it as inside is how a broken metric looks like a healthy
            // one for as long as it stays broken.
            return true;
        }
        self.lower.is_some_and(|l| value < l) || self.upper.is_some_and(|u| value > u)
    }

    /// The bound the reading crossed, for the event record.
    #[must_use]
    pub fn crossed_bound(&self, value: f64) -> Option<f64> {
        if self.lower.is_some_and(|l| value < l) {
            return self.lower;
        }
        if self.upper.is_some_and(|u| value > u) {
            return self.upper;
        }
        None
    }
}

/// Why a promotion to `live` was refused (COMP-002 §3, amended).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromotionRefusal {
    /// No limits at all.
    NoLimitsDeclared { strategy_id: String },
    /// Limits exist but leave a required family unmeasured.
    FamilyUnmeasured {
        strategy_id: String,
        families: Vec<Family>,
    },
}

impl std::fmt::Display for PromotionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLimitsDeclared { strategy_id } => write!(
                f,
                "strategy {strategy_id} has no declared control limits and cannot be promoted to live"
            ),
            Self::FamilyUnmeasured {
                strategy_id,
                families,
            } => {
                let names: Vec<&str> = families.iter().map(|x| x.as_str()).collect();
                write!(
                    f,
                    "strategy {strategy_id} declares no limit in the {} {} and cannot be promoted to live",
                    names.join(", "),
                    if families.len() == 1 { "family" } else { "families" }
                )
            }
        }
    }
}

impl std::error::Error for PromotionRefusal {}

/// The families a live strategy must have a declared limit in.
///
/// Three rather than one. "Has at least one limit" satisfies the letter of the
/// gate and none of its intent: a strategy with a single limit on a metric nobody
/// watches is exactly as unmonitored as one with none, and the gate would be a
/// checkbox. Inputs first (§P3), so execution and behaviour are required alongside
/// P&L rather than P&L alone.
pub const REQUIRED_FOR_LIVE: [Family; 3] = [Family::Execution, Family::Behaviour, Family::Pnl];

/// Every declared limit, newest version per metric.
#[derive(Clone, Debug, Default)]
pub struct LimitBook {
    /// `(strategy_id, metric) -> limit`, holding the highest version seen.
    current: BTreeMap<(String, String), ControlLimit>,
}

impl LimitBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a declared limit, keeping the highest version per metric.
    pub fn declare(&mut self, limit: ControlLimit) {
        let key = (limit.strategy_id.clone(), limit.metric.clone());
        match self.current.get(&key) {
            Some(existing) if existing.version >= limit.version => {}
            _ => {
                self.current.insert(key, limit);
            }
        }
    }

    #[must_use]
    pub fn get(&self, strategy_id: &str, metric: &str) -> Option<&ControlLimit> {
        self.current
            .get(&(strategy_id.to_owned(), metric.to_owned()))
    }

    #[must_use]
    pub fn families_for(&self, strategy_id: &str) -> Vec<Family> {
        let mut fams: Vec<Family> = self
            .current
            .values()
            .filter(|l| l.strategy_id == strategy_id)
            .map(|l| l.family)
            .collect();
        fams.sort();
        fams.dedup();
        fams
    }

    /// The COMP-002 gate amendment: a strategy with no declared limits cannot be
    /// promoted to `live`.
    pub fn may_promote_to_live(&self, strategy_id: &str) -> Result<(), PromotionRefusal> {
        let declared = self.families_for(strategy_id);
        if declared.is_empty() {
            return Err(PromotionRefusal::NoLimitsDeclared {
                strategy_id: strategy_id.to_owned(),
            });
        }
        let missing: Vec<Family> = REQUIRED_FOR_LIVE
            .into_iter()
            .filter(|f| !declared.contains(f))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(PromotionRefusal::FamilyUnmeasured {
                strategy_id: strategy_id.to_owned(),
                families: missing,
            })
        }
    }
}

/// A limit change proposed while the metric is outside its limit (§3, P2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidBreachChange {
    pub strategy_id: String,
    pub metric: String,
    /// The `monitor_events` row this change overrides.
    pub overrode_event_id: uuid::Uuid,
    /// The scope name the human typed, verbatim.
    pub confirmation: String,
    pub reason: String,
    pub changed_by: String,
}

/// Why a mid-breach limit change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeRefusal {
    /// The typed confirmation did not match the metric being changed.
    ConfirmationMismatch { expected: String, typed: String },
    /// No reason given.
    ReasonMissing,
    /// An agent tried to widen the limit it is being measured against.
    AgentAuthor { changed_by: String },
}

impl std::fmt::Display for ChangeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationMismatch { expected, typed } => write!(
                f,
                "type {expected:?} to confirm this change; received {typed:?}"
            ),
            Self::ReasonMissing => f.write_str("a limit change during a breach requires a reason"),
            Self::AgentAuthor { changed_by } => {
                write!(f, "{changed_by} may not declare or change a control limit")
            }
        }
    }
}

impl MidBreachChange {
    /// The two-step check. Nothing is written until this returns `Ok`.
    pub fn validate(&self) -> Result<(), ChangeRefusal> {
        if self.changed_by.starts_with("agent:") {
            // A limit the watched party can widen is not a control. This is the
            // same rule as §5's, one layer down, and it is checked here as well as
            // in the database because each catches the other's blind spot.
            return Err(ChangeRefusal::AgentAuthor {
                changed_by: self.changed_by.clone(),
            });
        }
        if self.reason.trim().is_empty() {
            return Err(ChangeRefusal::ReasonMissing);
        }
        if self.confirmation != self.metric {
            return Err(ChangeRefusal::ConfirmationMismatch {
                expected: self.metric.clone(),
                typed: self.confirmation.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(strategy: &str, metric: &str, family: Family) -> ControlLimit {
        ControlLimit {
            strategy_id: strategy.to_owned(),
            metric: metric.to_owned(),
            family,
            severity: LimitSeverity::Breach,
            version: 1,
            lower: Some(0.8),
            upper: None,
            method: LimitMethod::PaperEwma3Sigma,
            declared_from: "paper-run-7".to_owned(),
            declared_at: Utc::now(),
            declared_by: "human:mason".to_owned(),
        }
    }

    fn fully_declared(book: &mut LimitBook, strategy: &str) {
        book.declare(limit(strategy, "fill_rate", Family::Execution));
        book.declare(limit(strategy, "orders_per_minute", Family::Behaviour));
        book.declare(limit(strategy, "max_drawdown", Family::Pnl));
    }

    #[test]
    fn a_reading_below_the_floor_breaches() {
        let l = limit("s", "fill_rate", Family::Execution);
        assert!(l.breached_by(0.5));
        assert!(!l.breached_by(0.9));
        assert_eq!(l.crossed_bound(0.5), Some(0.8));
    }

    #[test]
    fn a_nan_reading_is_a_breach_and_not_a_pass() {
        // A metric whose computation failed must not read as healthy. This is the
        // difference between a monitor that is quiet because everything is fine
        // and one that is quiet because it stopped measuring.
        let l = limit("s", "fill_rate", Family::Execution);
        assert!(l.breached_by(f64::NAN));
    }

    #[test]
    fn promotion_is_refused_with_no_limits_at_all() {
        // COMP-007 §8.8.
        let book = LimitBook::new();
        assert_eq!(
            book.may_promote_to_live("s"),
            Err(PromotionRefusal::NoLimitsDeclared {
                strategy_id: "s".to_owned()
            })
        );
    }

    #[test]
    fn one_token_limit_does_not_satisfy_the_gate() {
        // The adversarial one: "has at least one limit" is a checkbox anybody can
        // tick, and a strategy with a single limit on a metric nobody watches is
        // exactly as unmonitored as one with none.
        let mut book = LimitBook::new();
        book.declare(limit("s", "fill_rate", Family::Execution));
        let err = book.may_promote_to_live("s").expect_err("must refuse");
        match err {
            PromotionRefusal::FamilyUnmeasured { families, .. } => {
                assert_eq!(families, vec![Family::Behaviour, Family::Pnl]);
            }
            other @ PromotionRefusal::NoLimitsDeclared { .. } => {
                panic!("wrong refusal: {other:?}")
            }
        }
    }

    #[test]
    fn a_fully_declared_strategy_may_be_promoted() {
        let mut book = LimitBook::new();
        fully_declared(&mut book, "s");
        assert!(book.may_promote_to_live("s").is_ok());
    }

    #[test]
    fn one_strategys_limits_do_not_promote_another() {
        let mut book = LimitBook::new();
        fully_declared(&mut book, "s");
        assert!(book.may_promote_to_live("other").is_err());
    }

    #[test]
    fn the_newest_version_of_a_limit_wins_and_an_older_one_cannot_overwrite_it() {
        let mut book = LimitBook::new();
        let mut v2 = limit("s", "fill_rate", Family::Execution);
        v2.version = 2;
        v2.lower = Some(0.5);
        book.declare(v2);
        book.declare(limit("s", "fill_rate", Family::Execution)); // version 1
        assert_eq!(book.get("s", "fill_rate").and_then(|l| l.lower), Some(0.5));
    }

    fn change() -> MidBreachChange {
        MidBreachChange {
            strategy_id: "s".to_owned(),
            metric: "fill_rate".to_owned(),
            overrode_event_id: uuid::Uuid::new_v4(),
            confirmation: "fill_rate".to_owned(),
            reason: "venue changed its fee tier".to_owned(),
            changed_by: "human:mason".to_owned(),
        }
    }

    #[test]
    fn a_confirmed_reasoned_mid_breach_change_is_allowed() {
        assert!(change().validate().is_ok());
    }

    #[test]
    fn a_mid_breach_change_without_the_typed_confirmation_is_refused() {
        let mut c = change();
        c.confirmation = "yes".to_owned();
        assert!(matches!(
            c.validate(),
            Err(ChangeRefusal::ConfirmationMismatch { .. })
        ));
    }

    #[test]
    fn a_mid_breach_change_without_a_reason_is_refused() {
        let mut c = change();
        c.reason = "   ".to_owned();
        assert_eq!(c.validate(), Err(ChangeRefusal::ReasonMissing));
    }

    #[test]
    fn an_agent_cannot_widen_the_limit_it_is_measured_against() {
        let mut c = change();
        c.changed_by = "agent:run-9".to_owned();
        assert!(matches!(
            c.validate(),
            Err(ChangeRefusal::AgentAuthor { .. })
        ));
    }
}
