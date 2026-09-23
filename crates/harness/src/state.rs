//! The working state the model **reads** instead of reconstructing.
//!
//! AGENT-008 §2. Set S task S-1.13.
//!
//! # The failure this replaces
//!
//! `drive.rs` pushed a `Section::WorkingState` block at nine separate sites, each
//! appending. So "what has happened so far" was a transcript the model had to
//! re-read and re-derive on every step, and it grew with the run — on a 7B, past
//! about step eight, the derivation is most of what the model is doing. The local
//! traces show the consequence directly: instruments re-listed, timeframes
//! re-chosen, the same decision made three times, because nothing in the prompt
//! said it had already been made.
//!
//! A [`WorkingState`] is one object the loop owns and **rewrites** each step. It
//! renders to exactly one block, at the top of the attention window, and it is
//! never appended to. That is the whole mechanism: the model is told where it is
//! rather than asked to work it out.
//!
//! # What is and is not wired yet
//!
//! The type is the contract five other Set S phases build on, so it carries every
//! field AGENT-008 §2 names — including the lifecycle fields
//! ([`Mode`], [`Workflow`], [`Ring`], [`Stage`], [`PreregRef`], [`Hypothesis`])
//! that AGENT-013 defines and Set S phase 10 enforces. **Declaring them here does
//! not enforce them.** They default to the narrowest reading
//! ([`Ring::Research`], [`Stage::Explore`], no preregistration) so that a phase
//! which has not yet been built cannot be mistaken for one that has.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::context::estimate_tokens;

// ── What kind of question this is ───────────────────────────────────────────

/// What the goal is asking for, decided deterministically at step 0 (§3.1).
///
/// The charter used to *plead* that many goals are lookups. A plea is not a
/// mechanism: a `Lookup` gets at most three read-only tools and no trials,
/// enforced, which is what stops a one-fact question turning into a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    /// One fact that already exists somewhere. No trials, ever.
    #[default]
    Lookup,
    /// A question that needs evidence produced.
    Research,
    /// Something is wrong and the run must find out what.
    Diagnose,
    /// Two or more things, held to the same measurement.
    Compare,
}

impl GoalKind {
    /// The exposure ceiling for this kind of goal (§3.1).
    #[must_use]
    pub fn max_tools(self) -> usize {
        match self {
            GoalKind::Lookup => 3,
            _ => usize::MAX,
        }
    }

    /// Whether a run of this kind may spend a trial at all.
    #[must_use]
    pub fn may_spend_trials(self) -> bool {
        self != GoalKind::Lookup
    }

    /// Classify a goal **before** planning (§3.1, task S-1.16).
    ///
    /// Deterministic, and deliberately so. The charter used to carry a plea —
    /// "many goals are lookups, do not plan five steps for one" — and a plea is
    /// not a mechanism: the 7B read it every turn and planned five steps anyway.
    /// This is the same sentence turned into something the loop can act on, at
    /// zero model calls.
    ///
    /// The ordering of the checks is the design. `Compare` is looked for first
    /// because "compare A and B" is also a research question and the narrower
    /// reading is the useful one; `Diagnose` before `Research` for the same
    /// reason. `Lookup` is **last and is the default**, because it is the only
    /// kind that withholds the trial budget, and a run that was going to be a
    /// lookup and is treated as research merely costs more, while the reverse
    /// spends trials on a question that never needed one.
    ///
    /// Wrong is survivable in one direction only, and this is the direction.
    #[must_use]
    pub fn classify(goal: &str, constraints: &[String]) -> Self {
        let text = format!("{goal} {}", constraints.join(" ")).to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));

        // "compare", "versus", "which of", "better than" — two or more things
        // held to one measurement.
        if has(&[
            "compare",
            " versus ",
            " vs ",
            "which of",
            "better than",
            "against each other",
        ]) {
            return GoalKind::Compare;
        }
        // Something is already wrong and the run must find out what.
        if has(&[
            "why is",
            "why did",
            "why does",
            "diagnose",
            "debug",
            "root cause",
            "went wrong",
            "is broken",
            "is failing",
            "regression",
            "investigate the",
        ]) {
            return GoalKind::Diagnose;
        }
        // A question that needs evidence *produced* — the only kind that earns the
        // trial budget. Two ways in, and the second needs two signals.
        //
        // The first is a verb that only makes sense as producing evidence:
        // backtesting, sweeping, optimising. Those are unambiguous.
        //
        // The second is the hard case, and one keyword cannot decide it. "Find
        // whether BTC-USD has momentum at 1h" and "find whether we hold 1m bars for
        // BTC-USD" are the same sentence shape and different kinds: the first has
        // to be *measured*, the second is a stored fact and a `Lookup`. What
        // separates them is the subject, so a market property alone is not enough
        // either — "what is the sharpe of run_42" reads a number the platform
        // already holds.
        //
        // So: an inquiry verb AND a market property. Both, or it stays a lookup.
        if has(&[
            "backtest",
            "sweep",
            "run an experiment",
            "hypothes",
            "optimi",
            "find an edge",
            "is there an edge",
            "strategy that",
        ]) {
            return GoalKind::Research;
        }
        let asks = has(&[
            "find whether",
            "test whether",
            "does ",
            "do we know whether",
            "is there",
            "measure",
            "evaluate",
            "how well",
        ]);
        let about_the_market = has(&[
            "momentum",
            "mean revers",
            "edge",
            "alpha",
            "predict",
            "correlat",
            "seasonal",
            "autocorrel",
            "overfit",
            "signal",
            "drawdown",
            "sharpe",
        ]);
        if asks && about_the_market {
            return GoalKind::Research;
        }
        GoalKind::Lookup
    }

    /// What "done" means for a goal of this kind, in the run's own words.
    ///
    /// Derived rather than asked for: the model is a poor judge of when it is
    /// finished, and a criterion it wrote is one it can rewrite.
    ///
    /// Every criterion comes back [`Met::Unevaluated`], and stays that way for the
    /// whole run. **Nothing in this harness evaluates an acceptance criterion.**
    /// The nearest thing is [`Loop::on_finish`](crate::drive::Loop), which checks
    /// that `finish_task`'s `evidence` cites ids the session actually produced —
    /// a check on attribution, not on whether the question was answered — and
    /// [`report::critic`](crate::report::critic), which checks that criteria *the
    /// model's own draft claims to have met* name evidence. Neither decides
    /// whether a criterion holds, and the second reads model prose, which is the
    /// class of claim this platform keeps pulling out of free-form output.
    ///
    /// So the honest status of every criterion here is "not yet evaluated", and
    /// [`Met`] has a variant that says exactly that rather than a `bool` that
    /// would have to say `false`.
    #[must_use]
    pub fn acceptance_for(self, goal: &str) -> Vec<AcceptanceCriterion> {
        let goal = goal.trim();
        let cite = "every number reported is in a tool result this run cited".to_string();
        let texts: Vec<String> = match self {
            GoalKind::Lookup => vec![
                format!("the question \"{goal}\" is answered in one sentence"),
                cite,
                "no trial was spent".to_string(),
            ],
            GoalKind::Research => vec![
                format!("a claim about \"{goal}\" is stated with its evidence"),
                cite,
                "the claim survives the null it was measured against".to_string(),
            ],
            GoalKind::Diagnose => vec![
                "the cause is named, not the symptom".to_string(),
                "a reproduction is given that a reader could follow".to_string(),
                cite,
            ],
            GoalKind::Compare => vec![
                "every item is measured the same way over the same window".to_string(),
                "the comparison states which is better AND by how much, with its uncertainty"
                    .to_string(),
                cite,
            ],
        };
        texts
            .into_iter()
            .map(AcceptanceCriterion::unevaluated)
            .collect()
    }
}

