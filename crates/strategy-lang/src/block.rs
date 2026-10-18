//! Which clause emptied the strategy (S-5.15; AGENT-011 §4, "It says which
//! clause blocked the entry").
//!
//! # The finding this exists to produce
//!
//! `never_fires_because` tells a model that something is wrong. It does not
//! tell it *what*, so the next edit is a guess and the guess costs a round trip
//! the model pays for out of the same budget as its research. What a model can
//! act on in one step is:
//!
//! > `rsi(close, 14) < 30` was true on 0.4% of bars and was the sole blocker on
//! > 91% of the rest.
//!
//! That turns "it doesn't fire" into "this threshold is the problem", and it is
//! what [`crate::suggest`] reads so that a suggestion cites a measurement rather
//! than a heuristic.
//!
//! # This module is the analysis half; the preview owns the bars
//!
//! `preview_signals` lives in `crates/backtest` and walks the window. It has the
//! bars and the feature engine; it does not have the language. So the split is:
//!
//! - [`entry_clauses()`] takes a stored definition and returns every top-level
//!   conjunct of every entry condition, each with **the SLv2 text a person
//!   reads** and **the v1 expression string the preview already knows how to
//!   evaluate**. Nothing new has to be taught to the evaluator.
//! - The preview evaluates each clause per bar and tallies, per entry, how many
//!   bars it saw, on how many every clause was true, and per clause how many
//!   bars that clause was true and on how many it was the *only* clause that
//!   was false.
//! - [`blocking_profile()`] turns those counts into shares and a [`Verdict`].
//!
//! # Why the counts are checked at the boundary
//!
//! [`EntryCounts::new`] refuses a tally that no window could have produced — a
//! clause true on more bars than were evaluated, sole-blocker counts that
//! overlap, an all-true count larger than the rarest clause's. A profile built
//! from a miscounted preview would read as a confident measurement and point a
//! model at the wrong clause, which is worse than no profile at all. The
//! arithmetic is the constraint; there is no comment asking callers to be
//! careful.

use domain::strategy_def::{nodes::NodeKind, StrategyDefinition};

use crate::ast::{BinOp, Expr, Side};
use crate::catalogue::Catalogue;
use crate::decompile::is_encoded_bool;
use crate::diagnostic::{Code, Diagnostic, Span};

/// A profile has no source text of its own to point at.
const NO_SOURCE: Span = Span::new(0, 0, 0, 0);

/// The share of failing bars at which one clause is called *the* blocker.
///
/// Half: a clause responsible for more than half of the bars on which the
/// conjunction failed is the one to edit first, and no two clauses can both
/// clear the bar, so the verdict names one clause or none.
pub const SOLE_BLOCKER_SHARE: f64 = 0.5;

// ── the shape the preview needs ──────────────────────────────────────────────

/// One top-level conjunct of one entry condition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// `entry.long[0].clause[1]` — the same addressing [`crate::diff`] uses, so
    /// a blocking profile and a diff name the same clause the same way.
    pub path: String,
    pub side: Side,
    /// The index of the entry within its side.
    pub entry: usize,
    /// The index of the clause within the conjunction.
    pub index: usize,
    /// SLv2, with `input` names inlined: what the author reads.
    pub text: String,
    /// The frozen v1 expression the simulator's evaluator already accepts. This
    /// is the field that makes the preview's job arithmetic rather than a
    /// second compiler.
    pub expr: String,
}

/// One entry condition, decomposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryClauses {
    /// `entry.long[0]`.
    pub path: String,
    pub side: Side,
    /// The condition node the clauses came from, for a caller that wants to
    /// reach back into the stored form.
    pub node: String,
    pub clauses: Vec<Clause>,
}

/// Every entry condition of a stored definition, clause by clause.
///
/// # Errors
///
/// Whatever the definition has that SLv2 has no text for, named by
/// [`crate::decompile()`].
pub fn entry_clauses(def: &StrategyDefinition) -> Result<Vec<EntryClauses>, Diagnostic> {
    entry_clauses_with(def, &Catalogue::engine())
}

