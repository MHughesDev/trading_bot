//! Assembly is a query (AGENT-005 §15, §24, §27.2; Set S tasks S-9.4, S-9.13,
//! S-9.14, S-9.18).
//!
//! # The change
//!
//! Part I answers "what happened recently". This answers "what does this step
//! need". The query is the [`WorkingState`](crate::state::WorkingState): the goal
//! and its kind, the active plan step and the tool it names, the hypothesis, the
//! last error, the ids already cited. Every signal is structural and cheap —
//! there is no model call and no wall clock anywhere in this path, which is what
//! makes the whole of Part II replayable and A/B-testable offline.
//!
//! # Chosen by relevance, read in a stable order
//!
//! Winners are **re-sorted into the fixed layout** before rendering: pinned
//! first, then by section, then by step ascending, then by id. The model reads a
//! stable, chronological-looking prompt while the harness chose it by relevance.
//! Both properties matter and they are not in tension — mixing them up would
//! either thrash the provider cache or hand the model a prompt whose order
//! changes meaning between calls.
//!
//! # The five ways a weighted sum goes wrong (§24)
//!
//! A weighted sum is not obviously better than recency, and in four specific ways
//! it is worse. Each has a mechanism here rather than a caveat:
//!
//! * **Diversity.** Top-k picks ten records about one subject, because whatever
//!   scores one highly scores its neighbours highly. That is *strictly worse than
//!   recency*, which at least samples across time. Fixed by a redundancy penalty
//!   applied as the set is built, plus hard floors per kind.
//! * **Requirements.** The record that produced the `experiment_id` the next step
//!   needs is not a candidate, it is a **must-include**, satisfied before scoring.
//! * **Normalisation.** Recency is `[0,1]` and citation count is unbounded; summed
//!   raw, the unbounded signal wins and the weights are decoration. Every signal
//!   is percentile-ranked against the current store.
//! * **Hysteresis.** One threshold makes a record at the cut enter and leave on
//!   alternate calls, which invalidates the prefix cache every time. Two
//!   thresholds and a stickiness bonus.
//! * **Ties.** `(step asc, id asc)`, never map order — a `HashMap` iteration
//!   order would make one store render two different prompts.
//!
//! And two things the penalty must never do (§24.7, §24.8): drop a correction for
//! a tool the model is **about to** reach, and silently pick a winner between two
//! records that *disagree*.

use std::collections::{BTreeMap, BTreeSet};

use super::body::Body;
use super::degrade::{self, Candidate, Fit};
use super::policy::{ContextPolicy, Normalisation, SelectionMode};
use super::store::{Record, RecordId, RecordKind, RunStore, Subject};
use crate::state::{GoalKind, WorkingState};

/// How many records the marginal-relevance loop scores against each other.
///
/// See the comment at its use: the cap bounds the quadratic term so assembly
/// time stays flat past a thousand records, and it cannot hide a must-include or
/// a floor because both of those search the whole store.
pub const MMR_CANDIDATES: usize = 200;

/// What the step needs, derived from the working state.
///
/// Built by [`Query::from_state`] rather than assembled by callers, so the query
/// cannot drift from the state it claims to be a view of.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub step: u32,
    pub goal_kind: GoalKind,
    pub subject: Subject,
    /// The tool the active plan step names.
    pub active_tool: Option<String>,
    /// The tools the *remaining* plan steps name (§24.7).
    pub upcoming_tools: Vec<String>,
    /// Ids the next call is known to need. Records carrying them are
    /// must-includes, not candidates (§24.2).
    pub required_ids: BTreeSet<String>,
    /// Ids the run has already cited.
    pub cited_ids: BTreeSet<String>,
    /// What the model said it is working on (§18's `focus`). A better relevance
    /// signal than anything the harness can infer, so it is taken at face value.
    pub focus: Option<String>,
    /// What was rendered on the previous call, for stickiness (§24.5).
    pub previously_rendered: BTreeSet<RecordId>,
    /// Records held out of this assembly.
    ///
    /// Exists for the counterfactual ablation (§28.1), which asks what the step
    /// would have seen without one record. It is a *query* field rather than a
    /// separate entry point so the ablation runs the real selector — an ablation
    /// against a simplified copy of selection would measure the copy.
    pub excluded: BTreeSet<RecordId>,
}