// ── Acceptance: three states, because two of them would be a lie ────────────

/// Whether an acceptance criterion holds.
///
/// AGENT-012 §3.5 asks the desk to render "acceptance criteria **with ticks**".
/// A tick is a measurement, and this platform has exactly one thing to say about
/// most criteria for most of a run: nobody has looked yet. `Unevaluated` is that
/// third state, and it is **structural, not a convenience**.
///
/// # Why not `Option<bool>`
///
/// Because `Option<bool>` has `unwrap_or(false)`, and the shortest path from
/// "not yet evaluated" to a rendered red cross is one method call that reads as
/// defensive programming. Collapsing the third state into `false` reports every
/// criterion nobody has looked at as *failing*, which is the same defect class
/// as an unwritten counter rendering as a measured zero — it shipped twice on
/// this platform in one day.
///
/// # The compile-time part
///
/// There is deliberately **no** `bool` anywhere on this type: no `From<Met> for
/// bool`, no `is_met()`, no `PartialEq<bool>`, no accessor returning
/// `Option<bool>`. The only way to get a value out is [`Met::fold`], which takes
/// one argument per state, so "render an unevaluated criterion as failed" cannot
/// be written by omission — a caller has to type the word `unevaluated` and
/// choose it on purpose. `met_never_grows_a_bool_escape_hatch` in
/// `tests/acceptance_and_budget_are_honest.rs` keeps it that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Met {
    /// Nobody has checked. The honest default, and the status of every criterion
    /// this harness produces today.
    #[default]
    Unevaluated,
    /// Checked, and it holds.
    Met,
    /// Checked, and it does not hold. **Checked** is the load-bearing word: this
    /// is a measurement that came back negative, never an absence of one.
    NotMet,
}

impl Met {
    /// Collapse to one value per state. Three arguments, no default, no `bool`.
    pub fn fold<T>(self, met: T, not_met: T, unevaluated: T) -> T {
        match self {
            Met::Met => met,
            Met::NotMet => not_met,
            Met::Unevaluated => unevaluated,
        }
    }

    /// How the card says it. Words rather than `[x]`/`[ ]`, because a blank box
    /// beside a ticked one reads as "failed" and two of these three states are
    /// not that.
    #[must_use]
    pub fn mark(self) -> &'static str {
        self.fold("met", "NOT met", "not yet evaluated")
    }
}

/// `met: true | false | null` on the wire. `null` is [`Met::Unevaluated`], and it
/// is sent rather than skipped: an absent key would let a consumer's `??` invent
/// the state that a missing measurement does not have.
impl Serialize for Met {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Met::Unevaluated => s.serialize_none(),
            Met::Met => s.serialize_some(&true),
            Met::NotMet => s.serialize_some(&false),
        }
    }
}

impl<'de> Deserialize<'de> for Met {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<bool>::deserialize(d)? {
            None => Met::Unevaluated,
            Some(true) => Met::Met,
            Some(false) => Met::NotMet,
        })
    }
}

/// One thing that has to be true for the run to be done, and whether it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub text: String,
    /// `null` on the wire until something evaluates it. Nothing does yet — see
    /// [`GoalKind::acceptance_for`].
    #[serde(default)]
    pub met: Met,
}

impl AcceptanceCriterion {
    /// A criterion nobody has checked, which is every criterion this harness
    /// currently produces.
    #[must_use]
    pub fn unevaluated(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            met: Met::Unevaluated,
        }
    }
}

