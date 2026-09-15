//! Trajectories, model tapes and replay (AGENT-009 §2, P1; Set S task S-4.8).
//!
//! # Why replay is possible at all
//!
//! `Loop::next(Input) -> Vec<Effect>` is sans-IO. It names no provider, no
//! runtime and no HTTP client, and everything that blocks leaves as an `Effect`
//! and comes back as an `Input`. So a run is **completely** described by the
//! ordered inputs it was fed, and re-feeding them to the same `Loop` version must
//! produce the same effects, on a machine with no accelerator and no network.
//!
//! That is the property AGENT-009 P1 calls "replay before rerun", and it is what
//! makes most of the eval suite free.
//!
//! # Trajectory, tape, replay
//!
//! * A [`Trajectory`] is what a run **produced**: the ordered `(Input, Vec<Effect>)`
//!   pairs, plus the profile, the registry hash, the task and the per-step state
//!   hash. `agent_events` persists it (AGENT-010 §5).
//! * A [`Tape`] is the input half alone — in practice, the sequence of raw model
//!   replies. It is a plain JSONL document and is **hand-editable**, which is how
//!   a case is constructed: a truncated reply, a wrong tool, a fake id.
//! * [`replay`] feeds a tape through a fresh `Loop` and returns what came out.
//!   [`Trajectory::verify`] does both and diffs.
//!
//! # The diff is classified, not just counted
//!
//! AGENT-009 §6: a candidate harness that reorders two independent reads has not
//! regressed, and one that drops a tool call has. Reporting "3 differences" makes
//! those the same event and guarantees the shadow-replay report gets ignored
//! within a month. [`Diff::severity`] is the classification.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::drive::{Effect, Input, Loop, Outcome, State, Task};
use crate::hardware::{Admission, TaskDemand};
use crate::profile::Profile;
use crate::registry::ToolRegistry;

/// One input, in a form that survives a file.
///
/// `drive::Input` is deliberately not `Serialize` — it carries no data the loop
/// owns and adding a derive to it would invite persisting it from the driver.
/// This is its wire twin, and [`TapeEntry::to_input`] is the only bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum TapeEntry {
    Admit,
    Escalate { reason: String, to: String },
    Refuse { reason: String, fix: String },
    ModelReply { raw: String },
    ModelTruncated { raw: String },
    ModelError { detail: String, retryable: bool },
    Approval { granted: bool, note: String },
    ToolResult { output: String, source: String },
    ToolError { detail: String },
    Cancel,
}

impl TapeEntry {
    #[must_use]
    pub fn to_input(&self) -> Input {
        match self {
            TapeEntry::Admit => Input::Admission(Admission::Admit),
            TapeEntry::Escalate { reason, to } => Input::Admission(Admission::Escalate {
                reason: reason.clone(),
                to: to.clone(),
            }),
            TapeEntry::Refuse { reason, fix } => Input::Admission(Admission::Refuse {
                reason: reason.clone(),
                fix: fix.clone(),
            }),
            TapeEntry::ModelReply { raw } => Input::ModelReply { raw: raw.clone() },
            TapeEntry::ModelTruncated { raw } => Input::ModelTruncated { raw: raw.clone() },
            TapeEntry::ModelError { detail, retryable } => Input::ModelError {
                detail: detail.clone(),
                retryable: *retryable,
            },
            TapeEntry::Approval { granted, note } => Input::Approval {
                granted: *granted,
                note: note.clone(),
            },
            TapeEntry::ToolResult { output, source } => Input::ToolResult {
                output: output.clone(),
                source: source.clone(),
            },
            TapeEntry::ToolError { detail } => Input::ToolError {
                detail: detail.clone(),
            },
            TapeEntry::Cancel => Input::Cancel,
        }
    }

    /// A tool selection, in the two-step decode's step 1 shape.
    #[must_use]
    pub fn select(tool: &str) -> Self {
        TapeEntry::ModelReply {
            raw: format!("{{\"name\":\"{tool}\"}}"),
        }
    }

    /// An argument fill, in the two-step decode's step 2 shape.
    #[must_use]
    pub fn fill(args: &Value) -> Self {
        TapeEntry::ModelReply {
            raw: args.to_string(),
        }
    }

