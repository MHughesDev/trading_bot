//! `apply_move`: one typed edit in, one new version out, or a named refusal
//! (S-5.10; AGENT-011 §4, P5; FEAT-004 §6).
//!
//! # Why this does not take a [`MoveKind`]
//!
//! The obvious signature is `apply_move(def, MoveKind, args)`, and it is wrong
//! in a way that matters. [`crate::diff`] is the arbiter of what a change *is*:
//! it is the thing a reviewer and the model's next turn both read, and it
//! classifies from the two documents rather than from what anybody intended. An
//! applier that accepted a kind would be free to disagree with it — a caller
//! asking for `swap_signal` and supplying a replacement that differs only in a
//! threshold would get a version the differ calls `change_threshold`, and the
//! lineage would then say something the author does not believe.
//!
//! So an [`Edit`] says **what to touch**, never what the touch amounts to, and
//! the kind comes back from the differ. What this module guarantees instead is
//! the property the research protocol actually needs: **one move per version**.
//!
//! # The guarantee is the type, not a sentence
//!
//! [`Applied`] has private fields and no public constructor. The only way to
//! hold one is [`apply_move()`], which computes `diff(before, after)` itself and
//! refuses unless it is exactly one change. There is therefore no path that
//! produces a "new version" nobody classified, and [`Applied::kind`] cannot be
//! a claim that was never checked.
//!
//! # Why the edit goes through [`crate::form`]
//!
//! The form is the typed clause vocabulary this crate already has: `Filter`,
//! `Size`, `Exit` are closed enums with a JSON Schema, checked against the
//! engine's registry. Editing the form and re-rendering means a move is a typed
//! substitution rather than string surgery on a document, and it means an edit
//! cannot introduce a feature the engine does not serve — the fills cannot spell
//! one.
//!
//! The cost is real and is stated rather than hidden: the form is **narrower**
//! than the language (13 named [`crate::form::Limitation`]s). A definition
//! outside it is refused by name, carrying the limitation it hit, instead of
//! being edited into something whose meaning drifted.

use serde::{Deserialize, Serialize};

use domain::strategy_def::StrategyDefinition;

use crate::ast::Side;
use crate::catalogue::Catalogue;
use crate::diagnostic::{Code, Diagnostic, Span};
use crate::diff::{diff_with, Diff, MoveKind};
use crate::form::{self, Exit, Filter, FormStrategy, NonEmpty, Operand, Size};

/// An edit has no source text of its own to point at.
const NO_SOURCE: Span = Span::new(0, 0, 0, 0);

fn refuse(code: Code, message: impl Into<String>, fix: impl Into<String>) -> Diagnostic {
    Diagnostic::semantic(code, NO_SOURCE, message)
        .with_fix(fix)
        .with_see("guide#moves")
}

// ── addressing ───────────────────────────────────────────────────────────────

/// `entry.long[0]` — one entry condition.
///
/// A parsed address rather than the string, so a path that does not name an
/// entry is refused where it is written and not three calls later. The `Display`
/// form is byte-identical to [`crate::block::EntryClauses::path`], which is what
/// lets a blocking profile, a suggestion and an edit all name the same clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EntryPath {
    pub side: Side,
    /// The index of the entry within its side.
    pub index: usize,
}

impl EntryPath {
    #[must_use]
    pub const fn new(side: Side, index: usize) -> Self {
        Self { side, index }
    }

    /// Read `entry.long[0]`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("entry.")?;
        let (side, rest) = rest.split_once('[')?;
        let side = match side {
            "long" => Side::Long,
            "short" => Side::Short,
            _ => return None,
        };
        let index = rest.strip_suffix(']')?.parse().ok()?;
        Some(Self { side, index })
    }
}

impl std::fmt::Display for EntryPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "entry.{}[{}]", self.side.as_str(), self.index)
    }
}

/// `entry.long[0].clause[1]` — one top-level conjunct of one entry condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClausePath {
    pub entry: EntryPath,
    pub clause: usize,
}

impl ClausePath {
    #[must_use]
    pub const fn new(entry: EntryPath, clause: usize) -> Self {
        Self { entry, clause }
    }

