//! Context Manager (harness guide §4).
//!
//! Context is a budget, not a container. Two jobs:
//!
//! 1. **Assemble** every prompt in the fixed layout (§4.2), each section with its own
//!    sub-budget. Instructions the model must obey go at the START; the immediate
//!    task goes at the END. Never buried in the middle, which is where attention is
//!    thinnest.
//! 2. **Enforce** `effective_budget_tokens` as a hard cap. Compaction is
//!    deterministic first — drop, truncate, dedupe — and only summarises with a model
//!    call if that is not enough. **There is no code path that sends an over-budget
//!    prompt**, which is the property the whole module exists for.
//!
//! Truncation keeps head AND tail. A tail-only cut loses the summary line that most
//! tool output ends with; a head-only cut loses the error at the bottom of a log.

use serde::{Deserialize, Serialize};

use crate::profile::Profile;
use crate::provenance::{Provenance, Tagged};

/// Sections of the fixed layout, in assembly order (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// 1. Identity, non-negotiable rules, output contract. Small and static, so it
    ///    stays in the cache prefix.
    Charter,
    /// 2. The ONE thing being done now, with acceptance criteria.
    Subtask,
    /// 3. Worked examples. Local profiles only.
    FewShot,
    /// 4. Compressed progress + the last N raw tool results.
    WorkingState,
    /// 5. Retrieved reference material for this subtask only.
    Reference,
    //
    // There was a sixth, `Tools`, holding 20% of the window for the exposed
    // schemas. **Nothing ever rendered it.** The schemas do not travel in the body
    // at all — they leave on the wire as the request's `tools` array, or as the
    // selection menu under the two-step decode — which is why `assemble_reserving`
    // charges them separately (S-1.10).
    //
    // So the section was a fifth of the budget held for content that could not
    // arrive, and the other five were sized against a whole that was never
    // available. Deleting it and renormalising is the same window, honestly
    // divided: the working state, which is the section that actually grows, goes
    // from .45 to .55.
}

impl Section {
    pub const ORDER: &'static [Section] = &[
        Section::Charter,
        Section::Subtask,
        Section::FewShot,
        Section::WorkingState,
        Section::Reference,
    ];

    /// Share of the input budget this section may take.
    ///
    /// Working state gets the most because it is the section that grows; the charter
    /// gets little because a charter that needs a large budget is a charter that
    /// should have been a skill file.
    #[must_use]
    pub fn budget_share(self) -> f32 {
        match self {
            Section::Charter => 0.10,
            Section::Subtask => 0.10,
            Section::FewShot => 0.05,
            Section::WorkingState => 0.55,
            Section::Reference => 0.20,
        }
    }
}

/// One block of content destined for a section.
#[derive(Debug, Clone)]
pub struct Block {
    pub section: Section,
    pub tagged: Tagged,
    /// Lower numbers are dropped first during compaction. Raw tool results get low
    /// priority because their summaries survive them.
    pub priority: u8,
    /// A handle to the full content in the scratchpad, kept when the block is cut.
    /// This is what makes dropping recoverable: the model can re-read it.
    pub reference: Option<String>,
}

/// Human-readable section name, for the dropped-block placeholder.
fn section_name(s: Section) -> &'static str {
    match s {
        Section::Charter => "the charter",
        Section::Subtask => "the subtask",
        Section::FewShot => "the examples",
        Section::WorkingState => "working state",
        Section::Reference => "reference material",
    }
}

fn hash_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The fraction of the input budget at which compaction starts.
///
/// Below this, context passes through untouched. That is not a performance tweak: the
/// per-section sub-budgets are *shares of the whole window*, so applying them
/// unconditionally truncated a prompt that was nowhere near full. Measured on a live
/// run — 3,474 tokens cut to 420 against a 6,000-token budget, because the tool
/// schemas exceeded the Tools section's 20% share while the prompt as a whole used
/// well under half the window.
///
/// Compaction is a response to pressure. With no pressure it is just deletion.
pub const COMPACTION_TRIGGER: f32 = 0.95;

/// Budget allowance for a dropped-block placeholder. Small and fixed: the line
/// names a size, a section and a handle, and nothing else.
const PLACEHOLDER_TOKENS: u32 = 48;

