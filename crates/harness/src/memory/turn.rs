//! Episodic memory: the turn summary (AGENT-007 §2.1; Set S task S-4.1).
//!
//! # What it replaces
//!
//! `conversations::prior_context` built a `"you concluded: …"` string from **the
//! model's own final prose**, and the hosted driver then ignored it entirely
//! (AGENT-005 §0). So the only cross-turn memory this platform had was a
//! paraphrase, written by the thing being remembered, that nothing read.
//!
//! A [`TurnSummary`] is written by the *harness*, from [`WorkingState`] — the
//! findings ledger and the effect log — and it is a schema, not a sentence. The
//! distinction is the whole point: a paraphrase loses the ids, and an id is the
//! only part of a finding the next turn can act on.
//!
//! # The truncation rule
//!
//! §2.1 says ≤ 600 tokens, "truncating lists by confidence, never the ids". Both
//! halves are enforced by [`TurnSummary::render`]:
//!
//! * `established` findings are rendered before `suggestive` ones, and the cut
//!   falls at the end.
//! * every rendered line carries its id, and when a line would not fit, the line
//!   is dropped **whole** and replaced by a counted elision. A half-rendered
//!   finding is a finding whose id survived and whose claim did not, which is
//!   worse than an absent one — the next turn would cite it.
//!
//! [`WorkingState`]: crate::state::WorkingState

use serde::{Deserialize, Serialize};

use crate::context::estimate_tokens;
use crate::state::{Stage, WorkingState};

use super::Confidence;

/// §2.1's budget. A cross-turn carry bigger than this is a carry that pushes out
/// the work.
pub const MAX_SUMMARY_TOKENS: u32 = 600;

/// One finding, as the next turn needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryFinding {
    pub id: String,
    pub claim: String,
    /// `{"experiment_id": "exp_…"}` — the handles the next turn may cite.
    #[serde(default)]
    pub ids: std::collections::BTreeMap<String, String>,
    pub n: Option<u64>,
    pub se: Option<f64>,
    pub confidence: Confidence,
}

/// One dead idea, and what would have to change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryNegative {
    pub ticket_id: String,
    pub class: String,
    pub why: String,
    #[serde(default)]
    pub retry_condition: Option<String>,
}

/// What the turn spent.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TurnBudget {
    pub tokens: u64,
    pub usd_tenth_cents: i64,
}

/// An idea the turn considered and put down, with the reason.
///
/// Without this the next turn re-proposes it, and the conversation loops at the
/// scale of turns rather than steps — the same barren-loop failure one level up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedIdea {
    pub idea: String,
    pub reason: String,
}

/// The structured record of one turn (AGENT-007 §2.1's schema, exactly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnSummary {
    pub turn: i32,
    pub goal: String,
    /// `finished | refused | escalated | fenced | cancelled`.
    pub outcome: String,
    #[serde(default)]
    pub findings: Vec<SummaryFinding>,
    #[serde(default)]
    pub negative_results: Vec<SummaryNegative>,
    pub trials_spent: u32,
    pub n_eff_after: Option<f64>,
    #[serde(default)]
    pub gate_states: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub rejected_ideas: Vec<RejectedIdea>,
    pub budget: TurnBudget,
    pub yield_ref: Option<String>,
    pub stage: Stage,
    pub current_step: Option<String>,
}

impl TurnSummary {
    /// Build the summary from what the harness holds.
    ///
    /// Note the inputs: [`WorkingState`] and the two ledgers. **Not** the
    /// model's final message. That is S-4.1 in one function signature — there is
    /// no parameter here through which prose could arrive.
    #[must_use]
    pub fn from_state(
        turn: i32,
        outcome: &str,
        state: &WorkingState,
        findings: Vec<SummaryFinding>,
        negative_results: Vec<SummaryNegative>,
        budget: TurnBudget,
    ) -> Self {
        Self {
            turn,
            goal: state.goal.clone(),
            outcome: outcome.to_string(),
            findings,
            negative_results,
            trials_spent: state.budget.trials_spent,
            n_eff_after: state.budget.n_eff,
            gate_states: std::collections::BTreeMap::new(),
            open_questions: state.open_questions.clone(),
            rejected_ideas: state
                .decided
                .iter()
                .filter(|d| !d.reason.trim().is_empty())
                .map(|d| RejectedIdea {
                    idea: d.question.clone(),
                    reason: format!("{} — {}", d.choice, d.reason),
                })
                .collect(),
            budget,
            yield_ref: None,
            stage: state.stage,
            current_step: state
                .active_step()
                .map(|(_, s)| s.goal.clone())
                .or_else(|| state.plan.last().map(|s| s.goal.clone())),
        }
    }

