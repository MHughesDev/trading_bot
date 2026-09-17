//! The manifest (AGENT-005 §17; Set S task S-9.2, and §27.3 for the ceiling).
//!
//! # What it is for
//!
//! The failure mode of every retrieval system is the miss, and a miss is silent:
//! the model cannot ask for what it does not know exists, so it reasons from what
//! it has and reaches a confident wrong answer. Recency-trimming has the same
//! failure and no cure.
//!
//! The manifest is the cure, and it is small. One line per record — id, step,
//! kind, source, subject, size, and whether it is currently in the prompt — is
//! roughly twelve tokens, so a hundred-record run costs about 1.2k tokens to
//! turn *every* silent absence into a visible, recoverable [`expand`]. That is
//! the difference between an agent that forgot and one that knows where that is,
//! and it is what licenses selection to be aggressive: the cost of leaving
//! something out is now one line and one optional call.
//!
//! # It degrades by grouping, and it is never cut
//!
//! Two rules, and the second is the one worth stating out loud:
//!
//! * The manifest is **never degraded below one line per group**. When it does
//!   not fit, adjacent records fold into `s3–s7 · 5 × ToolResult get_bars · 19k`
//!   — the ids still resolve, because a group names its range.
//! * It is **never truncated** (§27.3). A cut manifest lies about what exists,
//!   which is worse than no manifest: the model would then have a complete-looking
//!   index that silently omits records, and would stop asking.
//!
//! At the coarsest grouping the manifest is one line per record kind — at most
//! seven lines plus a header — so "group harder" always terminates well under any
//! sane ceiling. `the_manifest_never_truncates` is the test that keeps it honest.
//!
//! [`expand`]: super::store::RunStore::note_expansion

use std::collections::BTreeSet;

use super::store::{Record, RecordId, RecordKind, RunStore};

/// How hard the manifest is grouping. Escalated until it fits (§17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grouping {
    /// One line per record.
    PerRecord,
    /// Adjacent records sharing kind, source and shown-state fold together.
    Adjacent,
    /// All records sharing kind and source fold together.
    BySource,
    /// All records sharing a kind fold together. The floor.
    ByKind,
}

impl Grouping {
    const LADDER: &'static [Grouping] = &[
        Grouping::PerRecord,
        Grouping::Adjacent,
        Grouping::BySource,
        Grouping::ByKind,
    ];
}

/// A rendered manifest and the grouping it needed.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub text: String,
    pub grouping: Grouping,
    pub lines: usize,
    pub tokens: u32,
}

/// Render the whole store as an index, at the coarsest grouping that fits.
///
/// `shown` is the set selection chose to render this call; everything else is
/// marked `elided` and carries its id, which is the handle `expand` takes.
#[must_use]
pub fn render(store: &RunStore, shown: &BTreeSet<RecordId>, max_tokens: u32) -> Manifest {
    let mut last = None;
    for grouping in Grouping::LADDER {
        let text = render_at(store, shown, *grouping);
        let tokens = super::estimate_tokens(&text);
        let m = Manifest {
            lines: text.lines().count(),
            text,
            grouping: *grouping,
            tokens,
        };
        if tokens <= max_tokens {
            return m;
        }
        last = Some(m);
    }
    // The floor. Returned over budget rather than cut: §27.3 — a manifest that
    // is truncated lies about what exists, and the model stops asking.
    last.expect("the ladder is not empty")
}

fn render_at(store: &RunStore, shown: &BTreeSet<RecordId>, grouping: Grouping) -> String {
    let total = store.len();
    let mut out = format!("CONTEXT ({total} records, {} shown)", shown.len());
    for group in group(store, shown, grouping) {
        out.push('\n');
        out.push_str(&group.line(store));
    }
    out
}

/// One manifest line's worth of records.
struct Group {
    ids: Vec<RecordId>,
    first_step: u32,
    last_step: u32,
    kind: RecordKind,
    source: String,
    subject: String,
    tokens: u32,
    shown: usize,
}

