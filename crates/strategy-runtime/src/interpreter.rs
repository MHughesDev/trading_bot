//! Expression evaluator for the v1.0 strategy definition node graph.
//!
//! # Expression grammar (from nodes.rs, frozen at v1.0)
//!
//! ```text
//! expr       = comparison
//! comparison = term ( ( ">" | "<" | ">=" | "<=" | "==" | "!=" ) term )?
//! term       = factor ( ( "+" | "-" ) factor )*
//! factor     = unary ( ( "*" | "/" ) unary )*
//! unary      = "-" unary | primary
//! primary    = number | feature_call | bar_call | "(" expr ")"
//! feature_call = "feature" "(" "'" ident "'" ")"
//! bar_call     = "bar" "(" "'" field "'" ")"
//! ```
//!
//! All functions are pure; no I/O occurs during evaluation.

use std::collections::HashMap;

use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::nodes::{Node, NodeKind};
use rust_decimal::prelude::ToPrimitive;
use thiserror::Error;

use crate::history::{SignalVerdicts, Unknown, Unresolved, Verdict};

/// Errors produced by condition expression evaluation.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum EvalError {
    #[error("feature '{0}' not available")]
    FeatureNotFound(String),

    #[error("bar field '{0}' not available")]
    BarNotFound(String),

    #[error("unknown bar field '{0}'")]
    UnknownBarField(String),

    #[error("unknown identifier '{0}'")]
    UnknownIdent(String),

    #[error("unexpected token: {0}")]
    UnexpectedToken(String),

    #[error("unterminated string literal")]
    UnterminatedString,

    #[error("division by zero")]
    DivisionByZero,

    #[error("invalid number: '{0}'")]
    InvalidNumber(String),

    /// A history-bearing function reached an evaluator that has no register
    /// file to read an earlier bar from.
    ///
    /// Raised by name rather than falling through to [`Self::UnknownIdent`]:
    /// the function exists and compiles, it is *this* evaluator that cannot
    /// run it. Use [`SignalEvaluator`], which carries history across bars.
    #[error("'{0}' reads an earlier bar; evaluate it with SignalEvaluator, which carries history")]
    NeedsHistory(String),

    #[error("'{func}' needs a whole number of bars between 1 and {max}, not {given}")]
    BadHistoryDepth { func: String, given: f64, max: u16 },
}

/// Evaluate a condition expression string against feature values and bar data.
///
/// Returns `true` when the expression evaluates to a non-zero value.
/// Returns `false` (not an error) when a required feature or bar is absent —
/// the strategy simply does not trade on incomplete data.
pub fn evaluate_condition(
    expr: &str,
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
) -> bool {
    match eval_expr(expr, features, bars) {
        Ok(v) => v != 0.0,
        Err(_) => false,
    }
}

