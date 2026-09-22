//! Postfix bytecode compiler and interpreter for strategy condition expressions.
//!
//! Expressions are compiled once at `StrategyInstance` init; evaluation on each
//! tick walks a fixed-size stack with no heap allocation per call.
//!
//! # Example
//!
//! ```text
//! feature('ema_7') > feature('ema_21')
//! ```
//!
//! compiles to:
//!
//! ```text
//! [LoadFeature(0), LoadFeature(1), Gt]
//! feature_names = ["ema_7", "ema_21"]
//! ```
//!
//! # History-bearing functions
//!
//! `crossed_above(a, b)`, `crossed_below(a, b)`, `rising(x, n)` and
//! `falling(x, n)` read bars other than the current one. They compile to a
//! *site* — an index into a [`HistoryRegisters`] file that carries one value
//! per bar — plus one op that reads it:
//!
//! ```text
//! crossed_above(feature('ema_7'), feature('ema_21'))
//! → [LoadFeature(0), LoadFeature(1), Sub, CrossedAbove(0)]
//!   history_sites = [1]
//! ```
//!
//! The site stores the *difference*, not the two operands, which is why the
//! operands may be any sub-expression rather than the two bare feature slots
//! FEAT-004 §3.3 sketched — `crossed_above(bar('close'), feature('ema_21'))`
//! compiles with no extra machinery.
//!
//! # Absent data propagates as NaN, and NaN is not false
//!
//! A missing feature, a missing bar, a division by zero and a site that has
//! not warmed all push `f64::NAN` and record why. NaN propagates through the
//! arithmetic, and — unlike IEEE comparison, which would quietly answer
//! `false` — the comparison ops here propagate it too. The program's verdict
//! is [`Verdict::Unknown`] whenever the value at the top of the stack is NaN.
//!
//! Propagating rather than returning early matters for a second reason: a
//! history site must get its chance to record this bar's value even when some
//! *other* part of the expression could not be evaluated, or the site would
//! commit a gap and have to warm again from scratch.
//!
//! One conservatism falls out of it. `and` and `or` compile to `*` and `+`
//! (there is no `And`/`Or` op — FEAT-004 §3.3 lists them as unbuilt), so
//! `false and unknown` evaluates to NaN and reports unknown where strict
//! three-valued logic would allow false. Unknown is the safe direction: it
//! surfaces a bar as unresolved rather than claiming a "no" nobody computed.

use std::collections::HashMap;

use domain::payloads::bar::{BarPayload, Timeframe};
use rust_decimal::prelude::ToPrimitive;

use crate::history::{HistoryRegisters, Unknown, Verdict, MAX_HISTORY_DEPTH};
use crate::interpreter::{tokenize, EvalError, Tok};

// ── Public types ──────────────────────────────────────────────────────────────

/// Bar OHLCV field selector used in `LoadBarField`.
#[derive(Debug, Clone, PartialEq)]
pub enum BarField {
    Open,
    High,
    Low,
    Close,
    Volume,
}

/// A single postfix instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Push the value of `feature_names[slot]` onto the stack.
    LoadFeature(u16),
    /// Push the value of the given bar OHLCV field (Minutes1 bar).
    LoadBarField(BarField),
    /// Push a numeric constant.
    Const(f64),
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Gt,
    Lt,
    GtEq,
    LtEq,
    EqEq,
    BangEq,
    /// Pop `a - b`, record it at `history_sites[i]`, and push whether the
    /// difference went from at-or-below zero to above it since the last bar.
    CrossedAbove(u16),
    /// The mirror: at-or-above zero to below it.
    CrossedBelow(u16),
    /// Pop `x`, record it at `history_sites[i]`, and push whether `x` has
    /// strictly increased on each of the last `history_sites[i]` bars.
    Rising(u16),
    /// The mirror: strictly decreased.
    Falling(u16),
}

impl Op {
    /// The history site this op reads, when it reads one.
    #[must_use]
    pub const fn history_site(&self) -> Option<u16> {
        match self {
            Self::CrossedAbove(i) | Self::CrossedBelow(i) | Self::Rising(i) | Self::Falling(i) => {
                Some(*i)
            }
            _ => None,
        }
    }
}

/// A compiled condition program.
///
/// `feature_names[i]` is the feature name for `LoadFeature(i)`.
/// `ops` is the postfix instruction sequence.
/// `history_sites[i]` is the bar depth site `i` needs, in the order the sites
/// were allocated during compilation.
#[derive(Debug, Clone)]
pub struct Program {
    pub feature_names: Vec<String>,
    pub ops: Vec<Op>,
    pub history_sites: Vec<u16>,
}