    /// A plan, for a `planner_executor` tier. Every tape on the 7B profile opens
    /// with one, because that tier plans before it acts.
    #[must_use]
    pub fn plan(steps: &[(&str, &str)]) -> Self {
        let steps: Vec<Value> = steps
            .iter()
            .map(|(goal, ns)| serde_json::json!({"goal": goal, "namespace": ns}))
            .collect();
        TapeEntry::ModelReply {
            raw: serde_json::json!({ "steps": steps }).to_string(),
        }
    }
}

/// The task, in a form a case file can carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TapeTask {
    pub goal: String,
    pub charter: String,
    pub multi_step: bool,
    pub namespaces: Vec<String>,
}

impl TapeTask {
    #[must_use]
    pub fn to_task(&self) -> Task {
        Task {
            goal: self.goal.clone(),
            charter: self.charter.clone(),
            demand: if self.multi_step {
                TaskDemand::MultiStep
            } else {
                TaskDemand::SingleLookup
            },
            namespaces: self.namespaces.clone(),
        }
    }
}

/// The sequence of inputs a run was fed. Hand-editable by design.
pub type Tape = Vec<TapeEntry>;

/// One `(Input, Vec<Effect>)` pair as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub input: TapeEntry,
    /// Effects, serialised. `Effect` is `Serialize` and not `Deserialize`, which
    /// is the right asymmetry: a trajectory is a *record*, and nothing should be
    /// able to reconstitute an `Effect` from a file and hand it to a driver.
    pub effects: Vec<Value>,
    /// The loop's state after this input.
    pub state: String,
    /// `WorkingState`'s hash, when the driver recorded one.
    #[serde(default)]
    pub working_state_hash: Option<String>,
    /// What was exposed on this step. The `routing` suite's whole reference.
    #[serde(default)]
    pub exposed: Vec<String>,
}

/// What a run produced, completely (AGENT-009 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    pub trajectory_id: String,
    pub profile_id: String,
    /// The catalogue this ran against. A replay against a different catalogue is
    /// measuring the catalogue.
    pub registry_hash: String,
    pub harness_version: String,
    pub task: TapeTask,
    pub steps: Vec<TrajectoryStep>,
}

impl Trajectory {
    /// The input half, for replay.
    #[must_use]
    pub fn tape(&self) -> Tape {
        self.steps.iter().map(|s| s.input.clone()).collect()
    }

    /// Feed the tape back through a fresh loop and diff (AGENT-009 §8.1).
    #[must_use]
    pub fn verify(&self, profile: Profile, registry: ToolRegistry) -> Vec<Diff> {
        let r = replay(profile, registry, &self.task, &self.tape());
        diff(self, &r)
    }

    /// Whether this trajectory carries enough to replay at all.
    ///
    /// Blunt on purpose, like `trace::is_replayable`: a tape with no inputs, or
    /// a step whose effects were never recorded, reproduces nothing however
    /// complete it looks in a dashboard.
    #[must_use]
    pub fn is_replayable(&self) -> bool {
        !self.steps.is_empty()
            && !self.registry_hash.is_empty()
            && !self.profile_id.is_empty()
            && self.steps.iter().all(|s| !s.state.is_empty())
    }
}

/// What a replay produced.
#[derive(Debug, Clone)]
pub struct Replay {
    pub steps: Vec<TrajectoryStep>,
    pub outcome: Option<Outcome>,
    pub path: Vec<State>,
}

impl Replay {
    /// Every tool named by any effect, in order, including harness-owned ones.
    #[must_use]
    pub fn tools_called(&self) -> Vec<String> {
        self.steps
            .iter()
            .flat_map(|s| s.effects.iter().filter_map(tool_name_of))
            .collect()
    }

