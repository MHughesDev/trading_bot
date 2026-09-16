//! The native transport's transcript, managed rather than merely appended
//! (AGENT-005 §6.4, AGENT-010 §3; Set S tasks S-2.2 and S-2.13).
//!
//! The two decode styles want opposite things from context, and that difference is
//! the whole of this module.
//!
//! `select_fill` is **single-turn**: every call re-renders the assembled body from
//! current state, so compaction happens in `context::assemble` and nothing
//! accumulates on the provider's side. A 7B never sees a conversation.
//!
//! `native` is a **conversation**: the provider keeps the thread, the model's own
//! tool-call template drives it, and the prefix is cacheable precisely because it
//! does not change. Re-rendering a body each call would throw the cache away —
//! S-1.8's breakpoints only pay if the bytes before them are identical from one call
//! to the next.
//!
//! ## Why this is not just a `Vec<Message>`
//!
//! Because an append-only transcript grows without bound, and a thirty-step research
//! run would exceed any window. S-2.13 names that exactly: *a regression hidden
//! inside an improvement*. So this appends **and** elides, under four rules, each of
//! which exists because the obvious alternative is wrong:
//!
//! 1. **The opening message is never elided.** It carries the goal. A transcript
//!    that dropped it would have the model working on whatever the most recent tool
//!    result implies, which is drift with extra steps.
//! 2. **Tool results go before assistant prose.** A tool result is reproducible —
//!    call it again — and the assistant's reasoning is not. Dropping the cheaper one
//!    first is the only ordering that loses less.
//! 3. **The last few results stay.** The model is usually acting on what it just
//!    read, and eliding that is how a run forgets the answer it was about to use.
//! 4. **Everything elided leaves a `work/…` handle.** AGENT-005 P6 forbids
//!    `[elided — was N bytes]`: a placeholder that cannot be fetched back is a
//!    permanent hole, and the model cannot tell it from a tool that returned
//!    nothing.
//!
//! Elision is decided at **step boundaries** rather than per call, because moving a
//! cache breakpoint invalidates everything after it — doing that once per step costs
//! one re-read, doing it per call costs the cache entirely.

use serde::Serialize;

use crate::context::{estimate_tokens, truncate_middle, CompactionReport};
use crate::profile::Profile;

/// One tool call the model asked for, as the transcript holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallRef {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// One turn of the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Turn {
    /// What the model said, and what it asked to call.
    Assistant {
        text: Option<String>,
        calls: Vec<CallRef>,
    },
    /// What one call returned.
    ToolResult {
        call_id: String,
        name: String,
        body: String,
        is_error: bool,
        /// Where the full body can be read once the transcript has elided it.
        handle: Option<String>,
    },
}

impl Turn {
    fn tokens(&self) -> u32 {
        match self {
            Turn::Assistant { text, calls } => {
                estimate_tokens(text.as_deref().unwrap_or(""))
                    + calls
                        .iter()
                        .map(|c| estimate_tokens(&c.arguments) + estimate_tokens(&c.name))
                        .sum::<u32>()
            }
            Turn::ToolResult { body, name, .. } => {
                estimate_tokens(body) + estimate_tokens(name)
            }
        }
    }

    const fn is_result(&self) -> bool {
        matches!(self, Turn::ToolResult { .. })
    }
}

/// The provider-side thread for one native run.
///
/// Provider-neutral on purpose: `crates/harness` does not depend on `crates/llm`,
/// because the loop is sans-IO and knowing about a wire format is knowing about IO.
/// The driver maps these to `llm::Message`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Transcript {
    /// The first user message: goal, constraints, prior turns. Never elided.
    opening: String,
    turns: Vec<Turn>,
}

/// How many trailing tool results are protected from elision.
///
/// Three, because the model is usually acting on what it just read and a step that
/// reads three things before deciding is ordinary. Two would elide inside a single
/// reasoning step; ten would leave nothing to elide on the runs that need it.
const PROTECTED_RESULTS: usize = 3;

