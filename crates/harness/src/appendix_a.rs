//! Appendix A — defaults at a glance (harness guide v3).
//!
//! The table encoded as code, because a table nobody checks drifts. Every value is
//! a *starting default*, and the guide is explicit that **an eval beats the table**.
//!
//! So this module reports deviations rather than refusing them. What the conformance
//! test then requires is that each deviation is *stated* in the profile — an
//! explained deviation is a decision, an unexplained one is drift, and the table
//! exists to tell them apart.

use serde::Serialize;

use crate::profile::{Archetype, CodeExecution, Mode, Profile, SchemaStyle, Tier};

/// An inclusive range for one setting at one tier.
#[derive(Debug, Clone, Copy)]
pub struct Band {
    pub min: u64,
    pub max: u64,
}

impl Band {
    #[must_use]
    pub fn contains(self, v: u64) -> bool {
        v >= self.min && v <= self.max
    }

    fn exact(v: u64) -> Self {
        Self { min: v, max: v }
    }

    fn of(min: u64, max: u64) -> Self {
        Self { min, max }
    }
}

/// Appendix A's row for one tier.
#[derive(Debug, Clone, Copy)]
pub struct Defaults {
    pub max_exposed_per_step: Band,
    pub effective_budget_tokens: Band,
    pub max_steps: Band,
    pub schema_style: SchemaStyle,
    pub parallel_calls: bool,
    pub constrained_decoding_required: bool,
    pub mode: Mode,
    pub few_shot_examples: Band,
    pub code_execution: CodeExecution,
    pub reflection_allowed: bool,
}

/// The table, verbatim.
#[must_use]
pub fn defaults(tier: Tier) -> Defaults {
    match tier {
        // Below every Appendix A row on purpose. The table describes tiers the guide
        // expects to ship; `Degraded` describes a host that cannot run the reference
        // model, so its defaults are the tightest that still do anything useful.
        Tier::Degraded => Defaults {
            max_exposed_per_step: Band::of(1, 3),
            effective_budget_tokens: Band::of(2_000, 8_000),
            max_steps: Band::exact(1),
            schema_style: SchemaStyle::Flat,
            parallel_calls: false,
            constrained_decoding_required: true,
            mode: Mode::PlannerExecutor,
            few_shot_examples: Band::exact(2),
            code_execution: CodeExecution::Denied,
            reflection_allowed: false,
        },
        Tier::Frontier => Defaults {
            max_exposed_per_step: Band::of(20, 40),
            effective_budget_tokens: Band::of(60_000, 120_000),
            max_steps: Band::exact(30),
            schema_style: SchemaStyle::Rich,
            parallel_calls: true,
            constrained_decoding_required: false,
            mode: Mode::Freeform,
            few_shot_examples: Band::exact(0),
            code_execution: CodeExecution::Full,
            reflection_allowed: true,
        },
        Tier::LocalHigh => Defaults {
            max_exposed_per_step: Band::of(8, 10),
            effective_budget_tokens: Band::exact(32_000),
            max_steps: Band::exact(20),
            schema_style: SchemaStyle::Rich,
            parallel_calls: false,
            constrained_decoding_required: true,
            mode: Mode::PlannerExecutor,
            few_shot_examples: Band::exact(1),
            code_execution: CodeExecution::Full,
            reflection_allowed: false,
        },
        Tier::LocalMid => Defaults {
            max_exposed_per_step: Band::of(4, 6),
            effective_budget_tokens: Band::of(16_000, 24_000),
            max_steps: Band::exact(15),
            schema_style: SchemaStyle::Flat,
            parallel_calls: false,
            constrained_decoding_required: true,
            mode: Mode::PlannerExecutor,
            few_shot_examples: Band::exact(2),
            code_execution: CodeExecution::Templated,
            reflection_allowed: false,
        },
        Tier::LocalSmall => Defaults {
            max_exposed_per_step: Band::of(2, 3),
            effective_budget_tokens: Band::exact(8_000),
            max_steps: Band::exact(8),
            schema_style: SchemaStyle::Flat,
            parallel_calls: false,
            constrained_decoding_required: true,
            mode: Mode::PlannerExecutor,
            few_shot_examples: Band::exact(2),
            code_execution: CodeExecution::Denied,
            reflection_allowed: false,
        },
    }
}

/// Archetypes Appendix A lists a tier as eligible for.
///
/// A4 and A5 demand long-horizon reliability, so the guide gates them behind eval
/// evidence on local tiers (§9). This is what stops a local profile quietly
/// declaring itself a background agent.
#[must_use]
pub fn eligible_archetypes(tier: Tier) -> &'static [Archetype] {
    use Archetype::{
        Background, ChatAssistant, ComputerUse, KnowledgeWorker, MultiAgent, WorkflowAgent,
    };
    match tier {
        // A degraded host answers single questions. Anything that needs a chain of
        // tool calls is refused or escalated (see hardware::admit), which is the
        // whole point of naming the tier rather than silently running weaker.
        Tier::Degraded => &[ChatAssistant],
        Tier::Frontier => &[
            ChatAssistant,
            WorkflowAgent,
            KnowledgeWorker,
            ComputerUse,
            Background,
            MultiAgent,
        ],
        Tier::LocalHigh => &[
            ChatAssistant,
            WorkflowAgent,
            KnowledgeWorker,
            ComputerUse,
            Background,
        ],
        Tier::LocalMid => &[ChatAssistant, WorkflowAgent, KnowledgeWorker],
        Tier::LocalSmall => &[ChatAssistant, WorkflowAgent],
    }
}