impl Query {
    /// The query a working state implies.
    #[must_use]
    pub fn from_state(state: &WorkingState, step: u32) -> Self {
        let active = state.active_step().map(|(i, _)| i);
        let active_tool = state
            .active_step()
            .and_then(|(_, s)| s.kind.tool_name())
            .map(str::to_string);
        // Everything after the active step. §24.7's point: a rejection about a
        // tool the model is about to reach is worth more than one about the
        // tool it just used, because dropping it buys a repeated error — the
        // most expensive wasted step there is.
        let upcoming_tools = state
            .plan
            .iter()
            .skip(active.map_or(0, |i| i + 1))
            .filter_map(|s| s.kind.tool_name())
            .map(str::to_string)
            .collect();
        let cited_ids = state.evidence.iter().map(|e| e.id.clone()).collect();
        Self {
            step,
            goal_kind: state.kind,
            subject: Subject {
                family: state.hypothesis.as_ref().map(|h| h.family_id.clone()),
                ..Subject::default()
            },
            active_tool,
            upcoming_tools,
            required_ids: BTreeSet::new(),
            cited_ids,
            focus: None,
            previously_rendered: BTreeSet::new(),
            excluded: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_subject(mut self, s: Subject) -> Self {
        // The hypothesis family survives a caller-supplied subject: they answer
        // different questions and overwriting one with the other would lose the
        // only subject a research step has before its first result.
        let family = self.subject.family.clone();
        self.subject = Subject {
            family: s.family.or(family),
            ..s
        };
        self
    }

    #[must_use]
    pub fn requiring(mut self, id: impl Into<String>) -> Self {
        self.required_ids.insert(id.into());
        self
    }
}

/// The per-signal breakdown one record scored, for the report (§28.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    pub subject: f32,
    pub id_overlap: f32,
    pub kind_prior: f32,
    pub recency: f32,
    pub citations: f32,
    pub same_tool: f32,
    pub correction: f32,
    pub expansion: f32,
    pub stickiness: f32,
}

/// One line of the selection report.
#[derive(Debug, Clone)]
pub struct ScoreLine {
    pub id: RecordId,
    pub signals: Signals,
    pub score: f32,
    /// After the redundancy penalty and any must-include promotion.
    pub final_score: f32,
    pub selected: bool,
    /// One clause saying why this record is in the prompt (§28.3).
    pub reason: String,
}

/// What an assembly did, into the trajectory (§28.3).
///
/// This is what lets the desk's replay scrubber answer "why did the model not
/// know that?" — the most common question about an agent that did something
/// strange, and one no version of this system could previously answer.
#[derive(Debug, Clone, Default)]
pub struct SelectionReport {
    pub policy_version: String,
    pub candidates: usize,
    pub scores: Vec<ScoreLine>,
    pub selected: Vec<RecordId>,
    pub dropped: Vec<RecordId>,
    /// Constraints that actually bound this assembly, named.
    pub bound_by: Vec<String>,
}

/// A selected, fitted set plus the account of how it was chosen.
#[derive(Debug, Clone)]
pub struct Selection {
    pub fit: Fit,
    pub report: SelectionReport,
    /// Pairs §24.8 found disagreeing. Rendered adjacent with the disagreement
    /// named; never resolved here.
    pub disagreements: Vec<(RecordId, RecordId)>,
}

impl Selection {
    /// The chosen ids, in render order.
    #[must_use]
    pub fn ids(&self) -> Vec<RecordId> {
        self.fit.kept.iter().map(|k| k.id).collect()
    }
}

/// Order records take in the prompt once chosen — the stable layout.
///
/// Kind-major, because a section boundary is a stronger cache boundary than a
/// step boundary, and because the model is told in the charter what each section
/// is. Within a kind, `(step asc, id asc)`.
fn layout_rank(kind: RecordKind) -> u8 {
    match kind {
        RecordKind::Memory => 0,
        RecordKind::Finding => 1,
        RecordKind::Decision => 2,
        RecordKind::PlanChange => 3,
        RecordKind::ToolResult => 4,
        RecordKind::Note => 5,
        // Last, and therefore closest to the task, which is where §4.2 puts the
        // thing the model must not miss.
        RecordKind::Correction => 6,
    }
}

/// Choose what this call renders.
///
/// Under [`SelectionMode::Chronological`] this is a no-op returning every live
/// record in order, which is Part I's behaviour exactly.
#[must_use]
pub fn select(
    store: &RunStore,
    query: &Query,
    budget: u32,
    policy: &ContextPolicy,
) -> Selection {
    let live: Vec<&Record> = store
        .live()
        .filter(|r| !query.excluded.contains(&r.id))
        .collect();
    let mut report = SelectionReport {
        policy_version: policy.version.clone(),
        candidates: live.len(),
        ..SelectionReport::default()
    };

    if policy.selection == SelectionMode::Chronological {
        // The control. Everything, in arrival order; Part I's assembler does the
        // fitting, so nothing here degrades or drops.
        let candidates: Vec<Candidate<'_>> =
            live.iter().map(|r| Candidate::new(r, 1.0)).collect();
        report.selected = live.iter().map(|r| r.id).collect();
        report.bound_by.push("policy v1: no selection".into());
        return Selection {
            fit: Fit {
                kept: candidates
                    .iter()
                    .map(|c| degrade::Fitted {
                        id: c.record.id,
                        body: c.record.body.clone(),
                        tokens: c.record.body.tokens(),
                        degraded: None,
                    })
                    .collect(),
                ..Fit::default()
            },
            report,
            disagreements: vec![],
        };
    }

    // ---- signals ------------------------------------------------------------
    let raw: Vec<Signals> = live.iter().map(|r| signals(r, query, policy)).collect();
    let normalised = match policy.normalisation {
        Normalisation::Raw => raw.clone(),
        Normalisation::PercentileRank => percentile_rank(&raw),
    };
    let w = &policy.weights;
    let base: Vec<f32> = normalised
        .iter()
        .map(|s| {
            s.subject * w.subject
                + s.id_overlap * w.id_overlap
                + s.kind_prior * w.kind_prior
                + s.recency * w.recency
                + s.citations * w.citations
                + s.same_tool * w.same_tool
                + s.correction * w.correction
                + s.expansion * w.expansion
                + s.stickiness * w.stickiness
        })
        .collect();

    // ---- must-includes and pins --------------------------------------------
    let mut must: BTreeSet<usize> = BTreeSet::new();
    for (i, r) in live.iter().enumerate() {
        if r.ids.values().any(|v| query.required_ids.contains(v)) {
            must.insert(i);
            report.bound_by.push(format!("required id in {}", r.id));
        }
        // §24.7: pinned for the *upcoming* tool, not only the active one.
        if r.kind == RecordKind::Correction && mentions_tool(r, query) {
            must.insert(i);
        }
    }

    // ---- build the set, penalising redundancy as it grows -------------------
    let mut chosen: Vec<usize> = must.iter().copied().collect();
    let mut final_scores: Vec<f32> = base.clone();
    let mut remaining: Vec<usize> = (0..live.len()).filter(|i| !must.contains(i)).collect();
    // Bounded work, so assembly time per call stays flat as the store grows
    // (§26.4). The marginal-relevance loop is quadratic in the candidate set,
    // and a four-hundred-step run must not spend more wall clock assembling
    // prompts than waiting for models.
    //
    // Only the *scoring* is bounded. The must-includes above and the hard floors
    // below both search the whole store, so nothing this cap hides can be
    // something the set is required to contain — which is the property that
    // makes a cap safe rather than a silent quality cliff.
    if remaining.len() > MMR_CANDIDATES {
        remaining.sort_by(|a, b| {
            base[*b]
                .total_cmp(&base[*a])
                .then_with(|| (live[*a].step, live[*a].id).cmp(&(live[*b].step, live[*b].id)))
        });
        remaining.truncate(MMR_CANDIDATES);
        report
            .bound_by
            .push(format!("scored the top {MMR_CANDIDATES} of {} candidates", live.len()));
    }
    let all_indices: Vec<usize> = (0..live.len()).filter(|i| !must.contains(i)).collect();

    while let Some((pick, score)) = best(&remaining, &base, &chosen, &live, policy) {
        let sticky = query.previously_rendered.contains(&live[pick].id);
        let threshold = if sticky {
            policy.drop_threshold
        } else {
            policy.admit_threshold
        };
        if score < threshold {
            report
                .bound_by
                .push(format!("below the {} threshold", if sticky { "drop" } else { "admit" }));
            break;
        }
        final_scores[pick] = score;
        chosen.push(pick);
        remaining.retain(|i| *i != pick);
    }

    // ---- hard floors: no weighting starves a kind (§24.1) -------------------
    for kind in [RecordKind::Correction, RecordKind::Finding] {
        if !chosen.iter().any(|i| live[*i].kind == kind) {
            if let Some(i) = all_indices
                .iter()
                .copied()
                .filter(|i| !chosen.contains(i) && live[*i].kind == kind)
                .max_by(|a, b| base[*a].total_cmp(&base[*b]))
            {
                chosen.push(i);
                remaining.retain(|j| *j != i);
                report.bound_by.push(format!("floor: one {}", kind.as_str()));
            }
        }
    }
    // And at least one record from the last two steps, so a run cannot lose
    // sight of what it just did.
    let recent_floor = query.step.saturating_sub(2);
    if !chosen.iter().any(|i| live[*i].step >= recent_floor) {
        if let Some(i) = all_indices
            .iter()
            .copied()
            .filter(|i| !chosen.contains(i) && live[*i].step >= recent_floor)
            .max_by(|a, b| base[*a].total_cmp(&base[*b]))
        {
            chosen.push(i);
            remaining.retain(|j| *j != i);
            report.bound_by.push("floor: the last two steps".into());
        }
    }

    // ---- disagreement is not redundancy (§24.8) -----------------------------
    let mut disagreements = Vec::new();
    let chosen_snapshot = chosen.clone();
    for i in chosen_snapshot {
        for (j, other) in live.iter().enumerate() {
            if chosen.contains(&j) && j <= i {
                continue;
            }
            if disagree(live[i], other) {
                if !chosen.contains(&j) {
                    chosen.push(j);
                    remaining.retain(|k| *k != j);
                    report
                        .bound_by
                        .push(format!("{} disagrees with {}", live[i].id, other.id));
                }
                disagreements.push((live[i].id, other.id));
            }
        }
    }

    // ---- untrusted content cannot take the window (§27.2) -------------------
    let untrusted_cap = (budget as f32 * policy.untrusted_share) as u32;
    let mut untrusted_spend = 0u32;
    let mut capped = Vec::new();
    chosen.retain(|i| {
        let r = live[*i];
        if !r.provenance.is_untrusted() {
            return true;
        }
        if untrusted_spend + r.tokens_full <= untrusted_cap {
            untrusted_spend += r.tokens_full;
            true
        } else {
            capped.push(r.id);
            false
        }
    });
    if !capped.is_empty() {
        report.bound_by.push(format!(
            "untrusted share capped at {:.0}%: {} record(s) held back",
            policy.untrusted_share * 100.0,
            capped.len()
        ));
        report.dropped.extend(capped);
    }

    // ---- the stable layout --------------------------------------------------
    chosen.sort_by_key(|i| {
        let r = live[*i];
        (
            u8::from(!must.contains(i)),
            layout_rank(r.kind),
            r.step,
            r.id,
        )
    });

    // ---- fit ----------------------------------------------------------------
    let candidates: Vec<Candidate<'_>> = chosen
        .iter()
        .map(|i| {
            let c = Candidate::new(live[*i], final_scores[*i].max(0.0) + 0.01);
            if must.contains(i) {
                c.pinned()
            } else {
                c
            }
        })
        .collect();
    let fit = degrade::fit(&candidates, budget, policy.degrade_loss_factor);
    if !fit.dropped.is_empty() {
        report.bound_by.push("body budget".into());
    }

    let kept: BTreeSet<RecordId> = fit.kept.iter().map(|f| f.id).collect();
    report.scores = live
        .iter()
        .enumerate()
        .map(|(i, r)| ScoreLine {
            id: r.id,
            signals: normalised[i].clone(),
            score: base[i],
            final_score: final_scores[i],
            selected: kept.contains(&r.id),
            reason: reason_for(r, &normalised[i], must.contains(&i), query),
        })
        .collect();
    report.selected = fit.kept.iter().map(|f| f.id).collect();
    report.dropped.extend(fit.dropped.iter().copied());
    // Everything else the store offered and this call did not render. Listed
    // explicitly rather than inferred by subtraction, because the report is what
    // the scrubber answers "why did the model not know that?" from, and a
    // reader should not have to reconstruct the complement.
    let already: BTreeSet<RecordId> = report.dropped.iter().copied().collect();
    let rest: Vec<RecordId> = live
        .iter()
        .map(|r| r.id)
        .filter(|id| !kept.contains(id) && !already.contains(id))
        .collect();
    report.dropped.extend(rest);

    Selection {
        fit,
        report,
        disagreements,
    }
}

/// The best remaining candidate under the redundancy penalty (§24.1).
///
/// Maximal-marginal relevance: the penalty is against the set *already chosen*,
/// so a second record about a subject already covered is worth less than the
/// first one was. Without it, top-k returns ten records about one subject and is
/// strictly worse than recency.
fn best(
    remaining: &[usize],
    base: &[f32],
    chosen: &[usize],
    live: &[&Record],
    policy: &ContextPolicy,
) -> Option<(usize, f32)> {
    remaining
        .iter()
        .map(|i| {
            let r = live[*i];
            let overlap: f32 = chosen
                .iter()
                .map(|j| redundancy(r, live[*j]))
                .sum::<f32>();
            (
                *i,
                base[*i] - overlap * policy.weights.redundancy_penalty,
            )
        })
        // `(step asc, id asc)` on a tie, never map order: a `HashMap` iteration
        // order would render one store as two different prompts.
        .max_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| (live[b.0].step, live[b.0].id).cmp(&(live[a.0].step, live[a.0].id)))
        })
}