    /// Render for the next turn's opening message, within `max_tokens`.
    ///
    /// The order is the priority order: what was *established* first, what died
    /// second (because a dead idea saves the most budget), then the questions
    /// still open. Budget and stage are last and are one line each — they are
    /// context, not content.
    #[must_use]
    pub fn render(&self, max_tokens: u32) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "[turn {}] goal: {}\noutcome: {}",
            self.turn, self.goal, self.outcome
        ));
        if self.trials_spent > 0 {
            out.push_str(&format!(", trials spent: {}", self.trials_spent));
        }
        if let Some(n) = self.n_eff_after {
            out.push_str(&format!(", n_eff after: {n:.1}"));
        }
        out.push('\n');

        // `established` before `suggestive` before `refuted`: §2.1's truncation
        // rule, applied before the cut rather than after it.
        let mut ranked: Vec<&SummaryFinding> = self.findings.iter().collect();
        ranked.sort_by_key(|f| f.confidence);

        let mut sections: Vec<(String, Vec<String>)> = Vec::new();
        if !ranked.is_empty() {
            sections.push((
                "established".into(),
                ranked.iter().map(|f| finding_line(f)).collect(),
            ));
        }
        if !self.negative_results.is_empty() {
            sections.push((
                "ruled out".into(),
                self.negative_results.iter().map(negative_line).collect(),
            ));
        }
        if !self.gate_states.is_empty() {
            sections.push((
                "gates".into(),
                self.gate_states
                    .iter()
                    .map(|(k, v)| format!("- {k}: {v}"))
                    .collect(),
            ));
        }
        if !self.open_questions.is_empty() {
            sections.push((
                "still open".into(),
                self.open_questions
                    .iter()
                    .map(|q| format!("- {q}"))
                    .collect(),
            ));
        }
        if !self.rejected_ideas.is_empty() {
            sections.push((
                "already decided".into(),
                self.rejected_ideas
                    .iter()
                    .map(|r| format!("- {}: {}", r.idea, r.reason))
                    .collect(),
            ));
        }

        let tail = self.tail();
        // The tail is reserved before anything else is rendered. A summary that
        // truncates away "you have $0.02 left" has truncated the one line that
        // changes what the next turn should do.
        let reserve = estimate_tokens(&tail);
        for (title, lines) in sections {
            let header = format!("{title}:\n");
            if estimate_tokens(&out) + estimate_tokens(&header) + reserve >= max_tokens {
                break;
            }
            out.push_str(&header);
            let mut dropped = 0usize;
            for line in &lines {
                let candidate = format!("{line}\n");
                if estimate_tokens(&out) + estimate_tokens(&candidate) + reserve >= max_tokens {
                    dropped += 1;
                    continue;
                }
                out.push_str(&candidate);
            }
            if dropped > 0 {
                // Counted, so the next turn knows the list is partial and can ask
                // for the rest with `memory.recall_turn`. A silent truncation
                // reads as "that is all there was".
                out.push_str(&format!(
                    "- (+{dropped} more, lowest confidence first; memory.recall_turn({}) for all)\n",
                    self.turn
                ));
            }
        }
        out.push_str(&tail);
        out
    }

    fn tail(&self) -> String {
        let mut t = format!("stage: {:?}", self.stage);
        if let Some(s) = &self.current_step {
            t.push_str(&format!(", current step: {s}"));
        }
        t.push_str(&format!(
            "\nspent this turn: {} tokens, ${:.2}\n",
            self.budget.tokens,
            self.budget.usd_tenth_cents as f64 / 1000.0
        ));
        t
    }

    /// Every id this summary carries, in render order.
    ///
    /// The property S-4.1's acceptance names — "a second turn receives the
    /// first's conclusions **with ids**" — needs to be checkable without reading
    /// prose, on both a 7B render and a frontier render.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut v = Vec::new();
        for f in &self.findings {
            v.push(f.id.clone());
            v.extend(f.ids.values().cloned());
        }
        for n in &self.negative_results {
            v.push(n.ticket_id.clone());
        }
        v
    }
}

fn finding_line(f: &SummaryFinding) -> String {
    let mut s = format!("- [{}, {}", f.id, f.confidence.as_str());
    if let Some(n) = f.n {
        s.push_str(&format!(", n={n}"));
    }
    if let Some(se) = f.se {
        s.push_str(&format!(", se={se:.3}"));
    }
    for (role, id) in &f.ids {
        s.push_str(&format!(", {role}={id}"));
    }
    s.push_str(&format!("] {}", f.claim));
    s
}

