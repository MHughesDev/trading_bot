//! The workflow tier: a fixed battery behind one tool call (AGENT-006 §2.2, §6;
//! Set S task S-8.1).
//!
//! # What a workflow is, and what it is not
//!
//! A workflow is a **server-side** sequence of typed calls with decision points,
//! exposed as one tool. The model cannot tell it from a typed tool: same
//! declaration, same gate, same `ToolResult`. `Tier::Workflow` existed in the
//! catalogue's vocabulary before this file and nothing had ever been declared
//! with it, which is the same defect shape as a tool with no caller — a category
//! that describes nothing.
//!
//! It is not a prompt, a checklist, or a suggestion. The point of moving the
//! battery to the server is that its *steps are not negotiable*: a model asked to
//! "run the leakage checks" runs the ones it remembers, in the order it thinks of
//! them, and stops when one of them comes back clean. A battery runs all of them
//! every time, including the ones that would have embarrassed it.
//!
//! # One battery, one family
//!
//! Every step of a battery is a look, and every look is a multiple comparison. A
//! family-wise correction computed over a denominator that does not contain
//! them is a correction over the wrong family, which reads as a pass. So the
//! engine mints a family id **before the first step is admitted** and every step
//! reserves under it ([`crate::dispatch_in_family`], migration `0134`).
//!
//! The family is minted per battery rather than per run, because a run takes
//! many batteries: one family per run would over-correct, which is wrong in the
//! other direction and just as hard to notice.
//!
//! # The trial count is the sum of the steps'
//!
//! A workflow declares [`TrialKind::None`] itself. It spends nothing; its steps
//! spend exactly what they would have spent had the model called them one at a
//! time. A workflow that also declared a trial would charge twice for the same
//! compute and make the exploration floor's denominator wrong — and a workflow
//! that suppressed its steps' trials would be a way to run the simulator for
//! free, which is the one thing INV-16 exists to prevent.
//!
//! # A battery with a step nobody built is not offered
//!
//! [`Stage::Owed`] is how a battery says a step the spec requires has no tool
//! behind it yet. A workflow with any owed stage is **absent from the
//! catalogue** — not exposed and greyed out, not exposed with the step skipped,
//! absent. A five-step battery silently running four steps and emitting a
//! verdict is worse than no battery: it produces a verdict *shaped* like the one
//! the spec describes, and nothing downstream can tell the difference.
//!
//! [`owed`] is the list, and it is a public API so the plan and the tests read
//! the same one.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::spec::{Tier, ToolError, ToolResult, ToolSpec, TrialKind};
use crate::taxonomy::Risk;

pub mod batteries;

// ── Declaring a battery ──────────────────────────────────────────────────────

/// A value a step pins, which the model cannot set.
///
/// The pinned half is most of the point: `run_study`'s `kind` and `metric` are
/// what make a step *the cost shock* rather than whatever the caller felt like
/// running, and a battery whose steps are negotiable is a battery in name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg {
    Str(&'static str),
    Bool(bool),
    Int(i64),
    /// An id unique to this step of this battery: `<family>-<n>`.
    ///
    /// Derived rather than random so a re-run of the same battery over the same
    /// family is the same set of ids — an id nobody can reproduce is an id
    /// nobody can join evidence to.
    StepId,
}

impl Arg {
    fn render(self, family: &str, index: usize) -> Value {
        match self {
            Self::Str(s) => json!(s),
            Self::Bool(b) => json!(b),
            Self::Int(i) => json!(i),
            Self::StepId => json!(format!("{family}-{index}")),
        }
    }
}

/// When a step runs — §2.2's "decision points".
///
/// Deliberately small. A decision point is a *declared* branch on evidence the
/// battery already has, not a place for the server to improvise: the two
/// conditions here are "the caller said this applies" and "an earlier step
/// found this", and neither can be talked into being true.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Always,
    /// Run only when the workflow's own arguments set this flag true.
    ///
    /// The leakage audit's LLM-contamination step: it applies when the feature
    /// set contains text, and the caller is the only party that knows.
    IfArgument(&'static str),
    /// Run only when an earlier step reported this field truthy in its detail.
    IfEarlier {
        step: &'static str,
        field: &'static str,
    },
}

/// One call of a battery.
pub struct Step {
    pub tool: &'static str,
    /// What this step is *for*, in the battery's language rather than the
    /// tool's. `run_study` appears four times in `precapital_battery` and the
    /// label is the only thing that says which is the cost shock.
    pub label: &'static str,
    /// Workflow arguments forwarded verbatim, by name.
    pub forward: &'static [&'static str],
    /// `(step parameter, workflow argument)` for arguments whose names differ.
    pub rename: &'static [(&'static str, &'static str)],
    /// Parameters the battery fixes. The model cannot set these.
    pub pinned: &'static [(&'static str, Arg)],
    pub when: When,
}

/// One position in a battery: a call, or a call nobody has built.
pub enum Stage {
    Call(Step),
    /// A step the spec requires and no tool performs.
    ///
    /// Written down rather than omitted, so the battery's declaration is the
    /// spec's battery and the gap is visible where the battery is. A quietly
    /// shortened list reads as a complete one.
    Owed {
        label: &'static str,
        /// What would have to exist. Named, so the next person can build it.
        needs: &'static str,
        /// Why the verdict cannot honestly be emitted without it.
        why: &'static str,
        /// When this step would have applied.
        ///
        /// [`When::Always`] makes the whole battery unrunnable, which is the
        /// common case. A **conditional** owed stage does not: a step that only
        /// applies to text features is not a gap for a feature set that has
        /// none, and treating it as one would withhold a battery that could
        /// have run for every input it actually covers.
        ///
        /// The refusal moves rather than disappearing — [`run`] refuses when the
        /// condition *holds* for the arguments it was given, naming the step.
        /// So a leakage audit over a text feature set is still refused, and one
        /// over a price feature set runs.
        when: When,
    },
}