/// Evaluate all condition nodes; return the set of signal names that fire.
///
/// # This function cannot see a cross
///
/// It is stateless, so it has no previous bar to compare against and no way to
/// report that it had none: a node reading `crossed_above`, `crossed_below`,
/// `rising` or `falling` yields [`EvalError::NeedsHistory`], which collapses to
/// `false` here, and the signal is silently absent from the returned list.
///
/// Check [`nodes_need_history`] before using this, and use [`SignalEvaluator`]
/// when it answers true. `stateless_evaluate_signals_is_blind_to_a_cross` pins
/// the behaviour so that nobody has to discover it from a backtest that
/// reported no trades.
pub fn evaluate_signals(
    nodes: &[Node],
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
) -> Vec<String> {
    // Pass 1: evaluate conditions.
    let mut conditions: HashMap<&str, bool> = HashMap::new();
    for node in nodes {
        if let NodeKind::Condition { expr } = &node.kind {
            conditions.insert(&node.id, evaluate_condition(expr, features, bars));
        }
    }

    // Pass 2: collect emitted signals.
    nodes
        .iter()
        .filter_map(|node| {
            if let NodeKind::Signal { when, emit } = &node.kind {
                if conditions.get(when.as_str()).copied().unwrap_or(false) {
                    Some(emit.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

// ── History-aware graph evaluation ────────────────────────────────────────────

/// Every condition-bearing expression in a node graph.
fn condition_exprs(nodes: &[Node]) -> impl Iterator<Item = &str> {
    nodes.iter().filter_map(|node| match &node.kind {
        NodeKind::Condition { expr } | NodeKind::Filter { expr, .. } => Some(expr.as_str()),
        _ => None,
    })
}

/// Whether any node in this graph reads a bar other than the current one.
///
/// A driver that answers true here and then calls [`evaluate_signals`] is
/// under-counting entries without being told.
#[must_use]
pub fn nodes_need_history(nodes: &[Node]) -> bool {
    condition_exprs(nodes)
        .filter_map(|expr| crate::bytecode::compile(expr).ok())
        .any(|p| p.needs_history())
}

/// Bars of warm-up lead-in this graph needs **on top of** what its indicators
/// need to converge.
///
/// Zero for every strategy written against the frozen v1 grammar. One for a
/// `crossed_above`/`crossed_below`, `n` for a `rising(x, n)`, and the deepest
/// of them when a graph has several — depths do not add up, they overlap.
///
/// This is the number `backtest::requirements::derive_requirements` must add to
/// `DataRequirements::warmup_bars`. Without it the first tradable bar of every
/// window is one the strategy cannot answer on, and the study loses an entry
/// per fold without reporting it.
#[must_use]
pub fn nodes_history_warmup_bars(nodes: &[Node]) -> u16 {
    condition_exprs(nodes)
        .filter_map(|expr| crate::bytecode::compile(expr).ok())
        .map(|p| p.history_warmup_bars())
        .max()
        .unwrap_or(0)
}

/// Evaluates a v1 node graph bar by bar, carrying the history that
/// `crossed_above`, `crossed_below`, `rising` and `falling` read.
///
/// This is the stateful counterpart to [`evaluate_signals`], and the only one
/// of the two that can answer a cross. It differs in a second way that matters
/// more: it returns [`SignalVerdicts`], which separates "this signal did not
/// fire" from "nobody could say whether this signal fired". The leading bars of
/// every window land in the second category, and a driver that folds them into
/// the first under-counts entries at every window boundary.
///
/// # The bar clock is the caller's
///
/// [`evaluate`](Self::evaluate) takes `bar_seq`, and history rolls when it
/// increases. A per-bar replay passes the bar index. A live feed that publishes
/// one event per trade tick must pass the *bucket* of the declared timeframe,
/// not a per-event counter, or a cross would be measured between two trades
/// where the strategy declared minutes — the same unit mismatch
/// `exits::BarAggregator` exists to correct.
#[derive(Debug, Default)]
pub struct SignalEvaluator {
    /// node id → its compiled program and the base of its run of history
    /// sites. `None` when the expression did not compile.
    programs: HashMap<String, Option<(crate::bytecode::Program, usize)>>,
    history: crate::history::HistoryRegisters,
}

impl SignalEvaluator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An evaluator with every condition in `nodes` compiled and its history
    /// sites allocated, so [`warmup_bars`](Self::warmup_bars) is answerable
    /// before the first bar arrives.
    #[must_use]
    pub fn with_nodes(nodes: &[Node]) -> Self {
        let mut ev = Self::new();
        ev.prepare(nodes);
        ev
    }

    /// Compile any condition node not yet seen, in graph order.
    ///
    /// Order matters: site indices are handed out as programs are installed, so
    /// walking `nodes` rather than a `HashMap` is what keeps two runs of the
    /// same definition byte-identical.
    fn prepare(&mut self, nodes: &[Node]) {
        for node in nodes {
            let NodeKind::Condition { expr } = &node.kind else {
                continue;
            };
            if self.programs.contains_key(&node.id) {
                continue;
            }
            let entry = crate::bytecode::compile(expr).ok().map(|program| {
                let base = program.install_sites(&mut self.history);
                (program, base)
            });
            self.programs.insert(node.id.clone(), entry);
        }
    }

    /// Bars of history the graph needs before every condition can be answered.
    #[must_use]
    pub fn warmup_bars(&self) -> u16 {
        self.history.warmup_bars()
    }

    /// Whether any condition in this graph reads an earlier bar.
    #[must_use]
    pub fn needs_history(&self) -> bool {
        !self.history.is_empty()
    }

    /// The registers, for a caller that wants to inspect warm-up progress.
    #[must_use]
    pub const fn registers(&self) -> &crate::history::HistoryRegisters {
        &self.history
    }

    /// Evaluate every condition for the bar `bar_seq` identifies, then map the
    /// results onto signals.
    ///
    /// `bar_seq` must not decrease. If it does, the registers are cleared
    /// rather than read — a value written by a later bar is lookahead — and
    /// every history-bearing condition reports
    /// [`Unknown::OutOfOrderBar`](crate::history::Unknown::OutOfOrderBar) until
    /// they warm again.
    pub fn evaluate(
        &mut self,
        nodes: &[Node],
        features: &HashMap<String, f64>,
        bars: &HashMap<Timeframe, BarPayload>,
        bar_seq: i64,
    ) -> SignalVerdicts {
        self.prepare(nodes);
        self.history.open_bar(bar_seq);

        let mut conditions: HashMap<&str, Verdict> = HashMap::new();
        for node in nodes {
            if !matches!(node.kind, NodeKind::Condition { .. }) {
                continue;
            }
            let verdict = match self.programs.get(&node.id) {
                Some(Some((program, base))) => crate::bytecode::run_with_history(
                    program,
                    features,
                    bars,
                    &mut self.history,
                    *base,
                ),
                _ => Verdict::Unknown(Unknown::Malformed),
            };
            conditions.insert(&node.id, verdict);
        }

        let mut out = SignalVerdicts::default();
        for node in nodes {
            let NodeKind::Signal { when, emit } = &node.kind else {
                continue;
            };
            // A `when` naming a node that is not there is a broken definition,
            // not a quiet "no".
            let verdict = conditions
                .get(when.as_str())
                .copied()
                .unwrap_or(Verdict::Unknown(Unknown::Malformed));
            match verdict {
                Verdict::True => out.fired.push(emit.clone()),
                Verdict::False => {}
                Verdict::Unknown(reason) => out.unresolved.push(Unresolved {
                    signal: emit.clone(),
                    node: when.clone(),
                    reason,
                }),
            }
        }
        out
    }
}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum Tok {
    Num(f64),
    Ident(String),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    Gt,
    Lt,
    GtEq,
    LtEq,
    EqEq,
    BangEq,
    LParen,
    RParen,
    Comma,
    Eof,
}

pub(crate) fn tokenize(input: &str) -> Result<Vec<Tok>, EvalError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '0'..='9' | '.' => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s.parse().map_err(|_| EvalError::InvalidNumber(s))?;
                tokens.push(Tok::Num(n));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Tok::Ident(ident));
            }
            '\'' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(EvalError::UnterminatedString);
                }
                let s: String = chars[start..i].iter().collect();
                tokens.push(Tok::Str(s));
                i += 1;
            }
            '+' => {
                tokens.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Tok::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Tok::Slash);
                i += 1;
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::GtEq);
                    i += 2;
                } else {
                    tokens.push(Tok::Gt);
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::LtEq);
                    i += 2;
                } else {
                    tokens.push(Tok::Lt);
                    i += 1;
                }
            }
            '=' if i + 1 < chars.len() && chars[i + 1] == '=' => {
                tokens.push(Tok::EqEq);
                i += 2;
            }
            '!' if i + 1 < chars.len() && chars[i + 1] == '=' => {
                tokens.push(Tok::BangEq);
                i += 2;
            }
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Tok::Comma);
                i += 1;
            }
            c => return Err(EvalError::UnexpectedToken(c.to_string())),
        }
    }
    tokens.push(Tok::Eof);
    Ok(tokens)
}