/// Accepts the old wire shape — a bare `"string"` — as an unevaluated criterion.
///
/// Working states were persisted as `acceptance: ["…", "…"]` before `met`
/// existed. Those rows carry no evaluation, so they deserialize to exactly what
/// they are: text with no status. The alternative is a deploy that cannot read
/// yesterday's rows, which is how a schema change becomes an outage.
fn de_acceptance<'de, D>(d: D) -> Result<Vec<AcceptanceCriterion>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Text(String),
        Full(AcceptanceCriterion),
    }
    Ok(Vec::<Wire>::deserialize(d)?
        .into_iter()
        .map(|w| match w {
            Wire::Text(t) => AcceptanceCriterion::unevaluated(t),
            Wire::Full(c) => c,
        })
        .collect())
}

/// Chat or Research (AGENT-013 §2, ADR-0037). Two modes, not three.
///
/// Chat forces [`GoalKind::Lookup`]: a conversational turn does not get to spend
/// the trial budget, whatever it is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Chat,
    Research,
}

impl GoalKind {
    /// The workflow a Research run would follow for this kind of question
    /// (AGENT-013 §3).
    ///
    /// Only meaningful for the three kinds that need evidence produced.
    /// `Lookup` returns `None` rather than a default: a lookup does not need a
    /// Research run, and handing it `Investigate` would make the offer card
    /// describe work nobody asked for.
    ///
    /// `Research` maps to `Investigate` rather than to `Survey`, which is the
    /// cheaper neighbour: a survey takes no trials and answers "what is out
    /// there", and a question the classifier called `Research` is one that needs
    /// evidence produced. Offering the cheap one would be offering to not answer
    /// it.
    #[must_use]
    pub fn workflow(self) -> Option<Workflow> {
        match self {
            GoalKind::Lookup => None,
            GoalKind::Research => Some(Workflow::Investigate),
            GoalKind::Diagnose => Some(Workflow::Diagnose),
            GoalKind::Compare => Some(Workflow::Compare),
        }
    }
}

/// What shape of work this run is doing (AGENT-013 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    #[default]
    Investigate,
    Falsify,
    Replicate,
    Compare,
    Diagnose,
    Survey,
    Optimise,
}

impl Workflow {
    /// The name this serialises to, for a message or a column.
    ///
    /// Written out rather than round-tripped through `serde`, because a caller
    /// that needs a name in an error path should not be able to fail to produce
    /// one.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Workflow::Investigate => "investigate",
            Workflow::Falsify => "falsify",
            Workflow::Replicate => "replicate",
            Workflow::Compare => "compare",
            Workflow::Diagnose => "diagnose",
            Workflow::Survey => "survey",
            Workflow::Optimise => "optimise",
        }
    }

    /// Every workflow, for the exhaustiveness a per-tier table needs.
    pub const ALL: &'static [Workflow] = &[
        Workflow::Investigate,
        Workflow::Falsify,
        Workflow::Replicate,
        Workflow::Compare,
        Workflow::Diagnose,
        Workflow::Survey,
        Workflow::Optimise,
    ];

    /// The workflow a name refers to, or `None`.
    ///
    /// `None` rather than a default, because the two callers want opposite
    /// things from an unrecognised name: the session driver wants the run to
    /// carry no claim at all rather than to claim to be an investigation, and a
    /// route wants to refuse. A default here would make both of them wrong
    /// quietly.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|w| w.as_str() == name)
    }
}

/// How far from capital this run is allowed to reach (AGENT-013 §4).
///
/// Filters exposure *before* routing, so a tool outside the ring is not a tool
/// the model declines to use — it is a tool the model never sees. Rings 1 and 2
/// are reached only by a proposal a human decides; ring 3 has no door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ring {
    /// Research only. Nothing this run does reaches the platform's own artifacts.
    #[default]
    Research,
    /// May write platform artifacts (strategies, models, experiments).
    PlatformArtifacts,
    /// May reach paper trading.
    Paper,
}

/// The research lifecycle stage (AGENT-013 §5). Transitions are effects, never
/// something the model asserts about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Explore,
    Hypothesise,
    Preregister,
    Design,
    Run,
    Refine,
    Compare,
    Reproduce,
    Propose,
    Closed,
}

// ── The things a run accumulates ────────────────────────────────────────────

/// A sealed preregistration and its hash (AGENT-013 §7.2).
///
/// Its presence is what makes a trial-spending run legible afterwards: the
/// falsifier was written down before the first trial, so "it worked" and "we kept
/// looking until something worked" are distinguishable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreregRef {
    pub id: String,
    pub hash: String,
}

/// Whether the platform has decided about a hypothesis yet.
///
/// **Platform-written.** The model proposes the statement and the falsifier; it
/// never writes the status. That separation is the point (AGENT-013 §8.1) — a
/// model that can mark its own hypothesis `Supported` has no falsifier, it has a
/// formality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisStatus {
    #[default]
    Untested,
    Supported,
    Refuted,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub family_id: String,
    pub statement: String,
    /// What result would refute it. Written before the first trial or not at all.
    pub falsifier: String,
    pub role: String,
    pub status: HypothesisStatus,
}

/// Where a plan step has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "why")]
pub enum StepStatus {
    Pending,
    Active,
    Done,
    Skipped(String),
    Failed(String),
}

impl StepStatus {
    fn mark(&self) -> &'static str {
        match self {
            StepStatus::Pending => "[ ]",
            StepStatus::Active => "[>]",
            StepStatus::Done => "[x]",
            StepStatus::Skipped(_) => "[-]",
            StepStatus::Failed(_) => "[!]",
        }
    }
}

/// What a plan step *does*. AGENT-008 §3.2: plans are made of tools.
///
/// A step of free-form prose is a step nothing can bind to, and the local traces
/// show what follows — the planner emits "analyse the results", the router has no
/// namespace to route to, and the step is spent discovering that. A step that
/// cannot name a catalogue entry names `search_tools` instead, which is a real
/// move with a real result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum StepKind {
    Tool { name: String },
    Workflow { name: String },
    Reflect,
    AskUser,
}