/// One setting sitting outside its band.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Deviation {
    pub setting: &'static str,
    pub value: String,
    pub expected: String,
}

/// Compares a profile against Appendix A.
#[must_use]
pub fn deviations(p: &Profile) -> Vec<Deviation> {
    let d = defaults(p.tier);
    let mut out: Vec<Deviation> = Vec::new();
    let mut note = |setting: &'static str, value: String, expected: String| {
        out.push(Deviation {
            setting,
            value,
            expected,
        });
    };

    if !d
        .max_exposed_per_step
        .contains(p.tools.max_exposed_per_step as u64)
    {
        note(
            "tools.max_exposed_per_step",
            p.tools.max_exposed_per_step.to_string(),
            format!(
                "{}-{}",
                d.max_exposed_per_step.min, d.max_exposed_per_step.max
            ),
        );
    }
    if !d
        .effective_budget_tokens
        .contains(u64::from(p.context.effective_budget_tokens))
    {
        note(
            "context.effective_budget_tokens",
            p.context.effective_budget_tokens.to_string(),
            format!(
                "{}-{}",
                d.effective_budget_tokens.min, d.effective_budget_tokens.max
            ),
        );
    }
    if !d.max_steps.contains(u64::from(p.orchestration.max_steps)) {
        note(
            "orchestration.max_steps",
            p.orchestration.max_steps.to_string(),
            format!("{}-{}", d.max_steps.min, d.max_steps.max),
        );
    }
    if p.tools.schema_style != d.schema_style {
        note(
            "tools.schema_style",
            format!("{:?}", p.tools.schema_style),
            format!("{:?}", d.schema_style),
        );
    }
    if p.tools.parallel_calls != d.parallel_calls {
        note(
            "tools.parallel_calls",
            p.tools.parallel_calls.to_string(),
            d.parallel_calls.to_string(),
        );
    }
    if d.constrained_decoding_required && !p.output.constrained_decoding {
        note(
            "output.constrained_decoding",
            "false".into(),
            "true (required at this tier)".into(),
        );
    }
    if p.orchestration.mode != d.mode {
        note(
            "orchestration.mode",
            format!("{:?}", p.orchestration.mode),
            format!("{:?}", d.mode),
        );
    }
    if !d
        .few_shot_examples
        .contains(u64::from(p.orchestration.few_shot_examples))
    {
        note(
            "orchestration.few_shot_examples",
            p.orchestration.few_shot_examples.to_string(),
            format!("{}-{}", d.few_shot_examples.min, d.few_shot_examples.max),
        );
    }
    if p.orchestration.code_execution != d.code_execution {
        note(
            "orchestration.code_execution",
            format!("{:?}", p.orchestration.code_execution),
            format!("{:?}", d.code_execution),
        );
    }
    if p.orchestration.reflection && !d.reflection_allowed {
        note("orchestration.reflection", "true".into(), "false".into());
    }
    if !eligible_archetypes(p.tier).contains(&p.archetype) {
        note(
            "archetype",
            format!("{:?}", p.archetype),
            format!("one of {:?}", eligible_archetypes(p.tier)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::frontier_fixture;

    #[test]
    fn a_profile_matching_the_table_has_no_deviations() {
        let mut p = frontier_fixture();
        p.tools.max_exposed_per_step = 24;
        p.context.effective_budget_tokens = 120_000;
        p.orchestration.max_steps = 30;
        assert_eq!(deviations(&p), vec![]);
    }

    #[test]
    fn an_out_of_band_setting_is_reported_with_what_was_expected() {
        let mut p = frontier_fixture();
        p.tools.max_exposed_per_step = 24;
        p.context.effective_budget_tokens = 120_000;
        p.orchestration.max_steps = 999;
        let d = deviations(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].setting, "orchestration.max_steps");
        assert_eq!(d[0].expected, "30-30");
    }

    /// The rule that stops a local profile quietly claiming A5.
    #[test]
    fn a_local_mid_profile_declaring_a_background_archetype_deviates() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.archetype = Archetype::Background;
        let d = deviations(&p);
        assert!(
            d.iter().any(|x| x.setting == "archetype"),
            "A5 on a local tier needs eval evidence (guide §9)"
        );
    }

    #[test]
    fn the_table_tightens_monotonically_going_down_tier() {
        // Each step down the ladder is at most as permissive as the one above it.
        let tiers = [
            Tier::Frontier,
            Tier::LocalHigh,
            Tier::LocalMid,
            Tier::LocalSmall,
        ];
        for pair in tiers.windows(2) {
            let (hi, lo) = (defaults(pair[0]), defaults(pair[1]));
            assert!(lo.max_exposed_per_step.max <= hi.max_exposed_per_step.max);
            assert!(lo.effective_budget_tokens.max <= hi.effective_budget_tokens.max);
            assert!(lo.max_steps.max <= hi.max_steps.max);
            assert!(hi.parallel_calls || !lo.parallel_calls);
            assert!(eligible_archetypes(pair[1]).len() <= eligible_archetypes(pair[0]).len());
        }
    }

    #[test]
    fn every_local_tier_requires_constrained_decoding() {
        for t in [Tier::LocalSmall, Tier::LocalMid, Tier::LocalHigh] {
            assert!(defaults(t).constrained_decoding_required, "{t:?}");
            assert!(!defaults(t).reflection_allowed, "{t:?}");
        }
        assert!(!defaults(Tier::Frontier).constrained_decoding_required);
    }
}
