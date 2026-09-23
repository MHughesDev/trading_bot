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
    pub const ALL: &'static [Autonomy] =
        &[Autonomy::Supervised, Autonomy::Notify, Autonomy::Autonomous];

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

// ── The tier gate (AGENT-014 §8) ─────────────────────────────────────────────
//
// Continuous research is frontier-only. That is a user decision of 2026-09-15
// with a revisit trigger written into §8, not a capability judgement this code
// gets to soften, and it is enforced here rather than only in the charter form
// because a rule enforced only in the browser is a rule a `curl` walks around.

use std::collections::BTreeMap;

/// What the `judgement` suite (AGENT-009 §3) found for one workflow on one
/// profile.
///
/// Three values, and the third is the one that matters. A workflow the suite
/// has never run is **not** a workflow that failed and **not** one that passed:
/// reading absence either way is how a profile comes to be trusted with a day's
/// budget on evidence nobody gathered. It is removed from the permitted set
/// like a failure, and the note says which it was.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Judgement {
    Passed { rate: f64 },
    Failed { rate: f64 },
    NotMeasured,
}

impl Judgement {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Judgement::Passed { .. } => "passed",
            Judgement::Failed { .. } => "failed",
            Judgement::NotMeasured => "not_measured",
        }
    }

    /// The pass rate, if one was measured.
    ///
    /// Returns `Option` rather than 0.0 for the unmeasured case, and that is
    /// the whole reason this is a method rather than a field: `0.0` is a
    /// measurement meaning *it never passes*, and a caller handed one for a
    /// suite that never ran would compare it to a threshold and get a
    /// confident, false answer.
    /// A non-finite rate answers `None` too. `NaN` compares false against
    /// every threshold, so a rate of `NaN` would slip past `worst < threshold`
    /// and grant `autonomous` on a number that is not one.
    #[must_use]
    pub fn rate(self) -> Option<f64> {
        match self {
            Judgement::Passed { rate } | Judgement::Failed { rate } if rate.is_finite() => {
                Some(rate)
            }
            _ => None,
        }
    }

    /// Whether the suite passed.
    ///
    /// Deliberately **not** conditioned on the rate being finite: a
    /// `Passed { rate: NaN }` keeps the workflow (the suite said it passed) and
    /// then fails [`rate`](Self::rate), so `autonomous` is refused under
    /// `AutonomousNotMeasured` — which is the true sentence — rather than the
    /// workflow silently disappearing from the charter.
    #[must_use]
    pub fn passed(self) -> bool {
        matches!(self, Judgement::Passed { .. })
    }
}

/// The profile a session wants to run on, as §8's rule reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    /// Frontier or local. §8: local tiers run Chat and bounded Research with a
    /// human watching; they are not left alone with a budget over days.
    pub frontier: bool,
    /// AGENT-009 §5: the profile's own scorecard exists and is current.
    pub measured: bool,
    /// The `judgement` pass rate `autonomous` requires, which is the profile's
    /// own and not a platform constant — §8 says "the profile's threshold".
    pub autonomous_threshold: f64,
    /// Per workflow, what the suite found.
    pub judgement: BTreeMap<String, Judgement>,
    /// Every workflow this tier supports, which is what an empty
    /// `permitted_workflows` means (§2.2).
    pub supports: Vec<String>,
}

impl Profile {
    /// A frontier profile that has been measured and passes everything.
    #[must_use]
    pub fn frontier(name: &str, supports: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            frontier: true,
            measured: true,
            autonomous_threshold: 0.8,
            judgement: supports
                .iter()
                .map(|w| ((*w).to_string(), Judgement::Passed { rate: 1.0 }))
                .collect(),
            supports: supports.iter().map(|w| (*w).to_string()).collect(),
        }
    }

    /// What the suite found for a workflow.
    ///
    /// A workflow with no entry answers [`Judgement::NotMeasured`], which is
    /// the honest reading of a missing row and the one thing a `BTreeMap::get`
    /// followed by an `unwrap_or` would have got wrong.
    #[must_use]
    pub fn judgement_for(&self, workflow: &str) -> Judgement {
        self.judgement
            .get(workflow)
            .copied()
            .unwrap_or(Judgement::NotMeasured)
    }
}

/// Why a workflow was taken out of the charter's permitted set.
///
/// §8: *"a workflow it fails is removed from the charter's permitted set with a
/// note, not silently run"*. This is the note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedWorkflow {
    pub workflow: String,
    /// [`Judgement::as_str`], so `failed` and `not_measured` stay
    /// distinguishable all the way to the digest.
    pub why: String,
    pub sentence: String,
}

