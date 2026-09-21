//! Moves, diffs and lineage: what changed between two strategies, said in the
//! language they were written in (S-5.10; AGENT-011 §4 `diff_strategies`, P5;
//! FEAT-004 §6).
//!
//! # Why the diff is over text and not over JSON
//!
//! A JSON patch is a true statement nobody can argue with. `"/nodes/0/expr"`
//! changed from one parenthesised arithmetic encoding of a conjunction to
//! another is not a claim about a strategy; it is a claim about a serialisation,
//! and the reader has to decode it before they can disagree with it. "the
//! `entry long` condition gained `rsi(close, 14) < 60`" is a claim about a
//! strategy, and a reviewer — or the model on its next turn — can say *that was
//! the wrong filter*.
//!
//! So both definitions are run through [`crate::decompile()`] first and the diff
//! is computed over the resulting clauses. Three things fall out of that choice
//! rather than having to be built:
//!
//! - **Canonicalisation.** Generated node identifiers, clause order inside the
//!   stored form, and the arithmetic boolean encoding are all gone by the time
//!   anything is compared, so two definitions that differ only in how they were
//!   assembled produce no moves at all.
//! - **`input` hoisting cannot masquerade as a move.** The decompiler binds a
//!   feature read twice into an `input`, so adding one filter can change how a
//!   *different* clause is written. Every condition is inlined back to full
//!   feature calls before comparison —
//!   `hoisting_an_input_is_not_a_move` is the test that would fail if it were
//!   not.
//! - **Refusal comes for free.** A definition SLv2 has no text for is refused by
//!   the decompiler, naming what it was, rather than diffed into words that
//!   would be wrong.
//!
//! # The vocabulary is FEAT-004 §6's, and some of it a differ cannot produce
//!
//! [`MoveKind`] is FEAT-004 §6's list (which itself extends FEAT-003 §8.3's
//! eight), plus AGENT-011 §4's `change_threshold` and its `rewrite` fallback.
//! Seven of the kinds **cannot be recovered from two definitions**, each for a
//! concrete reason — an ablation decides it, the registry does not label it, the
//! canonical schema cannot store it. Rather than emit a plausible-looking
//! approximation, [`MoveKind::not_classifiable_because`] names the reason, and
//! `every_classifiable_kind_is_reachable_and_the_rest_name_a_reason` holds the
//! code to it in both directions: a kind that says it is classifiable and is
//! never produced fails the build, and so does one that says it cannot be.
//!
//! # What this module deliberately does not do
//!
//! It does not apply a move. `apply_move` (AGENT-011 §4) produces a new version
//! and must validate it; a diff reads two versions that already exist. Keeping
//! them apart is what lets [`crate::suggest`] hand a model a *proposal* with no
//! way to have already acted on it.

use std::collections::{BTreeMap, HashMap};

use domain::strategy_def::StrategyDefinition;

use crate::ast::{BinOp, Expr, ExitKind, Side, SizeKind, Strategy};
use crate::catalogue::Catalogue;
use crate::decompile::{decompile_with, expr_to_text};
use crate::diagnostic::{Code, Diagnostic, Span};

/// A diff has no source text of its own to point at.
const NO_SOURCE: Span = Span::new(0, 0, 0, 0);

// ── the vocabulary ───────────────────────────────────────────────────────────

/// One typed structural edit.
///
/// FEAT-004 §6's eleven kinds, the four rows of FEAT-003 §8.3 that §6 extends
/// rather than replaces, and AGENT-011 §4's two. A kind that is not here is a
/// spec change, not a string somebody passed through — which is the same
/// argument `research::propose::Move` makes for itself, and the reason this
/// enum's names are that enum's names wherever the two vocabularies overlap
/// (see [`MoveKind::research_kind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MoveKind {
    // ── FEAT-004 §6, in its order ────────────────────────────────────────────
    /// A conjunct was added to an entry condition.
    AddFilter,
    /// A conjunct was dropped from an entry condition.
    RemoveFilter,
    /// A filter that gates on a market regime.
    AddRegimeGate,
    /// An entry condition was replaced rather than edited.
    SwapSignal,
    /// A new exit rule.
    AddExit,
    /// The unit a distance is measured in changed.
    ChangeExitUnit,
    /// The size mode or the size changed.
    ChangeSizing,
    /// A re-entry guard on a state predicate.
    AddHysteresis,
    /// The bar the strategy decides on changed.
    ChangeTimeframe,
    /// One rule replaced by an ensemble over several lookbacks.
    LookbackEnsemble,
    /// Remove whatever the ablation says costs least.
    Simplify,

    // ── FEAT-003 §8.3 rows §6 extends rather than replaces ───────────────────
    /// A condition reads a different feature than it did.
    SwapFeature,
    /// A node was dropped and the graph rewired.
    RemoveNode,
    /// The exit logic was switched for different exit logic.
    ChangeExit,
    /// Crossover: entries from one parent, the rest from another.
    Combine,

    // ── AGENT-011 §4 ─────────────────────────────────────────────────────────
    /// One number moved and nothing else did.
    ChangeThreshold,
    /// Something changed that the vocabulary has no kind for. AGENT-011 §4's
    /// stated fallback, and it still says *what* in words.
    Rewrite,
}