impl Group {
    fn of(r: &Record, shown: bool) -> Self {
        Self {
            ids: vec![r.id],
            first_step: r.step,
            last_step: r.step,
            kind: r.kind,
            source: r.source.clone(),
            subject: r.subject.label(),
            tokens: r.tokens_full,
            shown: usize::from(shown),
        }
    }

    fn absorb(&mut self, r: &Record, shown: bool) {
        self.ids.push(r.id);
        self.first_step = self.first_step.min(r.step);
        self.last_step = self.last_step.max(r.step);
        self.tokens += r.tokens_full;
        self.shown += usize::from(shown);
        if self.subject != r.subject.label() {
            self.subject = String::new();
        }
    }

    fn line(&self, store: &RunStore) -> String {
        let steps = if self.first_step == self.last_step {
            format!("s{}", self.first_step)
        } else {
            format!("s{}-{}", self.first_step, self.last_step)
        };
        let state = if self.shown == self.ids.len() {
            "shown".to_string()
        } else if self.shown == 0 {
            "elided".to_string()
        } else {
            format!("{} shown", self.shown)
        };
        if self.ids.len() == 1 {
            let id = self.ids[0];
            // A superseded record collapses onto its replacement rather than
            // reading as a live observation (§26.1) — and it keeps its id, so it
            // is still `expand`able.
            let chain = match store.get(id).and_then(|r| r.superseded_by) {
                Some(by) => format!(" superseded by {by}"),
                None => {
                    let depth = store.chain_depth(id);
                    if depth > 0 {
                        format!(" (+{depth} earlier)")
                    } else {
                        String::new()
                    }
                }
            };
            format!(
                "  {id} {steps} {} {} {} {} [{state}]{chain}",
                self.kind.as_str(),
                self.source,
                self.subject,
                size(self.tokens)
            )
        } else {
            format!(
                "  {}-{} {steps} {} x {} {} {} {} [{state}]",
                self.ids[0],
                self.ids[self.ids.len() - 1],
                self.ids.len(),
                self.kind.as_str(),
                self.source,
                self.subject,
                size(self.tokens)
            )
        }
    }
}

fn size(tokens: u32) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", f64::from(tokens) / 1000.0)
    } else {
        tokens.to_string()
    }
}