/// As [`entry_clauses()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`entry_clauses()`].
pub fn entry_clauses_with(
    def: &StrategyDefinition,
    cat: &Catalogue,
) -> Result<Vec<EntryClauses>, Diagnostic> {
    // The readable half: the decompiler's own view, which has already inlined
    // the inputs and canonicalised the boolean encoding.
    let readable = crate::diff::shape_entries(def, cat)?;

    // The evaluable half: the stored expression, split on the same encoding.
    let conditions: Vec<(&str, &str)> = def
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Condition { expr } => Some((n.id.as_str(), expr.as_str())),
            _ => None,
        })
        .collect();

    let mut out = Vec::new();
    let mut counters = (0usize, 0usize);
    let mut readable = readable.into_iter();
    for node in &def.nodes {
        let NodeKind::Signal { when, emit } = &node.kind else {
            continue;
        };
        let side = match emit.as_str() {
            "long" => Side::Long,
            "short" => Side::Short,
            _ => continue,
        };
        let Some(shape) = readable.next() else {
            return Err(defect("more entry signals than the decompiler read back"));
        };
        if shape.side != side {
            return Err(defect(
                "the decompiler's entries and the definition's entry signals are in different \
                 orders",
            ));
        }
        let Some((_, source)) = conditions.iter().find(|(id, _)| *id == when.as_str()) else {
            return Err(defect(
                "an entry signal watches a node that is not a condition",
            ));
        };
        let parsed = crate::parse::expr_from_str(source).map_err(|d| {
            Diagnostic::semantic(
                Code::BadStoredExpression,
                NO_SOURCE,
                format!("node `{when}` does not parse: {}", d.message),
            )
            .with_fix("the stored expression must be in the frozen v1 grammar")
            .with_see("guide#json")
        })?;
        let encoded = split_encoded(&parsed);
        if encoded.len() != shape.conjuncts.len() {
            return Err(defect(
                "the stored condition and its readable form have different clause counts",
            ));
        }

        let k = match side {
            Side::Long => {
                counters.0 += 1;
                counters.0 - 1
            }
            Side::Short => {
                counters.1 += 1;
                counters.1 - 1
            }
        };
        let entry_path = format!("entry.{}[{k}]", side.as_str());
        let clauses = shape
            .conjuncts
            .iter()
            .zip(encoded.iter())
            .enumerate()
            .map(|(i, ((text, _), e))| Clause {
                path: format!("{entry_path}.clause[{i}]"),
                side,
                entry: k,
                index: i,
                text: text.clone(),
                expr: source
                    .get(e.span().range())
                    .map_or_else(|| source.to_string(), str::to_string),
            })
            .collect();
        out.push(EntryClauses {
            path: entry_path,
            side,
            node: when.clone(),
            clauses,
        });
    }
    Ok(out)
}

// ── what the preview hands back ──────────────────────────────────────────────

/// What one clause did over the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClauseCounts {
    /// The [`Clause::path`] this tally is for.
    pub path: String,
    /// Bars on which this clause evaluated true.
    pub bars_true: u64,
    /// Bars on which this clause was false **and every other clause was true**
    /// — the bars this clause, alone, cost the strategy.
    pub bars_sole_blocker: u64,
}

/// What one entry condition did over the window.
///
/// Built through [`EntryCounts::new`], which refuses a tally the arithmetic
/// says no window could have produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryCounts {
    path: String,
    bars_evaluated: u64,
    bars_all_true: u64,
    clauses: Vec<ClauseCounts>,
}