impl StepKind {
    /// The catalogue tool this step will call, when it names one.
    ///
    /// `Reflect` and `AskUser` name none, and that is not a missing value — they
    /// are the two steps the loop serves itself.
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            StepKind::Tool { name } | StepKind::Workflow { name } => Some(name),
            StepKind::Reflect | StepKind::AskUser => None,
        }
    }

    fn label(&self) -> String {
        match self {
            StepKind::Tool { name } | StepKind::Workflow { name } => name.clone(),
            StepKind::Reflect => "reflect".to_string(),
            StepKind::AskUser => "ask_user".to_string(),
        }
    }
}

/// One step of a harness-owned plan, in the working state's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedStep {
    pub goal: String,
    pub kind: StepKind,
    pub status: StepStatus,
}

/// An id a tool result produced, with where it came from.
///
/// The provenance is not decoration: it is what `finish_task` checks a citation
/// against. An id the model produces that no tool returned is a fabrication, and
/// the only way to tell is to have recorded what was actually seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub id: String,
    pub tool: String,
    pub step: u32,
}

/// A verified finding (AGENT-007 §3.1). The id, not the prose — the prose is in
/// the memory store, written by a tool from a verified result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingRef {
    pub id: String,
    pub headline: String,
    pub step: u32,
}

/// The most recent rejection, rendered **once**.
///
/// Rendered once because a correction that persists across steps reads as a
/// standing instruction, and the model starts answering it instead of the goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub code: String,
    pub message: String,
    /// An edit the caller can make. Not a restatement of the problem.
    pub fix: String,
    pub step: u32,
}

/// What a mid-run message from the user is asking for (AGENT-008 §3.4a,
/// Set S task S-2.14).
///
/// Classified **deterministically**, before any model call. A steer that needed a
/// model call to interpret would cost a step to read and could be misread, and the
/// thing being interpreted is a sentence the user just typed — the cheapest possible
/// input to be careful with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Steer {
    /// Narrows the existing goal. The plan stands.
    Constraint,
    /// Changes what is being asked. Re-classify, rewrite acceptance, abandon the
    /// remaining steps with a reason, re-plan — and **keep the findings**, which
    /// were true about the data rather than about the goal.
    Redirection,
    /// Neither. Encouragement, a question about progress, thinking aloud.
    Comment,
}

impl Steer {
    /// Read a steer from what the user actually wrote.
    ///
    /// **Ambiguity resolves to `Redirection`,** and the asymmetry is the whole
    /// design: an unnecessary re-plan costs one step, and a missed one costs a run
    /// aimed at the wrong question. The model is told which reading was taken, so a
    /// wrong call is one sentence to correct rather than something to discover at
    /// the end.
    ///
    /// `Comment` is the narrow case and has to be *recognised*, not defaulted to —
    /// treating "looking good" as a redirection would re-plan a healthy run, and
    /// treating a redirection as a comment is the failure this exists to prevent.
    #[must_use]
    pub fn classify(text: &str) -> Self {
        let t = text.trim().to_lowercase();
        if t.is_empty() {
            return Steer::Comment;
        }
        let has = |words: &[&str]| words.iter().any(|w| t.contains(w));

        // A redirection names a different subject or revokes the current one.
        if has(&[
            "instead",
            "actually",
            "forget",
            "never mind",
            "nevermind",
            "stop and",
            "change to",
            "switch to",
            "different question",
            "not what i",
            "i meant",
            "new goal",
            "rather than",
            "drop that",
            "abandon",
        ]) {
            return Steer::Redirection;
        }
        // A constraint narrows: same question, smaller. These are checked BEFORE the
        // comment words, because "just use 1h" contains "just" and is not a comment.
        if has(&[
            "only ",
            "just use",
            "restrict",
            "limit it",
            "narrow",
            "focus on",
            "stick to",
            "use the ",
            "on 1h",
            "on 4h",
            "exclude",
            "ignore the ",
            "without ",
        ]) {
            return Steer::Constraint;
        }
        // A comment has to look like one. Deliberately the smallest list here.
        if has(&[
            "looking good",
            "nice",
            "thanks",
            "ok",
            "okay",
            "sounds good",
            "keep going",
            "carry on",
            "how is it going",
            "how's it going",
            "what are you doing",
        ]) {
            return Steer::Comment;
        }
        // Everything else. See the doc comment: this is the cheap error.
        Steer::Redirection
    }

    /// What the loop tells the model it decided, so a wrong reading is one sentence
    /// to correct rather than something to find out about at the end.
    #[must_use]
    pub fn correction(self, text: &str) -> String {
        match self {
            Steer::Constraint => format!(
                "The user narrowed the task: \"{text}\". Same goal, smaller. \
                 Keep the plan and apply the constraint from here."
            ),
            Steer::Redirection => format!(
                "The user changed the task: \"{text}\". Re-plan for the new \
                 question. Your findings so far are KEPT - they were true about the data, \
                 not about the goal - but the remaining plan steps are abandoned. If this \
                 was only a narrowing and not a change, say so and carry on."
            ),
            Steer::Comment => {
                format!("The user said: \"{text}\". Nothing to change. Carry on.")
            }
        }
    }
}

/// A choice this run has already made, and why (§3.6).
///
/// Re-asking a decided question is the dominant waste in the local traces:
/// instruments re-listed, timeframes re-chosen, the same tool re-searched. The
/// reason is stored with the choice so that changing it is possible and
/// deliberate — the correction says "decided at step 4: …; say why it should
/// change or move on", not "you already decided".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub question: String,
    pub choice: String,
    pub reason: String,
    pub step: u32,
}