impl Program {
    /// Whether this program reads any bar other than the current one.
    #[must_use]
    pub fn needs_history(&self) -> bool {
        !self.history_sites.is_empty()
    }

    /// Bars of lead-in this program needs **on top of** whatever its features
    /// need to converge.
    ///
    /// A level test needs none. `crossed_above(…)` needs one: the bar it
    /// compares against. `rising(x, n)` needs `n`. A caller deriving a
    /// strategy's data requirements adds this to the indicator warm-up, or the
    /// first tradable bar of every window is one the strategy cannot answer on.
    #[must_use]
    pub fn history_warmup_bars(&self) -> u16 {
        self.history_sites.iter().copied().max().unwrap_or(0)
    }

    /// Allocate this program's sites in `regs` and return their base index.
    pub fn install_sites(&self, regs: &mut HistoryRegisters) -> usize {
        regs.push_sites(&self.history_sites)
    }
}

// ── Compiler ──────────────────────────────────────────────────────────────────

/// Functions that read a bar other than the current one.
pub const HISTORY_FUNCTIONS: &[&str] = &["crossed_above", "crossed_below", "rising", "falling"];

struct Compiler {
    tokens: Vec<Tok>,
    pos: usize,
    ops: Vec<Op>,
    feature_names: Vec<String>,
    history_sites: Vec<u16>,
}

impl Compiler {
    fn feature_slot(&mut self, name: String) -> u16 {
        if let Some(pos) = self.feature_names.iter().position(|n| n == &name) {
            return pos as u16;
        }
        let slot = self.feature_names.len() as u16;
        self.feature_names.push(name);
        slot
    }