impl EntryCounts {
    /// # Errors
    ///
    /// [`Code::ImpossibleTally`], naming the count that cannot be right. Each
    /// check is an identity over a window, not a policy:
    ///
    /// - a clause cannot be true on more bars than were evaluated;
    /// - the conjunction cannot be true more often than its rarest clause;
    /// - a clause can only be the sole blocker on a bar it was false on;
    /// - two clauses cannot be the sole blocker on the same bar, and neither
    ///   can be one on a bar the conjunction was true.
    pub fn new(
        path: impl Into<String>,
        bars_evaluated: u64,
        bars_all_true: u64,
        clauses: Vec<ClauseCounts>,
    ) -> Result<Self, Diagnostic> {
        let path = path.into();
        if clauses.is_empty() {
            return Err(impossible(&path, "a condition with no clauses was tallied"));
        }
        if bars_all_true > bars_evaluated {
            return Err(impossible(
                &path,
                format!(
                    "the condition was true on {bars_all_true} of {bars_evaluated} bars \
                     evaluated"
                ),
            ));
        }
        for c in &clauses {
            if c.bars_true > bars_evaluated {
                return Err(impossible(
                    &path,
                    format!(
                        "`{}` was true on {} of {bars_evaluated} bars evaluated",
                        c.path, c.bars_true
                    ),
                ));
            }
            if bars_all_true > c.bars_true {
                return Err(impossible(
                    &path,
                    format!(
                        "the conjunction was true on {bars_all_true} bars and `{}` on only {} \
                         — a conjunction cannot outrun its rarest clause",
                        c.path, c.bars_true
                    ),
                ));
            }
            if c.bars_sole_blocker > bars_evaluated - c.bars_true {
                return Err(impossible(
                    &path,
                    format!(
                        "`{}` was the sole blocker on {} bars and false on only {}",
                        c.path,
                        c.bars_sole_blocker,
                        bars_evaluated - c.bars_true
                    ),
                ));
            }
        }
        let blocked: u64 = clauses.iter().map(|c| c.bars_sole_blocker).sum();
        if blocked > bars_evaluated - bars_all_true {
            return Err(impossible(
                &path,
                format!(
                    "the clauses were the sole blocker on {blocked} bars between them, and the \
                     condition failed on only {}",
                    bars_evaluated - bars_all_true
                ),
            ));
        }
        Ok(Self {
            path,
            bars_evaluated,
            bars_all_true,
            clauses,
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn bars_evaluated(&self) -> u64 {
        self.bars_evaluated
    }

    #[must_use]
    pub const fn bars_all_true(&self) -> u64 {
        self.bars_all_true
    }

    #[must_use]
    pub fn clauses(&self) -> &[ClauseCounts] {
        &self.clauses
    }
}

// ── the answer ───────────────────────────────────────────────────────────────

/// What one clause did, as shares.
#[derive(Clone, Debug, PartialEq)]
pub struct ClauseProfile {
    pub path: String,
    /// SLv2 — the clause the author wrote.
    pub text: String,
    pub bars_true: u64,
    /// Of every bar evaluated.
    pub true_share: f64,
    /// Of the bars on which the conjunction failed — AGENT-011 §4's "the rest".
    pub sole_blocker_share: f64,
}

/// What the profile concluded about one entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// The window had no bars past warm-up. Nothing was measured, so nothing is
    /// claimed — a zero fire rate over zero bars is not a finding.
    NothingEvaluated,
    /// It fired. The share of bars on which every clause was true.
    Fires { share: f64 },
    /// It never fired, and one clause is responsible for most of the failures.
    Blocked {
        /// Index into [`EntryProfile::clauses`].
        clause: usize,
        sole_blocker_share: f64,
    },
    /// It never fired, and no single clause carries the blame — more than one
    /// is usually false at the same time, so moving any one of them alone will
    /// not make it fire.
    NoSingleBlocker,
}

/// One entry condition's profile.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryProfile {
    pub path: String,
    pub side: Side,
    pub bars_evaluated: u64,
    pub bars_all_true: u64,
    pub fire_share: f64,
    pub clauses: Vec<ClauseProfile>,
    pub verdict: Verdict,
}

impl EntryProfile {
    /// The clause responsible for most of the failures, whatever the verdict.
    ///
    /// A condition that fires on 0.1% of bars still has a binding clause, and a
    /// caller tuning it wants to know which one; the verdict is about whether
    /// the profile is confident enough to call it *the* blocker.
    #[must_use]
    pub fn binding_clause(&self) -> Option<&ClauseProfile> {
        self.clauses
            .iter()
            .filter(|c| c.sole_blocker_share > 0.0)
            .max_by(|a, b| {
                a.sole_blocker_share
                    .partial_cmp(&b.sole_blocker_share)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// The finding, in the sentence AGENT-011 §4 asks for.
    #[must_use]
    pub fn words(&self) -> String {
        match &self.verdict {
            Verdict::NothingEvaluated => format!(
                "`{}` was never evaluated: no bar in the window was past warm-up",
                self.path
            ),
            Verdict::Fires { share } => {
                let extra = self.binding_clause().map_or_else(String::new, |c| {
                    format!(
                        ", and `{}` was the sole blocker on {} of the bars it did not",
                        c.text,
                        pct(c.sole_blocker_share)
                    )
                });
                format!("`{}` fired on {} of bars{extra}", self.path, pct(*share))
            }
            Verdict::Blocked {
                clause,
                sole_blocker_share,
            } => {
                let c = &self.clauses[*clause];
                format!(
                    "`{}` never fired: `{}` was true on {} of bars and was the sole blocker on \
                     {} of the rest",
                    self.path,
                    c.text,
                    pct(c.true_share),
                    pct(*sole_blocker_share)
                )
            }
            Verdict::NoSingleBlocker => format!(
                "`{}` never fired, and no one clause is responsible: more than one was false \
                 together, so moving any single threshold will not make it fire",
                self.path
            ),
        }
    }
}

/// Every entry condition's profile.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockingProfile {
    pub entries: Vec<EntryProfile>,
}

impl BlockingProfile {
    /// The whole profile as lines a person reads.
    #[must_use]
    pub fn render(&self) -> String {
        self.entries
            .iter()
            .map(EntryProfile::words)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether no entry condition fired anywhere in the window.
    #[must_use]
    pub fn never_fires(&self) -> bool {
        !self.entries.is_empty()
            && self
                .entries
                .iter()
                .all(|e| !matches!(e.verdict, Verdict::Fires { .. }))
    }
}

/// Turn a preview's counts into the finding.
///
/// # Errors
///
/// [`Code::NotExpressible`] when the definition has no readable form, and
/// [`Code::ImpossibleTally`] when the counts are not counts of *this* strategy
/// — a missing entry, a clause path the definition does not contain, or a
/// clause list of the wrong length. A profile computed against the wrong
/// strategy would name a real clause of the wrong document.
pub fn blocking_profile(
    def: &StrategyDefinition,
    counts: &[EntryCounts],
) -> Result<BlockingProfile, Diagnostic> {
    blocking_profile_with(def, counts, &Catalogue::engine())
}

/// As [`blocking_profile()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`blocking_profile()`].
pub fn blocking_profile_with(
    def: &StrategyDefinition,
    counts: &[EntryCounts],
    cat: &Catalogue,
) -> Result<BlockingProfile, Diagnostic> {
    let shape = entry_clauses_with(def, cat)?;
    let mut entries = Vec::with_capacity(shape.len());
    for entry in &shape {
        let Some(tally) = counts.iter().find(|c| c.path() == entry.path) else {
            return Err(impossible(
                &entry.path,
                "the preview returned no counts for this entry",
            ));
        };
        if tally.clauses().len() != entry.clauses.len() {
            return Err(impossible(
                &entry.path,
                format!(
                    "the counts cover {} clauses and the condition has {}",
                    tally.clauses().len(),
                    entry.clauses.len()
                ),
            ));
        }
        let failed = tally.bars_evaluated() - tally.bars_all_true();
        let mut clauses = Vec::with_capacity(entry.clauses.len());
        for (clause, count) in entry.clauses.iter().zip(tally.clauses()) {
            if count.path != clause.path {
                return Err(impossible(
                    &entry.path,
                    format!(
                        "the counts name `{}` where the condition's clause is `{}`",
                        count.path, clause.path
                    ),
                ));
            }
            clauses.push(ClauseProfile {
                path: clause.path.clone(),
                text: clause.text.clone(),
                bars_true: count.bars_true,
                true_share: share(count.bars_true, tally.bars_evaluated()),
                sole_blocker_share: share(count.bars_sole_blocker, failed),
            });
        }

        let fire_share = share(tally.bars_all_true(), tally.bars_evaluated());
        let verdict = if tally.bars_evaluated() == 0 {
            Verdict::NothingEvaluated
        } else if tally.bars_all_true() > 0 {
            Verdict::Fires { share: fire_share }
        } else {
            let worst = clauses
                .iter()
                .enumerate()
                .max_by(|a, b| {
                    a.1.sole_blocker_share
                        .partial_cmp(&b.1.sole_blocker_share)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .filter(|(_, c)| c.sole_blocker_share >= SOLE_BLOCKER_SHARE);
            match worst {
                Some((i, c)) => Verdict::Blocked {
                    clause: i,
                    sole_blocker_share: c.sole_blocker_share,
                },
                None => Verdict::NoSingleBlocker,
            }
        };

        entries.push(EntryProfile {
            path: entry.path.clone(),
            side: entry.side,
            bars_evaluated: tally.bars_evaluated(),
            bars_all_true: tally.bars_all_true(),
            fire_share,
            clauses,
            verdict,
        });
    }
    Ok(BlockingProfile { entries })
}

// ── measuring it without a second evaluator ─────────────────────────────────

/// What one probe run is the answer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measures {
    /// Every clause together: the bars on which the entry is true.
    AllClauses,
    /// This clause alone: the bars on which it is true.
    OnlyClause(usize),
    /// Every clause **except** this one: with [`Self::AllClauses`], the bars
    /// this clause alone cost the strategy.
    AllButClause(usize),
}

/// One preview run the caller must make, and the signal to read the answer from.
///
/// A probe is the stored definition with exactly one thing changed: the entry's
/// condition expression. Its declared inputs are untouched, and the preview
/// takes its feature list from them, so every probe has the same warm-up and
/// the same `bars_evaluated` as the strategy itself. That sameness is the
/// whole reason this is correct: shares of different windows cannot be
/// subtracted.
#[derive(Clone, Debug)]
pub struct Probe {
    pub measures: Measures,
    pub definition: StrategyDefinition,
    /// The `emit` name whose `bars_true` the preview reports for this probe.
    pub signal: String,
}

/// The probes for one entry condition.
#[derive(Clone, Debug)]
pub struct ProbeSet {
    entry: EntryClauses,
    probes: Vec<Probe>,
}

/// An expression no bar satisfies, in the frozen v1 grammar. It stands in for
/// the other entries on the same side, so a probe measures one entry and not
/// the union the preview's `signals` list would otherwise report under one name.
pub const NEVER: &str = "(0 > 1)";

impl ProbeSet {
    #[must_use]
    pub const fn entry(&self) -> &EntryClauses {
        &self.entry
    }

    /// In the order [`Self::counts`] reads their answers.
    #[must_use]
    pub fn probes(&self) -> &[Probe] {
        &self.probes
    }

    /// Fold the preview's answers into the tally [`blocking_profile()`] reads.
    ///
    /// `bars_true[i]` is the `bars_true` of [`Self::probes`]`[i]`'s signal, and
    /// `bars_evaluated` is the preview's own `bars_evaluated` (identical for
    /// every probe, by construction).
    ///
    /// # Errors
    ///
    /// [`Code::ImpossibleTally`] when the answers are the wrong number, or
    /// cannot have come from one window — including a conjunction that was
    /// true on more bars than a subset of its own clauses.
    pub fn counts(
        &self,
        bars_evaluated: u64,
        bars_true: &[u64],
    ) -> Result<EntryCounts, Diagnostic> {
        let path = self.entry.path.as_str();
        if bars_true.len() != self.probes.len() {
            return Err(impossible(
                path,
                format!(
                    "{} answers for {} probes",
                    bars_true.len(),
                    self.probes.len()
                ),
            ));
        }
        let n = self.entry.clauses.len();
        let all = bars_true[0];
        let only = |i: usize| -> u64 {
            if n == 1 {
                all
            } else {
                bars_true[1 + i]
            }
        };
        let mut clauses = Vec::with_capacity(n);
        for (i, c) in self.entry.clauses.iter().enumerate() {
            let sole = if n == 1 {
                // No other clause to be true: every bar this one was false on.
                bars_evaluated.checked_sub(all)
            } else {
                // Every other clause true, this one false: the others' joint
                // count minus the bars on which this one was true as well.
                let others = if n == 2 {
                    only(1 - i)
                } else {
                    bars_true[1 + n + i]
                };
                others.checked_sub(all)
            };
            let Some(sole) = sole else {
                return Err(impossible(
                    path,
                    format!(
                        "the conjunction was true on {all} bars and the other clauses together \
                         on fewer — a conjunction cannot outrun a subset of itself"
                    ),
                ));
            };
            clauses.push(ClauseCounts {
                path: c.path.clone(),
                bars_true: only(i),
                bars_sole_blocker: sole,
            });
        }
        EntryCounts::new(path, bars_evaluated, all, clauses)
    }
}

/// The preview runs that measure every entry condition clause by clause.
///
/// This is how the blocking profile reaches a model without a second walk over
/// the bars: the preview already reports how many bars a signal was true on, so
/// each question "how often is this clause true, and how often are all the
/// others?" is a preview of the same definition with that expression swapped
/// in. Cost: `1` run for a one-clause entry, `n + 1` for two, `2n + 1` for
/// three or more.
///
/// # Errors
///
/// Whatever [`entry_clauses()`] refuses.
pub fn probes(def: &StrategyDefinition) -> Result<Vec<ProbeSet>, Diagnostic> {
    probes_with(def, &Catalogue::engine())
}

/// As [`probes()`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`probes()`].
pub fn probes_with(def: &StrategyDefinition, cat: &Catalogue) -> Result<Vec<ProbeSet>, Diagnostic> {
    let entries = entry_clauses_with(def, cat)?;

    // Every feature the strategy declares, once each, in declaration order.
    let mut declared: Vec<&str> = Vec::new();
    for input in &def.inputs {
        for f in &input.features {
            if !declared.contains(&f.as_str()) {
                declared.push(f.as_str());
            }
        }
    }
    let guard: Option<String> = declared
        .iter()
        .map(|f| format!("(feature('{f}') == feature('{f}'))"))
        .reduce(|a, b| format!("({a} * {b})"));

    let mut out = Vec::with_capacity(entries.len());
    for entry in &entries {
        // The signal this entry's condition feeds.
        let emits: Vec<&str> = def
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Signal { when, emit } if *when == entry.node => Some(emit.as_str()),
                _ => None,
            })
            .collect();
        let [signal] = emits.as_slice() else {
            return Err(defect(
                "an entry condition does not feed exactly one signal",
            ));
        };
        let signal = (*signal).to_string();
        // Every other condition feeding the same signal name: the preview
        // reports one count per name, so they have to be silenced for a probe
        // to measure this entry alone.
        let siblings: Vec<&str> = def
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Signal { when, emit } if *emit == signal && *when != entry.node => {
                    Some(when.as_str())
                }
                _ => None,
            })
            .collect();

        let make = |measures: Measures, expr: String| -> Probe {
            // Restricted to the bars the preview counts as evaluated: the
            // expression is multiplied by `x == x` for every declared feature,
            // which is true where the feature has a value and makes the whole
            // condition false where it has none. Without it a clause that reads
            // one early-warming feature is "true" on bars before the strategy
            // as a whole has any answer, and the tallies stop being counts of
            // one window.
            let expr = match &guard {
                Some(g) => format!("(({expr}) * {g})"),
                None => expr,
            };
            let mut d = def.clone();
            for node in &mut d.nodes {
                if let NodeKind::Condition { expr: e } = &mut node.kind {
                    if node.id == entry.node {
                        e.clone_from(&expr);
                    } else if siblings.contains(&node.id.as_str()) {
                        *e = NEVER.to_string();
                    }
                }
            }
            Probe {
                measures,
                definition: d,
                signal: signal.clone(),
            }
        };

        let exprs: Vec<&str> = entry.clauses.iter().map(|c| c.expr.as_str()).collect();
        // The compiled `and` encoding, so the probe is evaluated by the same
        // rule the strategy itself is: `((A) * (B))`. Each clause is
        // parenthesised here because `Clause::expr` is a slice of the stored
        // text with the parser's parentheses already stripped — `a > b`, not
        // `(a > b)` — and `(a > b * c < d)` is a different expression from the
        // conjunction it was cut out of.
        let conj = |skip: Option<usize>| -> String {
            let mut it = exprs
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != skip)
                .map(|(_, e)| format!("({e})"));
            let first = it.next().unwrap_or_default();
            it.fold(first, |acc, e| format!("({acc} * {e})"))
        };