fn group(store: &RunStore, shown: &BTreeSet<RecordId>, grouping: Grouping) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for r in store.all() {
        let is_shown = shown.contains(&r.id);
        let slot = match grouping {
            Grouping::PerRecord => None,
            Grouping::Adjacent => groups.last_mut().filter(|g| {
                g.kind == r.kind
                    && g.source == r.source
                    && (g.shown == g.ids.len()) == is_shown
                    && (g.shown == 0) != is_shown
            }),
            Grouping::BySource => groups
                .iter_mut()
                .find(|g| g.kind == r.kind && g.source == r.source),
            Grouping::ByKind => groups.iter_mut().find(|g| g.kind == r.kind),
        };
        match slot {
            Some(g) => g.absorb(r, is_shown),
            None => groups.push(Group::of(r, is_shown)),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Body;
    use crate::context::store::{Draft, Subject};

    fn store_of(n: u32) -> RunStore {
        let mut s = RunStore::new();
        for i in 0..n {
            s.insert(
                Draft::new(
                    i,
                    RecordKind::ToolResult,
                    "get_bars",
                    Body::text("x".repeat(400)),
                )
                .with_subject(Subject::instrument(format!("SYM-{i}")))
                .with_id("window", i.to_string()),
            );
        }
        s
    }

    /// Every record in the store appears in the manifest. The acceptance.
    #[test]
    fn every_record_appears() {
        let s = store_of(12);
        let m = render(&s, &BTreeSet::new(), 10_000);
        assert_eq!(m.grouping, Grouping::PerRecord);
        for r in s.all() {
            assert!(
                m.text.contains(&r.id.to_string()),
                "{} missing from\n{}",
                r.id,
                m.text
            );
        }
    }

    /// An elided record still carries the id `expand` takes.
    #[test]
    fn an_elided_record_is_expandable_by_the_id_shown() {
        let s = store_of(3);
        let shown = BTreeSet::from([RecordId(2)]);
        let m = render(&s, &shown, 10_000);
        assert!(m.text.contains("r001"), "{}", m.text);
        assert!(m.text.contains("[elided]"));
        assert!(m.text.contains("[shown]"));
        assert_eq!(
            RecordId::parse("r001"),
            Some(RecordId(1)),
            "the id in the line is the id the tool takes"
        );
    }

    /// Under pressure it groups; it never drops a record and never cuts.
    #[test]
    fn it_degrades_by_grouping_not_by_dropping() {
        let s = store_of(200);
        let m = render(&s, &BTreeSet::new(), 300);
        assert!(m.grouping > Grouping::PerRecord, "it had to group");
        assert!(
            m.text.contains(" x "),
            "a group states its count:\n{}",
            m.text
        );
        assert!(
            m.text.contains("200 records"),
            "and the header still states the true total"
        );
    }

    /// The ceiling is met by grouping harder, never by truncating (§27.3).
    #[test]
    fn the_manifest_never_truncates() {
        let s = store_of(500);
        for ceiling in [10_000, 1_000, 200, 50, 1] {
            let m = render(&s, &BTreeSet::new(), ceiling);
            assert!(
                m.text.starts_with("CONTEXT (500 records"),
                "the header is always whole at ceiling {ceiling}"
            );
            assert!(
                !m.text.contains('…') && !m.text.ends_with("..."),
                "a cut manifest lies about what exists"
            );
            // Every line is a complete line: it names a kind.
            for line in m.text.lines().skip(1) {
                assert!(
                    line.contains("ToolResult"),
                    "incomplete line at ceiling {ceiling}: {line}"
                );
            }
        }
    }

    /// The floor is one line per kind, which is at most seven plus a header.
    #[test]
    fn the_floor_is_one_line_per_kind() {
        let mut s = RunStore::new();
        for (i, kind) in RecordKind::ALL.iter().enumerate() {
            for j in 0..40 {
                s.insert(Draft::new(
                    u32::try_from(i * 40 + j).unwrap(),
                    *kind,
                    "src",
                    Body::text("x".repeat(400)),
                ));
            }
        }
        let m = render(&s, &BTreeSet::new(), 1);
        assert_eq!(m.grouping, Grouping::ByKind);
        assert_eq!(
            m.lines,
            RecordKind::ALL.len() + 1,
            "seven kinds and a header:\n{}",
            m.text
        );
    }

    /// A superseded record reads as superseded, not as a live observation.
    #[test]
    fn a_superseded_record_is_collapsed_and_still_listed() {
        let mut s = RunStore::new();
        let subject = Subject::instrument("BTC-USD");
        for step in [1, 5] {
            s.insert(
                Draft::new(step, RecordKind::ToolResult, "get_bars", Body::text("x"))
                    .with_subject(subject.clone())
                    .with_id("window", "2024-01..06"),
            );
        }
        let m = render(&s, &BTreeSet::new(), 10_000);
        assert!(
            m.text.contains("superseded by r002"),
            "the first read must not read as current:\n{}",
            m.text
        );
        assert!(
            m.text.contains("(+1 earlier)"),
            "and the head says how deep its chain is:\n{}",
            m.text
        );
    }

    /// Roughly twelve tokens a line, which is the cost claim §17 makes.
    #[test]
    fn a_line_costs_about_twelve_tokens() {
        let s = store_of(100);
        let m = render(&s, &BTreeSet::new(), 10_000);
        assert_eq!(m.grouping, Grouping::PerRecord);
        let per_line = m.tokens / 100;
        assert!(
            (8..=20).contains(&per_line),
            "a hundred-record manifest costs {} tokens, {per_line} a line",
            m.tokens
        );
    }
}