    /// Read `entry.long[0].clause[1]`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (entry, rest) = s.split_once(".clause[")?;
        Some(Self {
            entry: EntryPath::parse(entry)?,
            clause: rest.strip_suffix(']')?.parse().ok()?,
        })
    }
}

impl std::fmt::Display for ClausePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.clause[{}]", self.entry, self.clause)
    }
}

/// The wire form of an address is the address, so the string a blocking profile
/// printed is the string an edit is written with. A shape that is not one is
/// refused at the boundary rather than inside the applier.
#[derive(Debug, thiserror::Error)]
#[error("`{0}` is not an address this crate writes; they look like `entry.long[0]` and `entry.long[0].clause[1]`, exactly as `explain_strategy` prints them")]
pub struct BadAddress(String);

impl From<EntryPath> for String {
    fn from(p: EntryPath) -> Self {
        p.to_string()
    }
}

impl TryFrom<String> for EntryPath {
    type Error = BadAddress;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s).ok_or(BadAddress(s))
    }
}

impl From<ClausePath> for String {
    fn from(p: ClausePath) -> Self {
        p.to_string()
    }
}

impl TryFrom<String> for ClausePath {
    type Error = BadAddress;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s).ok_or(BadAddress(s))
    }
}

// ── the edit ─────────────────────────────────────────────────────────────────

/// One typed edit to one strategy.
///
/// Closed, and every variant carries everything the edit needs. A `Proposal`
/// from [`crate::suggest`] deliberately does **not** convert into one of these:
/// `MoveThreshold` says a threshold is binding and names a direction, not a
/// value, because "a number chosen here would be a guess wearing a
/// measurement's authority". Choosing the number is the caller's act, and this
/// is where they perform it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum Edit {
    /// Move the one number in a clause. `to` is the decimal **as written**:
    /// the canonical form stores fractions as decimal strings (ADR-0002) and a
    /// round trip through `f64` is a different number.
    Threshold { clause: ClausePath, to: String },
    /// `and` a filter into an entry condition.
    AddFilter {
        entry: EntryPath,
        filter: Box<Filter>,
    },
    /// Drop one conjunct. Refused when it is the only one — an entry with no
    /// condition is not a filter removal, it is a strategy with no entry.
    RemoveFilter { clause: ClausePath },
    /// Replace one conjunct with another.
    ReplaceFilter {
        clause: ClausePath,
        with: Box<Filter>,
    },
    /// The bar the strategy decides on.
    Timeframe { to: String },
    /// The sizing rule.
    Sizing { to: Size },
    /// A new exit rule, appended.
    AddExit { rule: Exit },
    /// Replace the exit rule at `index`.
    ReplaceExit { index: usize, with: Exit },
    /// Drop the exit rule at `index`. Refused when it is the only one: a 1.3
    /// document with no exits does not compile, and `exit none` is how a
    /// strategy says it delegates.
    RemoveExit { index: usize },
}

// ── the result ───────────────────────────────────────────────────────────────

/// A new version, and the single move the differ says it is.
///
/// Private fields and no public constructor: the only way to hold one is
/// [`apply_move()`], which verified that `diff(before, after)` is exactly one
/// change before it built this. A caller therefore cannot be holding a "new
/// version" that nobody classified, and [`Self::kind`] is a measurement rather
/// than an intention.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    definition: StrategyDefinition,
    text: String,
    diff: Diff,
}

impl Applied {
    #[must_use]
    pub fn definition(&self) -> &StrategyDefinition {
        &self.definition
    }