impl Stage {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Call(s) => s.label,
            Self::Owed { label, .. } => label,
        }
    }

    /// When this stage applies.
    #[must_use]
    pub fn when(&self) -> When {
        match self {
            Self::Call(s) => s.when,
            Self::Owed { when, .. } => *when,
        }
    }
}

/// What a battery concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// One of the workflow's [`Workflow::verdicts`]. Checked, not trusted.
    pub value: &'static str,
    /// Why, naming the step that decided it.
    pub because: String,
}

/// A battery.
pub struct Workflow {
    pub name: &'static str,
    pub namespace: &'static str,
    pub description: &'static str,
    /// `(name, description, required)` — the tool's input schema, built from
    /// this so the schema and the steps that read it cannot drift.
    pub arguments: &'static [(&'static str, &'static str, bool)],
    /// The argument naming what the battery is run over.
    ///
    /// Recorded with the verdict so a checkpoint can look a ticket up by the
    /// thing it is about rather than by an id the caller supplies alongside its
    /// request (S-8.4). A gate that took the subject from the request being
    /// judged could be handed another experiment's clean battery.
    pub subject: &'static str,
    pub stages: &'static [Stage],
    /// The closed set the verdict comes from (§6).
    ///
    /// A verdict outside it is a bug this crate catches rather than a string a
    /// reader has to interpret: `a_verdict_is_from_the_closed_set` asserts every
    /// battery's function only ever returns one of these.
    pub verdicts: &'static [&'static str],
    pub verdict: fn(&Battery) -> Verdict,
}

impl Workflow {
    /// The steps that will actually be called, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<&Step> {
        self.stages
            .iter()
            .filter_map(|s| match s {
                Stage::Call(c) => Some(c),
                Stage::Owed { .. } => None,
            })
            .collect()
    }

    /// The stages nobody has built, with what each needs.
    #[must_use]
    pub fn owed(&self) -> Vec<(&'static str, &'static str)> {
        self.stages
            .iter()
            .filter_map(|s| match s {
                Stage::Owed { label, needs, .. } => Some((*label, *needs)),
                Stage::Call(_) => None,
            })
            .collect()
    }

    /// The owed stages that apply to **every** call of this battery.
    ///
    /// The ones that make it unrunnable. A conditional owed stage is not here:
    /// it is a refusal waiting for an input that triggers it, not a gap in the
    /// battery.
    #[must_use]
    pub fn owed_unconditionally(&self) -> Vec<(&'static str, &'static str)> {
        self.stages
            .iter()
            .filter_map(|s| match s {
                Stage::Owed {
                    label,
                    needs,
                    when: When::Always,
                    ..
                } => Some((*label, *needs)),
                _ => None,
            })
            .collect()
    }

    /// Whether every stage the spec names has a tool behind it.
    ///
    /// **Pure: it does not consult the catalogue.** That is not a style
    /// preference — [`specs`] is called *from* `spec::catalog`'s initialiser,
    /// and a `spec::find` inside it re-enters a `OnceLock` that is mid-init.
    /// The first version did exactly that and the whole crate's test binary
    /// hung: not a panic, not a failure, a deadlock, which reads as "the tests
    /// are slow".
    ///
    /// The other half of runnability — that each named tool is a live catalogue
    /// entry — is [`Self::missing_tools`], and it is asserted at build time by
    /// `every_offered_batterys_steps_are_real_tools` rather than consulted at
    /// catalogue build.
    #[must_use]
    pub fn is_declared_complete(&self) -> bool {
        self.owed_unconditionally().is_empty()
    }

    /// Whether this battery can actually be run: every stage declared **and**
    /// every named tool live in the catalogue.
    ///
    /// Consults the catalogue, so it must not be called from anything
    /// `spec::catalog` itself calls. Safe everywhere else — by the time a
    /// dispatch or a test reaches it, the catalogue is built.
    #[must_use]
    pub fn is_runnable(&self) -> bool {
        self.is_declared_complete() && self.missing_tools().is_empty()
    }

    /// Steps whose tool is not an active catalogue entry.
    ///
    /// Consults the catalogue. See [`Self::is_declared_complete`].
    #[must_use]
    pub fn missing_tools(&self) -> Vec<&'static str> {
        self.calls()
            .iter()
            .filter(|s| !crate::spec::find(s.tool).is_some_and(|t| t.status.is_active()))
            .map(|s| s.tool)
            .collect()
    }

    fn input_schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, description, is_required) in self.arguments {
            props.insert((*name).to_string(), json!({ "description": description }));
            if *is_required {
                required.push(json!(name));
            }
        }
        json!({ "type": "object", "properties": props, "required": required })
    }
}

/// Every battery this crate declares, runnable or not.
#[must_use]
pub fn all() -> &'static [Workflow] {
    batteries::ALL
}