impl MoveKind {
    /// Every kind, in the order declared above.
    pub const ALL: [Self; 17] = [
        Self::AddFilter,
        Self::RemoveFilter,
        Self::AddRegimeGate,
        Self::SwapSignal,
        Self::AddExit,
        Self::ChangeExitUnit,
        Self::ChangeSizing,
        Self::AddHysteresis,
        Self::ChangeTimeframe,
        Self::LookbackEnsemble,
        Self::Simplify,
        Self::SwapFeature,
        Self::RemoveNode,
        Self::ChangeExit,
        Self::Combine,
        Self::ChangeThreshold,
        Self::Rewrite,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AddFilter => "add_filter",
            Self::RemoveFilter => "remove_filter",
            Self::AddRegimeGate => "add_regime_gate",
            Self::SwapSignal => "swap_signal",
            Self::AddExit => "add_exit",
            Self::ChangeExitUnit => "change_exit_unit",
            Self::ChangeSizing => "change_sizing",
            Self::AddHysteresis => "add_hysteresis",
            Self::ChangeTimeframe => "change_timeframe",
            Self::LookbackEnsemble => "lookback_ensemble",
            Self::Simplify => "simplify",
            Self::SwapFeature => "swap_feature",
            Self::RemoveNode => "remove_node",
            Self::ChangeExit => "change_exit",
            Self::Combine => "combine",
            Self::ChangeThreshold => "change_threshold",
            Self::Rewrite => "rewrite",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Why [`diff()`] can never produce this kind, when it cannot.
    ///
    /// `None` means the differ classifies it, and
    /// `every_classifiable_kind_is_reachable` proves each such kind really is
    /// produced by some pair of strategies rather than merely declared.
    ///
    /// These are not "not yet". Each names a mechanism that is absent, and each
    /// stops being true only when that mechanism arrives — at which point the
    /// test asks for the classification rather than for a wider excuse.
    #[must_use]
    pub const fn not_classifiable_because(self) -> Option<&'static str> {
        match self {
            Self::AddRegimeGate => Some(
                "`features::registry` does not say which families measure a regime, so an \
                 added `variance_ratio(close, 64, 4) < 1` and an added `rsi(close, 14) < 60` \
                 are the same shape to a differ. It is reported as `add_filter`; a caller who \
                 knows the regime says so through `suggest::RegimeOutcome`",
            ),
            Self::ChangeExitUnit => Some(
                "`domain::strategy_def::exits::ExitRule` stores a stop, a target and a \
                 trailing stop as a bare fraction, so every stored distance is already in \
                 per-cent and there is no second unit to change to (Q-S5-01)",
            ),
            Self::AddHysteresis => Some(
                "hysteresis is written with the state predicates (`bars_in_trade`, \
                 `last_exit_reason`), which the compiler refuses by name \
                 (`needs_position_state`) — so no compiling definition can contain one to be \
                 diffed",
            ),
            Self::LookbackEnsemble => Some(
                "no `lookback_ensemble` family exists in the engine registry (DATA-006 F2, \
                 unbuilt), so no definition can read one",
            ),
            Self::Simplify => Some(
                "FEAT-003 §8.3 decides which node `simplify` removes by ablation Runs, and an \
                 ablation is a trial. A removal in the finished text is indistinguishable \
                 from `remove_filter`, so calling it `simplify` would claim the ablation was \
                 run",
            ),
            Self::RemoveNode => Some(
                "SLv2 has clauses, not nodes: every node a definition can lose is a \
                 conjunct or an exit rule, reported as `remove_filter` or `change_exit`. The \
                 node identifiers are generated by the compiler from clause order, so a node \
                 id is not stable across an edit and naming one would be misleading",
            ),
            Self::Combine => Some(
                "a crossover is a claim about two parents. Two definitions do not carry \
                 their ancestry, so a differ reading only `a` and `b` cannot tell a combine \
                 from an ordinary edit; `create_strategy` records the lineage instead",
            ),
            _ => None,
        }
    }

    /// Whether [`diff()`] classifies this kind.
    #[must_use]
    pub const fn classifiable_by_diff(self) -> bool {
        self.not_classifiable_because().is_none()
    }

    /// The `research::propose::Move` variant this kind corresponds to, by name.
    ///
    /// `research` speaks FEAT-003 §8.3's eight-row vocabulary over *node ids*;
    /// this crate speaks FEAT-004 §6's extension of it over *clauses*. Where a
    /// row survives the extension the name is carried over unchanged, so a
    /// caller that holds both can join them without a table of its own. Where a
    /// kind is new in §6 there is no §8.3 row and this returns `None`.
    ///
    /// The mapping is one-directional on purpose: `research` cannot depend on
    /// this crate today (see the module docs of [`crate::suggest`]), and a
    /// two-way table maintained in both places is the second-implementation
    /// shape this crate has already removed once.
    #[must_use]
    pub const fn research_kind(self) -> Option<&'static str> {
        match self {
            Self::AddFilter | Self::AddRegimeGate => Some("add_condition_filter"),
            Self::RemoveFilter | Self::RemoveNode => Some("remove_node"),
            Self::SwapFeature => Some("swap_feature"),
            Self::ChangeExit | Self::AddExit | Self::ChangeExitUnit => Some("change_exit"),
            Self::ChangeSizing => Some("change_sizing"),
            Self::Simplify => Some("simplify"),
            Self::Combine => Some("combine"),
            Self::SwapSignal
            | Self::AddHysteresis
            | Self::ChangeTimeframe
            | Self::LookbackEnsemble
            | Self::ChangeThreshold
            | Self::Rewrite => None,
        }
    }
}

impl std::fmt::Display for MoveKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── one change ───────────────────────────────────────────────────────────────

/// One classified difference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: MoveKind,
    /// Where, in SLv2 terms: `entry.long[0].when`, `exit[1]`, `size`,
    /// `timeframe`. Never a JSON pointer — a reader who has the text in front
    /// of them can find the clause this names.
    pub path: String,
    /// The clause as it was, when there was one.
    pub before: Option<String>,
    /// The clause as it is, when there is one.
    pub after: Option<String>,
    /// One sentence, in the language the strategy was written in.
    pub words: String,
}

/// Everything that changed between two definitions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    changes: Vec<Change>,
    renamed: Option<(String, String)>,
}

