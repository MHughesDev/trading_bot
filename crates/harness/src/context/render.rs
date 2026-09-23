//! Rendering economy (AGENT-005 §25; Set S tasks S-9.15 and S-9.16's overhead
//! cap).
//!
//! # Five losses of nothing
//!
//! Every saving here is a saving of tokens that carry no information. None of
//! them involve choosing what the model does *not* see — that is selection's job
//! and it is the expensive kind of decision. These are cheap:
//!
//! 1. **Tables are not JSON.** A 200-row result spends ~40% of its tokens on
//!    repeated keys, braces and quotes. [`Body::render`] already emits a header
//!    and delimited rows; this module keeps it that way across the whole prompt.
//! 2. **Shared schemas are stated once.** Five results from the same tool repeat
//!    one header five times.
//! 3. **Precision follows the error bar.** A mean printed as `0.0234871629`
//!    against `se = 0.004` spends eight tokens on noise *and* invites the model
//!    to quote a precision the measurement does not support — the second cost is
//!    the one that matters.
//! 4. **The second observation is a delta.** "as r007, except close 42,118 →
//!    42,340; 3 rows changed", with the full record one `expand` away. The
//!    largest single saving on polling- and re-read-shaped runs, and exactly
//!    what the idempotent-repeat guard already detects.
//! 5. **Repeated corrections fold** into one with a count — which also makes
//!    "3rd rejection on this path" legible, and that is the signal to hand over
//!    a worked example rather than a sixth rejection.
//!
//! # The harness's own furniture has a budget
//!
//! Degraded markers, handles, reason clauses and the manifest grow with record
//! *count*, not content, so on a long run they creep. One `overhead` line item,
//! capped at a share of the body (§25.7), and the cap is met by **shortening the
//! furniture** — never by dropping content to make room for labels about
//! content. That inversion would be absurd, which is why it is stated: greedy
//! budget code does it by accident.

use std::collections::BTreeMap;

use super::body::{format_sig, sig_figs_for, Body};
use super::degrade::Fitted;
use super::policy::ContextPolicy;
use super::select::Selection;
use super::store::{Record, RecordId, RecordKind, RunStore};

/// One record as it will appear, kept separate so its provenance survives.
///
/// The pieces are **not** joined into one string before they reach the
/// assembler, and that is a safety property rather than a style choice: an
/// untrusted record has to travel in its own `Tagged` so `wrap_untrusted` fences
/// it. Flattening the body into a single system-tagged block would silently
/// unfence every fetched page in the prompt — the exact failure the provenance
/// lane exists to prevent, arrived at by a rendering convenience.
#[derive(Debug, Clone)]
pub struct Piece {
    pub id: RecordId,
    pub provenance: crate::provenance::Provenance,
    pub source: String,
    /// Header, measurement line and body, in that order.
    pub text: String,
}

/// A rendered prompt body and what economy did to it.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub text: String,
    /// The same content, one entry per record, provenance intact.
    pub pieces: Vec<Piece>,
    /// Lines the harness wrote about the set as a whole — the disagreement
    /// notices. System provenance by construction.
    pub notices: Vec<String>,
    /// Tokens spent on the harness's own labels.
    pub overhead_tokens: u32,
    /// Tokens spent on record content.
    pub content_tokens: u32,
    /// What each economy did, for the report.
    pub savings: Vec<String>,
}

impl Rendered {
    #[must_use]
    pub fn tokens(&self) -> u32 {
        super::estimate_tokens(&self.text)
    }

    /// Share of the rendered body spent on furniture.
    #[must_use]
    pub fn overhead_share(&self) -> f32 {
        let total = self.overhead_tokens + self.content_tokens;
        if total == 0 {
            0.0
        } else {
            self.overhead_tokens as f32 / total as f32
        }
    }
}

/// How much furniture each record carries. Shortened in this order when the
/// overhead cap binds — the reason clause is the first to go because it is the
/// only one a reader can infer, and the handle is never dropped because it is
/// what makes the loss recoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Furniture {
    /// Id, kind, source, degradation marker, reason clause.
    Full,
    /// Id, kind, source, degradation marker.
    NoReason,
    /// Id and the degradation marker.
    Terse,
}