/// How much one record duplicates another, in `[0,1]`.
///
/// **Zero when they disagree** (§24.8). Two records answering the same question
/// differently look like near-duplicates to a similarity measure, so exactly one
/// would survive and the harness would have silently picked a winner. The
/// disagreement is the finding.
fn redundancy(a: &Record, b: &Record) -> f32 {
    if disagree(a, b) {
        return 0.0;
    }
    let mut score = 0.0;
    if !a.subject.is_empty() && a.subject == b.subject {
        score += 0.5;
    }
    if a.kind == b.kind && a.source == b.source {
        score += 0.3;
    }
    let shared = a
        .ids
        .values()
        .filter(|v| b.ids.values().any(|w| w == *v))
        .count();
    if shared > 0 {
        score += 0.2;
    }
    score
}

fn disagree(a: &Record, b: &Record) -> bool {
    if a.id == b.id || a.subject.is_empty() || a.subject != b.subject {
        return false;
    }
    match (&a.measurement, &b.measurement) {
        (Some(x), Some(y)) => x.disagrees_with(y),
        _ => false,
    }
}

fn mentions_tool(r: &Record, query: &Query) -> bool {
    let tools = query
        .active_tool
        .iter()
        .chain(query.upcoming_tools.iter())
        .map(String::as_str);
    for t in tools {
        if r.source == t || r.ids.get("tool").map(String::as_str) == Some(t) {
            return true;
        }
    }
    false
}