/// What each battery is still owed, as `(workflow, stage, needs)`.
///
/// Public because the plan, the `COMPLACENCY_LOG` entry and the test that counts
/// them all have to be reading one list. A battery that becomes runnable
/// disappears from here by construction rather than by somebody remembering to
/// cross it off.
#[must_use]
pub fn owed() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out = Vec::new();
    for wf in all() {
        for stage in wf.stages {
            if let Stage::Owed { label, needs, .. } = stage {
                out.push((wf.name, *label, *needs));
            }
        }
    }
    out
}

/// The catalogue entries for the batteries whose every stage is declared.
///
/// A workflow with an owed stage is not here. See the module header for why
/// exposing a short battery is worse than exposing none.
///
/// **Called from `spec::catalog`'s initialiser**, so the filter is
/// [`Workflow::is_declared_complete`] and not `is_runnable`: the latter consults
/// the catalogue, and consulting a `OnceLock` from inside its own initialiser
/// deadlocks. `every_offered_batterys_steps_are_real_tools` is what holds the
/// other half — that every step names a live tool — and it runs at build time,
/// where the catalogue exists.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    all()
        .iter()
        .filter(|w| w.is_declared_complete())
        .map(|w| {
            ToolSpec::new(
                w.name,
                w.namespace,
                // A battery reads and computes; it changes no platform artifact
                // of its own. Its *steps* carry their own risk and their own
                // envelopes, and each is admitted separately — which is the
                // property that makes this safe to declare `Read`.
                Risk::Read,
                w.description,
                w.input_schema(),
            )
            .internal()
            // Nothing. The steps spend; see the module header.
            .spends_trial(TrialKind::None)
            .tier(Tier::Workflow {
                steps: w.step_names(),
            })
            .evidence(&["family_id"])
        })
        .collect()
}

impl Workflow {
    /// The step names, leaked as a `&'static [&'static str]` for
    /// [`Tier::Workflow`].
    ///
    /// Leaked rather than stored: the declaration's source of truth is
    /// [`Self::stages`], and a second hand-written list beside it is the exact
    /// duplication `spec.rs` exists to end. One leak per battery, once, at
    /// catalogue build.
    fn step_names(&'static self) -> &'static [&'static str] {
        Box::leak(
            self.calls()
                .iter()
                .map(|s| s.tool)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }
}

// ── Running one ──────────────────────────────────────────────────────────────

/// What became of one stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ran,
    /// Its `when` did not hold. Recorded, because a step that did not apply and
    /// a step that was forgotten look identical in a list of results.
    Skipped(String),
    /// It ran and came back an error.
    Failed,
    /// The battery stopped before reaching it.
    ///
    /// A third word rather than a reuse of `Skipped`, because "this step did not
    /// apply" and "we stopped before it" are different facts about the run and
    /// this whole module is about not letting two meanings share one value.
    NotAttempted(String),
}

/// One stage's outcome.
#[derive(Clone, Debug)]
pub struct Ran {
    pub tool: &'static str,
    pub label: &'static str,
    pub status: Status,
    pub ids: BTreeMap<String, String>,
    pub detail: Value,
}

impl Ran {
    /// A field of this step's answer, looked for where the platform puts them:
    /// `detail`, then `summary_fields`, then the top level.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.detail
            .get("detail")
            .and_then(|d| d.get(name))
            .or_else(|| {
                self.detail
                    .get("summary_fields")
                    .and_then(|d| d.get(name))
            })
            .or_else(|| self.detail.get(name))
    }

    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.field(name).and_then(Value::as_f64)
    }

    #[must_use]
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.field(name).and_then(Value::as_bool)
    }

    #[must_use]
    pub fn ran(&self) -> bool {
        self.status == Status::Ran
    }

    /// A field of the `verdict` object a `run_study` step answers with.
    ///
    /// Separate from [`Self::field`] because a Study's conclusion is nested:
    /// `StudyView` is flat and its `verdict` is a `StudyVerdict` object holding
    /// `positive_median`, `survivable_worst5` and `plateau`.
    ///
    /// **A Study has no p-value.** The first version of the batteries read one
    /// and would have rejected every strategy forever, silently, because a field
    /// nothing writes reads as `None` and `None` is the conservative branch. It
    /// is the defect shape this repository keeps producing, in a new place, and
    /// `invariants::a_battery_reads_the_study_it_ran` is what stops it
    /// recurring — the field names below are held against `backtest`'s own
    /// serialisation.
    #[must_use]
    pub fn study_flag(&self, name: &str) -> Option<bool> {
        self.field("verdict")
            .and_then(|v| v.get(name))
            .and_then(Value::as_bool)
    }

    /// Whether a Study's distribution is one a gate would plan around: a median
    /// above zero **and** a survivable worst-5%.
    ///
    /// Both, because either alone is a different claim. A positive median with
    /// an unsurvivable tail is a strategy that works until it does not, and the
    /// tail is what the platform plans around (`Distribution::worst_5pct`).
    ///
    /// `None` when the step did not answer — never `false`, so a caller has to
    /// decide what an unanswered Study means rather than inheriting "it failed".
    #[must_use]
    pub fn study_survived(&self) -> Option<bool> {
        Some(self.study_flag("positive_median")? && self.study_flag("survivable_worst5")?)
    }
}

/// Everything one run of a battery produced.
#[derive(Clone, Debug)]
pub struct Battery {
    pub workflow: &'static str,
    pub family_id: String,
    pub steps: Vec<Ran>,
}