/// What is left to spend, and out of what.
///
/// # A cap and a remainder are two different facts
///
/// AGENT-012 §6.2 wants four bars, each showing "used of cap". `used` is
/// `cap - remaining`, so a bar needs **both** numbers and a consumer handed only
/// one will reach for the other. It did: the desk computed
/// `tokensCap = tokens_remaining`, made `used` identically zero, and printed
/// `0 / 40,000` for a run that had burned half a million tokens. The trials bar
/// did the same trick backwards and printed `4 / 4 · 90%+` for a run nothing had
/// capped.
///
/// Neither was the desk's invention to make: a cap that exists belongs on the
/// wire, and a cap that does not exist has to be **visibly** absent so nothing
/// downstream can substitute a number that is lying around.
///
/// So every axis here is two fields, and every one that is not measured or not
/// capped is `Option::None` — `null` on the wire, never `0`.
///
/// # What is actually known
///
/// | axis | cap | burn |
/// |---|---|---|
/// | steps | `orchestration.max_steps`, enforced by the loop | written every step |
/// | tokens | `budgets.max_tokens_per_task` | **not counted anywhere** (OQ-16) |
/// | dollars | refused: no price table (ADR-D-03 / OQ-16) | not counted |
/// | trials | **does not exist** in [`profile::Budgets`](crate::profile::Budgets) | written by whoever owns the state; not by this loop |
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct BudgetView {
    pub steps_remaining: u32,
    /// The step cap (`orchestration.max_steps`). Not an `Option`: the loop
    /// refuses to run without one and enforces it every step, so a step budget is
    /// the one axis that is always both capped and counted.
    pub steps_total: u32,
    /// `budgets.max_tokens_per_task`, or `None` when the profile declares no
    /// usable ceiling.
    ///
    /// A real cap, which is why it is here — but see `tokens_remaining`: knowing
    /// the ceiling is not knowing the burn, and a renderer must not derive one
    /// from the other.
    #[serde(default)]
    pub tokens_cap: Option<u64>,
    /// Tokens left of that cap, and **always `None` today**.
    ///
    /// Nothing in this loop counts tokens (OQ-16). It was a `u32` that no code
    /// path ever assigned, so it read `0` — and `0` next to a cap means "the
    /// whole budget is spent", which is the opposite of what an uncounted axis
    /// should say. `None` is the measurement that was not taken.
    #[serde(default)]
    pub tokens_remaining: Option<u64>,
    /// The dollar cap. **Refused**, so always `None`.
    ///
    /// `budgets.max_usd_per_task` is declared in every profile and nothing on
    /// this platform converts tokens or seconds into money (OQ-16). ADR-D-03
    /// settles that with a static, versioned price table where an unverified or
    /// stale row is treated as absent everywhere a cap is *reported*; until a
    /// fresh row exists for the model in question there is no dollar figure to
    /// send, and a number sent anyway would be read as a measurement.
    #[serde(default)]
    pub usd_tenth_cents_cap: Option<i64>,
    /// Dollars left, in tenths of a cent so the budget is integer arithmetic —
    /// money is never `f64` here or anywhere (SC-1). **Always `None`**: same
    /// missing price table, and nothing counts spend per run either.
    #[serde(default)]
    pub usd_tenth_cents_remaining: Option<i64>,
    /// The trials cap. **Always `None`, because no such cap exists here.**
    ///
    /// ADR-D-08 assumed one in [`profile::Budgets`](crate::profile::Budgets);
    /// that struct holds `max_usd_per_task`, `max_wall_clock_s` (dead),
    /// `max_tokens_per_task` and `ask_above_usd`, and nothing else. The trial
    /// ceilings on this platform are session- and campaign-scoped and live in the
    /// API, not in a profile, so a `Loop` built from a profile has not been told
    /// one. The field is here to say that in the one place the desk looks, so the
    /// bar renders uncapped instead of reaching for `trials_spent` and printing
    /// `4 / 4`.
    #[serde(default)]
    pub trials_cap: Option<u32>,
    /// Trials this run has spent. Written by whoever owns the state — **not by
    /// this loop**, which has no trial counter of its own.
    pub trials_spent: u32,
    /// Effective sample size after selection-bias correction. Not money and not a
    /// size: a statistic, so a float is the right type.
    pub n_eff: Option<f64>,
}

// ── The state ───────────────────────────────────────────────────────────────

/// Everything the model needs to know about where it is.
///
/// Owned by `Loop`, rewritten each step, rendered to exactly one
/// `Section::Subtask` block at priority 255.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkingState {
    pub goal: String,
    pub kind: GoalKind,
    pub mode: Mode,
    pub workflow: Workflow,
    pub ring: Ring,
    pub stage: Stage,
    pub prereg: Option<PreregRef>,
    /// What "done" means for this run, derived from the goal and the
    /// constraints, each carrying its status.
    ///
    /// Every status is [`Met::Unevaluated`] today — see
    /// [`GoalKind::acceptance_for`] for what does and does not evaluate these.
    #[serde(default, deserialize_with = "de_acceptance")]
    pub acceptance: Vec<AcceptanceCriterion>,
    pub hypothesis: Option<Hypothesis>,
    pub plan: Vec<PlannedStep>,
    pub evidence: Vec<EvidenceRef>,
    pub findings: Vec<FindingRef>,
    pub open_questions: Vec<String>,
    pub last_error: Option<Correction>,
    pub budget: BudgetView,
    pub decided: Vec<Decision>,
}

/// How much of the state a tier gets to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// ≤ 400 tokens: goal, active step, last error, budget, the three most recent
    /// evidence ids, the findings count. Everything a decision needs and nothing
    /// a decision does not.
    Flat,
    /// The card, plus the plan, the hypothesis and the open questions.
    Rich,
}

/// The token ceiling on a flat card (§2).
pub const FLAT_CARD_TOKENS: u32 = 400;