impl Diff {
    #[must_use]
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// AGENT-011 §4's `n_moves`.
    #[must_use]
    pub fn n_moves(&self) -> usize {
        self.changes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// AGENT-011 §4's `is_single_change`, and P5's "one change at a time"
    /// becoming checkable rather than exhorted.
    ///
    /// A rename is not counted: `strategy_id` is the version's name, not its
    /// structure, and a lineage that renamed as it went would otherwise report
    /// every step as two moves.
    #[must_use]
    pub fn is_single_change(&self) -> bool {
        self.changes.len() == 1
    }

    /// The slug this version was given, when it differs.
    #[must_use]
    pub fn renamed(&self) -> Option<(&str, &str)> {
        self.renamed
            .as_ref()
            .map(|(a, b)| (a.as_str(), b.as_str()))
    }

    /// The whole diff as lines a person reads.
    #[must_use]
    pub fn render(&self) -> String {
        if self.changes.is_empty() {
            return "no structural change".to_string();
        }
        self.changes
            .iter()
            .map(|c| format!("{}: {}", c.kind, c.words))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── the entry points ─────────────────────────────────────────────────────────

/// What changed between two definitions, against the engine's registry.
///
/// # Errors
///
/// Whatever either definition has that SLv2 has no text for, named by
/// [`crate::decompile()`].
pub fn diff(a: &StrategyDefinition, b: &StrategyDefinition) -> Result<Diff, Diagnostic> {
    diff_with(a, b, &Catalogue::engine())
}

/// As [`diff()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`diff()`].
pub fn diff_with(
    a: &StrategyDefinition,
    b: &StrategyDefinition,
    cat: &Catalogue,
) -> Result<Diff, Diagnostic> {
    let sa = Shape::read(a, cat)?;
    let sb = Shape::read(b, cat)?;
    Ok(sa.diff(&sb, cat))
}

/// One version in a lineage: what it was called, and what it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub label: String,
    pub definition: StrategyDefinition,
}

/// One step of a lineage: a version, and what it changed from the one before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub from: String,
    pub to: String,
    pub diff: Diff,
}

/// What was tried, in what order, and what each step changed.
///
/// This is the half of S-5.10 that neither a diff nor a move is on its own: a
/// reader looking at the twelfth version of a strategy wants the eleven
/// decisions that produced it, in order, in words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineage {
    steps: Vec<Step>,
}

impl Lineage {
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The steps that changed more than one thing — P5's "a version with more
    /// than one move is flagged".
    #[must_use]
    pub fn multi_change_steps(&self) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|s| s.diff.n_moves() > 1)
            .collect()
    }

    /// The lineage as lines a person reads.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            out.push_str(&format!("{} → {}\n", step.from, step.to));
            for line in step.diff.render().lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        out
    }
}

/// The diff between each consecutive pair of versions, in the order given.
///
/// # Errors
///
/// As [`diff()`], for the first version SLv2 has no text for.
pub fn lineage(versions: &[Version]) -> Result<Lineage, Diagnostic> {
    lineage_with(versions, &Catalogue::engine())
}

/// As [`lineage()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`diff()`].
pub fn lineage_with(versions: &[Version], cat: &Catalogue) -> Result<Lineage, Diagnostic> {
    let mut steps = Vec::new();
    for pair in versions.windows(2) {
        let [a, b] = pair else { continue };
        steps.push(Step {
            from: a.label.clone(),
            to: b.label.clone(),
            diff: diff_with(&a.definition, &b.definition, cat)?,
        });
    }
    Ok(Lineage { steps })
}

// ── the normalised view both sides are compared in ───────────────────────────

pub(crate) struct ShapeEntry {
    pub(crate) side: Side,
    /// Every top-level conjunct, with `input` names already inlined.
    pub(crate) conjuncts: Vec<(String, Expr)>,
}

struct ShapeExit {
    text: String,
    tag: &'static str,
    value: Option<String>,
    unit: Option<&'static str>,
    anchor: Option<&'static str>,
    when: Option<(String, Expr)>,
}

struct Shape {
    strategy_id: String,
    instrument: String,
    timeframe: String,
    version: Option<String>,
    asset_class: Option<String>,
    trust: Option<String>,
    params: BTreeMap<String, String>,
    constraints: Vec<String>,
    entries: Vec<ShapeEntry>,
    size: Option<String>,
    exits: Vec<ShapeExit>,
    risk: BTreeMap<String, String>,
}

/// Read a stored definition as the clauses its author would read.
pub(crate) fn shape_entries(
    def: &StrategyDefinition,
    cat: &Catalogue,
) -> Result<Vec<ShapeEntry>, Diagnostic> {
    Ok(Shape::read(def, cat)?.entries)
}

impl Shape {
    fn read(def: &StrategyDefinition, cat: &Catalogue) -> Result<Self, Diagnostic> {
        let text = decompile_with(def, cat)?;
        let ast = crate::parse::parse(&text).map_err(|ds| {
            // The decompiler is checked against the parser by the round trip,
            // so reaching this is a defect in this crate rather than in the
            // caller's definition. Say which, rather than reporting it as
            // something the caller wrote.
            let first = ds
                .first()
                .map_or_else(|| "no diagnostic".to_string(), |d| d.message.clone());
            Diagnostic::semantic(
                Code::BadStoredExpression,
                NO_SOURCE,
                format!("the decompiler wrote text the parser refuses: {first}"),
            )
            .with_fix(
                "this is a defect in `strategy-lang`, not in the definition; \
                 `tests/round_trip.rs` is where it should have been caught",
            )
            .with_see("guide#json")
        })?;
        Ok(Self::of(&ast, &text, cat))
    }

    fn of(ast: &Strategy, source: &str, cat: &Catalogue) -> Self {
        let bindings: HashMap<String, Expr> = ast
            .inputs
            .iter()
            .map(|b| (b.name.clone(), b.expr.clone()))
            .collect();

        let entries = ast
            .entries
            .iter()
            .map(|e| {
                let when = inline(&e.when, &bindings);
                ShapeEntry {
                    side: e.side,
                    conjuncts: split_conjuncts(&when)
                        .into_iter()
                        .map(|c| (expr_to_text(c, cat), c.clone()))
                        .collect(),
                }
            })
            .collect();

        let exits = ast
            .exits
            .iter()
            .map(|x| {
                let text = slice(source, x.span);
                let (tag, value, unit, anchor, when) = match &x.kind {
                    ExitKind::None => ("none", None, None, None, None),
                    ExitKind::Stop(d) => (
                        "stop",
                        Some(d.value.raw.clone()),
                        Some(d.unit.as_str()),
                        Some(d.anchor.as_str()),
                        None,
                    ),
                    ExitKind::Target(d) => (
                        "target",
                        Some(d.value.raw.clone()),
                        Some(d.unit.as_str()),
                        Some(d.anchor.as_str()),
                        None,
                    ),
                    ExitKind::Trailing(d) => (
                        "trailing",
                        Some(d.value.raw.clone()),
                        Some(d.unit.as_str()),
                        Some(d.anchor.as_str()),
                        None,
                    ),
                    ExitKind::Time(n) => ("time", Some(n.raw.clone()), None, None, None),
                    ExitKind::When(e) => {
                        let e = inline(e, &bindings);
                        ("when", None, None, None, Some((expr_to_text(&e, cat), e)))
                    }
                };
                ShapeExit {
                    text,
                    tag,
                    value,
                    unit,
                    anchor,
                    when,
                }
            })
            .collect();

        Self {
            strategy_id: ast.name.clone(),
            instrument: ast.instrument.clone(),
            timeframe: ast.timeframe.clone(),
            version: ast.version.as_ref().map(|(v, _)| v.clone()),
            asset_class: ast.asset_class.as_ref().map(|(v, _)| v.clone()),
            trust: ast.trust.as_ref().map(|(v, _)| v.clone()),
            params: ast
                .params
                .iter()
                .map(|p| (p.name.clone(), slice(source, p.span)))
                .collect(),
            constraints: ast.constraints.iter().map(|c| c.text.clone()).collect(),
            entries,
            size: ast.size.as_ref().map(|s| match &s.kind {
                SizeKind::Fixed(n) => format!("fixed {}", n.raw),
                SizeKind::EquityFraction(n) => format!("equity_fraction {}", n.raw),
                SizeKind::RiskPerTrade(n) => format!("risk_per_trade {}", n.raw),
                SizeKind::Other { word, rest } => match rest {
                    Some(n) => format!("{word} {}", n.raw),
                    None => word.clone(),
                },
            }),
            exits,
            risk: ast
                .risk
                .iter()
                .map(|r| (r.kind.as_str().to_string(), r.value.raw.clone()))
                .collect(),
        }
    }