/// Render one selected set.
///
/// `manifest` is prepended untouched: it is furniture by nature and §27.3 puts
/// it outside the cap, because shortening the index of what exists is how a
/// manifest starts lying.
#[must_use]
pub fn render(
    store: &RunStore,
    sel: &Selection,
    policy: &ContextPolicy,
    manifest: &str,
) -> Rendered {
    let mut out = None;
    for level in [Furniture::Full, Furniture::NoReason, Furniture::Terse] {
        let r = render_at(store, sel, level);
        if r.overhead_share() <= policy.overhead_cap_share {
            out = Some(r);
            break;
        }
        out = Some(r);
    }
    let mut rendered = out.unwrap_or_default();
    if !manifest.is_empty() {
        rendered.text = format!("{manifest}\n\n{}", rendered.text);
    }
    rendered
}

/// The rendered set as assembler blocks, one per record.
///
/// One block per record rather than one for the body, so an untrusted record
/// keeps its own `Tagged` and `wrap_untrusted` still fences it. Flattening the
/// pieces into a single system-tagged block would unfence every fetched page in
/// the prompt — which is why `pieces` exists at all.
///
/// The manifest leads, at `Section::WorkingState` and the highest priority in
/// the section: it is the index of everything that exists, and dropping it turns
/// every elision back into a silent miss.
#[must_use]
pub fn blocks(
    store: &RunStore,
    sel: &Selection,
    policy: &ContextPolicy,
    manifest: &str,
) -> Vec<crate::context::Block> {
    use crate::context::{Block, Section};
    use crate::provenance::Tagged;

    let rendered = render(store, sel, policy, "");
    let mut out = Vec::with_capacity(rendered.pieces.len() + 2);
    if !manifest.is_empty() {
        out.push(Block {
            section: Section::WorkingState,
            tagged: Tagged::system("context", manifest.to_string()),
            // Above every record: §17's index is what makes any of them
            // recoverable, and it is the cheapest block in the section.
            priority: 254,
            reference: None,
        });
    }
    for piece in &rendered.pieces {
        let tagged = if piece.provenance.is_untrusted() {
            Tagged::untrusted(&piece.source, &piece.text)
        } else {
            Tagged::system(&piece.source, piece.text.clone())
        };
        out.push(Block {
            section: Section::WorkingState,
            tagged,
            priority: 100,
            reference: Some(format!("expand {}", piece.id)),
        });
    }
    for notice in &rendered.notices {
        out.push(Block {
            section: Section::WorkingState,
            tagged: Tagged::system("context", notice.clone()),
            priority: 250,
            reference: None,
        });
    }
    out
}

