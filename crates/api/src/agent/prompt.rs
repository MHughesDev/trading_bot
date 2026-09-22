//! The charter, the run constraints, and the opening message (AGENT-005 §3).
//!
//! Before Set S task S-1.5 this module built one 15.5 KB `system_prompt` per
//! call: identity, a five-step research protocol, the whole 10.7 KB authoring
//! guide inlined, the run constraints, and the termination contract. Every byte
//! of it was re-sent on every model call at every tier, and on the hosted driver
//! it was re-sent uncached — measured at ~51 KB of byte-identical prefix per
//! call once the tool schemas are counted with it (AGENT-005 §0).
//!
//! Three things were wrong with that beyond its size:
//!
//! 1. **It mixed the stable with the varying.** The constraints change per run
//!    and sat in the middle of the prefix, so nothing in front of them could
//!    ever be cached. Layering is the whole mechanism of §6.1: tools → charter →
//!    opening message → history, most-stable first.
//! 2. **It was not counted.** On a 7B the charter alone took 28% of a
//!    14,000-token input budget against a declared `Section::Charter` share of
//!    8%. A budget that is not charged is not a budget.
//! 3. **It named tools the agent does not have.** `prompt.rs:31` instructed the
//!    model to "BUILD IT WITH THE DRAFT BUILDER — new_strategy_draft…" while
//!    `INTERNAL_AGENT_TOOLS` deliberately excludes the builder. A prompt that
//!    names an absent tool spends the model's turn on a call that cannot be
//!    dispatched, and the model has no way to learn it was the prompt that was
//!    wrong.
//!
//! So: [`charter`] is byte-stable and ≤ [`CHARTER_MAX_BYTES`], the protocol it
//! used to carry is served by `get_authoring_guide`, and everything that varies
//! per run moves into [`opening_message`], which is a *user* message and
//! therefore lives behind every cache breakpoint rather than in front of them.

use serde_json::Value;

/// The charter's hard ceiling, in bytes (AGENT-005 §3.1).
///
/// A number, not a goal. 3,200 B is ~800 tokens: 5.7% of a 7B's 14,000-token
/// input budget, which fits inside `Section::Charter`'s 10% share with room for
/// the workspace map. The old prompt was 15.5 KB.
pub const CHARTER_MAX_BYTES: usize = 3_200;

/// Identity, the edge-finding pointer, the tools contract, and termination.
///
/// Byte-stable across every session of a build — that is what makes it the
/// cacheable layer (§6.1) and what makes a local prompt reproducible. Nothing
/// here may depend on a run, a conversation, a clock or an environment: if it
/// varies, it belongs in [`opening_message`].
#[must_use]
pub fn charter() -> String {
    let mut p = String::with_capacity(CHARTER_MAX_BYTES);

    // 1. Identity. Verbatim from the prompt this replaces.
    p.push_str(
        "You are the research agent inside a quantitative trading platform. Your job \
         is to ANSWER THE GOAL YOU WERE GIVEN, using the tools you are offered, and to \
         report only what you actually established. You cannot place orders or trade — \
         you only research.\n\n\
         Answer the question that was asked, and nothing else. Many goals are \
         lookups — what data is stored, which venues are connected, what a strategy \
         does — and are finished by reading the answer and reporting it. Do not begin \
         strategy work that was not asked for. Do not describe work you did not do, and \
         never report a result you cannot point to a tool call for: saying \"I could \
         not determine this\" is a correct answer, and an invented one is not.\n\n",
    );

    // 2. Edge-finding: a pointer, not a protocol (≤ 900 B). The five steps moved
    // into the served guide (§3.3) because a lookup goal should not pay for them.
    p.push_str(
        "── FINDING AN EDGE ──\n\n\
         This applies only when the goal is to find a trading edge. Call \
         `get_authoring_guide` first: it holds the research protocol and the \
         definition format. Then `validate_strategy`, then `create_strategy` with \
         the definition object. Never create a raw backtest — an Experiment is the \
         only path, its holdout is locked, and every Study you run is a counted \
         trial. You never pick a parameter value: narrow the ranges and let the \
         sampler choose, and build on `carried_forward`, never on a best sample. \
         Change one thing at a time and keep the slug versioned. Prefer `run_sweep` \
         and `run_study`, which block server-side at no cost to you, over polling. \
         A candidate that fails a gate is a finding, not a failure — say why it \
         failed and move on.\n\n",
    );

    // 3. Tools (≤ 300 B). P3: the catalogue may be any size; the model sees a
    // slice, and the way to widen it is to ask, not to improvise.
    p.push_str(
        "── TOOLS ──\n\n\
         You see a slice of the catalogue chosen for this task. If the tool you need \
         is not offered, call `search_tools` with what you want to do and it will be \
         added. Do not work around a missing tool with a different one.\n\n",
    );

    // 4. Termination. Verbatim from the prompt this replaces.
    p.push_str(
        "── TERMINATION CONTRACT ──\n\n\
         When you are satisfied with a result (or your budget is nearly spent), call \
         the `finish_task` tool. Prose does not end the task — only that call does, \
         and it is checked:\n\
         - `result`: one paragraph on what you built and how it performed.\n\
         - `evidence`: the ids that support it — experiment_id, study ids, gate ledger \
         entries. A conclusion nobody can check is not a finding, and a finish_task \
         with an empty `evidence` is refused and handed back to you.\n\
         - `assessment`: what is fragile (cite the surface and the gate ledger, with \
         the trial count) and what you would try next.\n\
         - `strategy_id`, `experiment_id`, `backtest_id` where you have them.\n\n\
         \"No edge here\" is a complete and valuable result. Report it the same way, \
         with the evidence that establishes it.\n",
    );

    debug_assert!(
        p.len() <= CHARTER_MAX_BYTES,
        "charter is {} bytes, over the {CHARTER_MAX_BYTES}-byte ceiling",
        p.len()
    );
    p
}