    fn diff(&self, other: &Self, cat: &Catalogue) -> Diff {
        let mut out = Vec::new();

        if self.timeframe != other.timeframe {
            out.push(Change {
                kind: MoveKind::ChangeTimeframe,
                path: "timeframe".into(),
                before: Some(self.timeframe.clone()),
                after: Some(other.timeframe.clone()),
                words: format!(
                    "the strategy decides on `{}` bars where it decided on `{}` bars",
                    other.timeframe, self.timeframe
                ),
            });
        }

        self.diff_entries(other, cat, &mut out);
        self.diff_exits(other, cat, &mut out);

        if self.size != other.size {
            out.push(Change {
                kind: MoveKind::ChangeSizing,
                path: "size".into(),
                before: self.size.clone(),
                after: other.size.clone(),
                words: match (&self.size, &other.size) {
                    (Some(a), Some(b)) => format!("sizing became `size {b}` where it was `size {a}`"),
                    (None, Some(b)) => format!("the strategy gained `size {b}`"),
                    (Some(a), None) => format!("the `size {a}` clause was removed"),
                    (None, None) => unreachable!("equal sizes do not reach here"),
                },
            });
        }

        self.diff_headers(other, &mut out);
        self.diff_params(other, &mut out);
        self.diff_risk(other, &mut out);

        Diff {
            changes: out,
            renamed: (self.strategy_id != other.strategy_id)
                .then(|| (self.strategy_id.clone(), other.strategy_id.clone())),
        }
    }

    fn diff_entries(&self, other: &Self, cat: &Catalogue, out: &mut Vec<Change>) {
        for side in [Side::Long, Side::Short] {
            let a: Vec<&ShapeEntry> = self.entries.iter().filter(|e| e.side == side).collect();
            let b: Vec<&ShapeEntry> = other.entries.iter().filter(|e| e.side == side).collect();
            for (k, (ea, eb)) in a.iter().zip(b.iter()).enumerate() {
                let path = format!("entry.{}[{k}].when", side.as_str());
                diff_condition(&path, side, &ea.conjuncts, &eb.conjuncts, cat, out);
            }
            for (k, extra) in b.iter().enumerate().skip(a.len()) {
                out.push(Change {
                    kind: MoveKind::Rewrite,
                    path: format!("entry.{}[{k}]", side.as_str()),
                    before: None,
                    after: Some(condition_text(&extra.conjuncts)),
                    words: format!(
                        "the strategy gained an `entry {}` clause: `{}`",
                        side.as_str(),
                        condition_text(&extra.conjuncts)
                    ),
                });
            }
            for (k, gone) in a.iter().enumerate().skip(b.len()) {
                out.push(Change {
                    kind: MoveKind::Rewrite,
                    path: format!("entry.{}[{k}]", side.as_str()),
                    before: Some(condition_text(&gone.conjuncts)),
                    after: None,
                    words: format!(
                        "the `entry {}` clause `{}` was removed",
                        side.as_str(),
                        condition_text(&gone.conjuncts)
                    ),
                });
            }
        }
    }

    fn diff_exits(&self, other: &Self, cat: &Catalogue, out: &mut Vec<Change>) {
        let mut ra: Vec<&ShapeExit> = Vec::new();
        let mut taken = vec![false; other.exits.len()];
        for x in &self.exits {
            let hit = other
                .exits
                .iter()
                .enumerate()
                .find(|(j, y)| !taken[*j] && y.text == x.text)
                .map(|(j, _)| j);
            match hit {
                Some(j) => taken[j] = true,
                None => ra.push(x),
            }
        }
        let rb: Vec<&ShapeExit> = other
            .exits
            .iter()
            .enumerate()
            .filter(|(j, _)| !taken[*j])
            .map(|(_, y)| y)
            .collect();

        let mut used = vec![false; rb.len()];
        let mut unmatched_a: Vec<&ShapeExit> = Vec::new();
        for x in ra {
            let hit = rb
                .iter()
                .enumerate()
                .find(|(j, y)| !used[*j] && y.tag == x.tag)
                .map(|(j, y)| (j, *y));
            match hit {
                Some((j, y)) => {
                    used[j] = true;
                    diff_exit_pair(x, y, cat, out);
                }
                None => unmatched_a.push(x),
            }
        }
        let unmatched_b: Vec<&ShapeExit> = rb
            .iter()
            .enumerate()
            .filter(|(j, _)| !used[*j])
            .map(|(_, y)| *y)
            .collect();

        // One rule replaced by one rule of a different kind is one move, not a
        // removal and an addition: `exit stop 2 % from entry` becoming
        // `exit trailing 3 % from high_water` is FEAT-004 §6's `change_exit`,
        // and reporting it as two would make `is_single_change` false for a
        // version that changed one thing.
        if unmatched_a.len() == 1 && unmatched_b.len() == 1 {
            let (x, y) = (unmatched_a[0], unmatched_b[0]);
            out.push(Change {
                kind: MoveKind::ChangeExit,
                path: "exit".into(),
                before: Some(x.text.clone()),
                after: Some(y.text.clone()),
                words: format!("the exit `{}` became `{}`", x.text, y.text),
            });
            return;
        }
        for x in unmatched_a {
            out.push(Change {
                kind: MoveKind::ChangeExit,
                path: "exit".into(),
                before: Some(x.text.clone()),
                after: None,
                words: format!("the exit `{}` was removed", x.text),
            });
        }
        for y in unmatched_b {
            out.push(Change {
                kind: MoveKind::AddExit,
                path: "exit".into(),
                before: None,
                after: Some(y.text.clone()),
                words: format!("a new exit: `{}`", y.text),
            });
        }
    }

