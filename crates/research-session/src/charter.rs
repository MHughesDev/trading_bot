//! The charter: what a user writes instead of a question when Continuous is on
//! (AGENT-014 §2.2), and the budget, schedule and autonomy that come with it.
//!
//! # Why the floor is a type and not a check somewhere
//!
//! `exploration_share` has a floor of 0.10 and the spec says it "cannot be set
//! below". A number that cannot be set below a floor is a type, not a field with
//! a rule written near it: every construction path goes through
//! [`ExplorationShare::new`], `serde` deserialisation included, and the column
//! carries the same floor as a CHECK so a database session cannot write what the
//! constructor refuses. Both halves are asserted against each other by
//! `crates/invariants/tests/exploration_floor_is_one_number.rs` — the floor is
//! written once, in this file, and the migration is read for it.
//!
//! # Defaults are pre-filled, not required
//!
//! AGENT-014 §14: the day budget has a platform default and is pre-filled. None
//! of [`SessionBudget`]'s fields is required input, so they live in `Default`
//! rather than in a form validator, and the desk reads them from here.

use serde::{Deserialize, Deserializer, Serialize};

/// The exploration floor (AGENT-014 §5.2, mirroring INV-21).
///
/// One definition. The migration's CHECK is held equal to it by an invariant
/// test rather than by both being written down and hoped about.
pub const EXPLORATION_FLOOR: f64 = 0.10;

/// The charter default when the user does not move the slider.
pub const EXPLORATION_DEFAULT: f64 = 0.15;

/// The platform's default day budget in USD (AGENT-014 §2.1, §14; user decision
/// 2026-09-15).
pub const DEFAULT_USD_PER_DAY: f64 = 25.0;

/// Days of day budget the total defaults to.
pub const DEFAULT_BUDGET_DAYS: f64 = 30.0;

/// The default cap on how many runs one session may start.
pub const DEFAULT_MAX_RUNS: i32 = 60;

/// A session may run at most this many children at once (AGENT-014 §2.1).
pub const MAX_CONCURRENT_RUNS: i16 = 3;

/// What a charter can be refused for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CharterError {
    #[error(
        "exploration_share {got} is below the floor {EXPLORATION_FLOOR}: at least one pick in ten \
         is a deliberate detour, and a session that never takes one can only confirm the ranking \
         it already has (AGENT-014 §5.2, INV-21)"
    )]
    BelowExplorationFloor { got: String },
    #[error("exploration_share {got} is not a share: it must be at most 1.0")]
    AboveOne { got: String },
    #[error("exploration_share {got} is not a number")]
    NotANumber { got: String },
    #[error(
        "max_concurrent_runs {got} is outside 1..={MAX_CONCURRENT_RUNS}: concurrency beyond that \
         is a way to spend a day budget in an hour, not a way to research faster"
    )]
    ConcurrencyOutOfRange { got: i16 },
}

/// A share of selections made as a deliberate detour, never below the floor.
///
/// Sealed: the inner `f64` is private and the only way in is [`Self::new`],
/// including through `serde`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ExplorationShare(f64);