impl WorkingState {
    #[must_use]
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            ..Self::default()
        }
    }

    /// The index of the first step that is not finished, if any.
    #[must_use]
    pub fn active_step(&self) -> Option<(usize, &PlannedStep)> {
        self.plan
            .iter()
            .enumerate()
            .find(|(_, s)| matches!(s.status, StepStatus::Active | StepStatus::Pending))
    }

    /// Has this run already decided this question?
    ///
    /// Matched case-insensitively on the question text because the model does not
    /// re-ask a question in the same words — that is precisely why it does not
    /// notice it is re-asking.
    #[must_use]
    pub fn already_decided(&self, question: &str) -> Option<&Decision> {
        let q = question.trim().to_lowercase();
        self.decided
            .iter()
            .find(|d| d.question.trim().to_lowercase() == q)
    }

    /// The correction for re-asking a settled question (§3.6).
    #[must_use]
    pub fn redecision_correction(&self, question: &str) -> Option<String> {
        self.already_decided(question).map(|d| {
            format!(
                "decided at step {}: {} ({}); say why it should change or move on",
                d.step, d.choice, d.reason
            )
        })
    }

    /// Record a choice. Re-recording the same question **updates** it rather than
    /// appending, so `decided` cannot grow without bound on a loop that keeps
    /// changing its mind.
    pub fn decide(
        &mut self,
        question: impl Into<String>,
        choice: impl Into<String>,
        reason: impl Into<String>,
        step: u32,
    ) {
        let question = question.into();
        let entry = Decision {
            question,
            choice: choice.into(),
            reason: reason.into(),
            step,
        };
        match self
            .decided
            .iter_mut()
            .find(|d| d.question == entry.question)
        {
            Some(existing) => *existing = entry,
            None => self.decided.push(entry),
        }
    }

    /// Record an id a tool returned. Duplicates are ignored: the same id seen
    /// twice is one piece of evidence.
    ///
    /// Named `record_evidence` rather than `observe` because
    /// `invariants::no_metric_name_embeds_an_instrument_id` scans for
    /// `observe(format!(…))` — the shape that puts an instrument id into a metric
    /// name and melts the metrics backend (§16.1). This records evidence, not a
    /// metric, but a scanner that has to tell them apart by context is a scanner
    /// that will get it wrong in the other direction one day. Cheaper to not use
    /// the word.
    pub fn record_evidence(&mut self, id: impl Into<String>, tool: impl Into<String>, step: u32) {
        let id = id.into();
        if self.evidence.iter().any(|e| e.id == id) {
            return;
        }
        self.evidence.push(EvidenceRef {
            id,
            tool: tool.into(),
            step,
        });
    }

    /// Whether an id the model wants to cite was actually produced this run.
    #[must_use]
    pub fn can_cite(&self, id: &str) -> bool {
        self.evidence.iter().any(|e| e.id == id)
    }

    /// The anchor re-stated immediately before every decision.
    ///
    /// Derived from the state rather than built beside it. It was built beside it,
    /// and the two could disagree — the anchor said "plan step 3 of 5" from one
    /// cursor while the working-state block described a different step from
    /// another. Two renderings of one fact is one rendering too many.
    #[must_use]
    pub fn anchor(&self) -> String {
        let (position, step_goal) = match self.active_step() {
            Some((i, s)) => (
                format!(" (plan step {} of {})", i + 1, self.plan.len()),
                s.goal.clone(),
            ),
            None => (String::new(), self.goal.clone()),
        };
        let established = if self.findings.is_empty() {
            String::new()
        } else {
            format!("\nEstablished so far: {} finding(s).", self.findings.len())
        };
        format!(
            "THE TASK YOU WERE GIVEN: {}\nTHIS STEP{}: {}{}",
            self.goal, position, step_goal, established
        )
    }

    /// Render the card. Bounded by construction at [`Detail::Flat`]: the pieces
    /// are chosen and each is clamped, rather than a long render being cut at the
    /// end — a truncated card loses the budget line, which is the part that stops
    /// a run continuing past its means.
    #[must_use]
    pub fn render(&self, detail: Detail) -> String {
        let mut out = String::with_capacity(1_024);

        out.push_str("## Where you are\n\n");
        out.push_str(&format!("GOAL: {}\n", clamp(&self.goal, 240)));
        out.push_str(&format!(
            "KIND: {} · mode {} · workflow {} · ring {} · stage {}\n",
            json_name(&self.kind),
            json_name(&self.mode),
            json_name(&self.workflow),
            json_name(&self.ring),
            json_name(&self.stage),
        ));

        match self.active_step() {
            Some((i, s)) => out.push_str(&format!(
                "NOW: step {} of {} — {} (via {})\n",
                i + 1,
                self.plan.len(),
                clamp(&s.goal, 160),
                s.kind.label()
            )),
            None if self.plan.is_empty() => out.push_str("NOW: no plan yet\n"),
            None => out.push_str("NOW: every planned step is finished\n"),
        }

        let b = &self.budget;
        out.push_str(&format!(
            "BUDGET: {}/{} steps left · {} trials spent{}\n",
            b.steps_remaining,
            b.steps_total,
            b.trials_spent,
            match b.n_eff {
                Some(n) => format!(" · n_eff {n:.1}"),
                None => String::new(),
            }
        ));

        if let Some(e) = &self.last_error {
            out.push_str(&format!(
                "LAST REJECTION (step {}): {} — {}\n",
                e.step,
                e.code,
                clamp(&e.message, 200),
            ));
            // Only when it says something the message did not. A rejection whose
            // message already names the edit — most of the validation ladder's do —
            // would otherwise print the same sentence twice under two headings, and
            // a model reading it twice weighs it twice.
            if e.fix.trim() != e.message.trim() && !e.fix.trim().is_empty() {
                out.push_str(&format!("FIX: {}\n", clamp(&e.fix, 200)));
            }
        }

        if !self.evidence.is_empty() {
            let recent: Vec<String> = self
                .evidence
                .iter()
                .rev()
                .take(3)
                .map(|e| format!("{} (from {})", e.id, e.tool))
                .collect();
            out.push_str(&format!(
                "EVIDENCE: {} id(s); most recent {}\n",
                self.evidence.len(),
                recent.join(", ")
            ));
        }
        out.push_str(&format!("FINDINGS: {}\n", self.findings.len()));

        if !self.decided.is_empty() {
            out.push_str("ALREADY DECIDED (do not re-ask):\n");
            for d in self
                .decided
                .iter()
                .rev()
                .take(if detail == Detail::Flat { 3 } else { 12 })
            {
                out.push_str(&format!(
                    "  - {}: {} (step {})\n",
                    clamp(&d.question, 80),
                    clamp(&d.choice, 80),
                    d.step
                ));
            }
        }

        if detail == Detail::Flat {
            return out;
        }

        if !self.acceptance.is_empty() {
            out.push_str("\nDONE MEANS:\n");
            for a in &self.acceptance {
                out.push_str(&format!("  - [{}] {}\n", a.met.mark(), clamp(&a.text, 200)));
            }
        }
        if !self.plan.is_empty() {
            out.push_str("\nPLAN:\n");
            for (i, s) in self.plan.iter().enumerate() {
                out.push_str(&format!(
                    "  {} {}. {} → {}\n",
                    s.status.mark(),
                    i + 1,
                    clamp(&s.goal, 160),
                    s.kind.label()
                ));
                match &s.status {
                    StepStatus::Skipped(why) | StepStatus::Failed(why) => {
                        out.push_str(&format!("        {}\n", clamp(why, 160)));
                    }
                    _ => {}
                }
            }
        }
        if let Some(h) = &self.hypothesis {
            out.push_str(&format!(
                "\nHYPOTHESIS ({}): {}\nREFUTED IF: {}\n",
                json_name(&h.status),
                clamp(&h.statement, 300),
                clamp(&h.falsifier, 300)
            ));
        }
        if let Some(p) = &self.prereg {
            out.push_str(&format!("PREREGISTERED: {} ({})\n", p.id, &p.hash));
        }
        if !self.open_questions.is_empty() {
            out.push_str("\nOPEN QUESTIONS:\n");
            for q in &self.open_questions {
                out.push_str(&format!("  - {}\n", clamp(q, 200)));
            }
        }
        out
    }

    /// The card for a profile tier, already bounded.
    #[must_use]
    pub fn card(&self, detail: Detail) -> String {
        let rendered = self.render(detail);
        if detail == Detail::Rich || estimate_tokens(&rendered) <= FLAT_CARD_TOKENS {
            return rendered;
        }
        // A flat card that still does not fit is a state with an unusually long
        // goal or correction. Drop from the bottom, where the least
        // decision-relevant lines are, and say so — silently short state is the
        // thing this module exists to stop.
        let mut kept = String::new();
        for line in rendered.lines() {
            let candidate = format!("{kept}{line}\n");
            if estimate_tokens(&candidate) > FLAT_CARD_TOKENS.saturating_sub(12) {
                kept.push_str("… (state truncated; ask for the full card)\n");
                break;
            }
            kept = candidate;
        }
        kept
    }
}