    fn diff_headers(&self, other: &Self, out: &mut Vec<Change>) {
        for (what, a, b) in [
            ("instrument", Some(&self.instrument), Some(&other.instrument)),
            ("version", self.version.as_ref(), other.version.as_ref()),
            (
                "asset_class",
                self.asset_class.as_ref(),
                other.asset_class.as_ref(),
            ),
            ("trust", self.trust.as_ref(), other.trust.as_ref()),
        ] {
            if a == b {
                continue;
            }
            out.push(Change {
                kind: MoveKind::Rewrite,
                path: what.into(),
                before: a.cloned(),
                after: b.cloned(),
                words: format!(
                    "`{what}` became {} where it was {}",
                    b.map_or_else(|| "unset".into(), |v| format!("`{v}`")),
                    a.map_or_else(|| "unset".into(), |v| format!("`{v}`")),
                ),
            });
        }
        if self.constraints != other.constraints {
            out.push(Change {
                kind: MoveKind::Rewrite,
                path: "constraint".into(),
                before: Some(self.constraints.join("; ")),
                after: Some(other.constraints.join("; ")),
                words: format!(
                    "the parameter constraints became [{}] where they were [{}]",
                    other.constraints.join("; "),
                    self.constraints.join("; ")
                ),
            });
        }
    }

    fn diff_params(&self, other: &Self, out: &mut Vec<Change>) {
        for (name, decl) in &other.params {
            match self.params.get(name) {
                None => out.push(Change {
                    kind: MoveKind::Rewrite,
                    path: format!("param.{name}"),
                    before: None,
                    after: Some(decl.clone()),
                    words: format!("a new swept parameter: `{decl}`"),
                }),
                Some(was) if was != decl => out.push(Change {
                    kind: MoveKind::Rewrite,
                    path: format!("param.{name}"),
                    before: Some(was.clone()),
                    after: Some(decl.clone()),
                    words: format!("the parameter `{name}` is now `{decl}` where it was `{was}`"),
                }),
                Some(_) => {}
            }
        }
        for (name, decl) in &self.params {
            if !other.params.contains_key(name) {
                out.push(Change {
                    kind: MoveKind::Rewrite,
                    path: format!("param.{name}"),
                    before: Some(decl.clone()),
                    after: None,
                    words: format!("the swept parameter `{decl}` was removed"),
                });
            }
        }
    }

    fn diff_risk(&self, other: &Self, out: &mut Vec<Change>) {
        let mut keys: Vec<&String> = self.risk.keys().chain(other.risk.keys()).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let (a, b) = (self.risk.get(key), other.risk.get(key));
            if a == b {
                continue;
            }
            out.push(Change {
                kind: MoveKind::Rewrite,
                path: format!("risk.{key}"),
                before: a.cloned(),
                after: b.cloned(),
                words: format!(
                    "`risk {key}` is {} where it was {}",
                    b.map_or_else(|| "unset".into(), |v| format!("`{v}`")),
                    a.map_or_else(|| "unset".into(), |v| format!("`{v}`")),
                ),
            });
        }
    }
}

// ── conditions ───────────────────────────────────────────────────────────────

fn condition_text(conjuncts: &[(String, Expr)]) -> String {
    conjuncts
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join(" and ")
}

fn diff_condition(
    path: &str,
    side: Side,
    a: &[(String, Expr)],
    b: &[(String, Expr)],
    cat: &Catalogue,
    out: &mut Vec<Change>,
) {
    let mut taken = vec![false; b.len()];
    let mut ra: Vec<&(String, Expr)> = Vec::new();
    for x in a {
        let hit = b
            .iter()
            .enumerate()
            .find(|(j, y)| !taken[*j] && y.0 == x.0)
            .map(|(j, _)| j);
        match hit {
            Some(j) => taken[j] = true,
            None => ra.push(x),
        }
    }
    let rb: Vec<&(String, Expr)> = b
        .iter()
        .enumerate()
        .filter(|(j, _)| !taken[*j])
        .map(|(_, y)| y)
        .collect();
    if ra.is_empty() && rb.is_empty() {
        return;
    }

    let where_ = format!("the `entry {}` condition", side.as_str());

    // A one-clause condition replaced by a one-clause condition is one move,
    // whatever the move is — there is no filter to have been added or removed.
    if a.len() == 1 && b.len() == 1 {
        match pair(&a[0].1, &b[0].1, cat) {
            PairChange::Same => {}
            PairChange::Threshold(x, y) => out.push(Change {
                kind: MoveKind::ChangeThreshold,
                path: path.to_string(),
                before: Some(a[0].0.clone()),
                after: Some(b[0].0.clone()),
                words: format!("in {where_}, `{x}` became `{y}` (`{}`)", b[0].0),
            }),
            PairChange::Feature(x, y) => out.push(Change {
                kind: MoveKind::SwapFeature,
                path: path.to_string(),
                before: Some(a[0].0.clone()),
                after: Some(b[0].0.clone()),
                words: format!("{where_} reads `{y}` where it read `{x}`"),
            }),
            PairChange::Unpairable => out.push(Change {
                kind: MoveKind::SwapSignal,
                path: path.to_string(),
                before: Some(a[0].0.clone()),
                after: Some(b[0].0.clone()),
                words: format!("{where_} is now `{}` where it was `{}`", b[0].0, a[0].0),
            }),
        }
        return;
    }

    let mut used = vec![false; rb.len()];
    let mut left: Vec<&(String, Expr)> = Vec::new();
    for x in ra {
        let found = rb
            .iter()
            .enumerate()
            .find(|(j, y)| !used[*j] && !matches!(pair(&x.1, &y.1, cat), PairChange::Unpairable))
            .map(|(j, y)| (j, *y));
        match found {
            Some((j, y)) => {
                used[j] = true;
                match pair(&x.1, &y.1, cat) {
                    PairChange::Threshold(p, q) => out.push(Change {
                        kind: MoveKind::ChangeThreshold,
                        path: path.to_string(),
                        before: Some(x.0.clone()),
                        after: Some(y.0.clone()),
                        words: format!("in {where_}, `{p}` became `{q}` (`{}`)", y.0),
                    }),
                    PairChange::Feature(p, q) => out.push(Change {
                        kind: MoveKind::SwapFeature,
                        path: path.to_string(),
                        before: Some(x.0.clone()),
                        after: Some(y.0.clone()),
                        words: format!("in {where_}, `{p}` became `{q}` (`{}`)", y.0),
                    }),
                    PairChange::Same | PairChange::Unpairable => {}
                }
            }
            None => left.push(x),
        }
    }
    for x in left {
        out.push(Change {
            kind: MoveKind::RemoveFilter,
            path: path.to_string(),
            before: Some(x.0.clone()),
            after: None,
            words: format!("{where_} lost `{}`", x.0),
        });
    }
    for (j, y) in rb.iter().enumerate() {
        if used[j] {
            continue;
        }
        out.push(Change {
            kind: MoveKind::AddFilter,
            path: path.to_string(),
            before: None,
            after: Some(y.0.clone()),
            words: format!("{where_} gained `and {}`", y.0),
        });
    }
}