// ── Recursive-descent parser/evaluator ───────────────────────────────────────

struct Parser<'d> {
    tokens: Vec<Tok>,
    pos: usize,
    features: &'d HashMap<String, f64>,
    bars: &'d HashMap<Timeframe, BarPayload>,
}

impl<'d> Parser<'d> {
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

    fn expect_str(&mut self) -> Result<String, EvalError> {
        match self.advance() {
            Tok::Str(s) => Ok(s),
            t => Err(EvalError::UnexpectedToken(format!("{t:?}"))),
        }
    }

    fn eval_expr(&mut self) -> Result<f64, EvalError> {
        self.eval_comparison()
    }

    fn eval_comparison(&mut self) -> Result<f64, EvalError> {
        let lhs = self.eval_term()?;
        let op = match self.peek() {
            Tok::Gt => Some(">"),
            Tok::Lt => Some("<"),
            Tok::GtEq => Some(">="),
            Tok::LtEq => Some("<="),
            Tok::EqEq => Some("=="),
            Tok::BangEq => Some("!="),
            _ => None,
        };
        let Some(op) = op else {
            return Ok(lhs);
        };
        self.advance();
        let rhs = self.eval_term()?;
        let result = match op {
            ">" => lhs > rhs,
            "<" => lhs < rhs,
            ">=" => lhs >= rhs,
            "<=" => lhs <= rhs,
            "==" => (lhs - rhs).abs() < f64::EPSILON,
            "!=" => (lhs - rhs).abs() >= f64::EPSILON,
            _ => unreachable!(),
        };
        Ok(if result { 1.0 } else { 0.0 })
    }