/// Why a session may not be started continuous on this profile.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TierRefusal {
    #[error(
        "`{profile}` is not a frontier profile. Continuous research runs on frontier profiles \
         only (AGENT-014 §8, user decision 2026-09-15): a local tier runs Chat and bounded \
         Research with a human watching, and is not left alone with a budget over days. Run this \
         question as bounded Research, or pick a frontier profile."
    )]
    NotFrontier { profile: String },
    #[error(
        "`{profile}` has no current scorecard. A profile that has not been measured (AGENT-009 \
         §5) cannot be said to pass `judgement` for anything, and a session may not spend days \
         on that basis."
    )]
    NotMeasured { profile: String },
    #[error(
        "`{profile}` passes `judgement` for none of the workflows this charter permits, so there \
         is nothing it could be chartered to do: {removed}"
    )]
    NothingPermittedRemains { profile: String, removed: String },
    #[error(
        "`autonomous` is refused on `{profile}`: its `judgement` pass rate is {rate} and its own \
         threshold is {threshold}. Refused rather than quietly downgraded to `notify`, because a \
         setting that silently becomes another setting is a setting nobody can rely on \
         (AGENT-014 §8)."
    )]
    AutonomousBelowThreshold {
        profile: String,
        rate: f64,
        threshold: f64,
    },
    #[error(
        "`autonomous` is refused on `{profile}`: the `judgement` pass rate has not been measured \
         for {unmeasured}. An unmeasured suite is not a passing one, and `autonomous` is the one \
         setting where nobody is watching."
    )]
    AutonomousNotMeasured { profile: String, unmeasured: String },
}

/// A charter that passed §8, and what §8 took out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Admitted {
    pub charter: Charter,
    pub autonomy: Autonomy,
    /// Every workflow §8 removed, each with its note. Empty is the ordinary
    /// case; a non-empty list is something the human is told at creation and
    /// again in the digest.
    pub removed: Vec<RemovedWorkflow>,
}