impl Battery {
    /// One step by its label.
    #[must_use]
    pub fn step(&self, label: &str) -> Option<&Ran> {
        self.steps.iter().find(|s| s.label == label)
    }

    /// Whether every step that was meant to run did.
    ///
    /// A skipped step counts as fine — its condition said it did not apply. A
    /// *failed* one does not, and neither does one the battery stopped before
    /// reaching; that is what separates a verdict from an `inconclusive`.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.steps
            .iter()
            .all(|s| !matches!(s.status, Status::Failed | Status::NotAttempted(_)))
    }

    /// The labels of the steps that failed.
    #[must_use]
    pub fn failures(&self) -> Vec<&'static str> {
        self.steps
            .iter()
            .filter(|s| s.status == Status::Failed)
            .map(|s| s.label)
            .collect()
    }
}

/// How a step's status is written on the wire.
///
/// One copy, because there are two writers — the tool's own result and the
/// ledger row a checkpoint reads (`0135`) — and a status they disagree about is
/// a verdict whose stored evidence says something different from the answer the
/// model was given. It was two copies for exactly as long as it took to add a
/// third status, which the compiler caught in one of them and not the other.
fn status_word(status: &Status) -> Value {
    match status {
        Status::Ran => json!("ran"),
        Status::Failed => json!("failed"),
        Status::Skipped(why) => json!({ "skipped": why }),
        Status::NotAttempted(why) => json!({ "not_attempted": why }),
    }
}

fn step_params(step: &Step, params: &Value, family: &str, index: usize) -> Value {
    let mut out = serde_json::Map::new();
    for name in step.forward {
        if let Some(v) = params.get(*name) {
            out.insert((*name).to_string(), v.clone());
        }
    }
    for (to, from) in step.rename {
        if let Some(v) = params.get(*from) {
            out.insert((*to).to_string(), v.clone());
        }
    }
    // Pinned last, and unconditionally: a forwarded argument colliding with a
    // pinned one must lose, or the caller can turn the cost shock into a
    // parameter sweep by naming the field.
    for (name, arg) in step.pinned {
        out.insert((*name).to_string(), arg.render(family, index));
    }
    Value::Object(out)
}

/// Whether calling this tool registers a trial.
///
/// Read from the catalogue's own declaration rather than from a list here, for
/// the reason `spec.rs` exists: a second list is a second thing to keep in
/// agreement, and the direction of a miss would be permissive — a
/// trial-spending tool this did not recognise would be spent after a failure.
///
/// A tool the catalogue does not know is treated as spending. It cannot be
/// dispatched at all, so the only effect is to stop the battery, which is the
/// safe direction.
fn spends_a_trial(tool: &str) -> bool {
    crate::spec::find(tool).is_none_or(|s| s.spends_trial != TrialKind::None)
}

fn should_run(step: &Step, params: &Value, done: &[Ran]) -> Result<(), String> {
    match step.when {
        When::Always => Ok(()),
        When::IfArgument(flag) => {
            if params.get(flag).and_then(Value::as_bool).unwrap_or(false) {
                Ok(())
            } else {
                Err(format!("`{flag}` was not set, so this step does not apply"))
            }
        }
        When::IfEarlier { step: earlier, field } => {
            let held = done
                .iter()
                .find(|r| r.label == earlier)
                .and_then(|r| r.flag(field))
                .unwrap_or(false);
            if held {
                Ok(())
            } else {
                Err(format!(
                    "`{earlier}` did not report `{field}`, so this step does not apply"
                ))
            }
        }
    }
}