fn render_at(store: &RunStore, sel: &Selection, level: Furniture) -> Rendered {
    let reasons: BTreeMap<RecordId, &str> = sel
        .report
        .scores
        .iter()
        .map(|l| (l.id, l.reason.as_str()))
        .collect();

    let mut pieces: Vec<Piece> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    let mut overhead = 0u32;
    let mut content = 0u32;
    let mut savings: Vec<String> = Vec::new();

    // ---- 5. repeated corrections fold --------------------------------------
    let (kept, folded) = fold_corrections(store, &sel.fit.kept);
    for note in folded {
        savings.push(note);
    }

    // ---- 2. a schema shared by selected records is stated once --------------
    let mut schema_stated: BTreeMap<String, RecordId> = BTreeMap::new();
    // ---- 4. a re-read renders as a delta ------------------------------------
    let mut last_table: BTreeMap<String, (RecordId, Body)> = BTreeMap::new();

    for item in &kept {
        let record = store.get(item.id);
        let mut text = String::new();
        let head = header(item, record, reasons.get(&item.id).copied(), level);
        overhead += super::estimate_tokens(&head);
        text.push_str(&head);
        text.push('\n');

        // A measurement renders at the precision its error bar earns (§25.3).
        if let Some(m) = record.and_then(|r| r.measurement.as_ref()) {
            let digits = m.se.map_or(4, |se| sig_figs_for(m.value, se));
            let mut line = format!("{} = {}", m.quantity, format_sig(m.value, digits));
            if let Some(se) = m.se {
                line.push_str(&format!(" (se {})", format_sig(se, 2)));
            }
            content += super::estimate_tokens(&line);
            text.push_str(&line);
            text.push('\n');
        }

        let key = shape_key(&item.body, record);
        let body_text = match (&item.body, last_table.get(&key)) {
            (Body::Table { .. }, Some((base, prev))) => match delta(prev, &item.body) {
                Some(Change::Identical) => {
                    savings.push(format!("{} is identical to {base}", item.id));
                    format!("identical to {base}")
                }
                Some(Change::Delta(d)) => {
                    savings.push(format!("{} rendered as a delta on {base}", item.id));
                    format!("as {base}, except {d}")
                }
                None => render_with_schema(&item.body, &mut schema_stated, key.clone(), item.id),
            },
            _ => render_with_schema(&item.body, &mut schema_stated, key.clone(), item.id),
        };
        if matches!(item.body, Body::Table { .. }) {
            last_table.insert(key, (item.id, item.body.clone()));
        }
        content += super::estimate_tokens(&body_text);
        text.push_str(&body_text);
        pieces.push(Piece {
            id: item.id,
            provenance: record.map_or(crate::provenance::Provenance::System, |r| r.provenance),
            source: record.map(|r| r.source.clone()).unwrap_or_default(),
            text,
        });
    }

    // ---- disagreements are named, never resolved (§24.8) --------------------
    for (a, b) in &sel.disagreements {
        let line = format!(
            "NOTE: {a} and {b} measure the same quantity and disagree beyond their stated \
             error. Both are shown; the platform does not pick between them."
        );
        overhead += super::estimate_tokens(&line);
        notices.push(line);
    }

    let text = pieces
        .iter()
        .map(|p| p.text.as_str())
        .chain(notices.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");

    Rendered {
        text,
        pieces,
        notices,
        overhead_tokens: overhead,
        content_tokens: content,
        savings,
    }
}

fn header(
    item: &Fitted,
    record: Option<&Record>,
    reason: Option<&str>,
    level: Furniture,
) -> String {
    let mut h = format!("[{}]", item.id);
    if level < Furniture::Terse {
        if let Some(r) = record {
            h.push_str(&format!(" {} {}", r.kind.as_str(), r.source));
        }
    }
    if let Some(d) = item.degraded {
        h.push_str(&format!(
            " (degraded {}→{}, {}; expand for the rest)",
            d.from_tokens,
            d.to_tokens,
            d.how.as_str()
        ));
    }
    if level == Furniture::Full {
        if let Some(why) = reason {
            h.push_str(&format!(" — {why}"));
        }
    }
    h
}

/// Identity of a body's *shape*, for schema sharing and delta detection.
fn shape_key(body: &Body, record: Option<&Record>) -> String {
    match body {
        Body::Table { schema, .. } => format!(
            "{}|{}",
            record.map(|r| r.source.clone()).unwrap_or_default(),
            schema
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => String::new(),
    }
}

fn render_with_schema(
    body: &Body,
    stated: &mut BTreeMap<String, RecordId>,
    key: String,
    id: RecordId,
) -> String {
    let Body::Table {
        schema,
        rows,
        total_rows,
    } = body
    else {
        return body.render();
    };
    if key.is_empty() {
        return body.render();
    }
    match stated.get(&key) {
        Some(first) => {
            // The schema is already on screen; naming where it was is two tokens
            // against re-stating a header that can be twenty.
            let mut out = format!("(schema as {first})");
            for r in rows {
                out.push('\n');
                out.push_str(&r.join(" | "));
            }
            if *total_rows > rows.len() {
                out.push_str(&format!("\n({} of {total_rows} rows)", rows.len()));
            }
            out
        }
        None => {
            stated.insert(key, id);
            let _ = schema;
            body.render()
        }
    }
}

/// What changed between two same-shape tables, or `None` when a delta would not
/// be shorter.
///
/// The guard matters: a re-read where everything changed is not a delta, it is
/// the same table with extra words, and rendering it as one would cost tokens to
/// say nothing.
/// What a re-read of the same shape turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// Byte-for-byte the same read. Two tokens instead of the table.
    Identical,
    /// A small number of cells moved.
    Delta(String),
}

fn delta(base: &Body, now: &Body) -> Option<Change> {
    let (
        Body::Table {
            schema: s0,
            rows: r0,
            ..
        },
        Body::Table {
            schema: s1,
            rows: r1,
            ..
        },
    ) = (base, now)
    else {
        return None;
    };
    if s0 != s1 || r0.len() != r1.len() || r0.is_empty() {
        return None;
    }
    let mut changed = 0usize;
    let mut first: Option<String> = None;
    for (a, b) in r0.iter().zip(r1.iter()) {
        if a == b {
            continue;
        }
        changed += 1;
        if first.is_none() {
            for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                if x != y {
                    first = Some(format!(
                        "{} {x} → {y}",
                        s1.get(i).map_or("col", |c| c.name.as_str())
                    ));
                    break;
                }
            }
        }
    }
    if changed == 0 {
        return Some(Change::Identical);
    }
    // More than a third changed and the delta stops being cheaper than the
    // table.
    if changed * 3 > r0.len() {
        return None;
    }
    Some(Change::Delta(format!(
        "{}; {changed} row{} changed",
        first.unwrap_or_else(|| "values differ".into()),
        if changed == 1 { "" } else { "s" }
    )))
}