/// §8's gate: may this charter be started continuous on this profile?
///
/// The order of the checks is the order of the sentences a user would want:
/// *is this the right kind of profile*, *has it been measured at all*, *what
/// can it actually do*, and only then *may it be left alone*. Checking autonomy
/// first would refuse `autonomous` on a local profile and leave the user to
/// discover the real problem on the second try.
///
/// # Errors
///
/// Refuses a non-frontier profile, an unmeasured one, one that passes nothing
/// the charter permits, and `autonomous` below the profile's own threshold.
pub fn admit(
    charter: &Charter,
    autonomy: Autonomy,
    profile: &Profile,
) -> Result<Admitted, TierRefusal> {
    if !profile.frontier {
        return Err(TierRefusal::NotFrontier {
            profile: profile.name.clone(),
        });
    }
    if !profile.measured {
        return Err(TierRefusal::NotMeasured {
            profile: profile.name.clone(),
        });
    }

    // An empty `permitted_workflows` means "all the tier supports" (§2.2), so
    // that is the set §8 filters — not the empty list, which would pass the
    // gate by having nothing to check and then run anything at all.
    let asked: Vec<String> = if charter.permitted_workflows.is_empty() {
        profile.supports.clone()
    } else {
        charter.permitted_workflows.clone()
    };

    let mut kept = Vec::new();
    let mut removed = Vec::new();
    for w in asked {
        let j = profile.judgement_for(&w);
        if j.passed() {
            kept.push(w);
            continue;
        }
        let sentence = match j {
            Judgement::Failed { rate } => format!(
                "`{w}` was removed: `{}` passes the `judgement` suite for it at {rate}, and a \
                 workflow a profile fails is removed rather than silently run (AGENT-014 §8)",
                profile.name
            ),
            Judgement::NotMeasured => format!(
                "`{w}` was removed: the `judgement` suite has never been run for it on `{}`. \
                 Not a failure — an absence, and an absence is not a pass",
                profile.name
            ),
            Judgement::Passed { .. } => unreachable!("handled above"),
        };
        removed.push(RemovedWorkflow {
            workflow: w,
            why: j.as_str().to_string(),
            sentence,
        });
    }

    if kept.is_empty() {
        return Err(TierRefusal::NothingPermittedRemains {
            profile: profile.name.clone(),
            removed: removed
                .iter()
                .map(|r| format!("{} ({})", r.workflow, r.why))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

    if autonomy == Autonomy::Autonomous {
        // Measured against what the session will actually run, which is `kept`
        // — the removed ones are not going to run, and holding an unmeasured
        // workflow against a setting that governs the ones that remain would
        // refuse a charter over a capability it had just given up.
        let unmeasured: Vec<&str> = kept
            .iter()
            .filter(|w| profile.judgement_for(w).rate().is_none())
            .map(String::as_str)
            .collect();
        if !unmeasured.is_empty() {
            return Err(TierRefusal::AutonomousNotMeasured {
                profile: profile.name.clone(),
                unmeasured: unmeasured.join(", "),
            });
        }
        // The weakest workflow decides. A profile that is excellent at four
        // things and barely passes a fifth is being left alone with the fifth.
        let worst = kept
            .iter()
            .filter_map(|w| profile.judgement_for(w).rate())
            .fold(f64::INFINITY, f64::min);
        if worst < profile.autonomous_threshold {
            return Err(TierRefusal::AutonomousBelowThreshold {
                profile: profile.name.clone(),
                rate: worst,
                threshold: profile.autonomous_threshold,
            });
        }
    }

    let mut charter = charter.clone();
    charter.permitted_workflows = kept;
    Ok(Admitted {
        charter,
        autonomy,
        removed,
    })
}

/// A decision the platform asks a human about in every autonomy setting
/// (AGENT-014 §8, AGENT-008 §4).
///
/// There is no variant for ring 3, and that is §8's "ring 3 does not exist"
/// made unrepresentable rather than written near a check. The sibling half is
/// [`crate::harvest::Ring`], which is sealed to {1, 2} and refuses 0 and 3 at
/// its constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysAsks {
    SealedHoldout,
    Promotion,
    /// `Spend` asks above the ask line; below it, it does not.
    SpendAboveAskLine,
}

impl AlwaysAsks {
    pub const ALL: &'static [AlwaysAsks] = &[
        AlwaysAsks::SealedHoldout,
        AlwaysAsks::Promotion,
        AlwaysAsks::SpendAboveAskLine,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AlwaysAsks::SealedHoldout => "sealed_holdout",
            AlwaysAsks::Promotion => "promotion",
            AlwaysAsks::SpendAboveAskLine => "spend_above_ask_line",
        }
    }

    /// Whether a human is asked, given the session's autonomy.
    ///
    /// Takes the autonomy and ignores it, on purpose. The signature is what
    /// makes the rule checkable: a caller cannot ask the question in a form
    /// that omits the setting, and this method is where anyone looking for a
    /// place to make `autonomous` skip the question would arrive — and find a
    /// body that does not read it, with `autonomy_is_a_setting_on_starting_only`
    /// asserting it over every combination.
    #[must_use]
    pub fn asks(self, _autonomy: Autonomy) -> bool {
        true
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

    // ── The tier gate (AGENT-014 §8) ────────────────────────────────────────

    fn a_charter(permitted: &[&str]) -> Charter {
        Charter {
            scope: Scope::default(),
            objective: Objective::BuildKnowledge { about: vec![] },
            seeds: vec![],
            permitted_workflows: permitted.iter().map(|w| (*w).to_string()).collect(),
            exclusions: vec![],
            standing_constraints: vec![],
            exploration_share: ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        }
    }

    #[test]
    fn a_local_profile_is_refused_by_name_with_the_reason() {
        // §12.10's first clause.
        let mut p = Profile::frontier("qwen-local", &["investigate"]);
        p.frontier = false;
        let e = admit(&a_charter(&["investigate"]), Autonomy::Notify, &p)
            .expect_err("a local profile is not left alone with a budget over days");
        assert_eq!(
            e,
            TierRefusal::NotFrontier {
                profile: "qwen-local".into()
            }
        );
        assert!(e.to_string().contains("qwen-local"), "{e}");
        assert!(
            e.to_string().contains("bounded Research"),
            "the way out: {e}"
        );
    }

    #[test]
    fn an_unmeasured_frontier_profile_is_refused_too() {
        let mut p = Profile::frontier("opus", &["investigate"]);
        p.measured = false;
        assert!(matches!(
            admit(&a_charter(&["investigate"]), Autonomy::Notify, &p),
            Err(TierRefusal::NotMeasured { .. })
        ));
    }

    #[test]
    fn a_failing_workflow_is_removed_with_a_note_rather_than_silently_run() {
        let mut p = Profile::frontier("opus", &["investigate", "optimise"]);
        p.judgement
            .insert("optimise".into(), Judgement::Failed { rate: 0.3 });
        let a = admit(
            &a_charter(&["investigate", "optimise"]),
            Autonomy::Notify,
            &p,
        )
        .expect("investigate still passes");
        assert_eq!(a.charter.permitted_workflows, vec!["investigate"]);
        assert_eq!(a.removed.len(), 1);
        assert_eq!(a.removed[0].workflow, "optimise");
        assert_eq!(a.removed[0].why, "failed");
        assert!(!a.charter.permits("optimise"), "and the charter agrees");
    }

    #[test]
    fn a_workflow_nobody_measured_is_removed_under_its_own_word() {
        // Not a failure. An absence, and an absence is not a pass — but it is
        // also not the same thing as a measured failure, and a human reading
        // the note should be able to tell which they have.
        let mut p = Profile::frontier("opus", &["investigate"]);
        p.supports.push("diagnose".into());
        let a = admit(
            &a_charter(&["investigate", "diagnose"]),
            Autonomy::Notify,
            &p,
        )
        .expect("investigate passes");
        assert_eq!(a.removed[0].why, "not_measured");
        assert_ne!(a.removed[0].why, "failed");
        assert!(a.removed[0].sentence.contains("never been run"));
    }

    #[test]
    fn an_empty_permitted_set_is_filtered_rather_than_passing_by_being_empty() {
        // §2.2: empty means "all the tier supports". Read as an empty list to
        // check, the gate would pass trivially and the session would then run
        // whatever the tier supports, including what it fails.
        let mut p = Profile::frontier("opus", &["investigate", "optimise"]);
        p.judgement
            .insert("optimise".into(), Judgement::Failed { rate: 0.1 });
        let a = admit(&a_charter(&[]), Autonomy::Notify, &p).expect("investigate passes");
        assert_eq!(a.charter.permitted_workflows, vec!["investigate"]);
        assert_eq!(a.removed.len(), 1);
    }

    #[test]
    fn a_profile_that_passes_nothing_the_charter_asks_for_is_refused() {
        let mut p = Profile::frontier("opus", &["investigate"]);
        p.judgement
            .insert("investigate".into(), Judgement::Failed { rate: 0.2 });
        let e = admit(&a_charter(&["investigate"]), Autonomy::Notify, &p)
            .expect_err("a charter with nothing it may do is not a charter");
        assert!(matches!(e, TierRefusal::NothingPermittedRemains { .. }));
        assert!(e.to_string().contains("investigate (failed)"), "{e}");
    }

    #[test]
    fn autonomous_below_the_profiles_own_threshold_is_refused_not_ignored() {
        // §12.10's second clause, and the word is "refused". A setting that
        // silently becomes another setting is one nobody can rely on.
        let mut p = Profile::frontier("opus", &["investigate"]);
        p.autonomous_threshold = 0.8;
        p.judgement
            .insert("investigate".into(), Judgement::Passed { rate: 0.6 });
        let e = admit(&a_charter(&["investigate"]), Autonomy::Autonomous, &p)
            .expect_err("0.6 is below 0.8");
        assert!(matches!(e, TierRefusal::AutonomousBelowThreshold { .. }));
        // And the same charter under `notify` is fine, which is what makes it a
        // refusal of the setting rather than of the profile.
        assert!(admit(&a_charter(&["investigate"]), Autonomy::Notify, &p).is_ok());
    }

    #[test]
    fn the_threshold_is_the_profiles_own_and_not_a_constant() {
        let mut p = Profile::frontier("opus", &["investigate"]);
        p.judgement
            .insert("investigate".into(), Judgement::Passed { rate: 0.6 });
        p.autonomous_threshold = 0.9;
        assert!(admit(&a_charter(&["investigate"]), Autonomy::Autonomous, &p).is_err());
        p.autonomous_threshold = 0.5;
        assert!(admit(&a_charter(&["investigate"]), Autonomy::Autonomous, &p).is_ok());
    }

    #[test]
    fn the_weakest_permitted_workflow_decides_autonomous() {
        // A profile excellent at four things and barely passing a fifth is
        // being left alone with the fifth.
        let mut p = Profile::frontier("opus", &["investigate", "diagnose"]);
        p.autonomous_threshold = 0.8;
        p.judgement
            .insert("diagnose".into(), Judgement::Passed { rate: 0.55 });
        let e = admit(
            &a_charter(&["investigate", "diagnose"]),
            Autonomy::Autonomous,
            &p,
        )
        .expect_err("diagnose is the one it will be left alone with");
        let TierRefusal::AutonomousBelowThreshold { rate, .. } = e else {
            panic!("{e}")
        };
        assert!(
            (rate - 0.55).abs() < 1e-9,
            "the worst, not the mean: {rate}"
        );
    }

    #[test]
    fn a_removed_workflow_does_not_hold_autonomous_hostage() {
        // The removed one is not going to run. Refusing over it would refuse a
        // charter for a capability it had already given up.
        let mut p = Profile::frontier("opus", &["investigate", "optimise"]);
        p.autonomous_threshold = 0.8;
        p.judgement
            .insert("optimise".into(), Judgement::Failed { rate: 0.1 });
        let a = admit(
            &a_charter(&["investigate", "optimise"]),
            Autonomy::Autonomous,
            &p,
        )
        .expect("investigate passes at 1.0 and optimise is gone");
        assert_eq!(a.autonomy, Autonomy::Autonomous);
        assert_eq!(a.removed.len(), 1);
    }

    #[test]
    fn a_new_session_defaults_to_notify() {
        // §12.10's third clause, and the user's decision of 2026-09-15.
        assert_eq!(Autonomy::default(), Autonomy::Notify);
        assert!(!Autonomy::default().item_waits_for_approval());
    }

    #[test]
    fn a_day_budget_is_pre_filled_rather_than_required() {
        // The other half of §12.10's third clause.
        assert!((SessionBudget::default().usd_per_day - DEFAULT_USD_PER_DAY).abs() < 1e-9);
        assert!(SessionBudget::default().usd_total > 0.0);
        assert_eq!(SessionBudget::default().max_runs, DEFAULT_MAX_RUNS);
    }

    #[test]
    fn autonomy_is_a_setting_on_starting_only() {
        // §12.10's adversarial clause, swept rather than sampled: under every
        // autonomy, every hard envelope still asks.
        for a in Autonomy::ALL {
            for e in AlwaysAsks::ALL {
                assert!(
                    e.asks(*a),
                    "`{}` stopped asking under `{}` — P6 is that autonomy governs \
                     *starting*, never *approving*",
                    e.as_str(),
                    a.as_str()
                );
            }
        }
    }

    #[test]
    fn ring_three_is_unrepresentable_rather_than_checked() {
        // §8: "ring 3 does not exist". Both halves: nothing in `AlwaysAsks`
        // names it, and `Ring` refuses it at the constructor.
        assert!(crate::harvest::Ring::parse(3).is_err());
        assert!(crate::harvest::Ring::parse(0).is_err());
        assert!(!AlwaysAsks::ALL.iter().any(|a| a.as_str().contains("ring")));
    }

    #[test]
    fn a_judgement_nobody_measured_has_no_rate_to_compare() {
        // `0 ?? x` is `0`. A rate of 0.0 for an unrun suite would read as "it
        // never passes" and be compared confidently against a threshold.
        assert_eq!(Judgement::NotMeasured.rate(), None);
        assert!(!Judgement::NotMeasured.passed());
        assert_eq!(Judgement::Failed { rate: 0.0 }.rate(), Some(0.0));
    }

    #[test]
    fn a_rate_that_is_not_a_number_is_not_a_measurement() {
        // NaN compares false against everything, so `worst < threshold` would
        // let it through and grant `autonomous` on a number that is not one.
        assert_eq!(Judgement::Passed { rate: f64::NAN }.rate(), None);
        assert_eq!(
            Judgement::Passed {
                rate: f64::INFINITY
            }
            .rate(),
            None
        );

        let mut p = Profile::frontier("opus", &["investigate"]);
        p.judgement
            .insert("investigate".into(), Judgement::Passed { rate: f64::NAN });
        let e = admit(&a_charter(&["investigate"]), Autonomy::Autonomous, &p)
            .expect_err("a NaN pass rate is not a pass rate");
        assert!(
            matches!(e, TierRefusal::AutonomousNotMeasured { .. }),
            "and it is refused under the true sentence rather than as a low score: {e}"
        );
        // The suite did say it passed, so the workflow is kept and `notify`
        // still works — the refusal is of the setting, not of the capability.
        let a = admit(&a_charter(&["investigate"]), Autonomy::Notify, &p)
            .expect("the suite said it passed");
        assert_eq!(a.charter.permitted_workflows, vec!["investigate"]);
    }
}
