//! Tracing (harness guide §15.4).
//!
//! One trace per task, spanning every sub-agent. The bar the guide sets is the right
//! one and it is strict: **if you cannot replay a task from its trace, observability
//! is insufficient.** That rules out logging only what went wrong — a replay needs
//! the successful steps too.
//!
//! The span carries a `trace_id` that is shared across sub-agents and a `parent_id`
//! that records who delegated. Without the shared id, a multi-agent failure is
//! several unrelated logs; with it, it is one story.

use serde::{Deserialize, Serialize};

use crate::policy::Decision;
use crate::provenance::Provenance;
use crate::registry::Risk;

/// What happened at one point in a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A model call. The prompt is recorded by hash, not by content: traces are kept
    /// far longer than prompts should be, and the hash is what a replay needs to
    /// confirm it rebuilt the same prompt.
    ModelCall {
        provider: String,
        model: String,
        prompt_hash: String,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        latency_ms: u64,
    },
    /// A tool call, with everything the policy keyed on.
    ToolCall {
        name: String,
        namespace: String,
        risk: Risk,
        /// Arguments, after coercion.
        args: serde_json::Value,
        result_bytes: usize,
        /// Whether the result was capped before entering context.
        truncated: bool,
        provenance: Provenance,
        latency_ms: u64,
        ok: bool,
    },
    /// A rung of the validation ladder rejected an output.
    ValidationFailure {
        code: String,
        message: String,
        attempt: u32,
    },
    /// The policy ruled on an action.
    PolicyRuling {
        tool: String,
        risk: Risk,
        decision: Decision,
        code: String,
    },
    /// A human answered an approval.
    Approval {
        approval_id: String,
        option: String,
        answered_by: String,
    },
    /// The context manager compacted.
    Compaction {
        dropped: usize,
        truncated: usize,
        deduped: usize,
        tokens_before: u32,
        tokens_after: u32,
    },
    /// Which tools were exposed for a step, and why.
    Exposure {
        namespaces: Vec<String>,
        exposed: Vec<String>,
        budget: usize,
    },
    /// A sub-agent was delegated to.
    Delegation {
        child_trace_id: String,
        goal: String,
        tool_grant: Vec<String>,
        budget_usd: f64,
    },
}

/// One entry in the task's trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    /// Shared by every agent working on one task (§13.3, §15.4).
    pub trace_id: String,
    /// The span that caused this one. `None` for the root.
    pub parent_id: Option<String>,
    pub span_id: String,
    /// Which agent: the orchestrator, or a named sub-agent.
    pub agent: String,
    /// Step number within that agent.
    pub step: u32,
    pub at: String,
    pub event: Event,
}

impl Span {
    #[must_use]
    pub fn new(
        trace_id: impl Into<String>,
        span_id: impl Into<String>,
        agent: impl Into<String>,
        step: u32,
        event: Event,
    ) -> Self {
        Self {
            trace_id: trace_id.into(),
            parent_id: None,
            span_id: span_id.into(),
            agent: agent.into(),
            step,
            at: String::new(),
            event,
        }
    }

    #[must_use]
    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent_id = Some(parent.into());
        self
    }

    /// Whether this span records something a replay needs.
    ///
    /// Used by `a_trace_of_only_failures_is_not_replayable` to state the rule: a
    /// trace made only of validation failures and rulings has no model calls and no
    /// tool calls, so nothing can be rebuilt from it.
    #[must_use]
    pub fn is_replayable_step(&self) -> bool {
        matches!(self.event, Event::ModelCall { .. } | Event::ToolCall { .. })
    }
}

/// Whether a collected trace could actually replay its task (§15.4).
///
/// The check is deliberately blunt: at least one model call, and every tool call
/// carrying its arguments. A trace that logged tool *names* without arguments looks
/// complete in a dashboard and cannot reproduce anything.
#[must_use]
pub fn is_replayable(spans: &[Span]) -> bool {
    if !spans
        .iter()
        .any(|s| matches!(s.event, Event::ModelCall { .. }))
    {
        return false;
    }
    spans.iter().all(|s| match &s.event {
        Event::ToolCall { args, .. } => !args.is_null(),
        _ => true,
    })
}