fn negative_line(n: &SummaryNegative) -> String {
    let retry = n.retry_condition.as_deref().map_or_else(
        || " (no retry condition)".to_string(),
        |r| format!(" — {r}"),
    );
    format!("- [{}, {}] {}{retry}", n.ticket_id, n.class, n.why)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{GoalKind, WorkingState};

    fn f(id: &str, c: Confidence, claim: &str) -> SummaryFinding {
        let mut ids = std::collections::BTreeMap::new();
        ids.insert("experiment_id".to_string(), format!("exp_{id}0000"));
        SummaryFinding {
            id: id.to_string(),
            claim: claim.to_string(),
            ids,
            n: Some(812),
            se: Some(0.04),
            confidence: c,
        }
    }

    fn summary(findings: Vec<SummaryFinding>) -> TurnSummary {
        TurnSummary {
            turn: 3,
            goal: "does BTC-USD 1h momentum survive costs".into(),
            outcome: "finished".into(),
            findings,
            negative_results: vec![SummaryNegative {
                ticket_id: "tkt_00011122".into(),
                class: "weak_effect".into(),
                why: "residual t = 0.4".into(),
                retry_condition: None,
            }],
            trials_spent: 14,
            n_eff_after: Some(37.0),
            gate_states: [("exp_9001".to_string(), "gate_2".to_string())]
                .into_iter()
                .collect(),
            open_questions: vec!["does it hold at 1d".into()],
            rejected_ideas: vec![RejectedIdea {
                idea: "which timeframe".into(),
                reason: "1h — 1m has no coverage before 2026-03".into(),
            }],
            budget: TurnBudget {
                tokens: 41_200,
                usd_tenth_cents: 830,
            },
            yield_ref: None,
            stage: Stage::Run,
            current_step: Some("compare against the null".into()),
        }
    }

    /// S-4.1's acceptance, the part that matters: the next turn gets ids, not
    /// prose. Asserted on a small render (7B) and a large one (frontier).
    #[test]
    fn a_second_turn_receives_the_first_conclusions_with_ids_on_both_tiers() {
        let s = summary(vec![
            f("f_1", Confidence::Established, "momentum beats the null"),
            f("f_2", Confidence::Suggestive, "carry may add"),
        ]);
        for budget in [180u32, MAX_SUMMARY_TOKENS] {
            let out = s.render(budget);
            assert!(
                out.contains("f_1"),
                "the highest-confidence id must survive every budget ({budget}):\n{out}"
            );
            assert!(out.contains("exp_f_10000"), "{out}");
            assert!(
                out.contains("tkt_00011122"),
                "a dead idea is the cheapest carry:\n{out}"
            );
            assert!(
                estimate_tokens(&out) <= budget,
                "render must fit {budget}: {}",
                estimate_tokens(&out)
            );
        }
    }

    /// §2.1: truncate by confidence, never the ids. A line is dropped whole.
    #[test]
    fn truncation_drops_the_least_confident_line_whole_and_says_so() {
        let mut findings = vec![f("f_1", Confidence::Established, "momentum beats the null")];
        for i in 0..12 {
            findings.push(f(
                &format!("f_s{i}"),
                Confidence::Suggestive,
                "a long suggestive claim that exists only to consume the budget",
            ));
        }
        let out = summary(findings).render(140);
        assert!(out.contains("f_1"), "{out}");
        assert!(out.contains("more, lowest confidence first"), "{out}");
        // Nothing half-rendered: every `[f_` opened is closed on the same line.
        for line in out.lines() {
            let opens = line.matches("[f_").count();
            if opens > 0 {
                assert!(line.contains(']'), "half-rendered finding: {line}");
            }
        }
    }

    /// The tail is reserved, not truncated. "You have $0.02 left" is the line
    /// that changes what the next turn does.
    #[test]
    fn the_budget_tail_survives_an_impossible_budget() {
        let s = summary(vec![f("f_1", Confidence::Established, "x")]);
        let out = s.render(30);
        assert!(out.contains("spent this turn"), "{out}");
    }

    #[test]
    fn the_summary_is_built_from_working_state_and_not_from_prose() {
        let mut st = WorkingState::new("does BTC-USD 1h momentum survive costs");
        st.kind = GoalKind::Research;
        st.open_questions.push("does it hold at 1d".into());
        st.budget.trials_spent = 14;
        st.budget.n_eff = Some(37.0);
        let s = TurnSummary::from_state(
            3,
            "finished",
            &st,
            vec![f("f_1", Confidence::Established, "momentum beats the null")],
            Vec::new(),
            TurnBudget::default(),
        );
        assert_eq!(s.trials_spent, 14);
        assert_eq!(s.n_eff_after, Some(37.0));
        assert_eq!(s.open_questions, vec!["does it hold at 1d".to_string()]);
        assert_eq!(s.ids()[0], "f_1");
    }

    #[test]
    fn the_schema_round_trips_so_the_table_and_the_renderer_agree() {
        let s = summary(vec![f("f_1", Confidence::Established, "x")]);
        let json = serde_json::to_string(&s).unwrap();
        let back: TurnSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // The §2.1 field names are the wire contract; migration 0065 stores this
        // document verbatim.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for key in [
            "turn",
            "goal",
            "outcome",
            "findings",
            "negative_results",
            "trials_spent",
            "n_eff_after",
            "gate_states",
            "open_questions",
            "rejected_ideas",
            "budget",
            "stage",
            "current_step",
        ] {
            assert!(v.get(key).is_some(), "missing §2.1 field {key}");
        }
    }
}