/// The run's constraints, rendered for the opening message (§3.2).
///
/// `None` when the caller supplied nothing worth stating. An empty constraints
/// block used to render four lines of "your choice" advice, which is protocol —
/// it belongs in the guide, where it is read once, not in the prefix of every
/// call.
///
/// The old `asset_class: crypto_spot_cex (this platform's v1 focus)` line is
/// gone: eleven asset classes are the target and the line was stale.
#[must_use]
pub fn run_constraints(constraints: &Value) -> Option<String> {
    let field = |k: &str| constraints.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());

    let mut lines = Vec::new();
    if let Some(i) = field("instrument_id") {
        lines.push(format!("- instrument: {i} (required — use this)"));
    }
    if let Some(t) = field("timeframe") {
        lines.push(format!("- timeframe: {t}"));
    }
    if let (Some(s), Some(e)) = (field("start"), field("end")) {
        lines.push(format!("- backtest window: {s} → {e}"));
    }
    if let Some(b) = field("initial_balance") {
        lines.push(format!("- initial balance: {b}"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("── RUN CONSTRAINTS ──\n\n{}", lines.join("\n")))
}

/// The first user message: prior conclusions, then constraints, then the goal.
///
/// This is L3 of §2.1 — the last layer before the transcript, and the first one
/// that varies. Putting it in a user message rather than the system prompt is
/// what lets the charter and the tool schemas in front of it stay cached across
/// a whole conversation.
///
/// It also closes a live defect: `DriverParams::prior` was computed in
/// `manager.rs` and handed to the hosted driver, which never read it, so every
/// turn after the first started as though the conversation had not happened.
#[must_use]
pub fn opening_message(prior: &str, constraints: &Value, goal: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let prior = prior.trim();
    if !prior.is_empty() {
        parts.push(prior.to_string());
    }
    if let Some(c) = run_constraints(constraints) {
        parts.push(c);
    }
    if parts.is_empty() {
        return goal.to_string();
    }
    parts.push(format!("The user now asks: {goal}"));
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// AGENT-005 §3.1, AC-1. The ceiling is a number, not an aspiration: the old
    /// prompt was 15.5 KB and nothing failed, which is how it got there.
    #[test]
    fn the_charter_stays_small_enough_for_a_7b_window() {
        let c = charter();
        assert!(
            c.len() <= CHARTER_MAX_BYTES,
            "charter is {} bytes, ceiling is {CHARTER_MAX_BYTES}",
            c.len()
        );
        assert_eq!(c, charter(), "the charter must be byte-stable");
    }

    /// The adversarial half of the ceiling: a charter over the bound must *fail*,
    /// not be truncated into compliance. `render_charter` is the bounded
    /// constructor; this proves the bound rejects rather than trims.
    #[test]
    fn a_charter_over_the_ceiling_is_rejected_rather_than_truncated() {
        let over = "x".repeat(CHARTER_MAX_BYTES + 1);
        assert!(
            checked_charter(&over).is_err(),
            "a charter one byte over the ceiling must be refused"
        );
        let exact = "x".repeat(CHARTER_MAX_BYTES);
        assert_eq!(
            checked_charter(&exact).expect("exactly at the ceiling fits").len(),
            CHARTER_MAX_BYTES,
            "the check must not shorten a charter that fits"
        );
        // And the real one passes the same gate it imposes on a synthetic one.
        assert!(checked_charter(&charter()).is_ok());
    }

    /// AC-1's second half. Every backticked token in the charter that names a
    /// catalogue tool must be one the internal-agent profile actually exposes.
    ///
    /// A backticked token that is *not* a catalogue name is prose
    /// (`carried_forward`, `evidence`) and is left alone. That is what makes the
    /// test adversarial rather than a spell-check: `new_strategy_draft` is a real
    /// tool that the internal profile excludes, so naming it fails here — which
    /// is exactly the defect this task removes.
    #[test]
    fn the_charter_names_no_tool_the_internal_profile_lacks() {
        use mcp_server_lib::ToolProfile;

        let exposed: std::collections::BTreeSet<&str> =
            mcp_server_lib::spec::exposed(ToolProfile::InternalAgent)
                .into_iter()
                .map(|s| s.name)
                .collect();
        // Registered by the harness itself rather than the MCP catalogue, and
        // exposed on every profile.
        let harness_owned = ["search_tools", "finish_task", "record_finding"];

        let c = charter();
        let mut named = Vec::new();
        let mut rest = c.as_str();
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            named.push(&after[..close]);
            rest = &after[close + 1..];
        }
        assert!(!named.is_empty(), "the charter names tools in backticks");

        for token in named {
            if harness_owned.contains(&token) || exposed.contains(token) {
                continue;
            }
            assert!(
                mcp_server_lib::spec::find(token).is_none(),
                "the charter names `{token}`, which is a catalogue tool the \
                 internal-agent profile does not expose"
            );
        }
    }

    /// The four tools the charter must point at, and the two things §3.1 removed.
    #[test]
    fn the_charter_points_at_the_guide_and_at_real_tools() {
        let c = charter();
        for needle in [
            "get_authoring_guide",
            "search_tools",
            "finish_task",
            "validate_strategy",
        ] {
            assert!(c.contains(needle), "the charter must name {needle}");
        }
        assert!(
            !c.contains("new_strategy_draft"),
            "the draft builder is not exposed to the internal agent"
        );
        assert!(
            !c.contains("## Strategy definition"),
            "the authoring guide is served, not inlined"
        );
    }

    #[test]
    fn constraints_render_into_the_opening_message_not_the_charter() {
        let constraints = json!({ "instrument_id": "BTC-USD", "timeframe": "1h" });
        assert!(!charter().contains("BTC-USD"));
        let opening = opening_message("", &constraints, "what bars do we have?");
        assert!(opening.contains("BTC-USD"));
        assert!(opening.contains("- timeframe: 1h"));
        assert!(opening.ends_with("The user now asks: what bars do we have?"));
    }

    #[test]
    fn a_turn_with_prior_conclusions_starts_with_them() {
        let opening = opening_message(
            "Earlier: BTC-USD has 1m bars from 2019.",
            &json!({}),
            "now check ETH-USD",
        );
        assert!(opening.starts_with("Earlier: BTC-USD has 1m bars from 2019."));
        assert!(opening.ends_with("The user now asks: now check ETH-USD"));
    }

    /// With neither prior nor constraints the goal is passed through unchanged —
    /// no header, no "The user now asks", nothing for the model to parse around.
    #[test]
    fn a_goal_with_neither_prior_nor_constraints_is_passed_through_unchanged() {
        let goal = "which venues are connected?";
        assert_eq!(opening_message("", &json!({}), goal), goal);
        assert_eq!(opening_message("   ", &json!({}), goal), goal);
        assert_eq!(run_constraints(&json!({})), None);
        // Present-but-empty is the same as absent: an empty string is not a
        // constraint, and rendering "- instrument: " teaches the model nothing.
        assert_eq!(run_constraints(&json!({ "instrument_id": "" })), None);
    }

    /// A start with no end is half a window and is not rendered as one.
    #[test]
    fn a_half_specified_window_is_not_rendered() {
        let c = run_constraints(&json!({ "start": "2024-01-01T00:00:00Z" }));
        assert_eq!(c, None);
    }
}

/// The bound as a function, so a test can prove it rejects.
///
/// [`charter`] cannot return an over-long string — it is a literal — so a test
/// that only asserted `charter().len() <= MAX` would prove nothing about the
/// *rule*. This is the rule, applied to the real charter by
/// `a_charter_over_the_ceiling_is_rejected_rather_than_truncated` and available
/// to anything that assembles a charter from parts.
///
/// # Errors
/// The byte length, when it exceeds [`CHARTER_MAX_BYTES`].
pub fn checked_charter(text: &str) -> Result<&str, usize> {
    if text.len() > CHARTER_MAX_BYTES {
        return Err(text.len());
    }
    Ok(text)
}