fn signals(r: &Record, q: &Query, policy: &ContextPolicy) -> Signals {
    let focus_hit = q.focus.as_ref().is_some_and(|f| {
        r.subject.label().contains(f.as_str()) || r.source == *f
    });
    let id_overlap = r
        .ids
        .values()
        .filter(|v| q.required_ids.contains(*v) || q.cited_ids.contains(*v))
        .count();
    // Exponential decay at the kind's own half-life (§26.2).
    let age = f32::from(u16::try_from(q.step.saturating_sub(r.step)).unwrap_or(u16::MAX));
    let recency = 0.5_f32.powf(age / policy.half_life(r.kind).max(0.001));
    Signals {
        subject: r.subject.coverage(&q.subject) + if focus_hit { 1.0 } else { 0.0 },
        id_overlap: id_overlap as f32,
        kind_prior: policy.kind_prior(q.goal_kind, r.kind),
        recency,
        citations: r.citations as f32,
        same_tool: f32::from(q.active_tool.as_deref() == Some(r.source.as_str())),
        correction: f32::from(r.kind == RecordKind::Correction && mentions_tool(r, q)),
        // §18's promotion: expanded twice means the model has told us twice
        // that the scoring was wrong, so the boost is persistent and steep.
        expansion: if r.expansions >= 2 {
            2.0
        } else {
            r.expansions as f32
        },
        stickiness: f32::from(q.previously_rendered.contains(&r.id)),
    }
}