fn diff_exit_pair(x: &ShapeExit, y: &ShapeExit, cat: &Catalogue, out: &mut Vec<Change>) {
    if x.unit != y.unit {
        out.push(Change {
            kind: MoveKind::ChangeExitUnit,
            path: format!("exit.{}", x.tag),
            before: Some(x.text.clone()),
            after: Some(y.text.clone()),
            words: format!("the `exit {}` distance is now measured in `{}`", x.tag, {
                y.unit.unwrap_or("?")
            }),
        });
        return;
    }
    if x.anchor != y.anchor {
        out.push(Change {
            kind: MoveKind::ChangeExit,
            path: format!("exit.{}", x.tag),
            before: Some(x.text.clone()),
            after: Some(y.text.clone()),
            words: format!("the `exit {}` is now measured from `{}`", x.tag, {
                y.anchor.unwrap_or("?")
            }),
        });
        return;
    }
    if let (Some((tx, ex)), Some((ty, ey))) = (&x.when, &y.when) {
        match pair(ex, ey, cat) {
            PairChange::Same => {}
            PairChange::Threshold(p, q) => out.push(Change {
                kind: MoveKind::ChangeThreshold,
                path: "exit.when".into(),
                before: Some(x.text.clone()),
                after: Some(y.text.clone()),
                words: format!("in the `exit when` condition, `{p}` became `{q}` (`{ty}`)"),
            }),
            PairChange::Feature(p, q) => out.push(Change {
                kind: MoveKind::SwapFeature,
                path: "exit.when".into(),
                before: Some(x.text.clone()),
                after: Some(y.text.clone()),
                words: format!("in the `exit when` condition, `{p}` became `{q}` (`{ty}`)"),
            }),
            PairChange::Unpairable => out.push(Change {
                kind: MoveKind::SwapSignal,
                path: "exit.when".into(),
                before: Some(x.text.clone()),
                after: Some(y.text.clone()),
                words: format!("the `exit when` condition is now `{ty}` where it was `{tx}`"),
            }),
        }
        return;
    }
    if x.value != y.value {
        out.push(Change {
            kind: MoveKind::ChangeThreshold,
            path: format!("exit.{}", x.tag),
            before: Some(x.text.clone()),
            after: Some(y.text.clone()),
            words: format!(
                "the `exit {}` distance is now `{}` where it was `{}`",
                x.tag,
                y.value.clone().unwrap_or_default(),
                x.value.clone().unwrap_or_default()
            ),
        });
    }
}

// ── expression comparison ────────────────────────────────────────────────────

/// The top-level conjuncts of a condition. An `or` is one conjunct: SLv2's
/// `and` is the only operator a filter can be attached with, and splitting an
/// `or` would report half a disjunction as an independently removable clause.
pub(crate) fn split_conjuncts(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
            ..
        } => {
            let mut v = split_conjuncts(lhs);
            v.extend(split_conjuncts(rhs));
            v
        }
        other => vec![other],
    }
}

/// Replace every reference to a declared `input` with what it was bound to.
pub(crate) fn inline(e: &Expr, bindings: &HashMap<String, Expr>) -> Expr {
    match e {
        Expr::Name { name, .. } => bindings
            .get(name)
            .map_or_else(|| e.clone(), |b| inline(b, bindings)),
        Expr::Unary { op, rhs, span } => Expr::Unary {
            op: *op,
            rhs: Box::new(inline(rhs, bindings)),
            span: *span,
        },
        Expr::Binary { op, lhs, rhs, span } => Expr::Binary {
            op: *op,
            lhs: Box::new(inline(lhs, bindings)),
            rhs: Box::new(inline(rhs, bindings)),
            span: *span,
        },
        Expr::Call {
            name,
            args,
            span,
            name_span,
        } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| inline(a, bindings)).collect(),
            span: *span,
            name_span: *name_span,
        },
        other => other.clone(),
    }
}

enum Leaf {
    Number(String),
    Reads(String),
}

enum PairChange {
    Same,
    Threshold(String, String),
    Feature(String, String),
    Unpairable,
}

/// The one-leaf difference between two clauses, when there is exactly one.
fn pair(a: &Expr, b: &Expr, cat: &Catalogue) -> PairChange {
    let mut deltas = Vec::new();
    if !leaf_deltas(a, b, cat, &mut deltas) {
        return PairChange::Unpairable;
    }
    match deltas.as_slice() {
        [] => PairChange::Same,
        [(Leaf::Number(x), Leaf::Number(y))] => PairChange::Threshold(x.clone(), y.clone()),
        [(Leaf::Reads(x), Leaf::Reads(y))] => PairChange::Feature(x.clone(), y.clone()),
        _ => PairChange::Unpairable,
    }
}