impl ExplorationShare {
    /// # Errors
    ///
    /// Refuses anything below [`EXPLORATION_FLOOR`], above 1.0, or not finite.
    pub fn new(value: f64) -> Result<Self, CharterError> {
        if !value.is_finite() {
            return Err(CharterError::NotANumber {
                got: value.to_string(),
            });
        }
        if value < EXPLORATION_FLOOR {
            return Err(CharterError::BelowExplorationFloor {
                got: value.to_string(),
            });
        }
        if value > 1.0 {
            return Err(CharterError::AboveOne {
                got: value.to_string(),
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl Default for ExplorationShare {
    fn default() -> Self {
        Self(EXPLORATION_DEFAULT)
    }
}

impl<'de> Deserialize<'de> for ExplorationShare {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = f64::deserialize(d)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

/// Whether the human confirms each agenda item before it starts (AGENT-014 §8).
///
/// Never about approving: a proposal into ring 1 or 2 waits for a human in every
/// setting, which is why this enum has no variant that could be read as "and
/// approves its own proposals".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Every item waits in *Needs you* before it starts.
    Supervised,
    /// Items start; the human is notified per start and per verdict.
    Notify,
    /// Items start; the digest is the only notification.
    Autonomous,
}

impl Autonomy {
    pub const ALL: &'static [Autonomy] = &[
        Autonomy::Supervised,
        Autonomy::Notify,
        Autonomy::Autonomous,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Supervised => "supervised",
            Autonomy::Notify => "notify",
            Autonomy::Autonomous => "autonomous",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.as_str() == s)
    }

    /// Whether an item must be approved by a human before it is claimed.
    #[must_use]
    pub fn item_waits_for_approval(self) -> bool {
        matches!(self, Autonomy::Supervised)
    }
}

impl Default for Autonomy {
    /// `notify` (user decision, 2026-09-15): items start, the human hears about
    /// each one. Not `supervised`, because a first window spent clicking approve
    /// teaches the human nothing and the session nothing.
    fn default() -> Self {
        Autonomy::Notify
    }
}

/// What the charter is trying to get (AGENT-014 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Objective {
    /// Find something worth deploying, under SPEC §10's objective shape.
    FindDeployable { constraints: serde_json::Value },
    /// Learn about a list of subjects, deployable or not.
    BuildKnowledge { about: Vec<String> },
    /// Re-verify named strategies, watch for decay, diagnose what moves.
    Maintain { strategy_ids: Vec<String> },
    /// Several at once.
    Mixed { parts: Vec<Objective> },
}

impl Objective {
    /// How much this objective cares about a subject, used as the `value`
    /// signal's first factor (AGENT-014 §5.1).
    ///
    /// Deliberately blunt in this build: 1.0 for anything the objective names,
    /// 0.6 for anything it does not. A weighted model of subject relevance would
    /// be a guess dressed as a number, and the scorer already multiplies by a
    /// source weight that is grounded in the spec's own table.
    #[must_use]
    pub fn weight_for(&self, subject: &str) -> f64 {
        if self.names(subject) {
            1.0
        } else {
            0.6
        }
    }

    fn names(&self, subject: &str) -> bool {
        match self {
            Objective::FindDeployable { constraints } => constraints
                .to_string()
                .to_lowercase()
                .contains(&subject.to_lowercase()),
            Objective::BuildKnowledge { about } => {
                about.iter().any(|a| a.eq_ignore_ascii_case(subject))
            }
            Objective::Maintain { strategy_ids } => {
                strategy_ids.iter().any(|s| s.eq_ignore_ascii_case(subject))
            }
            Objective::Mixed { parts } => parts.iter().any(|p| p.names(subject)),
        }
    }
}

/// What makes the session stop on its own (AGENT-014 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopWhen {
    Budget,
    ScheduleEnd,
    AgendaDrained,
    Stall,
    DeployableFound { k: u32 },
}

/// Where the session is allowed to look (AGENT-014 §2.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub instruments: Vec<String>,
    #[serde(default)]
    pub asset_classes: Vec<String>,
    #[serde(default)]
    pub families: Vec<String>,
    #[serde(default)]
    pub timeframes: Vec<String>,
    #[serde(default)]
    pub windows: Vec<String>,
}

impl Scope {
    /// Whether a subject falls in scope.
    ///
    /// An empty scope is *everything*, not nothing: a charter that names no
    /// instruments has not excluded them all, and reading it the other way would
    /// silently drain every agenda fed by a subscription.
    #[must_use]
    pub fn admits(&self, subject: &str) -> bool {
        let lists = [
            &self.instruments,
            &self.asset_classes,
            &self.families,
            &self.timeframes,
        ];
        if lists.iter().all(|l| l.is_empty()) {
            return true;
        }
        lists
            .iter()
            .flat_map(|l| l.iter())
            .any(|s| s.eq_ignore_ascii_case(subject))
    }
}

/// The charter itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Charter {
    pub scope: Scope,
    pub objective: Objective,
    #[serde(default)]
    pub seeds: Vec<Seed>,
    #[serde(default)]
    pub permitted_workflows: Vec<String>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub standing_constraints: Vec<String>,
    #[serde(default)]
    pub exploration_share: ExplorationShare,
    #[serde(default)]
    pub stop_when: Vec<StopWhen>,
    #[serde(default)]
    pub delta_practical_default: Option<f64>,
    /// Bumped on every amendment; an amendment appends an event and shows in the
    /// digest rather than replacing what ran under the old text.
    #[serde(default = "one")]
    pub version: i32,
}

fn one() -> i32 {
    1
}

/// A question the user wrote into the charter, ready to become an agenda item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seed {
    pub question: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub workflow: Option<String>,
}

impl Charter {
    /// Whether a workflow is one this charter permits.
    ///
    /// An empty `permitted_workflows` means "all the tier supports"
    /// (AGENT-014 §2.2), which is why the empty case answers yes. The tier's own
    /// slice is applied separately, by the catalogue, so this is a charter
    /// question and not a capability question.
    #[must_use]
    pub fn permits(&self, workflow: &str) -> bool {
        if self.excludes(workflow) {
            return false;
        }
        self.permitted_workflows.is_empty()
            || self
                .permitted_workflows
                .iter()
                .any(|w| w.eq_ignore_ascii_case(workflow))
    }