fn clamp(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.len() <= max {
        return t.to_string();
    }
    let mut cut = max.saturating_sub(1);
    while cut > 0 && !t.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &t[..cut])
}

/// The serde name of a unit-ish enum, so the card and the wire agree on spelling.
fn json_name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other
            .as_object()
            .and_then(|o| o.keys().next().cloned())
            .unwrap_or_else(|| other.to_string()),
        Err(_) => String::new(),
    }
}

/// The evidence map a `ToolResult`'s `ids` field contributes, folded into state.
pub fn record_evidence_ids(
    state: &mut WorkingState,
    ids: &BTreeMap<String, String>,
    tool: &str,
    step: u32,
) {
    for value in ids.values() {
        state.record_evidence(value.clone(), tool, step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WorkingState {
        let mut s = WorkingState::new("find the best timeframe for BTC momentum");
        s.kind = GoalKind::Research;
        s.mode = Mode::Research;
        s.budget = BudgetView {
            steps_remaining: 9,
            steps_total: 15,
            trials_spent: 2,
            ..Default::default()
        };
        s.plan = vec![
            PlannedStep {
                goal: "list what data exists".into(),
                kind: StepKind::Tool {
                    name: "list_instruments".into(),
                },
                status: StepStatus::Done,
            },
            PlannedStep {
                goal: "sweep the parameters".into(),
                kind: StepKind::Tool {
                    name: "run_sweep".into(),
                },
                status: StepStatus::Active,
            },
        ];
        s
    }

    #[test]
    fn the_card_says_where_the_run_is() {
        let s = state();
        let card = s.render(Detail::Flat);
        assert!(card.contains("GOAL: find the best timeframe"));
        assert!(card.contains("NOW: step 2 of 2 — sweep the parameters (via run_sweep)"));
        assert!(card.contains("9/15 steps left"));
        assert!(card.contains("2 trials spent"));
    }

    /// The property the whole module exists for: rewriting, not appending.
    ///
    /// Adversarial — it drives twenty updates through the state and asserts the
    /// render does not grow with them, which is exactly what the nine append sites
    /// in `drive.rs` did.
    #[test]
    fn the_card_stops_growing_once_it_is_populated() {
        fn drive(steps: u32) -> WorkingState {
            let mut s = state();
            for step in 0..steps {
                s.record_evidence(format!("bt-{step}"), "run_sweep", step);
                s.decide("timeframe", "1h", "the only one with coverage", step);
                s.last_error = Some(Correction {
                    code: "window_exceeds_coverage".into(),
                    message: "asked before the first bar".into(),
                    fix: "start at 2024-03-01".into(),
                    step,
                });
            }
            s
        }

        let short = drive(5);
        let long = drive(200);
        assert_eq!(
            long.decided.len(),
            1,
            "re-deciding the same question updates, not appends"
        );
        assert_eq!(long.evidence.len(), 200, "the ids themselves are all kept");

        // Forty times the history, and the card the model reads is the same size
        // to within the width of a few step numbers. That is the property: state
        // is a fixed-size view over an unbounded history, not the history.
        let a = short.card(Detail::Flat).len();
        let b = long.card(Detail::Flat).len();
        assert!(
            b.abs_diff(a) < 40,
            "the card went from {a} to {b} bytes while the history grew 40x — \
             it is accumulating, which is the bug this type replaces"
        );
        assert!(estimate_tokens(&long.card(Detail::Flat)) <= FLAT_CARD_TOKENS);

        // The rich card grows only in the plan, which is bounded by the plan.
        assert!(estimate_tokens(&long.card(Detail::Rich)) < 1_000);
    }

    /// The bound holds even when the inputs are hostile.
    #[test]
    fn a_flat_card_is_bounded_however_long_the_goal_is() {
        let mut s = state();
        s.goal = "why ".repeat(4_000);
        s.open_questions = (0..50)
            .map(|i| format!("question {i} ").repeat(40))
            .collect();
        s.acceptance = (0..50)
            .map(|i| AcceptanceCriterion::unevaluated(format!("criterion {i} ").repeat(40)))
            .collect();
        let card = s.card(Detail::Flat);
        assert!(
            estimate_tokens(&card) <= FLAT_CARD_TOKENS,
            "flat card is {} tokens",
            estimate_tokens(&card)
        );
        assert!(card.contains("GOAL:"), "the goal survives the clamp");
    }

    #[test]
    fn the_rich_card_carries_the_plan_and_the_hypothesis() {
        let mut s = state();
        s.hypothesis = Some(Hypothesis {
            family_id: "f1".into(),
            statement: "momentum survives costs on 1h".into(),
            falsifier: "median Sharpe <= 0 after the cost ladder".into(),
            role: "primary".into(),
            status: HypothesisStatus::Untested,
        });
        s.open_questions = vec!["does it hold on 4h".into()];
        let card = s.render(Detail::Rich);
        assert!(card.contains("[x] 1. list what data exists → list_instruments"));
        assert!(card.contains("[>] 2. sweep the parameters → run_sweep"));
        assert!(card.contains("HYPOTHESIS (untested)"));
        assert!(card.contains("REFUTED IF: median Sharpe"));
        assert!(card.contains("does it hold on 4h"));
        // And the flat card does not, which is what makes it flat.
        assert!(!s.render(Detail::Flat).contains("HYPOTHESIS"));
    }

    /// §3.6: re-asking a settled question is corrected with the step number.
    #[test]
    fn re_asking_a_decided_question_is_corrected() {
        let mut s = state();
        s.decide("which timeframe", "1h", "the only one with coverage", 4);
        let c = s
            .redecision_correction("Which Timeframe")
            .expect("matched case-insensitively");
        assert!(c.starts_with("decided at step 4: 1h"));
        assert!(c.contains("say why it should change or move on"));
        assert!(s.redecision_correction("which instrument").is_none());
    }

    /// Adversarial: an id no tool produced cannot be cited.
    ///
    /// This is the check `finish_task` makes, and the reason `EvidenceRef` records
    /// the tool and the step rather than being a set of strings.
    #[test]
    fn an_id_no_tool_returned_cannot_be_cited() {
        let mut s = state();
        s.record_evidence("bt-real", "get_backtest", 3);
        assert!(s.can_cite("bt-real"));
        assert!(
            !s.can_cite("bt-invented"),
            "an unattributable id must not pass"
        );
        // Seeing it twice is still one piece of evidence.
        s.record_evidence("bt-real", "get_backtest", 5);
        assert_eq!(s.evidence.len(), 1);
    }

    /// A lookup gets three tools and no trials — the mechanism the charter's plea
    /// was standing in for (§3.1).
    #[test]
    fn a_lookup_may_not_spend_trials() {
        assert_eq!(GoalKind::Lookup.max_tools(), 3);
        assert!(!GoalKind::Lookup.may_spend_trials());
        for kind in [GoalKind::Research, GoalKind::Diagnose, GoalKind::Compare] {
            assert!(kind.may_spend_trials(), "{kind:?}");
            assert!(kind.max_tools() > 3);
        }
    }

    /// The defaults are the narrowest reading: an unset state cannot be mistaken
    /// for an authorised one.
    #[test]
    fn an_unset_state_authorises_nothing() {
        let s = WorkingState::new("g");
        assert_eq!(s.ring, Ring::Research, "never default to reaching paper");
        assert_eq!(s.mode, Mode::Chat);
        assert_eq!(s.kind, GoalKind::Lookup, "and so: no trials by default");
        assert_eq!(s.stage, Stage::Explore);
        assert!(s.prereg.is_none());
        assert!(s.hypothesis.is_none());
        assert!(!s.kind.may_spend_trials());
    }

    #[test]
    fn the_anchor_is_derived_from_the_state() {
        let s = state();
        let a = s.anchor();
        assert!(a.contains("THE TASK YOU WERE GIVEN: find the best timeframe"));
        assert!(a.contains("THIS STEP (plan step 2 of 2): sweep the parameters"));
        assert!(!a.contains("Established so far"), "no findings yet");
    }
}