/// Walk both trees together. `false` means the shapes differ, so there is no
/// leaf-level story to tell and the clause was replaced rather than edited.
fn leaf_deltas(a: &Expr, b: &Expr, cat: &Catalogue, out: &mut Vec<(Leaf, Leaf)>) -> bool {
    match (a, b) {
        (Expr::Number(x), Expr::Number(y)) => {
            if x.raw != y.raw {
                out.push((Leaf::Number(x.raw.clone()), Leaf::Number(y.raw.clone())));
            }
            true
        }
        (Expr::Bar { field: x, .. }, Expr::Bar { field: y, .. }) => {
            if x != y {
                out.push((
                    Leaf::Reads(x.as_str().to_string()),
                    Leaf::Reads(y.as_str().to_string()),
                ));
            }
            true
        }
        (Expr::Name { name: x, .. }, Expr::Name { name: y, .. })
        | (Expr::Text { value: x, .. }, Expr::Text { value: y, .. }) => {
            if x != y {
                out.push((Leaf::Reads(x.clone()), Leaf::Reads(y.clone())));
            }
            true
        }
        // A feature call is compared whole. `ema(close, 7)` becoming
        // `ema(close, 21)` is FEAT-003 §8.3's `swap_feature` — the condition
        // reads a different feature — and not a threshold move, because 7 is
        // not a threshold: it is part of the feature's name.
        (Expr::Call { .. }, Expr::Call { .. }) => {
            let (x, y) = (expr_to_text(a, cat), expr_to_text(b, cat));
            if x != y {
                out.push((Leaf::Reads(x), Leaf::Reads(y)));
            }
            true
        }
        (
            Expr::Unary {
                op: ox, rhs: rx, ..
            },
            Expr::Unary {
                op: oy, rhs: ry, ..
            },
        ) => ox == oy && leaf_deltas(rx, ry, cat, out),
        (
            Expr::Binary {
                op: ox,
                lhs: lx,
                rhs: rx,
                ..
            },
            Expr::Binary {
                op: oy,
                lhs: ly,
                rhs: ry,
                ..
            },
        ) => ox == oy && leaf_deltas(lx, ly, cat, out) && leaf_deltas(rx, ry, cat, out),
        _ => false,
    }
}

