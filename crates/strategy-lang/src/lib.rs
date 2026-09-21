//! SLv2 — the text a model writes a strategy in (AGENT-011, revising FEAT-004 §3).
//!
//! # Why this crate exists
//!
//! `crates/features` computes about fifty feature families and
//! `crates/backtest/src/sim.rs` resolves strategy features through it, so the
//! *engine* has never been the limit. The limit is what a model can **say**: it
//! writes a `StrategyDefinition` as JSON, and a JSON document is a shape to fill
//! in rather than a thing to think in. AGENT-011's argument is that the
//! authoring surface is the capability ceiling, and this crate is that surface.
//!
//! Nothing here reaches a database, a model or the network. It is text in,
//! either a definition or a diagnostic out — which is what makes the whole of it
//! testable without services, and what keeps the validator's answers the same
//! whoever asks.
//!
//! # The three directions
//!
//! ```text
//! SLv2 text ──parse──▶ syntax tree ──compile──▶ StrategyDefinition
//!      ▲                                               │
//!      └───────────────── decompile ◀──────────────────┘
//! ```
//!
//! - [`parse()`] turns text into a tree, or into every reason it is not one. Every
//!   failure names a span and, when the text was ill-formed, what was expected
//!   there — enforced by the type, not by a convention (see [`diagnostic`]).
//! - [`compile()`] turns the tree into the canonical definition ADR-0010 keeps as
//!   the storage and runtime form. Unknown families, wrong arities, and the
//!   clauses FEAT-004 has that the canonical form cannot hold are refused *by
//!   name*, so the model learns the shape of the platform rather than the shape
//!   of the parser.
//! - [`decompile()`] turns a stored definition back into text, so every strategy
//!   has a readable form and so the text and the JSON are one object.
//!   `compile(parse(decompile(d))) == d` is checked in `tests/round_trip.rs`.
//!
//! # The two authoring surfaces
//!
//! The [`skeleton`](mod@crate::skeleton) module is the worked starting point
//! per hypothesis family, so a first version is never a syntax lesson, and
//! [`form`] is the clause grammar a
//! `select_fill` tier writes through. Neither is a second language: a skeleton
//! is text that compiles, and the form assembles text and sends it through
//! [`parse()`] and [`compile()`] above rather than beside them.
//!
//! # The three things a strategy's *history* needs
//!
//! [`diff`] says what changed between two definitions, in SLv2 terms and in the
//! FEAT-004 §6 move vocabulary, with a lineage over a sequence of versions.
//! [`block`] says which clause emptied a strategy that never fires, given the
//! per-clause counts a preview collects. [`suggest`] reads both and proposes the
//! next moves — and, structurally, cannot have made them: everything it hands
//! back is a [`suggest::Fragment`], which is one line, and no SLv2 document is
//! one line.
//!
//! # What is not here
//!
//! `apply_move` and the tools that serve all of it (S-5.8) are separate tasks.
//! The comment machinery is parsed and kept on the tree ([`ast::Comments`]) and
//! stops there: the canonical `StrategyDefinition` has no field to carry a
//! comment, so the *first* half of S-5.15, "intent survives the round trip",
//! needs a schema change in `domain` and not a change here. The second half —
//! the preview naming the blocking clause — is [`block`].

#![allow(clippy::module_name_repetitions)]
// A `Diagnostic` is large because it teaches: a code, a span, a message, what
// was expected, a concrete fix and a guide anchor. Boxing it to shrink the `Err`
// variant would add an allocation to the one path that is already allocating
// several strings, and would put a `Box` in the signature of every public
// entry point for no reader's benefit. Recorded rather than silenced blind.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod block;
pub mod catalogue;
pub mod compile;
pub mod decompile;
pub mod diagnostic;
pub mod diff;
pub mod form;
pub mod lex;
pub mod parse;
pub mod skeleton;
pub mod suggest;

pub use block::{blocking_profile, entry_clauses, BlockingProfile, EntryCounts, Verdict};
pub use catalogue::Catalogue;
pub use compile::{compile, compile_with};
pub use decompile::{decompile, decompile_with};
pub use diagnostic::{Code, Diagnostic, Expected, Kind, Span};
pub use diff::{diff, diff_with, lineage, Change, Diff, Lineage, MoveKind, Version};
pub use form::{FormStrategy, Limitation, Refusal};
pub use parse::parse;
pub use skeleton::{skeleton, HypothesisFamily, Skeleton};
pub use suggest::{suggest_moves, Evidence, Findings, Fragment, Suggestion, Suggestions};

use domain::strategy_def::StrategyDefinition;

/// Text in, canonical definition out: the whole front door in one call.
///
/// # Errors
///
/// Every diagnostic the text produced, in source order.
pub fn text_to_definition(source: &str) -> Result<StrategyDefinition, Vec<Diagnostic>> {
    let ast = parse(source)?;
    compile(&ast)
}

/// Text in, canonical definition out, against a caller-supplied catalogue.
///
/// # Errors
///
/// Every diagnostic the text produced, in source order.
pub fn text_to_definition_with(
    source: &str,
    catalogue: &Catalogue,
) -> Result<StrategyDefinition, Vec<Diagnostic>> {
    let ast = parse(source)?;
    compile_with(&ast, catalogue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_door_compiles_a_strategy_and_reports_diagnostics_as_a_list() {
        let ok = text_to_definition(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit time 24 bars
",
        )
        .expect("compiles");
        assert_eq!(ok.strategy_id, "s");

        let errs = text_to_definition("strategy s on $bound_at_init timeframe 1h\nentry long when nope(close, 14) < 30\nsize fixed 1\nexit none\n")
            .expect_err("refused");
        assert!(!errs.is_empty());
        assert!(errs.iter().all(|d| d.fix.is_some()));
    }
}