/// Percentile-rank every signal against the current candidate set (§24.3).
///
/// The alternative is summing raw, where recency lives in `[0,1]` and citation
/// count is unbounded — so the unbounded signal decides every comparison and the
/// weights are decoration. Ranking makes the weights mean what they say.
fn percentile_rank(raw: &[Signals]) -> Vec<Signals> {
    if raw.len() <= 1 {
        return raw.to_vec();
    }
    let getters: [fn(&Signals) -> f32; 9] = [
        |s| s.subject,
        |s| s.id_overlap,
        |s| s.kind_prior,
        |s| s.recency,
        |s| s.citations,
        |s| s.same_tool,
        |s| s.correction,
        |s| s.expansion,
        |s| s.stickiness,
    ];
    let setters: [fn(&mut Signals, f32); 9] = [
        |s, v| s.subject = v,
        |s, v| s.id_overlap = v,
        |s, v| s.kind_prior = v,
        |s, v| s.recency = v,
        |s, v| s.citations = v,
        |s, v| s.same_tool = v,
        |s, v| s.correction = v,
        |s, v| s.expansion = v,
        |s, v| s.stickiness = v,
    ];
    let mut out = vec![Signals::default(); raw.len()];
    let n = raw.len() as f32;
    for (get, set) in getters.iter().zip(setters.iter()) {
        let values: Vec<f32> = raw.iter().map(get).collect();
        for (i, v) in values.iter().enumerate() {
            // Fraction of the set this value is at least as large as. A signal
            // every record shares ranks 1.0 for all of them, which is correct:
            // a signal with no variance carries no information and the weighted
            // sum shifts by a constant.
            let rank = values.iter().filter(|o| *o <= v).count() as f32 / n;
            set(&mut out[i], rank);
        }
    }
    out
}