/// Fold repeated corrections on the same path into one line with a count.
///
/// The count is the point, not the saving: "3rd rejection on this path" is the
/// signal to hand the model a worked example, and three separate rejections read
/// as three unrelated problems.
fn fold_corrections(store: &RunStore, kept: &[Fitted]) -> (Vec<Fitted>, Vec<String>) {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<Fitted> = Vec::new();
    let mut notes = Vec::new();

    for item in kept {
        let Some(r) = store.get(item.id) else {
            out.push(item.clone());
            continue;
        };
        if r.kind != RecordKind::Correction {
            out.push(item.clone());
            continue;
        }
        let path = format!(
            "{}|{}",
            r.source,
            r.ids.get("code").cloned().unwrap_or_default()
        );
        match seen.get(&path).copied() {
            Some(idx) => {
                let count = out[idx]
                    .body
                    .render()
                    .rsplit_once("(x")
                    .and_then(|(_, t)| t.trim_end_matches(')').parse::<u32>().ok())
                    .unwrap_or(1)
                    + 1;
                let base = store
                    .get(out[idx].id)
                    .map_or_else(String::new, |b| b.body.render());
                out[idx].body = Body::text(format!("{base} (x{count})"));
                out[idx].tokens = out[idx].body.tokens();
                notes.push(format!(
                    "{count} corrections on {path} folded into {}",
                    out[idx].id
                ));
            }
            None => {
                seen.insert(path, out.len());
                out.push(item.clone());
            }
        }
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Column;
    use crate::context::policy::ContextPolicy;
    use crate::context::select::{select, Query};
    use crate::context::store::{Draft, Measurement, Subject};
    use crate::state::GoalKind;

    fn q(step: u32) -> Query {
        Query {
            step,
            goal_kind: GoalKind::Research,
            ..Query::default()
        }
    }

    fn bars_table(vals: &[(&str, &str)]) -> Body {
        Body::Table {
            schema: vec![Column::new("ts", 200), Column::new("close", 200)],
            rows: vals
                .iter()
                .map(|(a, b)| vec![(*a).to_string(), (*b).to_string()])
                .collect(),
            total_rows: vals.len(),
        }
    }

    fn render_all(store: &RunStore) -> Rendered {
        let p = ContextPolicy::v2();
        let sel = select(store, &q(20), 100_000, &p);
        render(store, &sel, &p, "")
    }

    /// Nothing in the prompt is JSON that a table would say cheaper (§25.1).
    #[test]
    fn no_rendered_table_is_json() {
        let mut s = RunStore::new();
        s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "get_bars",
                bars_table(&[("t1", "100")]),
            )
            .with_subject(Subject::instrument("BTC-USD")),
        );
        let r = render_all(&s);
        assert!(r.text.contains("ts | close"), "{}", r.text);
        assert!(!r.text.contains("{\"ts\""), "{}", r.text);
    }

    /// A schema shared by two selected records appears once (§25.2).
    #[test]
    fn a_shared_schema_is_stated_once() {
        let mut s = RunStore::new();
        // Every row different, so the delta is refused as wholesale change and
        // the thing under test is schema sharing rather than the delta path.
        for i in 0..3u32 {
            s.insert(
                Draft::new(
                    i,
                    RecordKind::ToolResult,
                    "get_bars",
                    bars_table(&[
                        (&format!("t{i}a"), &format!("{}", 100 + i * 10)),
                        (&format!("t{i}b"), &format!("{}", 200 + i * 10)),
                        (&format!("t{i}c"), &format!("{}", 300 + i * 10)),
                    ]),
                )
                .with_subject(Subject::instrument(format!("SYM-{i}")))
                .with_id("window", format!("w{i}")),
            );
        }
        let r = render_all(&s);
        assert_eq!(
            r.text.matches("ts | close").count(),
            1,
            "the header is stated once and referred to after:\n{}",
            r.text
        );
        assert!(r.text.contains("(schema as r"), "{}", r.text);
    }

    /// A re-read renders as a delta naming its base (§25.4).
    #[test]
    fn a_re_read_renders_as_a_delta() {
        let mut s = RunStore::new();
        // Different subjects, so supersession does not collapse them: this is
        // the same *shape* read twice, which is what a delta is for.
        s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "get_quote",
                bars_table(&[("t1", "42118"), ("t2", "200"), ("t3", "300")]),
            )
            .with_subject(Subject::instrument("BTC-USD")),
        );
        s.insert(
            Draft::new(
                2,
                RecordKind::ToolResult,
                "get_quote",
                bars_table(&[("t1", "42340"), ("t2", "200"), ("t3", "300")]),
            )
            .with_subject(Subject::instrument("ETH-USD")),
        );
        let r = render_all(&s);
        assert!(
            r.text.contains("close 42118 → 42340"),
            "the delta names the column and both values:\n{}",
            r.text
        );
        assert!(r.text.contains("1 row changed"), "{}", r.text);
        assert!(!r.savings.is_empty(), "and it is reported as a saving");
    }

    /// A re-read where most of it changed is not a delta.
    ///
    /// The guard matters: that is the same table with extra words, and
    /// rendering it as a delta costs tokens to say nothing.
    #[test]
    fn a_wholesale_change_is_not_rendered_as_a_delta() {
        let a = bars_table(&[("t1", "1"), ("t2", "2"), ("t3", "3")]);
        let b = bars_table(&[("t1", "9"), ("t2", "8"), ("t3", "7")]);
        assert_eq!(delta(&a, &b), None);
        assert_eq!(delta(&a, &a), Some(Change::Identical));
    }

    /// Precision follows the error bar (§25.3).
    #[test]
    fn a_number_is_not_printed_past_its_error_bar() {
        let mut s = RunStore::new();
        s.insert(
            Draft::new(
                1,
                RecordKind::Finding,
                "record_finding",
                Body::text("mean return"),
            )
            .with_subject(Subject::instrument("BTC-USD"))
            .measuring(Measurement::new("mean_return", 0.023_487_162_9).with_se(0.004)),
        );
        let r = render_all(&s);
        assert!(
            r.text.contains("mean_return = 0.023"),
            "eight tokens of noise, and it invites the model to quote them:\n{}",
            r.text
        );
        assert!(!r.text.contains("0.0234871"), "{}", r.text);
        assert!(r.text.contains("(se 0.004)"));
    }

    /// Three same-path corrections render as one with a count (§25.5).
    #[test]
    fn repeated_corrections_fold_into_a_count() {
        let mut s = RunStore::new();
        for step in 1..4u32 {
            s.insert(
                Draft::new(
                    step,
                    RecordKind::Correction,
                    "validate_strategy",
                    Body::text("when-names must be an expression"),
                )
                .with_id("code", "when_names_string"),
            );
        }
        let r = render_all(&s);
        assert!(
            r.text.contains("(x3)"),
            "three rejections read as three unrelated problems:\n{}",
            r.text
        );
        assert_eq!(
            r.text.matches("must be an expression").count(),
            1,
            "and the text is stated once:\n{}",
            r.text
        );
    }

    /// The furniture cap is met by shortening, never by dropping content.
    #[test]
    fn overhead_is_capped_by_shortening_the_labels() {
        let mut s = RunStore::new();
        // Many tiny records: furniture grows with count, content does not.
        for i in 0..40u32 {
            s.insert(
                Draft::new(
                    i,
                    RecordKind::ToolResult,
                    "a_tool_with_a_long_name",
                    Body::text("1"),
                )
                .with_subject(Subject::instrument(format!("SYM-{i}"))),
            );
        }
        let p = ContextPolicy::v2();
        let sel = select(&s, &q(40), 100_000, &p);

        let full = render_at(&s, &sel, Furniture::Full);
        let capped = render(&s, &sel, &p, "");

        assert!(
            capped.content_tokens >= full.content_tokens,
            "content is never dropped to make room for labels about content"
        );
        assert!(
            capped.overhead_tokens <= full.overhead_tokens,
            "the cap is met by shortening: {} vs {}",
            capped.overhead_tokens,
            full.overhead_tokens
        );
    }

    /// The handle survives every level of shortening.
    ///
    /// It is the one piece of furniture that is not a label: without it the
    /// record cannot be asked for back, and the loss stops being recoverable.
    #[test]
    fn the_handle_is_never_shortened_away() {
        let mut s = RunStore::new();
        let id = s.insert(Draft::new(1, RecordKind::Note, "harness", Body::text("x")));
        let p = ContextPolicy::v2();
        let sel = select(&s, &q(2), 100_000, &p);
        for level in [Furniture::Full, Furniture::NoReason, Furniture::Terse] {
            let r = render_at(&s, &sel, level);
            assert!(
                r.text.contains(&id.to_string()),
                "{level:?} lost the handle:\n{}",
                r.text
            );
        }
    }

    /// A disagreement is named in the prompt, not resolved.
    #[test]
    fn a_disagreement_is_stated_to_the_model() {
        let mut s = RunStore::new();
        let subject = Subject::instrument("BTC-USD");
        s.insert(
            Draft::new(1, RecordKind::ToolResult, "source_a", Body::text("a"))
                .with_subject(subject.clone())
                .measuring(Measurement::new("sharpe", 1.4).with_se(0.05)),
        );
        s.insert(
            Draft::new(2, RecordKind::ToolResult, "source_b", Body::text("b"))
                .with_subject(subject)
                .measuring(Measurement::new("sharpe", 2.4).with_se(0.05)),
        );
        let r = render_all(&s);
        assert!(
            r.text.contains("disagree beyond their stated error"),
            "{}",
            r.text
        );
        assert!(
            r.text.contains("does not pick between them"),
            "the disagreement is the finding"
        );
    }

    /// The manifest rides outside the overhead cap (§27.3).
    #[test]
    fn the_manifest_is_not_shortened_to_meet_the_cap() {
        let mut s = RunStore::new();
        s.insert(Draft::new(1, RecordKind::Note, "harness", Body::text("x")));
        let p = ContextPolicy::v2();
        let sel = select(&s, &q(2), 100_000, &p);
        let manifest = "CONTEXT (1 records, 1 shown)\n  r001 s1 Note harness  1 [shown]";
        let r = render(&s, &sel, &p, manifest);
        assert!(r.text.starts_with(manifest), "{}", r.text);
    }
}
