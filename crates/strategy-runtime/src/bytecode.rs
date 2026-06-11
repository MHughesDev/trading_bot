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

use std::collections::HashMap;

use domain::payloads::bar::{BarPayload, Timeframe};
use rust_decimal::prelude::ToPrimitive;

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
}

/// A compiled condition program.
///
/// `feature_names[i]` is the feature name for `LoadFeature(i)`.
/// `ops` is the postfix instruction sequence.
#[derive(Debug, Clone)]
pub struct Program {
    pub feature_names: Vec<String>,
    pub ops: Vec<Op>,
}

// ── Compiler ──────────────────────────────────────────────────────────────────

struct Compiler {
    tokens: Vec<Tok>,
    pos: usize,
    ops: Vec<Op>,
    feature_names: Vec<String>,
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
        let arg = self.expect_str()?;
        self.expect_rparen()?;

        match name {
            "feature" => {
                let slot = self.feature_slot(arg);
                self.ops.push(Op::LoadFeature(slot));
                Ok(())
            }
            "bar" => {
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
            other => Err(EvalError::UnknownIdent(other.to_owned())),
        }
    }
}

/// Compile an expression string into a `Program`.
///
/// Returns `Err` only for malformed expressions; missing features/bars are
/// handled at run time by returning `false`.
pub fn compile(expr: &str) -> Result<Program, EvalError> {
    let tokens = tokenize(expr)?;
    let mut compiler = Compiler {
        tokens,
        pos: 0,
        ops: Vec::new(),
        feature_names: Vec::new(),
    };
    compiler.compile_expr()?;
    Ok(Program {
        feature_names: compiler.feature_names,
        ops: compiler.ops,
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
    d.to_f64().unwrap_or(0.0)
}

/// Execute a compiled program against current feature and bar state.
///
/// Returns `true` when the program produces a non-zero result.
/// Returns `false` on missing feature/bar data, division by zero, or stack overflow.
pub fn run(
    program: &Program,
    features: &HashMap<String, f64>,
    bars: &HashMap<Timeframe, BarPayload>,
) -> bool {
    const STACK_DEPTH: usize = 32;
    let mut stack = [0.0_f64; STACK_DEPTH];
    let mut sp: usize = 0;

    for op in &program.ops {
        match op {
            Op::LoadFeature(slot) => {
                let name = &program.feature_names[*slot as usize];
                let val = match features.get(name) {
                    Some(&v) => v,
                    None => return false,
                };
                if sp >= STACK_DEPTH {
                    return false;
                }
                stack[sp] = val;
                sp += 1;
            }
            Op::LoadBarField(field) => {
                let bar = match bars.get(&Timeframe::Minutes1) {
                    Some(b) => b,
                    None => return false,
                };
                if sp >= STACK_DEPTH {
                    return false;
                }
                stack[sp] = bar_field_f64(bar, field);
                sp += 1;
            }
            Op::Const(v) => {
                if sp >= STACK_DEPTH {
                    return false;
                }
                stack[sp] = *v;
                sp += 1;
            }
            Op::Neg => {
                if sp == 0 {
                    return false;
                }
                stack[sp - 1] = -stack[sp - 1];
            }
            Op::Add => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] += b;
            }
            Op::Sub => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] -= b;
            }
            Op::Mul => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] *= b;
            }
            Op::Div => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                if b == 0.0 {
                    return false;
                }
                sp -= 1;
                stack[sp - 1] /= b;
            }
            Op::Gt => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] = if stack[sp - 1] > b { 1.0 } else { 0.0 };
            }
            Op::Lt => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] = if stack[sp - 1] < b { 1.0 } else { 0.0 };
            }
            Op::GtEq => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] = if stack[sp - 1] >= b { 1.0 } else { 0.0 };
            }
            Op::LtEq => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                stack[sp - 1] = if stack[sp - 1] <= b { 1.0 } else { 0.0 };
            }
            Op::EqEq => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                let a = stack[sp - 1];
                stack[sp - 1] = if (a - b).abs() < f64::EPSILON {
                    1.0
                } else {
                    0.0
                };
            }
            Op::BangEq => {
                if sp < 2 {
                    return false;
                }
                let b = stack[sp - 1];
                sp -= 1;
                let a = stack[sp - 1];
                stack[sp - 1] = if (a - b).abs() >= f64::EPSILON {
                    1.0
                } else {
                    0.0
                };
            }
        }
    }

    sp > 0 && stack[sp - 1] != 0.0
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

    #[test]
    fn compile_ema_cross() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        assert_eq!(prog.feature_names, ["ema_7", "ema_21"]);
        assert_eq!(prog.ops, [Op::LoadFeature(0), Op::LoadFeature(1), Op::Gt]);
    }

    #[test]
    fn run_ema_cross_true() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_7", 11.0), ("ema_21", 10.0)]);
        assert!(run(&prog, &features, &empty_bars()));
    }

    #[test]
    fn run_ema_cross_false() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_7", 9.0), ("ema_21", 10.0)]);
        assert!(!run(&prog, &features, &empty_bars()));
    }

    #[test]
    fn run_missing_feature_returns_false() {
        let prog = compile("feature('ema_7') > feature('ema_21')").unwrap();
        let features = feats(&[("ema_21", 10.0)]);
        assert!(!run(&prog, &features, &empty_bars()));
    }

    #[test]
    fn run_arithmetic_expression() {
        let prog = compile("feature('ema_7') + 5.0 > 14.0").unwrap();
        let features = feats(&[("ema_7", 10.0)]);
        assert!(run(&prog, &features, &empty_bars()));
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
        assert!(run(&prog, &HashMap::new(), &bars));
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

    #[test]
    fn run_results_match_interpreter() {
        use crate::interpreter::evaluate_condition;

        let expr = "feature('ema_7') * 2.0 > feature('ema_21') + 1.0";
        let prog = compile(expr).unwrap();
        let features = feats(&[("ema_7", 6.0), ("ema_21", 10.0)]);
        let bytecode_result = run(&prog, &features, &empty_bars());
        let interp_result = evaluate_condition(expr, &features, &empty_bars());
        assert_eq!(bytecode_result, interp_result);
    }
}