        let n = exprs.len();
        let mut probes = vec![make(Measures::AllClauses, conj(None))];
        if n > 1 {
            for (i, e) in exprs.iter().enumerate() {
                probes.push(make(Measures::OnlyClause(i), (*e).to_string()));
            }
        }
        if n > 2 {
            for i in 0..n {
                probes.push(make(Measures::AllButClause(i), conj(Some(i))));
            }
        }
        out.push(ProbeSet {
            entry: entry.clone(),
            probes,
        });
    }
    Ok(out)
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Split the compiled `and` encoding, on exactly the rule
/// [`crate::decompile`] reads it back with: `(A * B)` is a conjunction only
/// when both sides are themselves boolean-shaped, so `(x * 2)` where `x` is a
/// comparison stays one clause.
fn split_encoded(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::Binary {
            op: BinOp::Mul,
            lhs,
            rhs,
            ..
        } if is_encoded_bool(lhs) && is_encoded_bool(rhs) => {
            let mut v = split_encoded(lhs);
            v.extend(split_encoded(rhs));
            v
        }
        other => vec![other],
    }
}

fn share(n: u64, of: u64) -> f64 {
    if of == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        n as f64 / of as f64
    }
}

fn pct(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

fn impossible(path: &str, what: impl Into<String>) -> Diagnostic {
    Diagnostic::semantic(
        Code::ImpossibleTally,
        NO_SOURCE,
        format!("{path}: {}", what.into()),
    )
    .with_fix(
        "these counts cannot have come from one walk over one window; check the preview's \
         tallies before reading a profile off them",
    )
    .with_see("guide#preview.blocking")
}

fn defect(what: &str) -> Diagnostic {
    Diagnostic::semantic(
        Code::BadStoredExpression,
        NO_SOURCE,
        format!("{what} — this is a defect in `strategy-lang`, not in the definition"),
    )
    .with_fix("`tests/round_trip.rs` is where it should have been caught")
    .with_see("guide#json")
}

impl ClauseProfile {
    /// The clause's position in its condition, recovered from its path.
    ///
    /// Used by callers that hold a profile without the [`Clause`] list; the
    /// path is the addressing both halves agree on.
    #[must_use]
    pub fn index_hint(&self) -> usize {
        self.path
            .rsplit_once('[')
            .and_then(|(_, tail)| tail.strip_suffix(']'))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_CLAUSE: &str = "\
strategy s on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 30
size fixed 0.01
exit time 24 bars
";

    fn def(src: &str) -> StrategyDefinition {
        let ast = crate::parse(src).unwrap_or_else(|e| panic!("parse: {e:#?}"));
        crate::compile(&ast).unwrap_or_else(|e| panic!("compile: {e:#?}"))
    }

    #[test]
    fn a_condition_decomposes_into_clauses_the_preview_can_evaluate() {
        let cs = entry_clauses(&def(TWO_CLAUSE)).expect("clauses");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].path, "entry.long[0]");
        assert_eq!(cs[0].side, Side::Long);
        assert_eq!(cs[0].clauses.len(), 2);
        assert_eq!(cs[0].clauses[0].text, "ema(close, 7) > ema(close, 21)");
        assert_eq!(cs[0].clauses[1].text, "rsi(close, 14) < 30");
        assert_eq!(cs[0].clauses[1].path, "entry.long[0].clause[1]");

        // The evaluable half is the frozen v1 grammar, and it parses on its own
        // — which is the whole claim this field makes to the preview.
        for c in &cs[0].clauses {
            crate::parse::expr_from_str(&c.expr)
                .unwrap_or_else(|d| panic!("`{}` is not a v1 expression: {d:#?}", c.expr));
            assert!(
                c.expr.contains("feature('"),
                "the evaluable form should be v1, not SLv2: {}",
                c.expr
            );
        }
    }

    /// The compiled boolean encoding is arithmetic, and a real multiplication
    /// must not be mistaken for a conjunction — the same rule `decompile`
    /// applies in the other direction.
    #[test]
    fn a_real_multiplication_is_one_clause_not_two() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
entry long when ema(close, 7) * 2 > ema(close, 21)
size fixed 0.01
exit time 24 bars
";
        let cs = entry_clauses(&def(src)).expect("clauses");
        assert_eq!(cs[0].clauses.len(), 1, "{:#?}", cs[0].clauses);
    }

    fn counts(path: &str, bars: u64, all_true: u64, per: &[(u64, u64)]) -> EntryCounts {
        EntryCounts::new(
            path,
            bars,
            all_true,
            per.iter()
                .enumerate()
                .map(|(i, (t, b))| ClauseCounts {
                    path: format!("{path}.clause[{i}]"),
                    bars_true: *t,
                    bars_sole_blocker: *b,
                })
                .collect(),
        )
        .expect("a possible tally")
    }

    /// The fixture is built to have exactly one blocker: the trend clause is
    /// true most of the time, the oversold clause almost never, and nearly
    /// every failure is the oversold clause alone.
    #[test]
    fn the_profile_names_the_sole_blocker_on_a_fixture_built_to_have_one() {
        let d = def(TWO_CLAUSE);
        let c = counts("entry.long[0]", 1000, 0, &[(600, 400), (4, 596)]);
        let p = blocking_profile(&d, &[c]).expect("profile");
        assert_eq!(p.entries.len(), 1);
        let e = &p.entries[0];
        match e.verdict {
            Verdict::Blocked {
                clause,
                sole_blocker_share,
            } => {
                assert_eq!(clause, 1);
                assert!((sole_blocker_share - 0.596).abs() < 1e-9);
            }
            ref other => panic!("expected a sole blocker, got {other:?}"),
        }
        assert!(p.never_fires());
        let words = e.words();
        assert!(words.contains("rsi(close, 14) < 30"), "{words}");
        assert!(words.contains("0.4% of bars"), "{words}");
        assert!(words.contains("59.6% of the rest"), "{words}");
        assert_eq!(
            e.binding_clause().map(|c| c.path.as_str()),
            Some("entry.long[0].clause[1]")
        );
    }

    #[test]
    fn no_clause_carrying_the_blame_is_said_rather_than_guessed() {
        let d = def(TWO_CLAUSE);
        let c = counts("entry.long[0]", 1000, 0, &[(300, 200), (300, 200)]);
        let p = blocking_profile(&d, &[c]).expect("profile");
        assert_eq!(p.entries[0].verdict, Verdict::NoSingleBlocker);
        assert!(p.entries[0]
            .words()
            .contains("no one clause is responsible"));
    }

    #[test]
    fn an_empty_window_claims_nothing() {
        let d = def(TWO_CLAUSE);
        let c = counts("entry.long[0]", 0, 0, &[(0, 0), (0, 0)]);
        let p = blocking_profile(&d, &[c]).expect("profile");
        assert_eq!(p.entries[0].verdict, Verdict::NothingEvaluated);
        assert!(p.entries[0].words().contains("never evaluated"));
    }

    #[test]
    fn a_condition_that_fires_reports_its_share_and_still_names_the_binding_clause() {
        let d = def(TWO_CLAUSE);
        let c = counts("entry.long[0]", 1000, 50, &[(600, 300), (60, 640)]);
        let p = blocking_profile(&d, &[c]).expect("profile");
        match p.entries[0].verdict {
            Verdict::Fires { share } => assert!((share - 0.05).abs() < 1e-9),
            ref other => panic!("expected Fires, got {other:?}"),
        }
        assert!(!p.never_fires());
        assert_eq!(
            p.entries[0].binding_clause().map(|c| c.index_hint()),
            Some(1)
        );
    }

    #[test]
    fn a_tally_no_window_could_have_produced_is_refused_by_name() {
        let cases: Vec<(&str, Vec<ClauseCounts>, u64, u64)> = vec![
            (
                "a clause true more often than there were bars",
                vec![ClauseCounts {
                    path: "entry.long[0].clause[0]".into(),
                    bars_true: 101,
                    bars_sole_blocker: 0,
                }],
                100,
                0,
            ),
            (
                "a conjunction truer than its rarest clause",
                vec![ClauseCounts {
                    path: "entry.long[0].clause[0]".into(),
                    bars_true: 10,
                    bars_sole_blocker: 0,
                }],
                100,
                20,
            ),
            (
                "a sole blocker on a bar it was true",
                vec![ClauseCounts {
                    path: "entry.long[0].clause[0]".into(),
                    bars_true: 90,
                    bars_sole_blocker: 20,
                }],
                100,
                0,
            ),
        ];
        for (label, clauses, bars, all_true) in cases {
            let err = EntryCounts::new("entry.long[0]", bars, all_true, clauses).expect_err(label);
            assert_eq!(err.code, Code::ImpossibleTally, "{label}");
            assert!(err.fix.is_some(), "{label}");
        }
        assert!(
            EntryCounts::new("entry.long[0]", 10, 0, Vec::new()).is_err(),
            "a condition with no clauses"
        );
        // Two clauses cannot both be the sole blocker on the same bar.
        assert!(EntryCounts::new(
            "entry.long[0]",
            100,
            0,
            vec![
                ClauseCounts {
                    path: "entry.long[0].clause[0]".into(),
                    bars_true: 40,
                    bars_sole_blocker: 60,
                },
                ClauseCounts {
                    path: "entry.long[0].clause[1]".into(),
                    bars_true: 40,
                    bars_sole_blocker: 60,
                },
            ],
        )
        .is_err());
    }

    #[test]
    fn counts_for_a_different_strategy_are_refused_rather_than_read() {
        let d = def(TWO_CLAUSE);
        let wrong = EntryCounts::new(
            "entry.long[0]",
            100,
            0,
            vec![ClauseCounts {
                path: "entry.long[0].clause[0]".into(),
                bars_true: 10,
                bars_sole_blocker: 90,
            }],
        )
        .expect("a possible tally");
        let err = blocking_profile(&d, &[wrong]).expect_err("refused");
        assert_eq!(err.code, Code::ImpossibleTally);
        assert!(err.message.contains("1 clauses"), "{}", err.message);

        let err = blocking_profile(&d, &[]).expect_err("refused");
        assert_eq!(err.code, Code::ImpossibleTally);
        assert!(err.message.contains("no counts"), "{}", err.message);
    }

    #[test]
    fn both_sides_of_a_two_sided_strategy_get_their_own_profile() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
input r = rsi(close, 14)
entry long when r < 30
entry short when r > 70
size fixed 0.01
exit time 24 bars
";
        let cs = entry_clauses(&def(src)).expect("clauses");
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].path, "entry.long[0]");
        assert_eq!(cs[1].path, "entry.short[0]");
        // The input is inlined, so both sides read as what they measure.
        assert_eq!(cs[0].clauses[0].text, "rsi(close, 14) < 30");
        assert_eq!(cs[1].clauses[0].text, "rsi(close, 14) > 70");
    }
}