    /// Whether the charter excludes a subject, family, tool or data source.
    #[must_use]
    pub fn excludes(&self, what: &str) -> bool {
        self.exclusions.iter().any(|e| e.eq_ignore_ascii_case(what))
    }
}

/// What a session may spend (AGENT-014 §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBudget {
    pub usd_total: f64,
    pub usd_per_day: f64,
    pub trials_total: i64,
    pub max_runs: i32,
}

impl Default for SessionBudget {
    fn default() -> Self {
        Self {
            usd_per_day: DEFAULT_USD_PER_DAY,
            usd_total: DEFAULT_USD_PER_DAY * DEFAULT_BUDGET_DAYS,
            // Filled from the tier's `max_trials_per_task` × `max_runs` when a
            // profile is known; this is the shape's floor, not a policy.
            trials_total: 0,
            max_runs: DEFAULT_MAX_RUNS,
        }
    }
}

/// A weekly window the session is allowed to run in (AGENT-014 §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// ISO weekday numbers, Monday = 1.
    pub days: Vec<u8>,
    /// `HH:MM`, in `tz`.
    pub start: String,
    pub end: String,
    pub tz: String,
}

/// When the session may claim, and how many at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub windows: Vec<Window>,
    pub max_concurrent_runs: i16,
}

impl Default for Schedule {
    /// No windows means always open. One run at a time, because concurrency is
    /// something a user turns on knowing it divides the day budget.
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            max_concurrent_runs: 1,
        }
    }
}

impl Schedule {
    /// # Errors
    ///
    /// Refuses a concurrency outside `1..=`[`MAX_CONCURRENT_RUNS`].
    pub fn with_concurrency(mut self, n: i16) -> Result<Self, CharterError> {
        if !(1..=MAX_CONCURRENT_RUNS).contains(&n) {
            return Err(CharterError::ConcurrencyOutOfRange { got: n });
        }
        self.max_concurrent_runs = n;
        Ok(self)
    }
}

/// How often the platform writes a digest (AGENT-014 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewEvery {
    Runs { runs: u32 },
    Window,
}

impl Default for ReviewEvery {
    fn default() -> Self {
        ReviewEvery::Runs { runs: 5 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_holds_on_every_way_in() {
        assert!(ExplorationShare::new(0.05).is_err());
        assert!(ExplorationShare::new(EXPLORATION_FLOOR).is_ok());
        assert!(ExplorationShare::new(1.5).is_err());
        assert!(ExplorationShare::new(f64::NAN).is_err());

        // `serde` is a way in, and it goes through the same door. A charter
        // posted as JSON with 0.05 is refused at deserialisation, not accepted
        // and then checked by whoever remembers to.
        let refused: Result<ExplorationShare, _> = serde_json::from_str("0.05");
        assert!(refused.is_err());
        let accepted: ExplorationShare = serde_json::from_str("0.2").unwrap();
        assert!((accepted.get() - 0.2).abs() < 1e-12);
    }

    #[test]
    fn an_empty_scope_admits_everything_rather_than_nothing() {
        assert!(Scope::default().admits("BTC-USD"));
        let narrow = Scope {
            families: vec!["momentum".into()],
            ..Scope::default()
        };
        assert!(narrow.admits("Momentum"));
        assert!(!narrow.admits("carry"));
    }

    #[test]
    fn an_exclusion_beats_an_empty_permitted_list() {
        let c = Charter {
            scope: Scope::default(),
            objective: Objective::BuildKnowledge { about: vec![] },
            seeds: vec![],
            permitted_workflows: vec![],
            exclusions: vec!["optimise".into()],
            standing_constraints: vec![],
            exploration_share: ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        };
        assert!(c.permits("investigate"));
        assert!(!c.permits("optimise"));
    }

    #[test]
    fn concurrency_is_bounded_where_it_is_set() {
        assert!(Schedule::default().with_concurrency(0).is_err());
        assert!(Schedule::default().with_concurrency(4).is_err());
        assert!(Schedule::default().with_concurrency(3).is_ok());
    }

    #[test]
    fn the_defaults_are_the_ones_the_user_decided_on() {
        assert_eq!(Autonomy::default(), Autonomy::Notify);
        let b = SessionBudget::default();
        assert!((b.usd_per_day - 25.0).abs() < 1e-12);
        assert!((b.usd_total - 750.0).abs() < 1e-12);
        assert_eq!(b.max_runs, 60);
    }
}
