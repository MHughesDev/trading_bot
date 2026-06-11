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