    /// The new version as SLv2, so the caller can show the edit rather than
    /// describe it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn diff(&self) -> &Diff {
        &self.diff
    }

    /// The move this edit turned out to be, as [`crate::diff`] classifies it.
    ///
    /// Total, because the constructor refused anything that was not exactly one
    /// change. This is the join AGENT-011 §4's acceptance asks for: a move
    /// applied here comes back from `diff_strategies` as this kind.
    #[must_use]
    pub fn kind(&self) -> MoveKind {
        self.diff
            .changes()
            .first()
            .map_or(MoveKind::Rewrite, |c| c.kind)
    }

    /// One sentence: what changed, in the language the strategy was written in.
    #[must_use]
    pub fn words(&self) -> String {
        self.diff.render()
    }

    /// The new version under a new slug.
    ///
    /// `create_strategy` upserts by `strategy_id`, so an iteration that keeps
    /// the slug overwrites the version the evidence cites. Renaming is not a
    /// move — [`Diff::is_single_change`] ignores it for exactly that reason —
    /// so it happens here rather than as an [`Edit`].
    #[must_use]
    pub fn into_version(mut self, strategy_id: impl Into<String>) -> StrategyDefinition {
        self.definition.strategy_id = strategy_id.into();
        self.definition
    }

    #[must_use]
    pub fn into_definition(self) -> StrategyDefinition {
        self.definition
    }
}

// ── the entry points ─────────────────────────────────────────────────────────

/// Apply one typed edit, against the engine's registry.
///
/// # Errors
///
/// * The definition has something SLv2 or [`crate::form`] cannot hold, named.
/// * The edit addresses a clause or an exit that is not there.
/// * The edit changed nothing, or changed more than one thing.
pub fn apply_move(def: &StrategyDefinition, edit: &Edit) -> Result<Applied, Diagnostic> {
    apply_move_with(def, edit, &Catalogue::engine())
}

/// As [`apply_move()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`apply_move()`].
pub fn apply_move_with(
    def: &StrategyDefinition,
    edit: &Edit,
    cat: &Catalogue,
) -> Result<Applied, Diagnostic> {
    let text = crate::decompile::decompile_with(def, cat)?;
    let ast = crate::parse::parse(&text).map_err(|ds| {
        // The decompiler is held to the parser by `tests/round_trip.rs`, so
        // this is a defect here rather than something the caller wrote. Say so.
        let first = ds
            .first()
            .map_or_else(|| "no diagnostic".to_string(), |d| d.message.clone());
        refuse(
            Code::BadStoredExpression,
            format!("the decompiler wrote text the parser refuses: {first}"),
            "this is a defect in `strategy-lang`, not in the definition",
        )
    })?;
    let mut form = form::from_strategy_with(&ast, cat).map_err(|rs| {
        rs.into_iter().next().map_or_else(
            || {
                refuse(
                    Code::NotInTheForm,
                    "the clause grammar refused this definition and said nothing about why",
                    "this is a defect in `strategy-lang::form`",
                )
            },
            |r| r.diagnostic,
        )
    })?;

    edit.apply_to(&mut form)?;

    let after = form.compile().map_err(|ds| {
        ds.into_iter().next().unwrap_or_else(|| {
            refuse(
                Code::NotInTheForm,
                "the edited form did not compile and produced no diagnostic",
                "this is a defect in `strategy-lang::form`",
            )
        })
    })?;

    let d = diff_with(def, &after, cat)?;
    match d.n_moves() {
        1 => Ok(Applied {
            text: crate::decompile::decompile_with(&after, cat)?,
            definition: after,
            diff: d,
        }),
        0 => Err(refuse(
            Code::NothingToMove,
            "the edit produced a version the differ cannot tell apart from the original",
            "an edit that changes nothing is not a move; check the value you passed is not the \
             one already there",
        )),
        n => Err(refuse(
            Code::MoveIsNotSingle,
            format!("the edit fell out as {n} changes, not one: {}", d.render()),
            "one change per version, so the next run's result can be attributed to it. Apply \
             them one at a time and keep each version.",
        )),
    }
}

// ── performing the edit ──────────────────────────────────────────────────────