/// One clause saying why a record is in the prompt (§28.3).
fn reason_for(r: &Record, s: &Signals, must: bool, q: &Query) -> String {
    if must {
        if r.kind == RecordKind::Correction {
            return format!("a correction for {}", q.active_tool.as_deref().unwrap_or("a planned tool"));
        }
        return "carries an id the next step needs".into();
    }
    let mut strongest = ("recent", s.recency);
    for (name, v) in [
        ("about this subject", s.subject),
        ("shares an id with the step", s.id_overlap),
        ("already cited in this run", s.citations),
        ("from the step's own tool", s.same_tool),
        ("you asked for it back", s.expansion),
    ] {
        if v > strongest.1 {
            strongest = (name, v);
        }
    }
    strongest.0.to_string()
}

/// Render one selected set in the stable layout, with the furniture §28.3 asks
/// for: every record says what it is and why it is here.
///
/// The `overhead` cap (§25.7) is the caller's: this returns the furniture it
/// wrote so `render::fit_overhead` can shorten it rather than dropping content
/// to make room for labels about content.
#[must_use]
pub fn render_selected(store: &RunStore, sel: &Selection) -> Vec<(RecordId, String, String)> {
    let reasons: BTreeMap<RecordId, &str> = sel
        .report
        .scores
        .iter()
        .map(|l| (l.id, l.reason.as_str()))
        .collect();
    sel.fit
        .kept
        .iter()
        .map(|f| {
            let r = store.get(f.id);
            let mut head = format!(
                "[{}] {} {}",
                f.id,
                r.map_or("", |r| r.kind.as_str()),
                r.map(|r| r.source.clone()).unwrap_or_default()
            );
            if let Some(d) = f.degraded {
                head.push_str(&format!(
                    " (degraded {}→{} tokens, {})",
                    d.from_tokens,
                    d.to_tokens,
                    d.how.as_str()
                ));
            }
            if let Some(why) = reasons.get(&f.id) {
                head.push_str(&format!(" — {why}"));
            }
            (f.id, head, body_text(&f.body))
        })
        .collect()
}