/// The shortest a body is elided to. Enough to keep the shape of the answer, so a
/// model can tell "a large result lives behind this handle" from "the tool returned
/// nothing".
const ELIDED_BODY_BYTES: usize = 200;

impl Transcript {
    #[must_use]
    pub fn new(opening: impl Into<String>) -> Self {
        Self {
            opening: opening.into(),
            turns: Vec::new(),
        }
    }

    pub fn push_assistant(&mut self, text: Option<String>, calls: Vec<CallRef>) {
        self.turns.push(Turn::Assistant { text, calls });
    }

    pub fn push_result(
        &mut self,
        call_id: impl Into<String>,
        name: impl Into<String>,
        body: impl Into<String>,
        is_error: bool,
        handle: Option<String>,
    ) {
        self.turns.push(Turn::ToolResult {
            call_id: call_id.into(),
            name: name.into(),
            body: body.into(),
            is_error,
            handle,
        });
    }

    #[must_use]
    pub fn opening(&self) -> &str {
        &self.opening
    }

    #[must_use]
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Estimated tokens, opening included.
    #[must_use]
    pub fn tokens(&self) -> u32 {
        estimate_tokens(&self.opening) + self.turns.iter().map(Turn::tokens).sum::<u32>()
    }

    /// Bring the transcript inside the window, and say what it cost.
    ///
    /// `reserved` is what leaves the body on the wire and is charged to the same
    /// window — the tool schemas, on this transport — exactly as
    /// `context::assemble_reserving` treats them.
    ///
    /// Returns the same [`CompactionReport`] the block path returns, so one metric
    /// covers both transports. A platform with two compaction metrics has two
    /// numbers that disagree and no way to say which is the real one.
    pub fn compact(&mut self, profile: &Profile, reserved: u32) -> CompactionReport {
        let budget = profile
            .input_budget_tokens()
            .saturating_sub(reserved)
            // The opening is not negotiable, so it comes off the top rather than
            // competing with the turns for room.
            .saturating_sub(estimate_tokens(&self.opening));

        let mut report = CompactionReport {
            tokens_before: self.turns.iter().map(Turn::tokens).sum(),
            body_budget_tokens: budget,
            ..CompactionReport::default()
        };

        // Rule 3: the last few results are protected, and so is everything after the
        // earliest of them — the model is mid-reasoning over that stretch.
        let protected_from = self
            .turns
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, t)| t.is_result())
            .take(PROTECTED_RESULTS)
            .map(|(i, _)| i)
            .last()
            .unwrap_or(self.turns.len());

        // Rule 2: tool results before assistant prose. A result is reproducible —
        // call it again — and the assistant's reasoning is not.
        let mut order: Vec<usize> = (0..protected_from).filter(|i| self.turns[*i].is_result()).collect();
        order.extend((0..protected_from).filter(|i| !self.turns[*i].is_result()));

        let mut total: u32 = self.turns.iter().map(Turn::tokens).sum();
        for i in order {
            if total <= budget {
                break;
            }
            let before = self.turns[i].tokens();
            match &mut self.turns[i] {
                Turn::ToolResult { body, handle, .. } => {
                    if body.len() <= ELIDED_BODY_BYTES {
                        continue;
                    }
                    // Rule 4: a handle, always. `[elided — was N bytes]` is a
                    // permanent hole the model cannot tell from an empty result.
                    let where_to_read = handle.clone().unwrap_or_else(|| {
                        format!("work/transcript-turn-{i}.txt")
                    });
                    *body = format!(
                        "{}\n\n[the rest of this result is at {where_to_read}]",
                        truncate_middle(body, ELIDED_BODY_BYTES)
                    );
                    *handle = Some(where_to_read);
                    report.truncated += 1;
                }
                Turn::Assistant { text, .. } => {
                    // Prose only. The tool CALLS stay whatever happens: a transcript
                    // that dropped a call would leave its result answering nothing,
                    // and most providers reject a result with no matching call.
                    if text.as_deref().map_or(0, str::len) <= ELIDED_BODY_BYTES {
                        continue;
                    }
                    *text = text.as_deref().map(|t| truncate_middle(t, ELIDED_BODY_BYTES));
                    report.truncated += 1;
                }
            }
            total = total.saturating_sub(before.saturating_sub(self.turns[i].tokens()));
        }

        report.tokens_after = self.turns.iter().map(Turn::tokens).sum();
        // Honest about not having managed it. A report claiming success on a
        // transcript still over budget is the thing that makes the metric useless.
        report.needs_model_summary = report.tokens_after > budget;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSet;

    fn profile() -> Profile {
        ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
            .expect("the shipped profiles load")
            .get("claude-opus-5")
            .expect("the frontier profile ships")
            .clone()
    }

    /// A window that leaves a small but **positive** input budget.
    ///
    /// The frontier profile reserves 16,000 tokens for output, so any window below
    /// that leaves `input_budget_tokens() == 0` — and every test here would then
    /// pass by eliding everything, which is passing for the wrong reason. 20,000
    /// leaves 4,000 to compact into, which is small enough to force the choices
    /// these tests are about and large enough that making them matters.
    fn tight() -> Profile {
        let mut p = profile();
        p.context.effective_budget_tokens = 20_000;
        p
    }

    fn call(i: usize) -> CallRef {
        CallRef {
            id: format!("call_{i}"),
            name: "get_bars".into(),
            arguments: r#"{"instrument":"BTC-USD"}"#.into(),
        }
    }

    /// A two-hundred-step run stays inside the window. This is the regression S-2.13
    /// exists to prevent: an append-only transcript is an improvement that grows
    /// without bound, and the growth is invisible until a run fails at step ninety.
    #[test]
    fn a_two_hundred_step_transcript_stays_under_budget() {
        let p = profile();
        let mut t = Transcript::new("find whether BTC-USD has momentum at 1h");
        for i in 0..200 {
            t.push_assistant(Some("Let me read the bars for this window.".repeat(20)), vec![call(i)]);
            t.push_result(
                format!("call_{i}"),
                "get_bars",
                "x".repeat(4_000),
                false,
                None,
            );
        }
        let before = t.tokens();
        let report = t.compact(&p, 2_000);
        let after = t.tokens();

        assert!(after < before, "compaction did something: {before} -> {after}");
        assert!(
            after <= p.input_budget_tokens(),
            "a 200-step transcript must fit the window: {after} > {}",
            p.input_budget_tokens()
        );
        assert!(report.truncated > 0, "and it reports what it did");
        assert!(
            !report.needs_model_summary,
            "deterministic elision was enough; a report claiming otherwise would be honest \
             and this one does not need to be"
        );
    }

    /// The goal survives, whatever else does not. A transcript that elided its
    /// opening would have the model working on whatever the most recent tool result
    /// implies, which is drift with extra steps.
    #[test]
    fn the_opening_message_is_never_elided() {
        let p = profile();
        let goal = "find whether BTC-USD has momentum at 1h";
        let mut t = Transcript::new(goal);
        for i in 0..300 {
            t.push_result(format!("c{i}"), "get_bars", "y".repeat(8_000), false, None);
        }
        t.compact(&p, 2_000);
        assert_eq!(t.opening(), goal, "the goal is not negotiable");
    }

    /// Tool results are elided before assistant prose, because a result can be
    /// fetched again and the model's reasoning cannot.
    #[test]
    fn a_reproducible_result_is_elided_before_irreproducible_reasoning() {
        let p = tight();
        let mut t = Transcript::new("goal");
        // One long piece of reasoning, then many long results after it.
        t.push_assistant(Some("R".repeat(6_000)), vec![call(0)]);
        for i in 0..10 {
            t.push_result(format!("c{i}"), "get_bars", "z".repeat(6_000), false, None);
        }
        t.compact(&p, 0);

        let Turn::Assistant { text, .. } = &t.turns()[0] else {
            panic!("the first turn is the assistant's");
        };
        let reasoning_len = text.as_deref().unwrap_or("").len();
        let elided_results = t
            .turns()
            .iter()
            .filter(|x| matches!(x, Turn::ToolResult { handle: Some(_), .. }))
            .count();
        assert!(
            elided_results > 0,
            "results were elided first: {elided_results}"
        );
        assert!(
            reasoning_len > 200,
            "and the reasoning survived while there were results to take: {reasoning_len}"
        );
    }

    /// The last few results stay. The model is usually acting on what it just read,
    /// and eliding that is how a run forgets the answer it was about to use.
    #[test]
    fn the_most_recent_results_are_protected() {
        let p = tight();
        let mut t = Transcript::new("goal");
        for i in 0..40 {
            t.push_result(format!("c{i}"), "get_bars", "w".repeat(5_000), false, None);
        }
        t.compact(&p, 0);

        let tail = &t.turns()[t.len() - PROTECTED_RESULTS..];
        for turn in tail {
            let Turn::ToolResult { handle, .. } = turn else {
                panic!("all results here");
            };
            assert!(
                handle.is_none(),
                "the last {PROTECTED_RESULTS} results are untouched"
            );
        }
    }

    /// Every elided body leaves a readable handle. AGENT-005 P6 forbids the
    /// alternative: a placeholder that cannot be fetched back is a permanent hole,
    /// and the model cannot tell it from a tool that returned nothing.
    #[test]
    fn every_elided_body_says_where_to_read_the_rest() {
        let p = tight();
        let mut t = Transcript::new("goal");
        for i in 0..40 {
            t.push_result(format!("c{i}"), "get_bars", "v".repeat(5_000), false, None);
        }
        t.compact(&p, 0);

        for turn in t.turns() {
            let Turn::ToolResult { body, handle, .. } = turn else {
                continue;
            };
            if let Some(h) = handle {
                assert!(h.starts_with("work/"), "a fetchable handle: {h}");
                assert!(
                    body.contains(h.as_str()),
                    "and the body says where to read it: {body}"
                );
                assert!(
                    !body.contains("[elided"),
                    "never the unrecoverable placeholder AGENT-005 P6 forbids"
                );
            }
        }
    }

    /// A tool CALL is never dropped, only the prose beside it. A transcript missing
    /// a call leaves its result answering nothing, and most providers reject a
    /// result with no matching call outright.
    #[test]
    fn a_tool_call_is_never_elided_only_the_prose_around_it() {
        let p = tight();
        let mut t = Transcript::new("goal");
        for i in 0..30 {
            t.push_assistant(Some("P".repeat(4_000)), vec![call(i)]);
            t.push_result(format!("call_{i}"), "get_bars", "q".repeat(4_000), false, None);
        }
        t.compact(&p, 0);

        let calls: Vec<&CallRef> = t
            .turns()
            .iter()
            .filter_map(|x| match x {
                Turn::Assistant { calls, .. } => calls.first(),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 30, "every call survives");
        for (i, c) in calls.iter().enumerate() {
            assert_eq!(c.id, format!("call_{i}"), "and keeps its id, so the result matches");
        }
    }

    /// A transcript that already fits is not touched. Compaction that ran anyway
    /// would move the cache breakpoint for nothing, which costs the whole prefix.
    #[test]
    fn a_transcript_that_fits_is_left_alone() {
        let p = profile();
        let mut t = Transcript::new("goal");
        t.push_assistant(Some("short".into()), vec![call(0)]);
        t.push_result("call_0", "get_bars", "{\"bars\": 500}", false, None);
        let before = t.clone();
        let report = t.compact(&p, 1_000);
        assert_eq!(report.truncated, 0);
        assert_eq!(t.turns(), before.turns(), "untouched");
    }

    /// The same report type as the block path, so one metric covers both transports.
    /// Two compaction metrics is two numbers that disagree with no way to say which
    /// is real.
    #[test]
    fn compaction_reports_the_same_shape_as_the_block_path() {
        let p = tight();
        let mut t = Transcript::new("goal");
        for i in 0..20 {
            t.push_result(format!("c{i}"), "get_bars", "u".repeat(5_000), false, None);
        }
        let r: CompactionReport = t.compact(&p, 0);
        assert!(r.tokens_before > r.tokens_after);
        assert!(r.body_budget_tokens > 0);
    }
}