    fn eval_term(&mut self) -> Result<f64, EvalError> {
        let mut val = self.eval_factor()?;
        loop {
            match self.peek() {
                Tok::Plus => {
                    self.advance();
                    val += self.eval_factor()?;
                }
                Tok::Minus => {
                    self.advance();
                    val -= self.eval_factor()?;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn eval_factor(&mut self) -> Result<f64, EvalError> {
        let mut val = self.eval_unary()?;
        loop {
            match self.peek() {
                Tok::Star => {
                    self.advance();
                    val *= self.eval_unary()?;
                }
                Tok::Slash => {
                    self.advance();
                    let rhs = self.eval_unary()?;
                    if rhs == 0.0 {
                        return Err(EvalError::DivisionByZero);
                    }
                    val /= rhs;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn eval_unary(&mut self) -> Result<f64, EvalError> {
        if matches!(self.peek(), Tok::Minus) {
            self.advance();
            return Ok(-self.eval_unary()?);
        }
        self.eval_primary()
    }

    fn eval_primary(&mut self) -> Result<f64, EvalError> {
        match self.advance() {
            Tok::Num(n) => Ok(n),
            Tok::LParen => {
                let val = self.eval_expr()?;
                self.expect_rparen()?;
                Ok(val)
            }
            Tok::Ident(name) => self.eval_call(&name),
            other => Err(EvalError::UnexpectedToken(format!("{other:?}"))),
        }
    }

    fn eval_call(&mut self, name: &str) -> Result<f64, EvalError> {
        // Refused before the argument is even read. This evaluator has no
        // previous-bar register, and the one thing it must never do is guess
        // — a cross answered from the current bar alone is the level test,
        // which is a different claim about the market.
        if crate::bytecode::HISTORY_FUNCTIONS.contains(&name) {
            return Err(EvalError::NeedsHistory(name.to_owned()));
        }
        self.expect_lparen()?;
        let arg = self.expect_str()?;
        self.expect_rparen()?;

        match name {
            "feature" => self
                .features
                .get(&arg)
                .copied()
                .ok_or(EvalError::FeatureNotFound(arg)),
            "bar" => {
                let bar = self
                    .bars
                    .get(&Timeframe::Minutes1)
                    .ok_or_else(|| EvalError::BarNotFound(arg.clone()))?;
                decimal_bar_field(bar, &arg)
            }
            other => Err(EvalError::UnknownIdent(other.to_owned())),
        }
    }
}

fn decimal_bar_field(bar: &BarPayload, field: &str) -> Result<f64, EvalError> {
    let d = match field {
        "open" => bar.open.inner(),
        "high" => bar.high.inner(),
        "low" => bar.low.inner(),
        "close" => bar.close.inner(),
        "volume" => bar.volume.inner(),
        f => return Err(EvalError::UnknownBarField(f.to_owned())),
    };
    Ok(d.to_f64().unwrap_or(0.0))
}

fn eval_expr(
    input: &str,
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
) -> Result<f64, EvalError> {
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        features,
        bars,
    };
    parser.eval_expr()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn feats(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn empty_bars() -> HashMap<Timeframe, BarPayload> {
        HashMap::new()
    }

    #[test]
    fn ema_cross_true() {
        let features = feats(&[("ema_7", 11.0), ("ema_21", 10.0)]);
        assert!(evaluate_condition(
            "feature('ema_7') > feature('ema_21')",
            &features,
            &empty_bars()
        ));
    }

    #[test]
    fn ema_cross_false() {
        let features = feats(&[("ema_7", 9.0), ("ema_21", 10.0)]);
        assert!(!evaluate_condition(
            "feature('ema_7') > feature('ema_21')",
            &features,
            &empty_bars()
        ));
    }

    #[test]
    fn missing_feature_returns_false() {
        let features = feats(&[("ema_21", 10.0)]);
        assert!(!evaluate_condition(
            "feature('ema_7') > feature('ema_21')",
            &features,
            &empty_bars()
        ));
    }

    #[test]
    fn arithmetic_expression() {
        let features = feats(&[("ema_7", 10.0)]);
        assert!(evaluate_condition(
            "feature('ema_7') + 5.0 > 14.0",
            &features,
            &empty_bars()
        ));
    }

    #[test]
    fn bar_close_field() {
        use domain::money::{Price, Size};

        let mut bars = HashMap::new();
        let bar = domain::payloads::bar::BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str("100").unwrap(),
            Price::from_str("110").unwrap(),
            Price::from_str("95").unwrap(),
            Price::from_str("105").unwrap(),
            Size::from_str("500").unwrap(),
            200,
        );
        bars.insert(Timeframe::Minutes1, bar);
        assert!(evaluate_condition(
            "bar('close') > 100.0",
            &HashMap::new(),
            &bars
        ));
    }

    #[test]
    fn evaluate_signals_emits_on_true_condition() {
        let features = feats(&[("ema_7", 11.0), ("ema_21", 10.0)]);
        let nodes = vec![
            domain::strategy_def::nodes::Node {
                id: "n1".into(),
                kind: NodeKind::Condition {
                    expr: "feature('ema_7') > feature('ema_21')".into(),
                },
            },
            domain::strategy_def::nodes::Node {
                id: "n2".into(),
                kind: NodeKind::Signal {
                    when: "n1".into(),
                    emit: "long".into(),
                },
            },
        ];
        let signals = evaluate_signals(&nodes, &features, &empty_bars());
        assert_eq!(signals, ["long"]);
    }
}