/// Rough token estimate.
///
/// ~4 bytes per token: close enough for budgeting and, crucially, an *over*-estimate
/// for the dense English and JSON this system sends. A budget enforced with an
/// optimistic estimate is not enforced.
#[must_use]
pub fn estimate_tokens(s: &str) -> u32 {
    u32::try_from(s.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// What compaction did, for the trace and for the timeline's compaction marker.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CompactionReport {
    pub dropped: usize,
    pub truncated: usize,
    pub deduped: usize,
    pub tokens_before: u32,
    pub tokens_after: u32,
    /// True when deterministic compaction was not enough and a summarisation pass is
    /// still required (§4.3).
    pub needs_model_summary: bool,
    /// WHAT was removed, not just how much.
    ///
    /// The counts answer "was the prompt compacted"; this answers "what did the model
    /// not see". Those are different questions, and the second is usually the one
    /// being asked — a decision that looks inexplicable against the full record is
    /// very often obvious once you know the thing that would have explained it had
    /// been truncated out of the prompt before the model ever read it.
    ///
    /// Names the block, not its contents: the content is still in the transcript, and
    /// duplicating it here would double the cost of the record that exists to explain
    /// a budget problem.
    pub removed: Vec<Removed>,
    /// What was claimed by something outside `blocks` — the exposed tool schemas,
    /// above all (AGENT-005 §4.1).
    ///
    /// Recorded because "the prompt was compacted" and "the prompt was compacted
    /// because the schemas took two thirds of the window" are different findings, and
    /// only the second says what to change.
    pub reserved_tokens: u32,
    /// What the body was actually budgeted against, after the reservation and the
    /// [`MIN_BODY_SHARE`] floor. When this equals `window * MIN_BODY_SHARE` the floor
    /// bound and the caller is over-reserved.
    pub body_budget_tokens: u32,
}

/// One block that did not survive compaction intact.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Removed {
    /// Which section it was in.
    pub section: String,
    /// Where the content came from — the tool name, `charter`, `goal`, `findings`.
    pub source: String,
    /// How many bytes were in it before compaction touched it.
    pub bytes: usize,
    /// `dropped` (replaced by a scratchpad placeholder) or `truncated` (cut to fit a
    /// section sub-budget).
    pub how: &'static str,
}

/// The assembled prompt.
#[derive(Debug, Clone)]
pub struct Assembled {
    pub sections: Vec<(Section, String)>,
    pub tokens: u32,
    pub compaction: CompactionReport,
}