fn body_text(b: &Body) -> String {
    b.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Column;
    use crate::context::store::{Draft, Measurement};
    use crate::provenance::Provenance;

    fn store_with(n: u32, subject: &str) -> RunStore {
        let mut s = RunStore::new();
        for i in 0..n {
            s.insert(
                Draft::new(
                    i,
                    RecordKind::ToolResult,
                    "get_bars",
                    Body::Table {
                        schema: vec![Column::new("ts", 200), Column::new("close", 200)],
                        rows: vec![vec!["1".into(), "2".into()]],
                        total_rows: 1,
                    },
                )
                .with_subject(Subject::instrument(subject))
                .with_id("window", format!("w{i}")),
            );
        }
        s
    }

    fn q(step: u32) -> Query {
        Query {
            step,
            goal_kind: GoalKind::Research,
            ..Query::default()
        }
    }

    /// Under `v1` selection is a no-op and every record is rendered.
    #[test]
    fn the_control_selects_nothing_and_changes_nothing() {
        let s = store_with(5, "BTC-USD");
        let sel = select(&s, &q(5), 100, &ContextPolicy::v1());
        assert_eq!(sel.fit.kept.len(), 5, "v1 renders everything");
        assert!(sel.fit.kept.iter().all(|k| k.degraded.is_none()));
        assert_eq!(sel.report.policy_version, "v1");
    }

    /// Same store, same query, same policy — byte-identical choice.
    #[test]
    fn selection_is_deterministic() {
        let s = store_with(30, "BTC-USD");
        let p = ContextPolicy::v2();
        let a = select(&s, &q(30), 400, &p);
        let b = select(&s, &q(30), 400, &p);
        assert_eq!(a.ids(), b.ids());
        assert_eq!(a.report.dropped, b.report.dropped);
    }

    /// Ten records about one subject must not become ten selected records
    /// about one subject (§24.1).
    #[test]
    fn the_selected_set_is_not_ten_of_one_subject() {
        let mut s = store_with(10, "BTC-USD");
        // One correction and one finding, both about something else and both
        // scoring worse than the ten.
        s.insert(
            Draft::new(1, RecordKind::Correction, "validate_strategy", Body::text("bad name"))
                .with_subject(Subject::instrument("ETH-USD")),
        );
        s.insert(
            Draft::new(2, RecordKind::Finding, "record_finding", Body::text("eth funding"))
                .with_subject(Subject::instrument("ETH-USD")),
        );
        let sel = select(&s, &q(12), 10_000, &ContextPolicy::v2());
        let kinds: Vec<RecordKind> = sel
            .ids()
            .iter()
            .filter_map(|id| s.get(*id).map(|r| r.kind))
            .collect();
        assert!(
            kinds.contains(&RecordKind::Correction),
            "the floor guarantees one correction: {kinds:?}"
        );
        assert!(
            kinds.contains(&RecordKind::Finding),
            "and one finding: {kinds:?}"
        );
    }

    /// A required id is a must-include, not a candidate (§24.2).
    #[test]
    fn a_required_id_survives_below_the_score_cut() {
        let mut s = store_with(20, "BTC-USD");
        // Ancient, off-subject, uncited — it loses on every signal.
        let needed = s.insert(
            Draft::new(0, RecordKind::ToolResult, "run_backtest", Body::text("x".repeat(200)))
                .with_id("experiment_id", "exp_42"),
        );
        let query = q(40).requiring("exp_42");
        let sel = select(&s, &query, 200, &ContextPolicy::v2());
        assert!(
            sel.ids().contains(&needed),
            "the record that produced the id the step needs is not a candidate"
        );
        assert!(sel.report.bound_by.iter().any(|b| b.contains("required id")));
    }

    /// A correction for the *upcoming* tool survives a squeeze (§24.7).
    #[test]
    fn a_correction_for_the_next_tool_outlives_higher_scoring_records() {
        let mut s = store_with(20, "BTC-USD");
        let correction = s.insert(Draft::new(
            1,
            RecordKind::Correction,
            "validate_strategy",
            Body::text("when-names must be an expression, not a string"),
        ));
        let mut query = q(30);
        query.active_tool = Some("get_bars".into());
        query.upcoming_tools = vec!["validate_strategy".into()];

        let sel = select(&s, &query, 120, &ContextPolicy::v2());
        assert!(
            sel.ids().contains(&correction),
            "dropping it buys a repeated error, the most expensive wasted step there is"
        );
    }

    /// Two records that disagree both render; the harness never picks a winner.
    #[test]
    fn a_disagreement_is_not_a_near_duplicate() {
        let mut s = RunStore::new();
        let subject = Subject::instrument("BTC-USD");
        let a = s.insert(
            Draft::new(2, RecordKind::ToolResult, "source_a", Body::text("sharpe 1.42"))
                .with_subject(subject.clone())
                .measuring(Measurement::new("sharpe", 1.42).with_se(0.05)),
        );
        let b = s.insert(
            Draft::new(6, RecordKind::ToolResult, "source_b", Body::text("sharpe 2.40"))
                .with_subject(subject)
                .measuring(Measurement::new("sharpe", 2.40).with_se(0.05)),
        );
        let sel = select(&s, &q(7), 10_000, &ContextPolicy::v2());
        assert!(sel.ids().contains(&a) && sel.ids().contains(&b), "both render");
        assert!(
            sel.disagreements.contains(&(a, b)) || sel.disagreements.contains(&(b, a)),
            "and the disagreement is named: {:?}",
            sel.disagreements
        );
    }

    /// Three long fetched pages cannot take the body (§27.2).
    #[test]
    fn untrusted_content_cannot_take_the_window() {
        let mut s = RunStore::new();
        // Three *different* articles. Same-subject copies would be suppressed by
        // the redundancy penalty before the cap ever ran, and the test would
        // pass for the wrong reason — §27.2 is about three pages that are each
        // legitimately relevant, which is the case the penalty cannot help with.
        for (i, sym) in ["BTC-USD", "ETH-USD", "SOL-USD"].iter().enumerate() {
            s.insert(
                Draft::new(
                    u32::try_from(i).unwrap(),
                    RecordKind::ToolResult,
                    format!("fetch_url:{sym}"),
                    Body::text("u".repeat(4000)),
                )
                .with_provenance(Provenance::ExternalUntrusted)
                .with_subject(Subject::instrument(*sym)),
            );
        }
        s.insert(
            Draft::new(3, RecordKind::Finding, "record_finding", Body::text("a finding"))
                .with_subject(Subject::instrument("BTC-USD")),
        );
        let sel = select(&s, &q(4), 4_000, &ContextPolicy::v2());
        let untrusted: u32 = sel
            .ids()
            .iter()
            .filter_map(|id| s.get(*id))
            .filter(|r| r.provenance.is_untrusted())
            .map(|r| r.tokens_full)
            .sum();
        assert!(
            untrusted <= 1_000,
            "25% of a 4000-token body is 1000; got {untrusted}"
        );
        assert!(
            sel.report.bound_by.iter().any(|b| b.contains("untrusted")),
            "and the cap is reported when it binds: {:?}",
            sel.report.bound_by
        );
    }

    /// Percentile ranking stops the unbounded signal from deciding everything.
    #[test]
    fn an_unbounded_signal_does_not_swamp_the_weights() {
        let raw = vec![
            Signals { citations: 400.0, recency: 1.0, ..Signals::default() },
            Signals { citations: 1.0, recency: 0.9, ..Signals::default() },
            Signals { citations: 0.0, recency: 0.8, ..Signals::default() },
        ];
        let ranked = percentile_rank(&raw);
        for s in &ranked {
            assert!(
                (0.0..=1.0).contains(&s.citations),
                "every signal lands in [0,1]: {s:?}"
            );
        }
        assert!(ranked[0].citations > ranked[2].citations, "order survives");
        assert!(
            (ranked[0].citations - ranked[0].recency).abs() < f32::EPSILON,
            "and a 400-to-1 lead is worth the same rank as a 1.0-to-0.8 lead, \
             which is the whole point"
        );
    }

    /// Hysteresis: a record rendered last call is held on a lower threshold.
    #[test]
    fn a_record_at_the_cut_does_not_flicker() {
        let s = store_with(12, "BTC-USD");
        let mut p = ContextPolicy::v2();
        p.admit_threshold = 0.90;
        p.drop_threshold = 0.10;

        let first = select(&s, &q(12), 10_000, &p);
        let mut second_query = q(13);
        second_query.previously_rendered = first.ids().into_iter().collect();
        let second = select(&s, &second_query, 10_000, &p);

        let held = second
            .ids()
            .iter()
            .filter(|id| first.ids().contains(id))
            .count();
        assert!(
            held >= first.ids().len(),
            "everything rendered last call is held on the drop threshold: {} of {}",
            held,
            first.ids().len()
        );
    }

    /// Winners are re-sorted into the stable layout, not left in score order.
    #[test]
    fn winners_are_re_sorted_into_the_stable_layout() {
        let mut s = RunStore::new();
        s.insert(Draft::new(9, RecordKind::Correction, "t", Body::text("late correction")));
        s.insert(Draft::new(1, RecordKind::Finding, "record_finding", Body::text("early finding")));
        s.insert(Draft::new(5, RecordKind::Memory, "memory", Body::text("brief")));
        let sel = select(&s, &q(10), 10_000, &ContextPolicy::v2());
        let kinds: Vec<RecordKind> = sel.ids().iter().filter_map(|id| s.get(*id)).map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![RecordKind::Memory, RecordKind::Finding, RecordKind::Correction],
            "the correction sits last, closest to the task"
        );
    }

    /// Every rendered record carries a reason (§28.3).
    #[test]
    fn every_rendered_record_says_why_it_is_here() {
        let s = store_with(6, "BTC-USD");
        let sel = select(&s, &q(6), 10_000, &ContextPolicy::v2());
        let rendered = render_selected(&s, &sel);
        assert!(!rendered.is_empty());
        for (id, head, _) in &rendered {
            assert!(head.contains(&id.to_string()));
            assert!(
                head.contains(" — "),
                "no reason clause on {id}: {head}"
            );
        }
    }

    /// The report accounts for every candidate, selected or not.
    #[test]
    fn the_report_accounts_for_every_candidate() {
        let s = store_with(15, "BTC-USD");
        let sel = select(&s, &q(15), 150, &ContextPolicy::v2());
        assert_eq!(sel.report.scores.len(), s.live().count());
        let accounted: BTreeSet<RecordId> = sel
            .report
            .selected
            .iter()
            .chain(sel.report.dropped.iter())
            .copied()
            .collect();
        for r in s.live() {
            assert!(
                accounted.contains(&r.id),
                "{} is in neither the selected nor the dropped list",
                r.id
            );
        }
    }
}
