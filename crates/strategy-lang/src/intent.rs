//! Why a threshold is what it is: the comments an author wrote (S-5.15;
//! AGENT-011 §3, "Comments are part of the language").
//!
//! A strategy is read by a reviewer, by the model on its next turn and by whoever
//! opens the dossier in six months, and none of them can recover *why* a
//! threshold is 0.6 from the number. The parser attaches each `# …` to the
//! clause below it; this module reads them back off the syntax tree, addressed
//! the way every other part of the language addresses a clause, so that
//! `explain_strategy` can show them beside the clause they explain.
//!
//! # What this does not do, and why that is a decision and not an omission
//!
//! It does not store them. The task as written asks the compiler to put the
//! comment on the canonical node and the decompiler to emit it back, so that
//! intent survives compile → JSON → decompile. That needs a comment field on
//! `domain::strategy_def`, and the domain has deliberately avoided one:
//! `revision.rs` records that "two documents that differ only in a comment-like
//! field hash differently and trade identically", which would make every
//! rewording of a note a new revision — and a new counted-looking trial for
//! anything deduplicating on the content hash, the error `N_eff` exists to
//! prevent. SLv3's graph excludes comments for the same reason (ADR-0045:
//! "layout, comments and argument order are not part of the graph"), and v1 is
//! retired by U-8.4. So comments live **beside** the definition, in the author's
//! own text. Recorded as ADR-S5-07 with the alternative that was rejected. What
//! would close the gap without touching the hash is a `source_text` column on the
//! revision table, which is a ledger migration and is reported, not built.

use crate::ast::Strategy;

/// One comment block and the clause it is attached to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The clause, addressed as the rest of the language addresses it:
    /// `header`, `param fast`, `constraint`, `input fast_ma`, `entry.long[0]`,
    /// `size`, `exit[2]`, `order[0]`, `risk max_position`.
    pub clause: String,
    /// The comment lines, `#` and the space after it removed.
    pub lines: Vec<String>,
}

/// Every comment in `s`, in source order, each with the clause it belongs to.
///
/// A clause with no comment contributes nothing: an empty note is not
/// information and would only make a reader wonder whether it was deleted.
#[must_use]
pub fn notes(s: &Strategy) -> Vec<Note> {
    let mut out = Vec::new();
    let mut push = |clause: String, lines: &[String]| {
        if !lines.is_empty() {
            out.push(Note {
                clause,
                lines: lines.to_vec(),
            });
        }
    };

    push("header".into(), &s.header_comments);
    for p in &s.params {
        push(format!("param {}", p.name), &p.comments);
    }
    for c in &s.constraints {
        push("constraint".into(), &c.comments);
    }
    for i in &s.inputs {
        push(format!("input {}", i.name), &i.comments);
    }
    let (mut long, mut short) = (0usize, 0usize);
    for e in &s.entries {
        let k = match e.side {
            crate::ast::Side::Long => {
                long += 1;
                long - 1
            }
            crate::ast::Side::Short => {
                short += 1;
                short - 1
            }
        };
        push(format!("entry.{}[{k}]", e.side.as_str()), &e.comments);
    }
    if let Some(size) = &s.size {
        push("size".into(), &size.comments);
    }
    for (i, x) in s.exits.iter().enumerate() {
        push(format!("exit[{i}]"), &x.comments);
    }
    for (i, o) in s.orders.iter().enumerate() {
        push(format!("order[{i}]"), &o.comments);
    }
    for r in &s.risk {
        push(format!("risk {}", r.kind.as_str()), &r.comments);
    }
    out
}

/// [`notes`] of SLv2 text, or the diagnostics that stopped it parsing.
///
/// # Errors
///
/// Every diagnostic the text produced.
pub fn notes_of_text(text: &str) -> Result<Vec<Note>, Vec<crate::Diagnostic>> {
    Ok(notes(&crate::parse(text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "\
# a trend follower; the 60 gate keeps it out of blow-off tops
strategy s on $bound_at_init timeframe 1h
# the two baselines are what the hypothesis is about
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
input fast_ma = ema(close, fast)
input slow_ma = ema(close, slow)
# 60 rather than 70: this filter refuses an extended move, it does not call a top.
# Revisit if the sweep prefers 70.
entry long when fast_ma > slow_ma and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
# A trend that has not worked in two days was not the one this rule is about.
exit time 48 bars
";

    #[test]
    fn a_comment_is_addressed_to_the_clause_below_it() {
        let n = notes_of_text(TEXT).expect("parses");
        let by = |clause: &str| n.iter().find(|x| x.clause == clause);
        assert_eq!(
            by("header").unwrap().lines,
            ["a trend follower; the 60 gate keeps it out of blow-off tops"]
        );
        assert_eq!(by("param fast").unwrap().lines.len(), 1);
        // Two lines above one clause stay together, in order.
        let entry = by("entry.long[0]").expect("the entry has a note");
        assert_eq!(entry.lines.len(), 2);
        assert!(entry.lines[0].starts_with("60 rather than 70"));
        // Addressed to the clause below, not the one above.
        assert!(by("exit[1]").unwrap().lines[0].contains("two days"));
        assert!(by("exit[0]").is_none(), "the stop has no comment of its own");
    }

    #[test]
    fn a_clause_with_no_comment_has_no_note() {
        let n = notes_of_text(TEXT).expect("parses");
        assert!(n.iter().all(|x| !x.lines.is_empty()));
        assert!(n.iter().all(|x| x.clause != "size"));
    }

    /// The addresses are the language's own: an entry note names the same clause
    /// `explain_strategy` and `apply_move` do.
    #[test]
    fn the_entry_address_is_the_one_the_other_tools_use() {
        let def = crate::text_to_definition(TEXT).expect("compiles");
        let entries = crate::entry_clauses(&def).expect("clauses");
        let n = notes_of_text(TEXT).expect("parses");
        assert!(n.iter().any(|x| x.clause == entries[0].path), "{n:#?}");
    }

    /// The decision this module records: a comment does not reach the stored
    /// definition, so it cannot change a revision hash.
    #[test]
    fn comments_do_not_change_the_definition_or_its_revision() {
        let bare: String = TEXT
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let with = crate::text_to_definition(TEXT).expect("compiles");
        let without = crate::text_to_definition(&bare).expect("compiles");
        assert_eq!(with, without, "a comment changed what was compiled");
        let hash = |d| domain::strategy_def::revision::RevisionHash::of(d).expect("hashes");
        assert_eq!(hash(&with), hash(&without));
    }
}