/// Every span for one task, in order, across all agents.
#[must_use]
pub fn for_trace<'a>(spans: &'a [Span], trace_id: &str) -> Vec<&'a Span> {
    spans.iter().filter(|s| s.trace_id == trace_id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model_span(trace: &str, agent: &str, step: u32) -> Span {
        Span::new(
            trace,
            format!("{agent}-{step}"),
            agent,
            step,
            Event::ModelCall {
                provider: "anthropic-agent-sdk".into(),
                model: "claude-opus-5".into(),
                prompt_hash: "sha256:abc".into(),
                input_tokens: 100,
                output_tokens: 20,
                cached_tokens: 80,
                latency_ms: 900,
            },
        )
    }

    fn tool_span(trace: &str, agent: &str, step: u32, args: serde_json::Value) -> Span {
        Span::new(
            trace,
            format!("{agent}-{step}t"),
            agent,
            step,
            Event::ToolCall {
                name: "read_bars".into(),
                namespace: "data".into(),
                risk: Risk::Read,
                args,
                result_bytes: 2048,
                truncated: false,
                provenance: Provenance::ToolInternal,
                latency_ms: 40,
                ok: true,
            },
        )
    }

    #[test]
    fn one_trace_id_spans_the_orchestrator_and_its_sub_agents() {
        let spans = vec![
            model_span("t1", "orchestrator", 1),
            model_span("t1", "critic", 1).with_parent("orchestrator-1"),
            model_span("t2", "orchestrator", 1),
        ];
        let t1 = for_trace(&spans, "t1");
        assert_eq!(t1.len(), 2, "a sub-agent shares its parent's trace id");
        assert_eq!(t1[1].parent_id.as_deref(), Some("orchestrator-1"));
    }

    #[test]
    fn a_trace_with_model_and_tool_calls_is_replayable() {
        let spans = vec![
            model_span("t1", "orchestrator", 1),
            tool_span("t1", "orchestrator", 1, json!({"instrument": "BTC"})),
        ];
        assert!(is_replayable(&spans));
    }

    /// The guide's bar, stated as a test: logging only what went wrong is not a
    /// trace, because the successful steps are what a replay is made of.
    #[test]
    fn a_trace_of_only_failures_is_not_replayable() {
        let spans = vec![Span::new(
            "t1",
            "s1",
            "orchestrator",
            1,
            Event::ValidationFailure {
                code: "validation.bad_type".into(),
                message: "x".into(),
                attempt: 1,
            },
        )];
        assert!(!is_replayable(&spans));
        assert!(!spans[0].is_replayable_step());
    }

    /// A dashboard showing tool names looks complete. It cannot reproduce anything.
    #[test]
    fn a_tool_call_without_arguments_breaks_replay() {
        let spans = vec![
            model_span("t1", "orchestrator", 1),
            tool_span("t1", "orchestrator", 1, serde_json::Value::Null),
        ];
        assert!(!is_replayable(&spans));
    }

    #[test]
    fn a_policy_ruling_records_what_it_keyed_on() {
        let s = Span::new(
            "t1",
            "s1",
            "orchestrator",
            2,
            Event::PolicyRuling {
                tool: "delete_run".into(),
                risk: Risk::Destructive,
                decision: Decision::Ask,
                code: "policy.destructive".into(),
            },
        );
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["event"]["risk"], "destructive");
        assert_eq!(json["event"]["decision"], "ask");
    }

    #[test]
    fn events_round_trip_through_serde() {
        let s = tool_span("t1", "a", 1, json!({"x": 1}));
        let back: Span = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.trace_id, "t1");
    }
}