/// Run a battery.
///
/// # The pre-flight is not a formality
///
/// Every step's tool is checked against the catalogue **before the first one
/// runs**. A battery that discovers its fourth step does not exist has already
/// spent three trials on a verdict it cannot emit — and those trials are real,
/// counted, and in the denominator of every correction the run makes
/// afterwards.
///
/// The case it catches is a *renamed tool*, not an owed stage: a battery with an
/// owed stage is not in the catalogue, so `dispatch_under` answers
/// `unknown_tool` before this function is reached. A battery that was complete
/// when the catalogue was built and whose step tool has since been renamed or
/// deprecated is the reachable one, and it is exactly the failure the rest of
/// this crate keeps finding — a caller whose callee moved.
pub async fn run(ctx: &crate::McpContext, wf: &'static Workflow, params: &Value) -> Value {
    if !wf.is_runnable() {
        return ToolError::new(
            "workflow_incomplete",
            format!(
                "`{}` is a battery with {} step(s) nothing performs yet: {}",
                wf.name,
                wf.owed().len() + wf.missing_tools().len(),
                wf.owed()
                    .iter()
                    .map(|(l, n)| format!("{l} (needs {n})"))
                    .chain(wf.missing_tools().iter().map(|t| format!("{t} (not in the catalogue)")))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            "run the steps that do exist yourself and say which battery you could not complete; \
             a verdict from a short battery is shaped like the real one and is not",
        )
        .to_value();
    }

    // A conditional owed stage that *applies to these arguments* is a refusal,
    // not a skip. The battery is offered because the step does not apply to most
    // inputs; for the inputs it does apply to, the verdict would be missing the
    // one check that was supposed to cover them.
    for stage in wf.stages {
        let Stage::Owed {
            label, needs, why, when,
        } = stage
        else {
            continue;
        };
        if *when == When::Always {
            continue;
        }
        // `should_run` answers "does this step apply", over a `Step`; the same
        // question for an owed stage, with no earlier steps to read.
        let applies = match when {
            When::Always => true,
            When::IfArgument(flag) => {
                params.get(*flag).and_then(Value::as_bool).unwrap_or(false)
            }
            // An owed stage cannot depend on an earlier step's finding: there is
            // no step to have found it. Declaring one would be a stage that can
            // never apply, which is a stage that is never refused.
            When::IfEarlier { .. } => false,
        };
        if applies {
            return ToolError::new(
                "workflow_step_owed",
                format!(
                    "`{}` applies to these arguments and nothing performs it: {label}",
                    wf.name
                ),
                format!("{why}. It would need {needs}."),
            )
            .to_value();
        }
    }

    for (name, _, required) in wf.arguments {
        if *required && params.get(*name).is_none() {
            return ToolError::new(
                "missing_field",
                format!("`{}` needs `{name}`.", wf.name),
                "every step of this battery reads it",
            )
            .to_value();
        }
    }

    // Before the first admission, so no step of this battery can fall outside
    // its family.
    let family_id = format!("wf_{}_{}", wf.name, Uuid::new_v4());
    let mut steps: Vec<Ran> = Vec::new();

    let remaining = wf.calls();
    for (index, step) in remaining.iter().copied().enumerate() {
        if let Err(why) = should_run(step, params, &steps) {
            steps.push(Ran {
                tool: step.tool,
                label: step.label,
                status: Status::Skipped(why),
                ids: BTreeMap::new(),
                detail: Value::Null,
            });
            continue;
        }
        let p = step_params(step, params, &family_id, index);
        let out = crate::dispatch_step(ctx, step.tool, &p, &family_id).await;
        let failed = out.get("error").is_some();
        let mut ids = BTreeMap::new();
        if let Some(obj) = out.get("ids").and_then(Value::as_object) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    ids.insert(k.clone(), s.to_string());
                }
            }
        }
        steps.push(Ran {
            tool: step.tool,
            label: step.label,
            status: if failed { Status::Failed } else { Status::Ran },
            ids,
            detail: out,
        });

        // A failed step means the verdict is already unreachable —
        // `cannot_say` refuses to emit one over an incomplete battery. From
        // here, a step that spends a trial spends it for diagnostics the caller
        // could get by calling that tool directly, out of the one budget every
        // selection-adjusted statistic on this platform divides by.
        //
        // So the battery stops, but **only** when continuing would cost
        // something. A battery whose remaining steps are reads runs them: the
        // failures are worth having and they are free, and a caller told one
        // thing at a time reruns the battery to learn the next.
        if failed {
            let rest = &remaining[index + 1..];
            if rest.iter().any(|s| spends_a_trial(s.tool)) {
                for later in rest {
                    steps.push(Ran {
                        tool: later.tool,
                        label: later.label,
                        status: Status::NotAttempted(format!(
                            "`{}` failed, and this step spends a trial for a verdict this \
                             battery can no longer reach",
                            step.label
                        )),
                        ids: BTreeMap::new(),
                        detail: Value::Null,
                    });
                }
                break;
            }
        }
    }

    let battery = Battery {
        workflow: wf.name,
        family_id: family_id.clone(),
        steps,
    };
    let verdict = (wf.verdict)(&battery);
    debug_assert!(
        wf.verdicts.contains(&verdict.value),
        "`{}` returned a verdict outside its closed set: {}",
        wf.name,
        verdict.value
    );

    let mut out = render(wf, &battery, &verdict);
    let recorded = record(ctx, wf, params, &battery, &verdict).await;
    if let Some(obj) = out.as_object_mut() {
        if let Some(detail) = obj.get_mut("detail").and_then(Value::as_object_mut) {
            detail.insert("recorded".into(), json!(recorded.is_ok()));
            if let Err(why) = &recorded {
                // Said in the result rather than only logged. An unrecorded
                // verdict cannot license a gate, and a model that believes it
                // has one will spend a funnel finding out.
                detail.insert("not_recorded_because".into(), json!(why));
            }
        }
    }
    out
}

/// Write the battery's conclusion where a checkpoint can read it (S-8.4).
///
/// The engine writes this, from steps it dispatched itself. That is the whole
/// mechanism: `backtest::gates::tickets` reads rows, and a model cannot produce
/// one by asserting a verdict, because there is no route that takes a verdict
/// without the steps behind it.
async fn record(
    ctx: &crate::McpContext,
    wf: &Workflow,
    params: &Value,
    battery: &Battery,
    verdict: &Verdict,
) -> Result<(), String> {
    let Some(subject) = params.get(wf.subject).and_then(Value::as_str) else {
        return Err(format!(
            "this battery was called without `{}`, so there is nothing to file the verdict against",
            wf.subject
        ));
    };
    let body = json!({
        "family_id": battery.family_id,
        "workflow": wf.name,
        "verdict": verdict.value,
        "because": verdict.because,
        "subject": subject,
        "run_id": (ctx.run_id != Uuid::nil()).then(|| ctx.run_id.to_string()),
        "steps": battery.steps.iter().map(|s| json!({
            "label": s.label,
            "tool": s.tool,
            "status": status_word(&s.status),
            "ids": s.ids,
        })).collect::<Vec<_>>(),
        "complete": battery.complete(),
    });
    ctx.api
        .post("/api/ledger/workflow-verdicts", body)
        .await
        .map(|_| ())
        .map_err(|e| {
            e.body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the platform did not record the verdict")
                .to_string()
        })
}