    fn new_site(&mut self, depth: u16) -> u16 {
        let idx = self.history_sites.len() as u16;
        self.history_sites.push(depth);
        idx
    }

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Tok {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect_lparen(&mut self) -> Result<(), EvalError> {
        match self.advance() {
            Tok::LParen => Ok(()),
            t => Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), EvalError> {
        match self.advance() {
            Tok::RParen => Ok(()),
            t => Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        }
    }

    fn expect_comma(&mut self) -> Result<(), EvalError> {
        match self.advance() {
            Tok::Comma => Ok(()),
            t => Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        }
    }

    fn expect_str(&mut self) -> Result<String, EvalError> {
        match self.advance() {
            Tok::Str(s) => Ok(s),
            t => Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        }
    }

    /// A whole-number literal, used for the bar count of `rising`/`falling`.
    fn expect_depth(&mut self, func: &str) -> Result<u16, EvalError> {
        let n = match self.advance() {
            Tok::Num(n) => n,
            t => return Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        };
        if n.fract() != 0.0 || n < 1.0 || n > f64::from(MAX_HISTORY_DEPTH) {
            return Err(EvalError::BadHistoryDepth {
                func: func.to_owned(),
                given: n,
                max: MAX_HISTORY_DEPTH,
            });
        }
        Ok(n as u16)
    }

    fn compile_expr(&mut self) -> Result<(), EvalError> {
        self.compile_comparison()
    }

    fn compile_comparison(&mut self) -> Result<(), EvalError> {
        self.compile_term()?;
        let op = match self.peek() {
            Tok::Gt => Some(Op::Gt),
            Tok::Lt => Some(Op::Lt),
            Tok::GtEq => Some(Op::GtEq),
            Tok::LtEq => Some(Op::LtEq),
            Tok::EqEq => Some(Op::EqEq),
            Tok::BangEq => Some(Op::BangEq),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            self.compile_term()?;
            self.ops.push(op);
        }
        Ok(())
    }

    fn compile_term(&mut self) -> Result<(), EvalError> {
        self.compile_factor()?;
        loop {
            match self.peek() {
                Tok::Plus => {
                    self.advance();
                    self.compile_factor()?;
                    self.ops.push(Op::Add);
                }
                Tok::Minus => {
                    self.advance();
                    self.compile_factor()?;
                    self.ops.push(Op::Sub);
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn compile_factor(&mut self) -> Result<(), EvalError> {
        self.compile_unary()?;
        loop {
            match self.peek() {
                Tok::Star => {
                    self.advance();
                    self.compile_unary()?;
                    self.ops.push(Op::Mul);
                }
                Tok::Slash => {
                    self.advance();
                    self.compile_unary()?;
                    self.ops.push(Op::Div);
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn compile_unary(&mut self) -> Result<(), EvalError> {
        if matches!(self.peek(), Tok::Minus) {
            self.advance();
            self.compile_unary()?;
            self.ops.push(Op::Neg);
            return Ok(());
        }
        self.compile_primary()
    }

    fn compile_primary(&mut self) -> Result<(), EvalError> {
        match self.advance() {
            Tok::Num(n) => {
                self.ops.push(Op::Const(n));
                Ok(())
            }
            Tok::LParen => {
                self.compile_expr()?;
                self.expect_rparen()
            }
            Tok::Ident(name) => self.compile_call(&name),
            other => Err(EvalError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    fn compile_call(&mut self, name: &str) -> Result<(), EvalError> {
        self.expect_lparen()?;

        match name {
            "feature" => {
                let arg = self.expect_str()?;
                self.expect_rparen()?;
                let slot = self.feature_slot(arg);
                self.ops.push(Op::LoadFeature(slot));
                Ok(())
            }
            "bar" => {
                let arg = self.expect_str()?;
                self.expect_rparen()?;
                let field = match arg.as_str() {
                    "open" => BarField::Open,
                    "high" => BarField::High,
                    "low" => BarField::Low,
                    "close" => BarField::Close,
                    "volume" => BarField::Volume,
                    f => return Err(EvalError::UnknownBarField(f.to_owned())),
                };
                self.ops.push(Op::LoadBarField(field));
                Ok(())
            }
            // `crossed_above(a, b)` is `a - b` crossing zero upwards. Storing
            // the difference is what lets the operands be arbitrary
            // sub-expressions instead of two bare feature slots.
            "crossed_above" | "crossed_below" => {
                self.compile_expr()?;
                self.expect_comma()?;
                self.compile_expr()?;
                self.expect_rparen()?;
                self.ops.push(Op::Sub);
                let site = self.new_site(1);
                self.ops.push(if name == "crossed_above" {
                    Op::CrossedAbove(site)
                } else {
                    Op::CrossedBelow(site)
                });
                Ok(())
            }
            "rising" | "falling" => {
                self.compile_expr()?;
                self.expect_comma()?;
                let depth = self.expect_depth(name)?;
                self.expect_rparen()?;
                let site = self.new_site(depth);
                self.ops.push(if name == "rising" {
                    Op::Rising(site)
                } else {
                    Op::Falling(site)
                });
                Ok(())
            }
            other => Err(EvalError::UnknownIdent(other.to_owned())),
        }
    }
}

/// Compile an expression string into a `Program`.
///
/// # Errors
///
/// Returns `Err` only for malformed expressions; missing features and bars are
/// handled at run time by returning [`Verdict::Unknown`].
pub fn compile(expr: &str) -> Result<Program, EvalError> {
    let tokens = tokenize(expr)?;
    let mut compiler = Compiler {
        tokens,
        pos: 0,
        ops: Vec::new(),
        feature_names: Vec::new(),
        history_sites: Vec::new(),
    };
    compiler.compile_expr()?;
    Ok(Program {
        feature_names: compiler.feature_names,
        ops: compiler.ops,
        history_sites: compiler.history_sites,
    })
}

// ── Interpreter ───────────────────────────────────────────────────────────────

fn bar_field_f64(bar: &BarPayload, field: &BarField) -> f64 {
    let d = match field {
        BarField::Open => bar.open.inner(),
        BarField::High => bar.high.inner(),
        BarField::Low => bar.low.inner(),
        BarField::Close => bar.close.inner(),
        BarField::Volume => bar.volume.inner(),
    };
    d.to_f64().unwrap_or(f64::NAN)
}

/// Where a `LoadFeature` reads its value from.
enum FeatureSource<'a> {
    /// The global slot array, through a per-program local→global mapping.
    Slots {
        local_to_global: &'a [u16],
        slots: &'a [f64],
    },
    /// A name→value map, for callers that have no registry.
    Named(&'a HashMap<String, f64>),
}

impl FeatureSource<'_> {
    fn get(&self, program: &Program, local_slot: u16) -> Option<f64> {
        match self {
            Self::Slots {
                local_to_global,
                slots,
            } => {
                let global = *local_to_global.get(local_slot as usize)?;
                slots.get(global as usize).copied()
            }
            Self::Named(map) => {
                let name = program.feature_names.get(local_slot as usize)?;
                map.get(name).copied()
            }
        }
    }
}

const STACK_DEPTH: usize = 32;

/// The core postfix machine.
///
/// Absent data becomes NaN and the reason is remembered; NaN propagates through
/// every op, including the comparisons, so it cannot be laundered into a
/// `false`. Only a stack fault aborts, because that can only mean the program
/// itself is malformed.
#[allow(clippy::too_many_lines)]
fn exec(
    program: &Program,
    features: &FeatureSource<'_>,
    bars: &HashMap<Timeframe, BarPayload>,
    history: &mut HistoryRegisters,
    site_base: usize,
) -> Verdict {
    let mut stack = [0.0_f64; STACK_DEPTH];
    let mut sp: usize = 0;
    let mut first_unknown: Option<Unknown> = None;

    macro_rules! note {
        ($reason:expr) => {
            if first_unknown.is_none() {
                first_unknown = Some($reason);
            }
        };
    }
    macro_rules! push {
        ($v:expr) => {{
            if sp >= STACK_DEPTH {
                return Verdict::Unknown(Unknown::Malformed);
            }
            stack[sp] = $v;
            sp += 1;
        }};
    }
    macro_rules! pop2 {
        () => {{
            if sp < 2 {
                return Verdict::Unknown(Unknown::Malformed);
            }
            let b = stack[sp - 1];
            let a = stack[sp - 2];
            sp -= 2;
            (a, b)
        }};
    }
    macro_rules! pop1 {
        () => {{
            if sp == 0 {
                return Verdict::Unknown(Unknown::Malformed);
            }
            sp -= 1;
            stack[sp]
        }};
    }
    // A comparison on an absent operand answers "not known", not "no".
    macro_rules! cmp {
        ($op:tt) => {{
            let (a, b) = pop2!();
            if a.is_nan() || b.is_nan() {
                push!(f64::NAN);
            } else {
                push!(if a $op b { 1.0 } else { 0.0 });
            }
        }};
    }

    for op in &program.ops {
        match op {
            Op::LoadFeature(local_slot) => match features.get(program, *local_slot) {
                Some(v) if !v.is_nan() => push!(v),
                _ => {
                    note!(Unknown::FeatureNotReady);
                    push!(f64::NAN);
                }
            },
            Op::LoadBarField(field) => match bars.get(&Timeframe::Minutes1) {
                Some(bar) => push!(bar_field_f64(bar, field)),
                None => {
                    note!(Unknown::BarNotReady);
                    push!(f64::NAN);
                }
            },
            Op::Const(v) => push!(*v),
            Op::Neg => {
                let a = pop1!();
                push!(-a);
            }
            Op::Add => {
                let (a, b) = pop2!();
                push!(a + b);
            }
            Op::Sub => {
                let (a, b) = pop2!();
                push!(a - b);
            }
            Op::Mul => {
                let (a, b) = pop2!();
                push!(a * b);
            }
            Op::Div => {
                let (a, b) = pop2!();
                if b == 0.0 {
                    note!(Unknown::Undefined);
                    push!(f64::NAN);
                } else {
                    push!(a / b);
                }
            }
            Op::Gt => cmp!(>),
            Op::Lt => cmp!(<),
            Op::GtEq => cmp!(>=),
            Op::LtEq => cmp!(<=),
            Op::EqEq => {
                let (a, b) = pop2!();
                if a.is_nan() || b.is_nan() {
                    push!(f64::NAN);
                } else {
                    push!(if (a - b).abs() < f64::EPSILON { 1.0 } else { 0.0 });
                }
            }
            Op::BangEq => {
                let (a, b) = pop2!();
                if a.is_nan() || b.is_nan() {
                    push!(f64::NAN);
                } else {
                    push!(if (a - b).abs() >= f64::EPSILON {
                        1.0
                    } else {
                        0.0
                    });
                }
            }
            Op::CrossedAbove(i) | Op::CrossedBelow(i) => {
                let site = site_base + *i as usize;
                let diff = pop1!();
                // Record before testing readiness, and whatever else in this
                // expression failed: a site that skips a bar has to warm again.
                history.record(site, diff);
                match history.readiness(site) {
                    Err(reason) => {
                        note!(reason);
                        push!(f64::NAN);
                    }
                    Ok(()) => {
                        let Some(prev) = history.past(site, 0) else {
                            note!(Unknown::HistoryNotReady { have: 0, need: 1 });
                            push!(f64::NAN);
                            continue;
                        };
                        if diff.is_nan() {
                            note!(Unknown::FeatureNotReady);
                            push!(f64::NAN);
                        } else {
                            let crossed = if matches!(op, Op::CrossedAbove(_)) {
                                prev <= 0.0 && diff > 0.0
                            } else {
                                prev >= 0.0 && diff < 0.0
                            };
                            push!(if crossed { 1.0 } else { 0.0 });
                        }
                    }
                }
            }
            Op::Rising(i) | Op::Falling(i) => {
                let site = site_base + *i as usize;
                let x = pop1!();
                history.record(site, x);
                match history.readiness(site) {
                    Err(reason) => {
                        note!(reason);
                        push!(f64::NAN);
                    }
                    Ok(()) if x.is_nan() => {
                        note!(Unknown::FeatureNotReady);
                        push!(f64::NAN);
                    }
                    Ok(()) => {
                        let up = matches!(op, Op::Rising(_));
                        let need = history.depth(site);
                        let mut newer = x;
                        let mut holds = true;
                        for k in 0..need {
                            let Some(older) = history.past(site, k) else {
                                holds = false;
                                break;
                            };
                            if up && newer <= older || !up && newer >= older {
                                holds = false;
                                break;
                            }
                            newer = older;
                        }
                        push!(if holds { 1.0 } else { 0.0 });
                    }
                }
            }
        }
    }

    if sp == 0 {
        return Verdict::Unknown(Unknown::Malformed);
    }
    let top = stack[sp - 1];
    if top.is_nan() {
        return Verdict::Unknown(first_unknown.unwrap_or(Unknown::Malformed));
    }
    Verdict::from_bool(top != 0.0)
}

/// Execute a compiled program against a feature slot array.
///
/// `feature_slots` is the global `WorldState::feature_slots` array.
/// `local_to_global` maps each program-local feature slot index to a global
/// registry slot. It is built once at `StrategyInstance` init by
/// [`resolve_program_slots`].
///
/// `history` carries the previous-bar values this program's history sites read,
/// and `site_base` is where this program's run of sites starts in that file —
/// the value [`Program::install_sites`] returned. The caller must have called
/// [`HistoryRegisters::open_bar`] for the current bar first.
pub fn run_slots(
    program: &Program,
    local_to_global: &[u16],
    feature_slots: &[f64],
    bars: &HashMap<Timeframe, BarPayload>,
    history: &mut HistoryRegisters,
    site_base: usize,
) -> Verdict {
    exec(
        program,
        &FeatureSource::Slots {
            local_to_global,
            slots: feature_slots,
        },
        bars,
        history,
        site_base,
    )
}

/// Build the `local_to_global` slot mapping for a compiled program.
///
/// For each program-local feature slot (index into `program.feature_names`),
/// return the corresponding global registry slot from `registry`.
/// Any name not yet in the registry is assigned a new slot.
pub fn resolve_program_slots(
    program: &Program,
    registry: &mut crate::registry::FeatureRegistry,
) -> Vec<u16> {
    program
        .feature_names
        .iter()
        .map(|name| registry.get_or_assign(name))
        .collect()
}

/// Execute a compiled program against current feature and bar state, carrying
/// history across bars.
pub fn run_with_history(
    program: &Program,
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
    history: &mut HistoryRegisters,
    site_base: usize,
) -> Verdict {
    exec(
        program,
        &FeatureSource::Named(features),
        bars,
        history,
        site_base,
    )
}

/// Execute a compiled program with no history at all.
///
/// Correct only for programs where [`Program::needs_history`] is false. One
/// that does need history gets [`Verdict::Unknown`] from every history op,
/// on every bar, for ever — it does not silently degrade to a level test.
pub fn run(
    program: &Program,
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
) -> Verdict {
    let mut history = HistoryRegisters::with_depths(&program.history_sites);
    run_with_history(program, features, bars, &mut history, 0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn feats(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn empty_bars() -> HashMap<Timeframe, BarPayload> {
        HashMap::new()
    }

    /// Drive a program over a sequence of per-bar feature maps.
    fn drive(expr: &str, bars: &[Vec<(&str, f64)>]) -> Vec<Verdict> {
        let prog = compile(expr).unwrap();
        let mut hist = HistoryRegisters::with_depths(&prog.history_sites);
        let no_bars = empty_bars();
        bars.iter()
            .enumerate()
            .map(|(i, f)| {
                hist.open_bar(i as i64);
                run_with_history(&prog, &feats(f), &no_bars, &mut hist, 0)
            })
            .collect()
    }

    #[test]
    fn compile_ema_cross() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        assert_eq!(prog.feature_names, ["ema_7", "ema_21"]);
        assert_eq!(prog.ops, [Op::LoadFeature(0), Op::LoadFeature(1), Op::Gt]);
        assert!(!prog.needs_history());
        assert_eq!(prog.history_warmup_bars(), 0);
    }

    #[test]
    fn run_ema_cross_true() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_7", 11.0), ("ema_21", 10.0)]);
        assert_eq!(run(&prog, &features, &empty_bars()), Verdict::True);
    }

    #[test]
    fn run_ema_cross_false() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_7", 9.0), ("ema_21", 10.0)]);
        assert_eq!(run(&prog, &features, &empty_bars()), Verdict::False);
    }

    /// The rule the whole three-valued verdict exists for.
    #[test]
    fn run_missing_feature_is_unknown_not_false() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_21", 10.0)]);
        assert_eq!(
            run(&prog, &features, &empty_bars()),
            Verdict::Unknown(Unknown::FeatureNotReady)
        );
    }

    #[test]
    fn missing_bar_is_unknown_not_false() {
        let prog = compile("bar('close') > 100.0").unwrap();
        assert_eq!(
            run(&prog, &HashMap::new(), &empty_bars()),
            Verdict::Unknown(Unknown::BarNotReady)
        );
    }

    #[test]
    fn division_by_zero_is_unknown_not_false() {
        let prog = compile("feature('a') / feature('b') > 1.0").unwrap();
        let features = feats(&[("a", 1.0), ("b", 0.0)]);
        assert_eq!(
            run(&prog, &features, &empty_bars()),
            Verdict::Unknown(Unknown::Undefined)
        );
    }

    #[test]
    fn run_arithmetic_expression() {
        let prog = compile("feature('ema_7') + 5.0 > 14.0").unwrap();
        let features = feats(&[("ema_7", 10.0)]);
        assert_eq!(run(&prog, &features, &empty_bars()), Verdict::True);
    }

    #[test]
    fn run_bar_close_field() {
        use domain::money::{Price, Size};

        let prog = compile("bar('close') > 100.0").unwrap();
        let mut bars = HashMap::new();
        bars.insert(
            Timeframe::Minutes1,
            BarPayload::new(
                Timeframe::Minutes1,
                Price::from_str("100").unwrap(),
                Price::from_str("110").unwrap(),
                Price::from_str("95").unwrap(),
                Price::from_str("105").unwrap(),
                Size::from_str("500").unwrap(),
                200,
            ),
        );
        assert_eq!(run(&prog, &HashMap::new(), &bars), Verdict::True);
    }

    #[test]
    fn compile_deduplicates_feature_slots() {
        let prog = compile("feature('x') + feature('x') > 10.0").unwrap();
        assert_eq!(prog.feature_names, ["x"]);
        assert_eq!(
            prog.ops,
            [
                Op::LoadFeature(0),
                Op::LoadFeature(0),
                Op::Add,
                Op::Const(10.0),
                Op::Gt,
            ]
        );
    }

    // ── crosses ──────────────────────────────────────────────────────────────

    #[test]
    fn compile_crossed_above_emits_a_difference_and_a_site() {
        let prog = compile("crossed_above(feature('ema_7'), feature('ema_21'))").unwrap();
        assert_eq!(
            prog.ops,
            [
                Op::LoadFeature(0),
                Op::LoadFeature(1),
                Op::Sub,
                Op::CrossedAbove(0),
            ]
        );
        assert_eq!(prog.history_sites, [1]);
        assert!(prog.needs_history());
        assert_eq!(prog.history_warmup_bars(), 1);
    }

    /// The first bar has no previous bar, so the answer is not knowable —
    /// which is a different thing from the cross not having happened.
    #[test]
    fn the_first_bar_of_a_cross_is_unknown() {
        let v = drive(
            "crossed_above(feature('fast'), feature('slow'))",
            &[vec![("fast", 9.0), ("slow", 10.0)]],
        );
        assert_eq!(
            v[0],
            Verdict::Unknown(Unknown::HistoryNotReady { have: 0, need: 1 })
        );
    }

    #[test]
    fn a_cross_fires_once_on_the_bar_it_happens() {
        let v = drive(
            "crossed_above(feature('fast'), feature('slow'))",
            &[
                vec![("fast", 9.0), ("slow", 10.0)],  // below
                vec![("fast", 9.5), ("slow", 10.0)],  // still below
                vec![("fast", 10.5), ("slow", 10.0)], // crossed
                vec![("fast", 11.0), ("slow", 10.0)], // still above
            ],
        );
        assert_eq!(
            v,
            [
                Verdict::Unknown(Unknown::HistoryNotReady { have: 0, need: 1 }),
                Verdict::False,
                Verdict::True,
                Verdict::False,
            ]
        );
    }

    /// The point of the whole exercise: the level test the compiler currently
    /// suggests as a substitute is true on every bar the cross is not.
    #[test]
    fn the_level_test_is_not_the_cross() {
        let bars = [
            vec![("fast", 9.0), ("slow", 10.0)],
            vec![("fast", 10.5), ("slow", 10.0)],
            vec![("fast", 11.0), ("slow", 10.0)],
            vec![("fast", 12.0), ("slow", 10.0)],
        ];
        let cross = drive("crossed_above(feature('fast'), feature('slow'))", &bars);
        let level = drive("feature('fast') > feature('slow')", &bars);

        assert_eq!(cross.iter().filter(|v| v.is_true()).count(), 1);
        assert_eq!(level.iter().filter(|v| v.is_true()).count(), 3);
    }

    #[test]
    fn crossed_below_is_the_mirror() {
        let v = drive(
            "crossed_below(feature('fast'), feature('slow'))",
            &[
                vec![("fast", 11.0), ("slow", 10.0)],
                vec![("fast", 9.0), ("slow", 10.0)],
                vec![("fast", 8.0), ("slow", 10.0)],
            ],
        );
        assert_eq!(v[1], Verdict::True);
        assert_eq!(v[2], Verdict::False);
    }

    /// Touching the line and coming back up is not a cross below: the
    /// difference has to actually change sign.
    #[test]
    fn touching_without_passing_through_is_not_a_cross() {
        let v = drive(
            "crossed_above(feature('fast'), feature('slow'))",
            &[
                vec![("fast", 10.0), ("slow", 10.0)], // equal
                vec![("fast", 10.0), ("slow", 10.0)], // still equal
            ],
        );
        assert_eq!(v[1], Verdict::False);
    }

    /// Equal-then-above *is* a cross above: `prev <= 0 && now > 0`.
    #[test]
    fn coming_off_the_line_upwards_is_a_cross() {
        let v = drive(
            "crossed_above(feature('fast'), feature('slow'))",
            &[
                vec![("fast", 10.0), ("slow", 10.0)],
                vec![("fast", 10.1), ("slow", 10.0)],
            ],
        );
        assert_eq!(v[1], Verdict::True);
    }

    /// A bar where the feature was absent cannot be compared against, and the
    /// bar after it is unknown rather than compared with a stale value.
    #[test]
    fn a_gap_in_the_operands_makes_the_next_bar_unknown() {
        let v = drive(
            "crossed_above(feature('fast'), feature('slow'))",
            &[
                vec![("fast", 9.0), ("slow", 10.0)],
                vec![("slow", 10.0)], // fast absent
                vec![("fast", 11.0), ("slow", 10.0)],
            ],
        );
        assert!(v[1].is_unknown());
        assert!(
            v[2].is_unknown(),
            "the bar after a gap has no trustworthy previous value"
        );
    }

    /// A cross inside a conjunction — AGENT-011's headline example, in the
    /// shape `strategy-lang` compiles `and` to.
    #[test]
    fn a_cross_conjoined_with_a_level_test_compiles_and_runs() {
        let expr = "(crossed_above(feature('ema_7'), feature('ema_21'))) * ((feature('rsi_14') < 60))";
        let v = drive(
            expr,
            &[
                vec![("ema_7", 9.0), ("ema_21", 10.0), ("rsi_14", 50.0)],
                vec![("ema_7", 11.0), ("ema_21", 10.0), ("rsi_14", 50.0)],
                vec![("ema_7", 9.0), ("ema_21", 10.0), ("rsi_14", 50.0)],
                vec![("ema_7", 11.0), ("ema_21", 10.0), ("rsi_14", 70.0)],
            ],
        );
        assert_eq!(v[1], Verdict::True, "crossed and rsi below 60");
        assert_eq!(v[3], Verdict::False, "crossed but rsi too high");
    }

    /// A site records its bar even when another part of the expression could
    /// not be evaluated, so one absent operand costs one bar and not two.
    #[test]
    fn a_site_still_records_when_the_rest_of_the_expression_is_unknown() {
        let expr = "(crossed_above(feature('fast'), feature('slow'))) * ((feature('gate') > 0))";
        let v = drive(
            expr,
            &[
                vec![("fast", 9.0), ("slow", 10.0)], // gate absent
                vec![("fast", 11.0), ("slow", 10.0), ("gate", 1.0)],
            ],
        );
        assert!(v[0].is_unknown());
        assert_eq!(v[1], Verdict::True, "the site warmed on the unknown bar");
    }

    #[test]
    fn crossed_above_accepts_a_bar_field_operand() {
        let prog = compile("crossed_above(bar('close'), feature('ema_21'))").unwrap();
        assert_eq!(prog.history_sites, [1]);
        assert!(matches!(prog.ops.last(), Some(Op::CrossedAbove(0))));
    }

    #[test]
    fn two_crosses_in_one_expression_get_separate_sites() {
        let prog = compile(
            "(crossed_above(feature('a'), feature('b'))) + (crossed_below(feature('c'), feature('d')))",
        )
        .unwrap();
        assert_eq!(prog.history_sites, [1, 1]);
        assert!(prog.ops.contains(&Op::CrossedAbove(0)));
        assert!(prog.ops.contains(&Op::CrossedBelow(1)));
    }

    // ── rising / falling ─────────────────────────────────────────────────────

    #[test]
    fn rising_needs_n_bars_and_then_answers() {
        let v = drive(
            "rising(feature('x'), 2)",
            &[
                vec![("x", 1.0)],
                vec![("x", 2.0)],
                vec![("x", 3.0)], // 3 > 2 > 1
                vec![("x", 2.5)], // not rising
            ],
        );
        assert_eq!(
            v[0],
            Verdict::Unknown(Unknown::HistoryNotReady { have: 0, need: 2 })
        );
        assert_eq!(
            v[1],
            Verdict::Unknown(Unknown::HistoryNotReady { have: 1, need: 2 })
        );
        assert_eq!(v[2], Verdict::True);
        assert_eq!(v[3], Verdict::False);
    }

    #[test]
    fn falling_is_the_mirror() {
        let v = drive(
            "falling(feature('x'), 1)",
            &[vec![("x", 3.0)], vec![("x", 2.0)], vec![("x", 2.0)]],
        );
        assert_eq!(v[1], Verdict::True);
        assert_eq!(v[2], Verdict::False, "flat is not falling");
    }

    #[test]
    fn rising_reserves_the_depth_it_asked_for() {
        let prog = compile("rising(feature('x'), 7)").unwrap();
        assert_eq!(prog.history_sites, [7]);
        assert_eq!(prog.history_warmup_bars(), 7);
    }

    #[test]
    fn a_non_integer_or_out_of_range_depth_is_refused_at_compile_time() {
        assert!(matches!(
            compile("rising(feature('x'), 0)"),
            Err(EvalError::BadHistoryDepth { .. })
        ));
        assert!(matches!(
            compile("rising(feature('x'), 2.5)"),
            Err(EvalError::BadHistoryDepth { .. })
        ));
        assert!(matches!(
            compile("rising(feature('x'), 100000)"),
            Err(EvalError::BadHistoryDepth { .. })
        ));
    }

    #[test]
    fn a_cross_missing_its_comma_is_a_compile_error() {
        assert!(compile("crossed_above(feature('a') feature('b'))").is_err());
    }

    #[test]
    fn warmup_is_the_deepest_site_not_their_sum() {
        let prog =
            compile("(rising(feature('x'), 4)) + (crossed_above(feature('a'), feature('b')))")
                .unwrap();
        assert_eq!(prog.history_sites, [4, 1]);
        assert_eq!(prog.history_warmup_bars(), 4);
    }

    /// The stateless entry point does not degrade a cross into something that
    /// answers — it keeps saying it does not know.
    #[test]
    fn run_without_history_never_resolves_a_cross() {
        let prog = compile("crossed_above(feature('a'), feature('b'))").unwrap();
        let features = feats(&[("a", 2.0), ("b", 1.0)]);
        for _ in 0..5 {
            assert!(run(&prog, &features, &empty_bars()).is_unknown());
        }
    }

    #[test]
    fn run_results_match_interpreter() {
        use crate::interpreter::evaluate_condition;

        let expr = "feature('ema_7') * 2.0 > feature('ema_21') + 1.0";
        let prog = compile(expr).unwrap();
        let features = feats(&[("ema_7", 6.0), ("ema_21", 10.0)]);
        let bytecode_result = run(&prog, &features, &empty_bars()).unknown_as_false();
        let interp_result = evaluate_condition(expr, &features, &empty_bars());
        assert_eq!(bytecode_result, interp_result);
    }
}