impl Assembled {
    #[must_use]
    pub fn text(&self) -> String {
        self.sections
            .iter()
            .map(|(_, body)| body.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Truncates to head + tail with a marker naming what was removed.
///
/// The marker matters as much as the truncation: a model that knows the middle is
/// missing asks for it, and a model that does not reasons over a gap it cannot see.
#[must_use]
pub fn truncate_middle(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Leave room for the marker itself; if the cap is too small for head+tail+marker,
    // fall back to a head-only cut that still says it was cut.
    let marker_budget = 96;
    if max_bytes <= marker_budget {
        let mut end = max_bytes.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        return format!("{}… [truncated]", &s[..end]);
    }
    let usable = max_bytes - marker_budget;
    let head_len = usable * 2 / 3;
    let tail_len = usable - head_len;

    let mut head_end = head_len.min(s.len());
    while head_end > 0 && !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len().saturating_sub(tail_len);
    while tail_start < s.len() && !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    if tail_start <= head_end {
        return format!("{}… [truncated]", &s[..head_end]);
    }
    let removed = tail_start - head_end;
    format!(
        "{}\n… [{removed} bytes removed from the middle; read the full output from the scratchpad reference] …\n{}",
        &s[..head_end],
        &s[tail_start..]
    )
}

/// Assembles a prompt under the profile's hard budget.
///
/// Compaction order (§4.3), deterministic first:
/// 1. dedupe identical blocks,
/// 2. drop lowest-priority blocks that carry a scratchpad reference,
/// 3. truncate what remains, head + tail.
///
/// Only if all three leave it over budget is `needs_model_summary` set — and a model
/// summarisation pass on a local profile runs through the same validation ladder as
/// any other model output.
#[must_use]
pub fn assemble(profile: &Profile, blocks: Vec<Block>) -> Assembled {
    assemble_reserving(profile, blocks, 0)
}

/// The share of the window the body keeps however much is reserved (AGENT-005 §4.1).
///
/// A reservation is a claim on the window made by something that is not the body —
/// the tool schemas, most often. Without a floor, a profile whose schemas happen to
/// be large hands the body a budget near zero and every block is compacted away, so
/// the model reads a full description of its tools and nothing about its task. That
/// is the wrong thing to keep: a model with no working state cannot use the tools it
/// can see.
///
/// When the floor binds, the caller is over-reserved and the *reservation* is what
/// has to shrink — routing exposes fewer schemas. That is a decision for the caller,
/// which is why this returns a body budget rather than failing.
pub const MIN_BODY_SHARE: f32 = 0.25;

/// Assemble with part of the window already spoken for.
///
/// `reserved_tokens` is everything that will leave the body on the way to the model
/// but is not in `blocks` — the exposed tool schemas above all (AGENT-005 §4.2).
/// Budgeting the body against the whole window while the schemas are charged to the
/// same window is how a prompt that fits by arithmetic arrives over budget in fact.
///
/// The body budget is `max(window - reserved, window * MIN_BODY_SHARE)`.
#[must_use]
pub fn assemble_reserving(
    profile: &Profile,
    blocks: Vec<Block>,
    reserved_tokens: u32,
) -> Assembled {
    let window = profile.input_budget_tokens();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let floor = (window as f32 * MIN_BODY_SHARE) as u32;
    let input_budget = window.saturating_sub(reserved_tokens).max(floor);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let trigger = (input_budget as f32 * COMPACTION_TRIGGER) as u32;
    let mut report = CompactionReport {
        reserved_tokens,
        body_budget_tokens: input_budget,
        ..CompactionReport::default()
    };

    // Render first: an untrusted block costs its wrapper, and budgeting the
    // unwrapped text would put us over after wrapping.
    // The SOURCE travels with the rendered text, so a block that gets dropped or
    // truncated can be named in the report rather than counted anonymously.
    let mut rendered: Vec<(Section, u8, Option<String>, String, String)> = blocks
        .into_iter()
        .map(|b| {
            (
                b.section,
                b.priority,
                b.reference,
                b.tagged.source.clone(),
                b.tagged.render(),
            )
        })
        .collect();

    report.tokens_before = rendered.iter().map(|r| estimate_tokens(&r.4)).sum();

    // 1. Dedupe. Always, trigger or not: removing a byte-identical copy loses
    // nothing, so there is no reason to wait for pressure to do it.
    let before = rendered.len();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    rendered.retain(|item| seen.insert(hash_of(&item.4)));
    report.deduped = before - rendered.len();

    let after_dedupe: u32 = rendered.iter().map(|r| estimate_tokens(&r.4)).sum();

    // Under the trigger, hand the rest back whole. Everything below this point
    // removes information a caller might need; doing that to a prompt that fits is
    // loss with no benefit.
    if after_dedupe <= trigger {
        let mut sections: Vec<(Section, String)> = Vec::new();
        for section in Section::ORDER {
            let body: Vec<&str> = rendered
                .iter()
                .filter(|r| &r.0 == section)
                .map(|r| r.4.as_str())
                .collect();
            if !body.is_empty() {
                sections.push((*section, body.join("\n\n")));
            }
        }
        report.tokens_after = after_dedupe;
        return Assembled {
            sections,
            tokens: after_dedupe,
            compaction: report,
        };
    }

    // 2. Drop lowest-priority referenced blocks while over budget.
    let mut total: u32 = rendered.iter().map(|r| estimate_tokens(&r.4)).sum();
    if total > input_budget {
        // Ascending priority: least important first.
        let mut order: Vec<usize> = (0..rendered.len()).collect();
        order.sort_by_key(|&i| rendered[i].1);
        let mut drop: Vec<usize> = Vec::new();
        for i in order {
            if total <= input_budget {
                break;
            }
            // Only drop what can be re-read. A block with no reference is the only
            // copy, and losing it silently is worse than being over budget noisily.
            if rendered[i].2.is_some() {
                // The placeholder is not free; count what is actually reclaimed.
                total = total.saturating_sub(estimate_tokens(&rendered[i].4));
                total = total.saturating_add(PLACEHOLDER_TOKENS);
                drop.push(i);
            }
        }
        drop.sort_unstable();
        for i in drop.iter().rev() {
            // Guide §4.3: dropped raw results leave their summaries behind. A block
            // that vanished without trace is unrecoverable in practice even though
            // the scratchpad still holds it — the model cannot re-read a reference
            // it never knew existed, so the drop would read as the content never
            // having been there.
            let (section, priority, reference, source, body) = rendered[*i].clone();
            let handle = reference.clone().unwrap_or_default();
            let placeholder = format!(
                "[{} bytes from {} dropped to fit the context budget; read it with the \
                 scratchpad reference {}]",
                body.len(),
                section_name(section),
                handle
            );
            report.removed.push(Removed {
                section: section_name(section).to_string(),
                source: source.clone(),
                bytes: body.len(),
                how: "dropped",
            });
            rendered[*i] = (section, priority, reference, source, placeholder);
            report.dropped += 1;
        }
    }

    // 3. Truncate per section sub-budget.
    let mut sections: Vec<(Section, String)> = Vec::new();
    for section in Section::ORDER {
        let body: Vec<&str> = rendered
            .iter()
            .filter(|r| &r.0 == section)
            .map(|r| r.4.as_str())
            .collect();
        if body.is_empty() {
            continue;
        }
        let sources_here: Vec<String> = rendered
            .iter()
            .filter(|r| &r.0 == section)
            .map(|r| r.3.clone())
            .collect();
        let joined = body.join("\n\n");
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let sub_budget_tokens = (input_budget as f32 * section.budget_share()) as usize;
        let sub_budget_bytes = sub_budget_tokens.saturating_mul(4);
        let cut = truncate_middle(&joined, sub_budget_bytes.max(256));
        if cut.len() < joined.len() {
            report.truncated += 1;
            // Truncation is per section, so what was lost is "part of these blocks"
            // rather than any one of them. Naming the sources is still the useful
            // answer: it tells a reader which observations may be only half present
            // in what the model actually read.
            report.removed.push(Removed {
                section: section_name(*section).to_string(),
                source: sources_here.join(", "),
                bytes: joined.len() - cut.len(),
                how: "truncated",
            });
        }
        sections.push((*section, cut));
    }

    let tokens: u32 = sections.iter().map(|(_, body)| estimate_tokens(body)).sum();
    report.tokens_after = tokens;
    report.needs_model_summary = tokens > input_budget;

    Assembled {
        sections,
        tokens,
        compaction: report,
    }
}

/// Caps a tool result at the profile's limit before it enters context (§2.3).
///
/// Returns the capped text and whether it was cut, so the caller can write the full
/// output to the scratchpad and hand back a reference.
#[must_use]
pub fn cap_tool_result(profile: &Profile, text: &str) -> (String, bool) {
    cap_tool_result_within(profile, text, None)
}

/// The same cap, against the body that is actually left (AGENT-005 §2.2, S-1.18).
///
/// `max_tool_result_bytes` is a per-profile constant and the body budget is
/// computed per call, so the two can disagree by a lot. A 32 KiB cap against 8 KiB
/// of remaining body admits one result that evicts everything else in the working
/// state — **including the run's own findings**, which are the compressed form of
/// every earlier result and the thing least replaceable by re-reading.
///
/// So the effective cap is the smaller of the two, taken at the moment the result
/// arrives rather than when the profile was written. A result capped this way still
/// carries its handle, so nothing is lost that cannot be fetched back.
///
/// `remaining` is `None` where the caller genuinely has no per-call figure, and
/// then the profile constant stands. That is the honest reading of "no budget
/// information", not a licence to ignore one.
#[must_use]
pub fn cap_tool_result_within(
    profile: &Profile,
    text: &str,
    remaining_body_bytes: Option<usize>,
) -> (String, bool) {
    let cap = match remaining_body_bytes {
        // A floor of one line: a cap of zero is not a truncation, it is a
        // deletion, and a result reduced to nothing tells the model less than a
        // result reduced to its shape.
        Some(r) => profile.context.max_tool_result_bytes.min(r).max(MIN_RESULT_BYTES),
        None => profile.context.max_tool_result_bytes,
    };
    if text.len() <= cap {
        return (text.to_string(), false);
    }
    (truncate_middle(text, cap), true)
}

/// The smallest a capped tool result may be reduced to.
///
/// Enough for the shape of an answer — a key, a count, an ellipsis — because a
/// model that is handed an empty string learns nothing and one that is handed
/// `{"bars": … }` learns what kind of thing it asked for and can fetch the handle.
pub const MIN_RESULT_BYTES: usize = 256;

/// The factor to scale a token estimate by, from observed `estimated / actual`
/// ratios (AGENT-005 §7, S-1.18).
///
/// **p90, not the median.** The estimator's contract is that it OVER-estimates:
/// a budget built on an under-estimate overflows the window, and overflowing is
/// how a prompt loses its charter. Calibrating to the median satisfies the
/// contract on exactly half of all calls, which is the same as not having it.
///
/// p90 rather than the maximum because one pathological ratio — a result that was
/// almost all punctuation — should not inflate every later budget; the ninetieth
/// percentile is the strongest guarantee that is still robust to the tail.
///
/// Returns `None` below ten samples: a percentile of nine numbers is a number
/// about those nine numbers, and shipping a factor derived from it would be
/// calibration in name only.
#[must_use]
pub fn calibration_factor(ratios: &[f64]) -> Option<f64> {
    const MIN_SAMPLES: usize = 10;
    let mut usable: Vec<f64> = ratios.iter().copied().filter(|r| r.is_finite() && *r > 0.0).collect();
    if usable.len() < MIN_SAMPLES {
        return None;
    }
    usable.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Nearest-rank: the smallest value at or above the 90th percentile position.
    // Interpolating would produce a factor no observation supports.
    let rank = ((usable.len() as f64) * 0.9).ceil() as usize;
    usable.get(rank.saturating_sub(1)).copied()
}

/// Whether this section may carry content at this provenance.
///
/// The charter and the subtask are the harness speaking. Untrusted text there would
/// be indistinguishable from the rules themselves, which is the one place the data
/// block cannot help.
#[must_use]
pub fn provenance_allowed(section: Section, p: Provenance) -> bool {
    match section {
        Section::Charter | Section::Subtask | Section::FewShot => {
            !p.is_untrusted()
        }
        Section::WorkingState | Section::Reference => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::frontier_fixture;

    fn block(section: Section, text: &str, priority: u8, reference: bool) -> Block {
        Block {
            section,
            tagged: Tagged::tool("platform.test", text),
            priority,
            reference: reference.then(|| "art_ref_1".to_string()),
        }
    }

    fn small_profile() -> Profile {
        let mut p = frontier_fixture();
        p.context.effective_budget_tokens = 1_000;
        p.context.reserve_for_output = 200;
        p
    }

    /// A profile whose window is exactly 10,000 tokens, for the reservation
    /// arithmetic AGENT-005 §4.1 states in round numbers.
    fn window_of(tokens: u32) -> Profile {
        let mut p = frontier_fixture();
        p.context.effective_budget_tokens = tokens;
        p.context.reserve_for_output = 0;
        p
    }

    /// AGENT-005 §4.1, the worked example: with 10,000 tokens and 4,000 reserved,
    /// 6,500 tokens of blocks compact where at 9,500 reserved-free they would not.
    ///
    /// This is the whole point of the reservation. Nothing about the blocks changed
    /// between the two calls; what changed is that the schemas are now charged to
    /// the same window, which they always were in fact and were not in arithmetic.
    #[test]
    fn a_reservation_moves_the_compaction_point() {
        let p = window_of(10_000);
        // ~6,500 tokens at the estimator's 4 bytes per token, in one unreferenced
        // block so nothing can be dropped — only truncated.
        let body = "x".repeat(6_500 * 4);

        let relaxed = assemble(&p, vec![block(Section::WorkingState, &body, 9, false)]);
        assert_eq!(relaxed.compaction.reserved_tokens, 0);
        assert_eq!(relaxed.compaction.body_budget_tokens, 10_000);
        assert_eq!(
            relaxed.compaction.truncated, 0,
            "6,500 tokens against a 10,000-token window is under the trigger"
        );

        let reserved = assemble_reserving(
            &p,
            vec![block(Section::WorkingState, &body, 9, false)],
            4_000,
        );
        assert_eq!(reserved.compaction.reserved_tokens, 4_000);
        assert_eq!(reserved.compaction.body_budget_tokens, 6_000);
        assert!(
            reserved.compaction.truncated > 0,
            "the same blocks must compact once 4,000 tokens are spoken for"
        );
        assert!(reserved.tokens <= 6_000, "{} tokens", reserved.tokens);
    }

    /// Adversarial: a reservation that would take the whole window does not.
    ///
    /// Without the floor, a profile whose schemas happen to be large hands the body
    /// a budget near zero, every block compacts away, and the model reads a full
    /// description of its tools and nothing about its task. The floor is what makes
    /// over-reserving a visible caller error instead of a silently empty prompt.
    #[test]
    fn the_body_keeps_its_floor_when_a_reservation_would_take_everything() {
        let p = window_of(10_000);
        let body = "y".repeat(40_000);

        for reserved in [9_000, 10_000, 25_000, u32::MAX] {
            let out = assemble_reserving(
                &p,
                vec![block(Section::WorkingState, &body, 9, false)],
                reserved,
            );
            assert_eq!(
                out.compaction.body_budget_tokens, 2_500,
                "reserving {reserved} must still leave the body 25% of 10,000"
            );
            assert!(out.tokens > 0, "reserving {reserved} emptied the prompt");
            assert!(
                out.tokens <= 2_500,
                "reserving {reserved} gave {} tokens",
                out.tokens
            );
        }
    }

    /// And `assemble` is exactly `assemble_reserving(_, _, 0)`, so the existing
    /// callers cannot have changed behaviour.
    #[test]
    fn assemble_is_assemble_reserving_with_nothing_reserved() {
        let p = small_profile();
        let blocks = || {
            vec![
                block(Section::WorkingState, &"z".repeat(9_000), 3, true),
                block(Section::Charter, "charter", 9, false),
            ]
        };
        let a = assemble(&p, blocks());
        let b = assemble_reserving(&p, blocks(), 0);
        assert_eq!(a.sections, b.sections);
        assert_eq!(a.tokens, b.tokens);
    }

    #[test]
    fn sections_assemble_in_the_fixed_order() {
        let p = frontier_fixture();
        let out = assemble(
            &p,
            vec![
                block(Section::Reference, "ref", 5, false),
                block(Section::Charter, "charter", 9, false),
                block(Section::WorkingState, "state", 3, false),
                block(Section::Subtask, "task", 9, false),
            ],
        );
        let order: Vec<Section> = out.sections.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            order,
            vec![
                Section::Charter,
                Section::Subtask,
                Section::WorkingState,
                Section::Reference
            ],
            "rules at the start, the immediate task never buried in the middle"
        );
    }

    /// The property the module exists for.
    #[test]
    fn assembly_never_exceeds_the_input_budget() {
        let p = small_profile();
        let huge = "x".repeat(200_000);
        let out = assemble(
            &p,
            vec![
                block(Section::WorkingState, &huge, 1, true),
                block(Section::Reference, &huge, 1, true),
                block(Section::Charter, "rules", 9, false),
            ],
        );
        assert!(
            out.tokens <= p.input_budget_tokens(),
            "assembled {} tokens against a budget of {}",
            out.tokens,
            p.input_budget_tokens()
        );
    }

    #[test]
    fn deduplication_runs_before_anything_is_dropped() {
        let p = frontier_fixture();
        let out = assemble(
            &p,
            vec![
                block(Section::WorkingState, "same", 1, true),
                block(Section::WorkingState, "same", 1, true),
                block(Section::WorkingState, "other", 1, true),
            ],
        );
        assert_eq!(out.compaction.deduped, 1);
        assert_eq!(out.compaction.dropped, 0);
    }

    /// Dropping is only safe for content that can be re-read. The alternative —
    /// dropping the only copy — trades a visible over-budget error for a silent
    /// hole in the agent's knowledge.
    #[test]
    fn a_block_with_no_scratchpad_reference_is_never_dropped() {
        let p = small_profile();
        let unreferenced = "y".repeat(100_000);
        let referenced = "z".repeat(100_000);
        let out = assemble(
            &p,
            vec![
                block(Section::WorkingState, &unreferenced, 0, false),
                block(Section::Reference, &referenced, 0, true),
            ],
        );
        assert_eq!(out.compaction.dropped, 1, "only the referenced block goes");
        assert!(out
            .sections
            .iter()
            .any(|(s, _)| *s == Section::WorkingState));
    }

    /// Dedupe runs first and keeps the *first* copy, which may be the one with no
    /// reference. That is the right order — the surviving copy is the one that
    /// cannot be re-read, so keeping it is what the no-drop rule wants — but it
    /// means identical content in two blocks collapses to one before any dropping
    /// is considered. Pinned because the interaction is not obvious from either
    /// rule alone.
    #[test]
    fn dedupe_keeps_the_copy_that_cannot_be_re_read() {
        let p = small_profile();
        let same = "y".repeat(100_000);
        let out = assemble(
            &p,
            vec![
                block(Section::WorkingState, &same, 0, false),
                block(Section::Reference, &same, 0, true),
            ],
        );
        assert_eq!(out.compaction.deduped, 1);
        assert_eq!(out.compaction.dropped, 0, "nothing droppable is left");
        assert!(out
            .sections
            .iter()
            .any(|(s, _)| *s == Section::WorkingState));
    }

    /// Compaction is a response to pressure; with no pressure it is just deletion.
    ///
    /// Measured on a live run before this existed: 3,474 tokens cut to 420 against a
    /// 6,000-token budget, because the tool schemas exceeded the Tools section's 20%
    /// share while the prompt as a whole used well under half the window. The model
    /// was reading a truncated prompt for no reason.
    #[test]
    fn a_prompt_that_fits_is_not_compacted_at_all() {
        let p = frontier_fixture();
        // Comfortably under the trigger, and distinct — identical bodies would be
        // deduped, which is lossless and runs regardless of pressure.
        let tools_body = "x".repeat(1_000);
        let state_body = "y".repeat(1_000);
        let out = assemble(
            &p,
            vec![
                Block {
                    section: Section::Reference,
                    tagged: Tagged::system("tools", &tools_body),
                    priority: 1,
                    reference: Some("art_1".into()),
                },
                Block {
                    section: Section::WorkingState,
                    tagged: Tagged::tool("platform.test", &state_body),
                    priority: 1,
                    reference: Some("art_2".into()),
                },
            ],
        );
        assert_eq!(out.compaction.dropped, 0);
        assert_eq!(out.compaction.truncated, 0);
        assert_eq!(
            out.compaction.tokens_after, out.compaction.tokens_before,
            "nothing should be removed from a prompt that fits"
        );
    }

    /// And it still fires when the window is actually under pressure.
    #[test]
    fn a_prompt_over_the_trigger_is_still_compacted() {
        let p = frontier_fixture();
        let huge = "x".repeat(p.input_budget_tokens() as usize * 8);
        let out = assemble(
            &p,
            vec![Block {
                section: Section::WorkingState,
                tagged: Tagged::tool("platform.test", &huge),
                priority: 1,
                reference: Some("art_1".into()),
            }],
        );
        assert!(out.tokens <= p.input_budget_tokens());
        assert!(out.compaction.dropped + out.compaction.truncated > 0);
    }

    #[test]
    fn lowest_priority_is_dropped_first() {
        let p = small_profile();
        let huge = "z".repeat(80_000);
        let out = assemble(
            &p,
            vec![
                Block {
                    section: Section::Reference,
                    tagged: Tagged::tool("a", &huge),
                    priority: 1,
                    reference: Some("r1".into()),
                },
                Block {
                    section: Section::WorkingState,
                    tagged: Tagged::tool("b", "keep me"),
                    priority: 9,
                    reference: Some("r2".into()),
                },
            ],
        );
        assert!(out.text().contains("keep me"));
    }

    // ── Truncation ──────────────────────────────────────────────────────────

    #[test]
    fn truncation_keeps_head_and_tail_and_says_what_it_removed() {
        let s = format!("HEAD{}TAIL", "m".repeat(5_000));
        let cut = truncate_middle(&s, 500);
        assert!(cut.starts_with("HEAD"), "the head survives");
        assert!(
            cut.ends_with("TAIL"),
            "the tail survives — most logs put the error last"
        );
        assert!(cut.contains("bytes removed from the middle"));
        assert!(cut.len() <= 600);
    }

    #[test]
    fn truncation_is_a_no_op_under_the_cap() {
        assert_eq!(truncate_middle("short", 100), "short");
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let s = "é".repeat(1_000);
        let cut = truncate_middle(&s, 300);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
    }

    #[test]
    fn a_tiny_cap_still_marks_the_cut() {
        let cut = truncate_middle(&"a".repeat(1000), 40);
        assert!(cut.contains("truncated"));
    }

    #[test]
    fn tool_results_are_capped_at_the_profile_limit() {
        let mut p = frontier_fixture();
        p.context.max_tool_result_bytes = 1_000;
        let (capped, was_cut) = cap_tool_result(&p, &"q".repeat(50_000));
        assert!(was_cut);
        assert!(capped.len() <= 1_100);
        let (small, cut) = cap_tool_result(&p, "fine");
        assert_eq!(small, "fine");
        assert!(!cut);
    }

    // ── Provenance placement ────────────────────────────────────────────────

    #[test]
    fn untrusted_content_may_not_sit_in_the_charter_or_the_subtask() {
        assert!(!provenance_allowed(
            Section::Charter,
            Provenance::ExternalUntrusted
        ));
        assert!(!provenance_allowed(
            Section::Subtask,
            Provenance::ExternalUntrusted
        ));
        assert!(provenance_allowed(
            Section::Reference,
            Provenance::ExternalUntrusted
        ));
        assert!(provenance_allowed(
            Section::WorkingState,
            Provenance::ExternalUntrusted
        ));
    }

    #[test]
    fn an_untrusted_block_is_wrapped_when_it_is_assembled() {
        let p = frontier_fixture();
        let out = assemble(
            &p,
            vec![Block {
                section: Section::Reference,
                tagged: Tagged::untrusted("reddit", "buy now"),
                priority: 1,
                reference: None,
            }],
        );
        assert!(out.text().contains("<untrusted-data"));
    }

    #[test]
    fn the_token_estimate_does_not_under_count() {
        // Under-counting would let an over-budget prompt through, which is the one
        // failure this estimate must not have.
        let s = "hello world";
        assert!(estimate_tokens(s) >= 2);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    /// The cap that matters is the smaller of the two, and the reason is the
    /// asymmetry: a result too large evicts the run's own findings, which are the
    /// compressed form of everything earlier and the least replaceable thing in
    /// the window.
    #[test]
    fn a_result_larger_than_the_remaining_body_is_capped_to_it() {
        let p = small_profile();
        let huge = "x".repeat(40_000);

        // Against the profile constant alone: capped to the constant.
        // `truncate_middle` spends a fixed marker budget and then rounds the
        // head/tail split, so the result lands within a few bytes of the cap
        // rather than exactly on it. What matters is which cap it landed near.
        let (a, t) = cap_tool_result(&p, &huge);
        assert!(t);
        assert!(
            a.len().abs_diff(p.context.max_tool_result_bytes) < 16,
            "capped to {} against a profile cap of {}",
            a.len(),
            p.context.max_tool_result_bytes
        );

        // Against 8 KiB of remaining body: capped to the body, not the constant.
        let (b, t) = cap_tool_result_within(&p, &huge, Some(8_192));
        assert!(t);
        assert!(
            b.len().abs_diff(8_192.min(p.context.max_tool_result_bytes)) < 16,
            "capped to {} against a remaining body of 8,192",
            b.len()
        );
        assert!(
            b.len() <= a.len(),
            "the per-call cap can only tighten, never loosen"
        );
    }

    /// A cap of zero is a deletion, not a truncation. A result reduced to nothing
    /// teaches the model nothing; one reduced to its shape still says what kind of
    /// thing it asked for, and the handle fetches the rest.
    #[test]
    fn a_result_is_never_capped_below_the_shape_of_an_answer() {
        let p = small_profile();
        let (out, truncated) = cap_tool_result_within(&p, &"x".repeat(10_000), Some(0));
        assert!(truncated);
        assert!(
            out.len().abs_diff(MIN_RESULT_BYTES) < 16,
            "a zero budget must still leave the shape of an answer, got {} bytes",
            out.len()
        );
        assert!(out.contains("removed from the middle"), "and it says it was cut: {out}");
    }

    /// A result that already fits is returned whole, whichever cap is smaller.
    #[test]
    fn a_result_that_fits_is_not_touched() {
        let p = small_profile();
        let (out, truncated) = cap_tool_result_within(&p, "{\"bars\": 500}", Some(8_192));
        assert!(!truncated);
        assert_eq!(out, "{\"bars\": 500}");
    }

    /// The estimator's contract is that it OVER-estimates. Calibrating to the
    /// median satisfies that on exactly half of all calls, which is the same as
    /// not having the contract.
    #[test]
    fn the_calibration_factor_is_the_p90_not_the_median() {
        // 1..=10: median 5.5, p90 9.
        let ratios: Vec<f64> = (1..=10).map(f64::from).collect();
        let f = calibration_factor(&ratios).expect("ten samples is enough");
        assert!((f - 9.0).abs() < 1e-9, "expected the p90 (9), got {f}");

        let over = ratios.iter().filter(|r| **r <= f).count();
        assert!(
            over * 100 / ratios.len() >= 90,
            "the factor must cover at least 90% of observations; it covered {over}/10"
        );
    }

    /// One pathological ratio must not inflate every later budget — which is why
    /// this is p90 and not the maximum.
    #[test]
    fn one_outlier_does_not_set_the_factor() {
        let mut ratios: Vec<f64> = (1..=10).map(f64::from).collect();
        ratios.push(10_000.0);
        let f = calibration_factor(&ratios).expect("eleven samples");
        assert!(f < 100.0, "the outlier set the factor: {f}");
    }

    /// Below ten samples there is no factor, because a percentile of nine numbers
    /// is a statement about those nine numbers.
    #[test]
    fn too_few_samples_produce_no_factor_rather_than_a_confident_one() {
        assert_eq!(calibration_factor(&[]), None);
        let nine: Vec<f64> = (1..=9).map(f64::from).collect();
        assert_eq!(calibration_factor(&nine), None);
    }

    #[test]
    fn the_section_budget_shares_do_not_exceed_the_whole() {
        let total: f32 = Section::ORDER.iter().map(|s| s.budget_share()).sum();
        assert!(total <= 1.0 + f32::EPSILON, "shares sum to {total}");
    }
}