fn render(wf: &Workflow, battery: &Battery, verdict: &Verdict) -> Value {
    let mut result = ToolResult::new(format!(
        "{}: {} — {}",
        wf.name, verdict.value, verdict.because
    ))
    .with_id("family_id", battery.family_id.clone());

    // Every step's ticket, under the step's label rather than the tool's name:
    // `run_study` appears four times in one battery and four ids under one key
    // is three ids lost.
    for s in &battery.steps {
        for (k, v) in &s.ids {
            result = result.with_id(format!("{}.{k}", s.label), v.clone());
        }
    }

    result
        .with_detail(json!({
            "family_id": battery.family_id,
            "verdict": verdict.value,
            "because": verdict.because,
            "verdicts": wf.verdicts,
            "complete": battery.complete(),
            "steps": battery.steps.iter().map(|s| json!({
                "label": s.label,
                "tool": s.tool,
                "status": status_word(&s.status),
                "summary": s.detail.get("summary").cloned()
                    .or_else(|| s.detail.get("summary_fields").cloned()),
                "ids": s.ids,
                "detail": s.detail.get("detail").cloned().unwrap_or(Value::Null),
            })).collect::<Vec<_>>(),
        }))
        .render_within(usize::MAX, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(name: &str) -> &'static Workflow {
        all()
            .iter()
            .find(|w| w.name == name)
            .unwrap_or_else(|| panic!("no battery named {name}"))
    }

    /// The tier existed in the catalogue's vocabulary and nothing had ever been
    /// declared with it.
    #[test]
    fn at_least_one_battery_is_actually_declared_as_one() {
        let declared = specs();
        assert!(
            !declared.is_empty(),
            "no battery is in the catalogue, so `Tier::Workflow` is still a category that \
             describes nothing"
        );
        for s in &declared {
            assert!(
                matches!(s.tier, Tier::Workflow { .. }),
                "`{}` is offered as a workflow and declares tier {:?}",
                s.name,
                s.tier
            );
        }
    }

    /// S-8.1's acceptance, the half that is a property of the declaration: a
    /// workflow spends nothing itself, so its trial count is exactly the sum of
    /// its steps'.
    #[test]
    fn a_battery_spends_nothing_of_its_own() {
        for s in specs() {
            assert_eq!(
                s.spends_trial,
                TrialKind::None,
                "`{}` declares a trial of its own, so every battery it runs charges twice for \
                 the same compute and the exploration floor's denominator is wrong",
                s.name
            );
        }
    }

    /// The other half: the steps a battery runs are steps the catalogue has, and
    /// the declared step list is the one the engine will actually call.
    #[test]
    fn every_offered_batterys_steps_are_real_tools() {
        for w in all().iter().filter(|w| w.is_runnable()) {
            for step in w.calls() {
                let spec = crate::spec::find(step.tool)
                    .unwrap_or_else(|| panic!("`{}` step `{}` names no tool", w.name, step.label));
                assert!(spec.status.is_active(), "`{}` calls a deprecated tool", w.name);
            }
        }
        for s in specs() {
            let Tier::Workflow { steps } = s.tier else {
                panic!("not a workflow")
            };
            assert_eq!(steps.len(), wf(s.name).calls().len());
        }
    }

    /// `specs()` must not consult the catalogue: it is called from the
    /// catalogue's own initialiser, and a `spec::find` in there re-enters a
    /// `OnceLock` that is mid-init and hangs the process.
    ///
    /// Asserted structurally, because the symptom is a deadlock rather than a
    /// failure — the test binary simply stops, and cargo reports it as tests
    /// "running for over 60 seconds". That is a very expensive thing to
    /// rediscover.
    #[test]
    fn building_the_catalogue_does_not_re_enter_the_catalogue() {
        const SRC: &str = include_str!("workflows.rs");
        let body = SRC
            .split_once("pub fn specs() -> Vec<ToolSpec> {")
            .expect("specs exists")
            .1
            .split_once("\n}")
            .expect("it ends")
            .0;
        for forbidden in ["spec::find", "is_runnable", "missing_tools", "catalog("] {
            assert!(
                !body.contains(forbidden),
                "`specs()` reaches `{forbidden}`, which re-enters `spec::catalog`'s OnceLock \
                 from inside its own initialiser and deadlocks"
            );
        }
        // And the filter it does use is the pure one.
        assert!(body.contains("is_declared_complete"));
    }

    /// A battery with a step nobody built **that applies to every call** is not
    /// in the catalogue at all. The direction matters: absent is a tool the
    /// model cannot call, and a short battery is a verdict nothing downstream
    /// can distinguish from a real one.
    ///
    /// A *conditional* owed stage does not withhold the battery. It moves the
    /// refusal to `run`, for the arguments the step would have applied to — see
    /// `a_battery_is_refused_for_the_inputs_its_owed_step_covers`.
    #[test]
    fn a_battery_with_an_unconditionally_owed_step_is_not_offered() {
        let offered: Vec<&str> = specs().iter().map(|s| s.name).collect();
        for w in all() {
            if w.owed_unconditionally().is_empty() {
                continue;
            }
            assert!(
                !offered.contains(&w.name),
                "`{}` is offered and is owed {:?}",
                w.name,
                w.owed_unconditionally()
            );
        }
    }

    /// And a battery whose only gaps are conditional **is** offered — for the
    /// inputs those gaps do not cover.
    ///
    /// Withholding it entirely would be withholding a battery that covers every
    /// strategy this platform can currently represent, because of a step that
    /// applies to none of them.
    #[test]
    fn a_battery_owed_only_conditional_steps_is_still_offered() {
        let offered: Vec<&str> = specs().iter().map(|s| s.name).collect();
        let conditional: Vec<&str> = all()
            .iter()
            .filter(|w| !w.owed().is_empty() && w.owed_unconditionally().is_empty())
            .map(|w| w.name)
            .collect();
        assert!(
            !conditional.is_empty(),
            "no battery exercises the conditional-owed rule"
        );
        for name in conditional {
            assert!(offered.contains(&name), "`{name}` is owed only conditionally and is hidden");
        }
    }

    /// Every owed stage says what would have to exist, so the gap is actionable
    /// rather than only visible.
    #[test]
    fn every_owed_stage_names_what_it_needs() {
        for w in all() {
            for stage in w.stages {
                if let Stage::Owed { label, needs, why, .. } = stage {
                    assert!(!needs.trim().is_empty(), "`{}`/{label} needs nothing?", w.name);
                    assert!(!why.trim().is_empty(), "`{}`/{label} gives no reason", w.name);
                }
            }
        }
    }

    /// Pinned parameters win. Otherwise a caller turns the cost shock into a
    /// parameter sweep by naming the field the battery pinned.
    #[test]
    fn a_caller_cannot_unpin_a_pinned_step_parameter() {
        static STEP: Step = Step {
            tool: "run_study",
            label: "cost shock",
            forward: &["experiment_id", "kind"],
            rename: &[],
            pinned: &[("kind", Arg::Str("cost_sweep")), ("study_id", Arg::StepId)],
            when: When::Always,
        };
        let p = step_params(
            &STEP,
            &json!({ "experiment_id": "e1", "kind": "parameter_sweep" }),
            "fam",
            2,
        );
        assert_eq!(p["kind"], json!("cost_sweep"));
        assert_eq!(p["experiment_id"], json!("e1"));
        assert_eq!(p["study_id"], json!("fam-2"));
    }

    /// A step id is derived from the family and the position, so the same
    /// battery over the same family produces the same ids twice.
    #[test]
    fn a_step_id_is_reproducible() {
        assert_eq!(Arg::StepId.render("fam", 3), json!("fam-3"));
        assert_eq!(Arg::StepId.render("fam", 3), json!("fam-3"));
        assert_ne!(Arg::StepId.render("fam", 3), Arg::StepId.render("fam", 4));
    }

    /// A decision point branches on evidence the battery has, and a step that
    /// did not apply is recorded as skipped rather than dropped — a dropped step
    /// and a forgotten one are the same list.
    #[test]
    fn a_decision_point_reads_an_earlier_step() {
        static GATED: Step = Step {
            tool: "probe_data_quality",
            label: "contamination",
            forward: &[],
            rename: &[],
            pinned: &[],
            when: When::IfEarlier {
                step: "provenance",
                field: "has_text_features",
            },
        };
        let no = vec![Ran {
            tool: "probe_data_quality",
            label: "provenance",
            status: Status::Ran,
            ids: BTreeMap::new(),
            detail: json!({ "detail": { "has_text_features": false } }),
        }];
        assert!(should_run(&GATED, &json!({}), &no).is_err());

        let yes = vec![Ran {
            tool: "probe_data_quality",
            label: "provenance",
            status: Status::Ran,
            ids: BTreeMap::new(),
            detail: json!({ "detail": { "has_text_features": true } }),
        }];
        assert!(should_run(&GATED, &json!({}), &yes).is_ok());
    }

    /// An argument-gated step reads the caller, and a missing flag is a `false`
    /// rather than an error: "no text features" is a real answer.
    #[test]
    fn an_argument_decision_point_defaults_to_not_running() {
        static GATED: Step = Step {
            tool: "probe_data_quality",
            label: "contamination",
            forward: &[],
            rename: &[],
            pinned: &[],
            when: When::IfArgument("text_features"),
        };
        assert!(should_run(&GATED, &json!({}), &[]).is_err());
        assert!(should_run(&GATED, &json!({ "text_features": false }), &[]).is_err());
        assert!(should_run(&GATED, &json!({ "text_features": true }), &[]).is_ok());
    }

    /// Every battery's verdict function only ever answers from its own closed
    /// set, over an empty battery and over a complete one. §6's "one verdict
    /// from a closed set" is not a documentation claim if nothing checks it.
    #[test]
    fn a_verdict_is_from_the_closed_set() {
        for w in all() {
            assert!(
                w.verdicts.len() >= 2,
                "`{}` has a closed set of {} — a verdict that is always the same is not a \
                 verdict",
                w.name,
                w.verdicts.len()
            );
            let empty = Battery {
                workflow: w.name,
                family_id: "fam".into(),
                steps: Vec::new(),
            };
            let v = (w.verdict)(&empty);
            assert!(
                w.verdicts.contains(&v.value),
                "`{}` answered `{}`, which is not in {:?}",
                w.name,
                v.value,
                w.verdicts
            );
            assert!(!v.because.trim().is_empty(), "`{}` gave no reason", w.name);
        }
    }

    /// A battery that could not complete does not emit the verdict it would
    /// have emitted had every step run.
    #[test]
    fn a_failed_step_is_not_a_clean_battery() {
        for w in all() {
            let failed = Battery {
                workflow: w.name,
                family_id: "fam".into(),
                steps: w
                    .calls()
                    .iter()
                    .map(|s| Ran {
                        tool: s.tool,
                        label: s.label,
                        status: Status::Failed,
                        ids: BTreeMap::new(),
                        detail: json!({ "error": "backend_unavailable" }),
                    })
                    .collect(),
            };
            if failed.steps.is_empty() {
                continue;
            }
            assert!(!failed.complete());
            let v = (w.verdict)(&failed);
            assert!(w.verdicts.contains(&v.value));
            assert_ne!(
                v.value,
                w.verdicts[0],
                "`{}` returns its clean verdict when every step failed",
                w.name
            );
        }
    }

    /// The rendered result carries every step's ticket, keyed by the step's
    /// label — S-8.1's acceptance.
    #[test]
    fn the_result_carries_every_steps_ticket() {
        let w = wf("selection_bias_verdict");
        let battery = Battery {
            workflow: w.name,
            family_id: "fam_1".into(),
            steps: w
                .calls()
                .iter()
                .map(|s| {
                    let mut ids = BTreeMap::new();
                    ids.insert("reservation_id".to_string(), format!("res_{}", s.label));
                    Ran {
                        tool: s.tool,
                        label: s.label,
                        status: Status::Ran,
                        ids,
                        detail: json!({ "detail": {} }),
                    }
                })
                .collect(),
        };
        let out = render(w, &battery, &(w.verdict)(&battery));
        let ids = out["ids"].as_object().expect("ids");
        assert_eq!(ids["family_id"], json!("fam_1"));
        for s in &battery.steps {
            let key = format!("{}.reservation_id", s.label);
            assert_eq!(
                ids.get(&key).and_then(Value::as_str),
                Some(format!("res_{}", s.label).as_str()),
                "`{key}` is not in the result's ids"
            );
        }
        assert_eq!(
            out["detail"]["steps"].as_array().map(Vec::len),
            Some(w.calls().len())
        );
    }

    /// The dispatcher boxes the battery's future so an `async fn` that reaches
    /// itself is representable. What makes it *terminate* is this: no battery
    /// has a battery for a step.
    ///
    /// Without it, a battery naming itself would recurse until the stack ran
    /// out, having spent a trial on every turn of the loop.
    #[test]
    fn no_battery_has_a_battery_for_a_step() {
        let names: Vec<&str> = all().iter().map(|w| w.name).collect();
        for w in all() {
            for s in w.calls() {
                assert!(
                    !names.contains(&s.tool),
                    "`{}` has the battery `{}` for a step",
                    w.name,
                    s.tool
                );
            }
        }
    }

    /// Every battery names the argument its verdict is filed against, and that
    /// argument is one the schema declares and the caller must supply.
    ///
    /// A battery whose subject were optional would file verdicts against
    /// nothing, and a checkpoint looking one up would find no ticket for a
    /// battery that ran.
    #[test]
    fn every_battery_files_its_verdict_against_a_required_argument() {
        for w in all() {
            let declared = w
                .arguments
                .iter()
                .find(|(n, _, _)| *n == w.subject)
                .unwrap_or_else(|| panic!("`{}` files against `{}`, which it does not ask for", w.name, w.subject));
            assert!(
                declared.2,
                "`{}` files against `{}`, which is optional",
                w.name,
                w.subject
            );
        }
    }

    /// A battery that has already failed does not go on spending trials for a
    /// verdict it cannot emit.
    ///
    /// The budget this protects is the one every selection-adjusted statistic on
    /// this platform divides by, so a battery that burned five trials to produce
    /// `inconclusive` would be making every later correction on the run worse
    /// for nothing.
    #[test]
    fn a_failed_step_stops_a_battery_that_would_spend_to_continue() {
        // `run_study` spends; the audit tools do not. That asymmetry is the
        // whole rule, so assert it rather than assuming it.
        assert!(spends_a_trial("run_study"));
        assert!(!spends_a_trial("deflated_sharpe"));
        assert!(!spends_a_trial("probe_data_quality"));
        // A name the catalogue does not know stops the battery, which is the
        // safe direction.
        assert!(spends_a_trial("a_tool_that_does_not_exist"));
    }

    /// The three non-running statuses stay three. A `NotAttempted` collapsed
    /// into `Skipped` would report a battery that stopped as a battery whose
    /// conditions did not apply.
    #[test]
    fn stopping_early_is_not_the_same_as_not_applying() {
        let skipped = Status::Skipped("`text_features` was not set".into());
        let stopped = Status::NotAttempted("`ablation` failed".into());
        assert_ne!(skipped, stopped);
        assert_ne!(stopped, Status::Failed);

        // And a battery holding one is not complete, so no verdict function can
        // emit its clean answer over it.
        let b = Battery {
            workflow: "precapital_battery",
            family_id: "fam".into(),
            steps: vec![Ran {
                tool: "run_study",
                label: "cost shock",
                status: stopped,
                ids: BTreeMap::new(),
                detail: Value::Null,
            }],
        };
        assert!(!b.complete());
    }

    /// Two batteries are two families. One family per run would fold every
    /// battery a run takes into one denominator, which over-corrects.
    #[test]
    fn two_runs_of_a_battery_are_two_families() {
        let a = format!("wf_x_{}", Uuid::new_v4());
        let b = format!("wf_x_{}", Uuid::new_v4());
        assert_ne!(a, b);
    }
}