impl Edit {
    fn apply_to(&self, form: &mut FormStrategy) -> Result<(), Diagnostic> {
        match self {
            Self::Threshold { clause, to } => {
                let filters = entry_filters(form, clause.entry)?;
                let mut fs: Vec<Filter> = filters.iter().cloned().collect();
                let f = fs
                    .get_mut(clause.clause)
                    .ok_or_else(|| no_such_clause(*clause))?;
                set_threshold(f, to, *clause)?;
                replace_filters(form, clause.entry, fs)
            }
            Self::AddFilter { entry, filter } => {
                let mut fs: Vec<Filter> = entry_filters(form, *entry)?.iter().cloned().collect();
                fs.push((**filter).clone());
                replace_filters(form, *entry, fs)
            }
            Self::RemoveFilter { clause } => {
                let filters = entry_filters(form, clause.entry)?;
                if clause.clause >= filters.len() {
                    return Err(no_such_clause(*clause));
                }
                let fs: Vec<Filter> = filters
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != clause.clause)
                    .map(|(_, f)| f.clone())
                    .collect();
                if fs.is_empty() {
                    return Err(refuse(
                        Code::MoveEmptiesClause,
                        format!(
                            "`{clause}` is the only conjunct of `{}`, so removing it leaves an \
                             entry with no condition",
                            clause.entry
                        ),
                        "an entry that fires on every bar is a different strategy, not a \
                         narrower one. Replace the filter, or remove the whole entry.",
                    ));
                }
                replace_filters(form, clause.entry, fs)
            }
            Self::ReplaceFilter { clause, with } => {
                let filters = entry_filters(form, clause.entry)?;
                let mut fs: Vec<Filter> = filters.iter().cloned().collect();
                let slot = fs
                    .get_mut(clause.clause)
                    .ok_or_else(|| no_such_clause(*clause))?;
                *slot = (**with).clone();
                replace_filters(form, clause.entry, fs)
            }
            Self::Timeframe { to } => {
                form.header.timeframe.clone_from(to);
                Ok(())
            }
            Self::Sizing { to } => {
                form.size = to.clone();
                Ok(())
            }
            Self::AddExit { rule } => {
                let mut xs: Vec<Exit> = form.exits.iter().cloned().collect();
                xs.push(rule.clone());
                form.exits = non_empty(xs, "exits")?;
                Ok(())
            }
            Self::ReplaceExit { index, with } => {
                let mut xs: Vec<Exit> = form.exits.iter().cloned().collect();
                let slot = xs
                    .get_mut(*index)
                    .ok_or_else(|| no_such_exit(*index, form))?;
                *slot = with.clone();
                form.exits = non_empty(xs, "exits")?;
                Ok(())
            }
            Self::RemoveExit { index } => {
                if *index >= form.exits.len() {
                    return Err(no_such_exit(*index, form));
                }
                let xs: Vec<Exit> = form
                    .exits
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i != index)
                    .map(|(_, x)| x.clone())
                    .collect();
                if xs.is_empty() {
                    return Err(refuse(
                        Code::MoveEmptiesClause,
                        "that is the only exit rule, and a 1.3 document with no exits does not \
                         compile",
                        "write `exit none` if the risk layer really is the whole exit logic — \
                         saying it out loud is the point of that clause",
                    ));
                }
                form.exits = non_empty(xs, "exits")?;
                Ok(())
            }
        }
    }
}

fn no_such_clause(clause: ClausePath) -> Diagnostic {
    refuse(
        Code::NoSuchClause,
        format!("this strategy has no clause at `{clause}`"),
        "clause addresses come from `entry_clauses` (and from a blocking profile or a \
         suggestion, which use the same ones); they are not stable across an edit that changes \
         how many clauses there are",
    )
}

fn no_such_exit(index: usize, form: &FormStrategy) -> Diagnostic {
    refuse(
        Code::NoSuchClause,
        format!(
            "this strategy has {} exit rules, so there is none at index {index}",
            form.exits.len()
        ),
        "exit indices are positions in the `exit` clauses as `explain_strategy` lists them, \
         counting from zero",
    )
}

fn non_empty<T>(items: Vec<T>, what: &str) -> Result<NonEmpty<T>, Diagnostic> {
    NonEmpty::any(items).ok_or_else(|| {
        refuse(
            Code::MoveEmptiesClause,
            format!("the edit would leave no {what} at all"),
            "every strategy declares at least one",
        )
    })
}

