//! Stateful pipeline automation runtime.
//!
//! On each evaluation trigger, `PipelineRuntime::evaluate` re-computes per-instrument
//! stage membership, produces enter/exit deltas, and identifies instruments that have
//! cleared the final stage (ready for execution action).

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::automation::plan::{AutomationPlan, AutomationSpec, FilterStage};

/// Enter/exit delta for one pipeline stage after an evaluation.
#[derive(Debug, Clone)]
pub struct MembershipDelta {
    pub automation_id: Uuid,
    pub stage_id: String,
    /// Instruments that entered this stage this tick.
    pub entered: Vec<String>,
    /// Instruments that exited this stage this tick.
    pub exited: Vec<String>,
}

/// Result of evaluating a full pipeline plan.
pub struct PipelineEvalResult {
    /// Per-stage enter/exit deltas (only stages with changes are included).
    pub deltas: Vec<MembershipDelta>,
    /// Instruments that cleared the final stage this tick (ready for execution).
    pub final_stage_cleared: Vec<String>,
}

/// Stateful pipeline runtime — maintains stage membership across evaluations.
pub struct PipelineRuntime {
    /// Membership per `(automation_id, stage_id)`.
    memberships: HashMap<(Uuid, String), HashSet<String>>,
}

impl PipelineRuntime {
    pub fn new() -> Self {
        Self {
            memberships: HashMap::new(),
        }
    }

    /// Evaluate all stages of a pipeline automation.
    ///
    /// `filter_fn(stage, instrument_id)` returns `true` if `instrument_id` passes
    /// the filter for `stage`.  The caller supplies this closure so the runtime
    /// itself has no dependency on the strategy interpreter.
    pub fn evaluate<F>(&mut self, plan: &AutomationPlan, filter_fn: F) -> PipelineEvalResult
    where
        F: Fn(&FilterStage, &str) -> bool,
    {
        let AutomationSpec::Pipeline {
            universe, stages, ..
        } = &plan.spec
        else {
            return PipelineEvalResult {
                deltas: vec![],
                final_stage_cleared: vec![],
            };
        };

        let mut deltas = Vec::new();
        // Each stage narrows the universe for the next stage.
        let mut current_universe: Vec<String> = universe.clone();

        for stage in stages.iter() {
            let new_members: HashSet<String> = current_universe
                .iter()
                .filter(|inst| filter_fn(stage, inst.as_str()))
                .cloned()
                .collect();

            let key = (plan.id, stage.stage_id.clone());
            let old_members = self.memberships.entry(key.clone()).or_default();

            let entered: Vec<String> = new_members.difference(old_members).cloned().collect();
            let exited: Vec<String> = old_members.difference(&new_members).cloned().collect();

            if !entered.is_empty() || !exited.is_empty() {
                deltas.push(MembershipDelta {
                    automation_id: plan.id,
                    stage_id: stage.stage_id.clone(),
                    entered: entered.clone(),
                    exited: exited.clone(),
                });
            }

            *self.memberships.get_mut(&key).unwrap() = new_members.clone();

            // The next stage's universe is the current stage's survivors.
            current_universe = new_members.into_iter().collect();
        }

        // `current_universe` now holds the instruments that cleared ALL stages.
        let final_stage_cleared = current_universe;

        PipelineEvalResult {
            deltas,
            final_stage_cleared,
        }
    }

    /// Return the current member set for a specific stage.
    pub fn members(&self, automation_id: Uuid, stage_id: &str) -> HashSet<String> {
        self.memberships
            .get(&(automation_id, stage_id.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for PipelineRuntime {
    fn default() -> Self {
        Self::new()
    }
}