fn slice(source: &str, span: Span) -> String {
    source
        .get(span.range())
        .map_or_else(String::new, str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const BASE: &str = "\
strategy s on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21)
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

    fn def(src: &str) -> StrategyDefinition {
        let ast = crate::parse(src).unwrap_or_else(|e| panic!("parse {src}: {e:#?}"));
        crate::compile(&ast).unwrap_or_else(|e| panic!("compile {src}: {e:#?}"))
    }

    fn kinds(a: &str, b: &str) -> Vec<MoveKind> {
        diff(&def(a), &def(b))
            .expect("diffs")
            .changes()
            .iter()
            .map(|c| c.kind)
            .collect()
    }

    /// Every pair below is one edit to `BASE`, and the kind it must be
    /// classified as. It is also the reachability corpus: the guard below fails
    /// if any kind the differ claims to classify is not produced here.
    fn corpus() -> Vec<(&'static str, String, MoveKind)> {
        vec![
            (
                "add a filter",
                BASE.replace(
                    "ema(close, 7) > ema(close, 21)",
                    "ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60",
                ),
                MoveKind::AddFilter,
            ),
            (
                "swap the slow baseline",
                BASE.replace("ema(close, 21)", "ema(close, 50)"),
                MoveKind::SwapFeature,
            ),
            (
                "replace the signal outright",
                BASE.replace(
                    "ema(close, 7) > ema(close, 21)",
                    "rsi(close, 14) < 30",
                ),
                MoveKind::SwapSignal,
            ),
            (
                "add an exit",
                BASE.replace(
                    "exit time 48 bars",
                    "exit target 5 % from entry\nexit time 48 bars",
                ),
                MoveKind::AddExit,
            ),
            (
                "stop becomes a trailing stop",
                BASE.replace(
                    "exit stop 2 % from entry",
                    "exit trailing 3 % from high_water",
                ),
                MoveKind::ChangeExit,
            ),
            (
                "move the time exit",
                BASE.replace("exit time 48 bars", "exit time 24 bars"),
                MoveKind::ChangeThreshold,
            ),
            (
                "change the size mode",
                BASE.replace("size fixed 0.01", "size equity_fraction 0.1"),
                MoveKind::ChangeSizing,
            ),
            (
                "change the bar",
                BASE.replace("timeframe 1h", "timeframe 4h"),
                MoveKind::ChangeTimeframe,
            ),
            (
                "tighten the risk envelope",
                format!("{BASE}risk max_position 0.5\n"),
                MoveKind::Rewrite,
            ),
        ]
    }

    #[test]
    fn a_definition_diffed_against_itself_has_no_moves() {
        let d = diff(&def(BASE), &def(BASE)).expect("diffs");
        assert!(d.is_empty(), "{:#?}", d.changes());
        assert_eq!(d.render(), "no structural change");
        assert!(!d.is_single_change());
    }

    #[test]
    fn every_corpus_edit_is_one_move_of_the_kind_it_claims() {
        for (label, text, want) in corpus() {
            let d = diff(&def(BASE), &def(&text)).expect("diffs");
            assert!(
                d.is_single_change(),
                "{label}: expected one move, got {:#?}",
                d.changes()
            );
            assert_eq!(d.changes()[0].kind, want, "{label}: {:#?}", d.changes()[0]);
            assert!(
                !d.changes()[0].words.is_empty(),
                "{label}: a move with no words"
            );
        }
    }

    /// The corpus is also the reachability proof. A kind that says it is
    /// classifiable and is never produced is a kind nothing tests.
    #[test]
    fn every_classifiable_kind_is_reachable_and_the_rest_name_a_reason() {
        let mut seen: BTreeSet<MoveKind> = corpus()
            .into_iter()
            .flat_map(|(_, text, _)| {
                let forward = kinds(BASE, &text);
                let back = kinds(&text, BASE);
                forward.into_iter().chain(back)
            })
            .collect();
        // `remove_filter` is the reverse of `add_filter`, which the reverse
        // direction above already produced; assert it rather than assume it.
        assert!(seen.contains(&MoveKind::RemoveFilter));
        seen.insert(MoveKind::Rewrite);

        assert!(seen.len() >= 9, "the corpus only reached {seen:?}");
        for kind in MoveKind::ALL {
            match kind.not_classifiable_because() {
                None => assert!(
                    seen.contains(&kind),
                    "`{kind}` says a diff classifies it, and nothing in the corpus produces one"
                ),
                Some(why) => {
                    assert!(
                        !seen.contains(&kind),
                        "`{kind}` was produced by a diff while claiming it cannot be: {why}"
                    );
                    assert!(why.len() > 40, "`{kind}`'s reason is not a reason: {why}");
                }
            }
        }
    }

    /// The load-bearing one. `B` reads `rsi_14` twice, so the decompiler hoists
    /// it into an `input` and writes the *first* entry differently from the way
    /// `A` writes it — while nothing about that clause changed. Comparing the
    /// decompiled text as written would report a move nobody made.
    #[test]
    fn hoisting_an_input_is_not_a_move() {
        let a = "\
strategy s on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit time 24 bars
";
        let b = "\
strategy s on $bound_at_init timeframe 1h
input r = rsi(close, 14)
entry long when r < 30
entry short when r > 70
size fixed 0.01
exit time 24 bars
";
        let d = diff(&def(a), &def(b)).expect("diffs");
        assert_eq!(d.n_moves(), 1, "{:#?}", d.changes());
        assert_eq!(d.changes()[0].kind, MoveKind::Rewrite);
        assert!(
            d.changes()[0].words.contains("entry short"),
            "{}",
            d.changes()[0].words
        );
    }

    #[test]
    fn a_move_is_named_in_slv2_and_never_in_json() {
        let with_filter = BASE.replace(
            "ema(close, 7) > ema(close, 21)",
            "ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60",
        );
        let d = diff(&def(BASE), &def(&with_filter)).expect("diffs");
        let c = &d.changes()[0];
        assert_eq!(c.words, "the `entry long` condition gained `and rsi(close, 14) < 60`");
        assert_eq!(c.path, "entry.long[0].when");
        assert_eq!(c.after.as_deref(), Some("rsi(close, 14) < 60"));
        for text in [&c.words, &c.path] {
            assert!(
                !text.contains("feature('") && !text.contains("/nodes/"),
                "a diff leaked the JSON form: {text}"
            );
        }
    }

    #[test]
    fn a_threshold_move_names_the_number_that_moved() {
        let a = BASE.replace(
            "ema(close, 7) > ema(close, 21)",
            "ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60",
        );
        let b = a.replace("< 60", "< 70");
        let d = diff(&def(&a), &def(&b)).expect("diffs");
        assert_eq!(d.n_moves(), 1, "{:#?}", d.changes());
        assert_eq!(d.changes()[0].kind, MoveKind::ChangeThreshold);
        assert!(
            d.changes()[0].words.contains("`60` became `70`"),
            "{}",
            d.changes()[0].words
        );
    }

    #[test]
    fn two_edits_are_not_a_single_change() {
        let b = BASE
            .replace(
                "ema(close, 7) > ema(close, 21)",
                "ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60",
            )
            .replace("exit time 48 bars", "exit time 24 bars");
        let d = diff(&def(BASE), &def(&b)).expect("diffs");
        assert_eq!(d.n_moves(), 2, "{:#?}", d.changes());
        assert!(!d.is_single_change());
    }

    #[test]
    fn a_rename_is_not_a_move() {
        let b = BASE.replace("strategy s ", "strategy s_v2 ");
        let d = diff(&def(BASE), &def(&b)).expect("diffs");
        assert!(d.is_empty(), "{:#?}", d.changes());
        assert_eq!(d.renamed(), Some(("s", "s_v2")));
    }

    #[test]
    fn a_lineage_reads_in_order_and_flags_the_step_that_changed_two_things() {
        let one = BASE.replace(
            "ema(close, 7) > ema(close, 21)",
            "ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60",
        );
        let two = one
            .replace("< 60", "< 70")
            .replace("exit time 48 bars", "exit time 24 bars");
        let l = lineage(&[
            Version {
                label: "v1".into(),
                definition: def(BASE),
            },
            Version {
                label: "v2".into(),
                definition: def(&one),
            },
            Version {
                label: "v3".into(),
                definition: def(&two),
            },
        ])
        .expect("lineage");
        assert_eq!(l.steps().len(), 2);
        assert_eq!(l.steps()[0].from, "v1");
        assert_eq!(l.steps()[1].to, "v3");
        let flagged = l.multi_change_steps();
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].to, "v3");
        let rendered = l.render();
        assert!(rendered.contains("v1 → v2"), "{rendered}");
        assert!(rendered.contains("v2 → v3"), "{rendered}");
        assert!(rendered.contains("gained `and rsi(close, 14) < 60`"), "{rendered}");
    }

    /// The vocabulary is a join with `research::propose::Move`, and a join that
    /// nothing checks drifts. This does not prove the other crate's list — it
    /// cannot see it — but it does prove this side is internally consistent:
    /// every name it claims to map to is snake_case, and no two kinds that mean
    /// different things claim the same §8.3 row by accident.
    #[test]
    fn the_research_mapping_is_snake_case_and_only_maps_kinds_that_share_a_meaning() {
        for kind in MoveKind::ALL {
            let Some(name) = kind.research_kind() else {
                continue;
            };
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "`{kind}` maps to `{name}`, which is not snake_case"
            );
        }
        assert_eq!(
            MoveKind::ChangeThreshold.research_kind(),
            None,
            "FEAT-003 §8.3 has no threshold row; claiming one would let a campaign record a \
             move it never made"
        );
        assert_eq!(MoveKind::AddFilter.research_kind(), Some("add_condition_filter"));
    }

    #[test]
    fn every_kind_has_a_distinct_name_that_parses_back() {
        let mut names: Vec<&str> = MoveKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names.len(), 17);
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two kinds share a name");
        for kind in MoveKind::ALL {
            assert_eq!(MoveKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(MoveKind::parse("not_a_move"), None);
    }

    #[test]
    fn a_definition_slv2_cannot_write_is_refused_rather_than_diffed() {
        let mut broken = def(BASE);
        broken.nodes.push(domain::strategy_def::nodes::Node {
            id: "ai".into(),
            kind: domain::strategy_def::nodes::NodeKind::ModelForecast {
                model_ref: "m".into(),
                alias: "production".into(),
                direction: "bullish".into(),
                min_confidence: 0.6,
                input: None,
            },
        });
        let err = diff(&def(BASE), &broken).expect_err("refused");
        assert_eq!(err.code, Code::NotExpressible);
        assert!(err.message.contains("model_forecast"), "{}", err.message);
    }
}