/// The filters of one entry, addressed the way the rest of the crate addresses
/// entries: the k-th entry **of that side**, not the k-th entry overall.
fn entry_filters(form: &FormStrategy, path: EntryPath) -> Result<&NonEmpty<Filter>, Diagnostic> {
    form.entries
        .iter()
        .filter(|e| e.side == path.side)
        .nth(path.index)
        .map(|e| &e.filters)
        .ok_or_else(|| {
            refuse(
                Code::NoSuchClause,
                format!("this strategy has no `{path}`"),
                "entry addresses count from zero within each side; `explain_strategy` lists the \
                 ones this strategy has",
            )
        })
}

fn replace_filters(
    form: &mut FormStrategy,
    path: EntryPath,
    filters: Vec<Filter>,
) -> Result<(), Diagnostic> {
    let filters = non_empty(filters, "filters on that entry")?;
    let entry = form
        .entries
        .iter_mut()
        .filter(|e| e.side == path.side)
        .nth(path.index)
        .ok_or_else(|| {
            refuse(
                Code::NoSuchClause,
                format!("this strategy has no `{path}`"),
                "entry addresses count from zero within each side",
            )
        })?;
    entry.filters = filters;
    Ok(())
}

/// Move the one number in a comparison.
///
/// "The one" is load-bearing. A clause comparing two features has no threshold
/// to move and a `between` has two, and in both cases picking one would be this
/// module deciding what the author meant. Each is refused by name, with the edit
/// that does work.
fn set_threshold(filter: &mut Filter, to: &str, at: ClausePath) -> Result<(), Diagnostic> {
    let Filter::Compare { lhs, rhs, .. } = filter else {
        return Err(refuse(
            Code::NothingToMove,
            format!("`{at}` is a `between`, which has two bounds and no single threshold"),
            "use `ReplaceFilter` and write the bounds you want, so the version says which one \
             moved",
        ));
    };
    let numbers = usize::from(matches!(lhs, Operand::Number { .. }))
        + usize::from(matches!(rhs, Operand::Number { .. }));
    match numbers {
        1 => {
            for side in [lhs, rhs] {
                if let Operand::Number { value } = side {
                    if value == to {
                        return Err(refuse(
                            Code::NothingToMove,
                            format!("`{at}` already compares against `{to}`"),
                            "an edit that changes nothing is not a move",
                        ));
                    }
                    value.clear();
                    value.push_str(to);
                }
            }
            Ok(())
        }
        0 => Err(refuse(
            Code::NothingToMove,
            format!("`{at}` compares two values and has no threshold to move"),
            "there is no number here to tune. `suggest_moves` proposes dropping such a clause \
             rather than moving it; `ReplaceFilter` writes a different one.",
        )),
        _ => Err(refuse(
            Code::NothingToMove,
            format!("`{at}` compares two numbers, so which one moved would be a guess"),
            "use `ReplaceFilter` and write the clause you mean",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{Arg, Compare};

    const BASE: &str = "\
strategy joined on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 30
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

    fn def(src: &str) -> StrategyDefinition {
        let ast = crate::parse(src).unwrap_or_else(|e| panic!("parse failed:\n{src}\n{e:#?}"));
        crate::compile(&ast).unwrap_or_else(|e| panic!("compile failed:\n{src}\n{e:#?}"))
    }

    fn long0() -> EntryPath {
        EntryPath::new(Side::Long, 0)
    }

    fn rsi_under(n: &str) -> Filter {
        Filter::Compare {
            lhs: Operand::Feature {
                family: "rsi".into(),
                args: vec![Arg::Bars(14)],
            },
            op: Compare::Lt,
            rhs: Operand::Number { value: n.into() },
        }
    }

    /// An address written by one module is read by another, in both directions.
    #[test]
    fn a_clause_address_survives_the_round_trip_through_its_own_text() {
        let base = def(BASE);
        let clauses = crate::entry_clauses(&base).expect("clauses");
        for entry in &clauses {
            assert_eq!(
                EntryPath::parse(&entry.path).map(|p| p.to_string()),
                Some(entry.path.clone())
            );
            for c in &entry.clauses {
                assert_eq!(
                    ClausePath::parse(&c.path).map(|p| p.to_string()),
                    Some(c.path.clone()),
                    "`block` and `apply` must spell the same clause the same way"
                );
            }
        }
        assert!(EntryPath::parse("entry.sideways[0]").is_none());
        assert!(ClausePath::parse("entry.long[0]").is_none());
    }

    /// The wire form of an address is the address, and a string that is not one
    /// is refused at the boundary — an MCP caller writes these by hand, and
    /// `entry.long.0` arriving as a silently-defaulted `entry.long[0]` would
    /// edit a clause nobody named.
    #[test]
    fn an_address_crosses_json_as_itself_and_a_malformed_one_does_not_cross_at_all() {
        let path = ClausePath::new(long0(), 1);
        let wire = serde_json::to_string(&path).expect("serialises");
        assert_eq!(wire, "\"entry.long[0].clause[1]\"");
        assert_eq!(
            serde_json::from_str::<ClausePath>(&wire).expect("reads back"),
            path
        );
        for bad in [
            "\"entry.long.1\"",
            "\"entry.long[0]\"",
            "\"\"",
            "\"clause[1]\"",
        ] {
            assert!(
                serde_json::from_str::<ClausePath>(bad).is_err(),
                "{bad} was accepted as an address"
            );
        }

        // And a whole edit crosses, so the MCP tool's schema is this enum.
        let edit = Edit::Threshold {
            clause: path,
            to: "45".into(),
        };
        let wire = serde_json::to_value(&edit).expect("serialises");
        assert_eq!(wire["edit"], "threshold");
        assert_eq!(wire["clause"], "entry.long[0].clause[1]");
        assert_eq!(
            serde_json::from_value::<Edit>(wire).expect("reads back"),
            edit
        );
    }

    /// The property the whole module exists for, on the move a research turn
    /// makes most: the applied version diffs back as exactly the move it was.
    #[test]
    fn a_threshold_edit_comes_back_from_the_differ_as_one_change_threshold() {
        let base = def(BASE);
        let clause = ClausePath::new(long0(), 1);
        let applied = apply_move(
            &base,
            &Edit::Threshold {
                clause,
                to: "45".into(),
            },
        )
        .expect("applies");

        assert_eq!(applied.kind(), MoveKind::ChangeThreshold);
        assert!(applied.diff().is_single_change());
        assert!(
            applied.text().contains("rsi(close, 14) < 45"),
            "{}",
            applied.text()
        );
        // And the number is the digits that were passed, not a float that went
        // through `f64` and came back a neighbour.
        assert!(!applied.text().contains("< 30"));
    }

    /// Every classifiable move kind is reachable through some [`Edit`], and each
    /// one comes back from the differ as that kind.
    ///
    /// This is S-5.10's acceptance stated once. The kinds the differ cannot
    /// produce are excluded **by asking the differ**, not by a second list here:
    /// `MoveKind::not_classifiable_because` is the one place that is decided,
    /// and a kind that becomes classifiable fails this test until an `Edit`
    /// reaches it.
    #[test]
    fn every_move_kind_an_edit_can_reach_round_trips_through_the_differ() {
        let base = def(BASE);
        let edits: Vec<(MoveKind, Edit)> = vec![
            (
                MoveKind::ChangeThreshold,
                Edit::Threshold {
                    clause: ClausePath::new(long0(), 1),
                    to: "45".into(),
                },
            ),
            (
                MoveKind::AddFilter,
                Edit::AddFilter {
                    entry: long0(),
                    filter: Box::new(Filter::Compare {
                        lhs: Operand::Feature {
                            family: "atr".into(),
                            args: vec![Arg::Bars(14)],
                        },
                        op: Compare::Gt,
                        rhs: Operand::Number { value: "1".into() },
                    }),
                },
            ),
            (
                MoveKind::RemoveFilter,
                Edit::RemoveFilter {
                    clause: ClausePath::new(long0(), 1),
                },
            ),
            (
                MoveKind::SwapFeature,
                Edit::ReplaceFilter {
                    clause: ClausePath::new(long0(), 1),
                    with: Box::new(Filter::Compare {
                        lhs: Operand::Feature {
                            family: "rsi".into(),
                            args: vec![Arg::Bars(21)],
                        },
                        op: Compare::Lt,
                        rhs: Operand::Number { value: "30".into() },
                    }),
                },
            ),
            (
                MoveKind::ChangeTimeframe,
                Edit::Timeframe { to: "4h".into() },
            ),
            (
                MoveKind::ChangeSizing,
                Edit::Sizing {
                    to: Size::RiskPerTrade {
                        fraction: "0.01".into(),
                    },
                },
            ),
            (
                MoveKind::AddExit,
                Edit::AddExit {
                    rule: Exit::Target {
                        percent: "5".into(),
                    },
                },
            ),
            (
                MoveKind::ChangeExit,
                Edit::ReplaceExit {
                    index: 0,
                    with: Exit::Trailing {
                        percent: "2".into(),
                    },
                },
            ),
        ];

        let mut reached: Vec<MoveKind> = Vec::new();
        for (want, edit) in edits {
            let applied = apply_move(&base, &edit)
                .unwrap_or_else(|d| panic!("{edit:?} was refused: {}", d.message));
            assert!(
                applied.diff().is_single_change(),
                "{edit:?} produced {:#?}",
                applied.diff().changes()
            );
            assert_eq!(applied.kind(), want, "{edit:?} — {}", applied.words());
            reached.push(want);
        }

        // The other half: a kind the differ says it *can* classify and no edit
        // reaches is a gap in this module, and this is where it is noticed.
        let missing: Vec<MoveKind> = MoveKind::ALL
            .into_iter()
            .filter(|k| k.classifiable_by_diff())
            .filter(|k| !reached.contains(k))
            // `SwapSignal` and `Rewrite` are the differ's answers when it cannot
            // pair two clauses at all — they classify an edit, they are not an
            // edit a caller asks for. `ReplaceFilter` reaches them when the
            // replacement is unpairable, which is checked on its own below.
            .filter(|k| !matches!(k, MoveKind::SwapSignal | MoveKind::Rewrite))
            .collect();
        assert!(
            missing.is_empty(),
            "the differ classifies {missing:?} and no `Edit` produces them"
        );
    }

    /// A replacement the differ cannot pair with what was there is a
    /// `swap_signal`, and the caller is told that rather than being told the
    /// edit failed.
    #[test]
    fn an_unpairable_replacement_is_reported_as_a_swap_and_not_refused() {
        let one_clause = def("\
strategy one on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit time 48 bars
");
        let applied = apply_move(
            &one_clause,
            &Edit::ReplaceFilter {
                clause: ClausePath::new(long0(), 0),
                with: Box::new(Filter::Compare {
                    lhs: Operand::Bar {
                        field: crate::ast::BarField::Close,
                    },
                    op: Compare::Gt,
                    rhs: Operand::Feature {
                        family: "donchian_upper".into(),
                        args: vec![Arg::Bars(20)],
                    },
                }),
            },
        )
        .expect("applies");
        assert_eq!(applied.kind(), MoveKind::SwapSignal);
    }

    /// **An edit that changes nothing is not a move.**
    ///
    /// It also proves the form round trip is clean: if projecting into the form
    /// and rendering back moved anything of its own, this would come back as a
    /// change rather than as nothing at all.
    #[test]
    fn an_edit_that_changes_nothing_is_refused_rather_than_counted() {
        let base = def(BASE);
        let d = apply_move(
            &base,
            &Edit::Threshold {
                clause: ClausePath::new(long0(), 1),
                to: "30".into(),
            },
        )
        .expect_err("refused");
        assert_eq!(d.code, Code::NothingToMove);

        let same = apply_move(&base, &Edit::Timeframe { to: "1h".into() }).expect_err("refused");
        assert_eq!(same.code, Code::NothingToMove);
    }

    /// A clause comparing two features has nothing to tune, and saying so is
    /// the answer — inventing a side to move would be this module deciding what
    /// the author meant.
    #[test]
    fn a_clause_with_no_number_has_no_threshold_to_move() {
        let base = def(BASE);
        let d = apply_move(
            &base,
            &Edit::Threshold {
                clause: ClausePath::new(long0(), 0),
                to: "1".into(),
            },
        )
        .expect_err("refused");
        assert_eq!(d.code, Code::NothingToMove);
        assert!(d.message.contains("compares two values"), "{}", d.message);
        assert!(d.fix.is_some());
    }

    /// Removing the last conjunct is refused by name: an entry that fires on
    /// every bar is a different strategy, not a narrower one.
    #[test]
    fn removing_the_only_clause_is_refused_rather_than_emptying_the_entry() {
        let one_clause = def("\
strategy one on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit time 48 bars
");
        let d = apply_move(
            &one_clause,
            &Edit::RemoveFilter {
                clause: ClausePath::new(long0(), 0),
            },
        )
        .expect_err("refused");
        assert_eq!(d.code, Code::MoveEmptiesClause);
    }

    /// And the same for exits: a 1.3 document with no exits does not compile,
    /// and `exit none` is the clause that says the risk layer is the whole
    /// policy.
    #[test]
    fn removing_the_only_exit_names_the_clause_that_says_it_out_loud() {
        let one_exit = def("\
strategy one on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit time 48 bars
");
        let d = apply_move(&one_exit, &Edit::RemoveExit { index: 0 }).expect_err("refused");
        assert_eq!(d.code, Code::MoveEmptiesClause);
        assert!(
            d.fix.is_some_and(|f| f.contains("exit none")),
            "the refusal must name the clause that does work"
        );
    }

    /// An address that is not there is refused where it was written, naming
    /// where addresses come from.
    #[test]
    fn an_address_this_strategy_does_not_have_is_named() {
        let base = def(BASE);
        for edit in [
            Edit::Threshold {
                clause: ClausePath::new(long0(), 9),
                to: "1".into(),
            },
            Edit::AddFilter {
                entry: EntryPath::new(Side::Short, 0),
                filter: Box::new(rsi_under("70")),
            },
            Edit::RemoveExit { index: 9 },
        ] {
            let d = apply_move(&base, &edit).expect_err("refused");
            assert_eq!(d.code, Code::NoSuchClause, "{edit:?}");
            assert!(d.fix.is_some(), "{edit:?}");
        }
    }

    /// The version is renamed here rather than by an `Edit`, because a rename
    /// is not a move — and `create_strategy` upserts by slug, so an iteration
    /// that keeps the name overwrites the version the evidence cites.
    #[test]
    fn a_new_version_takes_a_new_slug_without_that_counting_as_a_move() {
        let base = def(BASE);
        let applied = apply_move(
            &base,
            &Edit::Threshold {
                clause: ClausePath::new(long0(), 1),
                to: "45".into(),
            },
        )
        .expect("applies");
        let v2 = applied.into_version("joined_v2");
        assert_eq!(v2.strategy_id, "joined_v2");

        let d = crate::diff(&base, &v2).expect("diffs");
        assert!(d.is_single_change(), "{:#?}", d.changes());
        assert_eq!(d.renamed(), Some(("joined", "joined_v2")));
    }

    /// The join S-5.10 asks for, end to end: a suggestion names a clause, the
    /// edit is made at that address, and the differ reports the move at the
    /// address the suggestion used.
    #[test]
    fn an_edit_at_a_suggested_address_diffs_at_that_address() {
        let base = def(BASE);
        let clauses = crate::entry_clauses(&base).expect("clauses");
        let path = &clauses[0].clauses[1].path;
        let applied = apply_move(
            &base,
            &Edit::Threshold {
                clause: ClausePath::parse(path).expect("a clause address"),
                to: "45".into(),
            },
        )
        .expect("applies");
        let change = &applied.diff().changes()[0];
        assert_eq!(change.path, format!("{}.when", clauses[0].path));
        assert_eq!(change.before.as_deref(), Some("rsi(close, 14) < 30"));
        assert_eq!(change.after.as_deref(), Some("rsi(close, 14) < 45"));
    }
}