    /// What was exposed on the step with this index.
    #[must_use]
    pub fn exposed_at(&self, step: usize) -> BTreeSet<String> {
        self.steps
            .get(step)
            .map(|s| s.exposed.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Every exposure set recorded, in order. What the `routing` suite reads.
    #[must_use]
    pub fn exposures(&self) -> Vec<BTreeSet<String>> {
        let mut out = Vec::new();
        for s in &self.steps {
            for e in &s.effects {
                if e.get("note").and_then(Value::as_str) == Some("exposure") {
                    out.push(
                        e.get("exposed")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    );
                }
            }
        }
        out
    }

    /// Every rejection code the validation ladder emitted.
    #[must_use]
    pub fn rejections(&self) -> Vec<String> {
        self.steps
            .iter()
            .flat_map(|s| {
                s.effects.iter().filter_map(|e| {
                    (e.get("note").and_then(Value::as_str) == Some("rejected"))
                        .then(|| e.get("code").and_then(Value::as_str).unwrap_or("").to_string())
                })
            })
            .collect()
    }

    /// Every note the harness wrote for a core tool, as `(name, detail)`.
    #[must_use]
    pub fn core_tool_notes(&self) -> Vec<(String, String)> {
        self.steps
            .iter()
            .flat_map(|s| {
                s.effects.iter().filter_map(|e| {
                    (e.get("note").and_then(Value::as_str) == Some("core_tool")).then(|| {
                        (
                            e.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                            e.get("detail")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        )
                    })
                })
            })
            .collect()
    }
}

/// The tool an effect names, if it names one.
fn tool_name_of(e: &Value) -> Option<String> {
    match e.get("effect").and_then(Value::as_str) {
        Some("execute_tool") => e.get("name").and_then(Value::as_str).map(str::to_string),
        Some("ask_human") => e.get("tool").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// Run a tape through a fresh loop.
///
/// `gated_actions` is deliberately not a parameter: a replay that gates
/// differently from the run is measuring the platform's action list, not the
/// harness. Cases that need gating build the `Loop` themselves.
#[must_use]
pub fn replay(
    profile: Profile,
    registry: ToolRegistry,
    task: &TapeTask,
    tape: &Tape,
) -> Replay {
    let mut l = Loop::new(profile, registry, task.to_task());
    let mut steps = Vec::with_capacity(tape.len());
    for entry in tape {
        let effects = l.next(entry.to_input());
        let serialised: Vec<Value> = effects
            .iter()
            .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
            .collect();
        steps.push(TrajectoryStep {
            input: entry.clone(),
            effects: serialised,
            state: format!("{:?}", l.state()),
            working_state_hash: None,
            exposed: l.exposed().iter().cloned().collect(),
        });
        if l.state().is_terminal() {
            // The loop refuses further input once terminal, and feeding it the
            // rest of the tape would record a run of empty steps that looks like
            // a divergence. Stopping here is the honest record.
            break;
        }
    }
    Replay {
        steps,
        outcome: l.outcome().cloned(),
        path: l.path().to_vec(),
    }
}

// ── Diffing (AGENT-009 §6) ──────────────────────────────────────────────────

/// How serious a difference between a recording and a replay is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The same tools in a different order. Not a regression.
    Neutral,
    /// The candidate did more work for the same answer.
    Cost,
    /// The candidate did not reach a tool the recording did.
    RoutingRegression,
    /// The candidate reached a different conclusion. Blocks promotion unless a
    /// human accepts it with a note.
    Blocking,
}

/// One difference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub severity: Severity,
    pub detail: String,
}

impl Diff {
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }
}

/// Classify the differences between a recorded trajectory and its replay.
#[must_use]
pub fn diff(recorded: &Trajectory, got: &Replay) -> Vec<Diff> {
    let mut out = Vec::new();

    let want_tools: Vec<String> = recorded
        .steps
        .iter()
        .flat_map(|s| s.effects.iter().filter_map(tool_name_of))
        .collect();
    let got_tools = got.tools_called();

    let want_set: BTreeSet<&String> = want_tools.iter().collect();
    let got_set: BTreeSet<&String> = got_tools.iter().collect();

    for missing in want_set.difference(&got_set) {
        out.push(Diff {
            severity: Severity::RoutingRegression,
            detail: format!("`{missing}` was called in the recording and not in the replay"),
        });
    }
    for extra in got_set.difference(&want_set) {
        out.push(Diff {
            severity: Severity::Cost,
            detail: format!("`{extra}` was called in the replay and not in the recording"),
        });
    }
    if want_set == got_set && want_tools != got_tools {
        out.push(Diff {
            severity: Severity::Neutral,
            detail: format!(
                "same tools, different order: {want_tools:?} then {got_tools:?}"
            ),
        });
    }

    // The verdict. Compared by discriminant, not by text: an `Outcome::Finished`
    // whose result prose differs is the model saying the same thing differently,
    // and treating that as blocking would make the shadow replay unusable on the
    // one distribution it exists for — real sessions.
    let want_verdict = recorded
        .steps
        .last()
        .and_then(|s| s.effects.iter().find_map(outcome_tag));
    let got_verdict = got
        .steps
        .last()
        .and_then(|s| s.effects.iter().find_map(outcome_tag));
    if want_verdict != got_verdict {
        out.push(Diff {
            severity: Severity::Blocking,
            detail: format!(
                "verdict changed: recording ended {want_verdict:?}, replay ended {got_verdict:?}"
            ),
        });
    }
    out
}

fn outcome_tag(e: &Value) -> Option<String> {
    if e.get("effect").and_then(Value::as_str) != Some("done") {
        return None;
    }
    e.get("outcome").and_then(Value::as_str).map(str::to_string)
}

/// A stable hash of a value, for [`TrajectoryStep::working_state_hash`].
///
/// `DefaultHasher`, because `harness` may not gain a `sha2` dependency (gate
/// H-1) — and because this is a change-detector, not a commitment. It is never
/// used to seal anything; the ledger's anchors are elsewhere and use a real
/// hash.
#[must_use]
pub fn change_hash(v: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    format!("ws:{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::cases;

    #[test]
    fn a_tape_entry_round_trips_through_jsonl() {
        let t: Tape = vec![
            TapeEntry::Admit,
            TapeEntry::select("read_bars"),
            TapeEntry::fill(&serde_json::json!({"instrument": "BTC-USD"})),
            TapeEntry::ToolResult {
                output: "{\"bars\": 812}".into(),
                source: "platform".into(),
            },
        ];
        let lines: Vec<String> = t
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        let back: Tape = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(back, t, "a tape is a file a person edits by hand");
    }

    /// AGENT-009 §8.1: a run replays through the same harness version to
    /// identical effects. Asserted by replaying twice and comparing.
    #[test]
    fn a_tape_replays_to_identical_effects() {
        let (profile, registry, task, tape) = cases::lookup_fixture();
        let a = replay(profile.clone(), registry.clone(), &task, &tape);
        let b = replay(profile, registry, &task, &tape);
        assert_eq!(a.steps, b.steps, "the loop is sans-IO; replay is exact");
        assert_eq!(format!("{:?}", a.outcome), format!("{:?}", b.outcome));
    }

    #[test]
    fn a_trajectory_verifies_against_its_own_replay_with_no_differences() {
        let (profile, registry, task, tape) = cases::lookup_fixture();
        let r = replay(profile.clone(), registry.clone(), &task, &tape);
        let traj = Trajectory {
            trajectory_id: "t1".into(),
            profile_id: "test-local".into(),
            registry_hash: "reg:abc".into(),
            harness_version: "S4".into(),
            task: task.clone(),
            steps: r.steps.clone(),
        };
        assert!(traj.is_replayable());
        assert_eq!(traj.verify(profile, registry), Vec::new());
    }

    /// §6's classification, which is the reason the shadow report stays
    /// readable: a dropped tool and a reordering are not the same event.
    #[test]
    fn a_missing_tool_is_a_routing_regression_and_reordering_is_neutral() {
        let (profile, registry, task, tape) = cases::lookup_fixture();
        let r = replay(profile, registry, &task, &tape);
        let mut recorded = Trajectory {
            trajectory_id: "t1".into(),
            profile_id: "p".into(),
            registry_hash: "reg:abc".into(),
            harness_version: "S4".into(),
            task,
            steps: r.steps.clone(),
        };
        // Plant a tool in the recording that the replay does not produce.
        recorded.steps[0].effects.push(serde_json::json!({
            "effect": "execute_tool", "name": "read_bars", "namespace": "data",
            "arguments": {}, "risk": "read", "idempotent": true, "step": 1
        }));
        let d = diff(&recorded, &r);
        assert!(
            d.iter()
                .any(|x| x.severity == Severity::RoutingRegression
                    && x.detail.contains("read_bars")),
            "{d:?}"
        );
    }

    #[test]
    fn a_trajectory_with_no_registry_hash_is_not_replayable() {
        let (_, _, task, _) = cases::lookup_fixture();
        let t = Trajectory {
            trajectory_id: "t".into(),
            profile_id: "p".into(),
            registry_hash: String::new(),
            harness_version: "S4".into(),
            task,
            steps: vec![TrajectoryStep {
                input: TapeEntry::Admit,
                effects: vec![],
                state: "Routing".into(),
                working_state_hash: None,
                exposed: vec![],
            }],
        };
        assert!(
            !t.is_replayable(),
            "a replay against a different catalogue measures the catalogue"
        );
    }

    #[test]
    fn the_change_hash_is_stable_and_sensitive() {
        let a = serde_json::json!({"step": 1, "stage": "run"});
        let b = serde_json::json!({"step": 2, "stage": "run"});
        assert_eq!(change_hash(&a), change_hash(&a));
        assert_ne!(change_hash(&a), change_hash(&b));
    }
}
