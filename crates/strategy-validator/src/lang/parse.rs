//! SLv2 text → [`Program`].
//!
//! A hand-written recursive-descent parser over one clause per line. There is no
//! parser generator because the error messages *are* the feature (AGENT-011 P4):
//! every failure carries a line, a code, and a concrete edit, and a generated
//! parser's "unexpected token" is exactly the message a small model cannot act
//! on.
//!
//! Line structure is significant and deliberately so. A clause never spans lines,
//! so a model that gets one clause wrong loses one clause, and a constrained tier
//! can be handed one line at a time (§3, P7).

use std::fmt;

use features::registry;

use super::{ArithOp, Binding, CmpOp, Entry, ExitClause, ExitKind, Expr, ParamDecl, Program, RiskItem, Side, Size};
use crate::teach::{fix_for, see_for};

/// A refusal, with where and what to do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line in the submitted text.
    pub line: usize,
    /// Stable code, matched by [`crate::teach`].
    pub code: &'static str,
    pub message: String,
    /// The concrete edit that fixes it.
    pub fix: Option<String>,
    /// A guide anchor.
    pub see: Option<String>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: [{}] {}", self.line, self.code, self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " — fix: {fix}")?;
        }
        Ok(())
    }
}

impl ParseError {
    fn new(line: usize, code: &'static str, message: impl Into<String>) -> Self {
        Self { line, code, message: message.into(), fix: fix_for(code).map(str::to_string), see: see_for(code).map(str::to_string) }
    }

    fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

/// The longest expression that may be written inline before it must be bound as
/// an `input` (AGENT-011 §3). Not a style rule: an expression a human reviewer
/// cannot hold in their head is one nobody checks, and the model re-reads its
/// own strategy every turn.
pub const MAX_INLINE_EXPR: usize = 120;

/// Parse SLv2 text.
///
/// Every refusal is collected, not just the first: a model that has to fix one
/// error per round trip spends its budget on syntax.
///
/// # Errors
/// One [`ParseError`] per problem, in line order.
pub fn parse(text: &str) -> Result<Program, Vec<ParseError>> {
    let mut errors = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut prog = Program {
        strategy_id: String::new(),
        asset_class: String::new(),
        timeframe: String::new(),
        params: Vec::new(),
        inputs: Vec::new(),
        entries: Vec::new(),
        size: None,
        exits: Vec::new(),
        risk: Vec::new(),
        comments: Vec::new(),
    };

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            pending.push(c.trim().to_string());
            continue;
        }
        let comments = std::mem::take(&mut pending);
        let (keyword, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
        let rest = rest.trim();
        let outcome = match keyword {
            "strategy" => take_ident(line, rest, "strategy").map(|v| {
                prog.strategy_id = v;
                prog.comments = comments;
            }),
            "asset_class" => take_ident(line, rest, "asset_class").map(|v| prog.asset_class = v),
            "timeframe" => take_ident(line, rest, "timeframe").map(|v| prog.timeframe = v),
            "param" => parse_param(line, rest, comments).map(|p| prog.params.push(p)),
            "input" => parse_input(line, rest, comments, &prog).map(|b| prog.inputs.push(b)),
            "entry" => parse_entry(line, rest, comments, &prog).map(|e| prog.entries.push(e)),
            "size" => parse_size(line, rest, comments).map(|s| prog.size = Some(s)),
            "exit" => parse_exit(line, rest, comments).map(|x| prog.exits.push(x)),
            "risk" => parse_risk(line, rest, comments).map(|r| prog.risk.push(r)),
            other => Err(ParseError::new(line, "unknown_clause", format!("`{other}` does not start a clause")).with_fix(
                "every line starts with one of: strategy, asset_class, timeframe, param, input, entry, size, exit, risk, or # for a comment",
            )),
        };
        if let Err(e) = outcome {
            errors.push(e);
        }
    }

    // Trailing comments have nothing to attach to. Silently dropping them would
    // lose exactly the intent this language exists to keep.
    if !pending.is_empty() {
        errors.push(
            ParseError::new(
                text.lines().count(),
                "dangling_comment",
                "a comment at the end of the file has no clause to attach to; comments describe the clause that follows them",
            )
            .with_fix("move it above the clause it explains, or delete it"),
        );
    }

    for (field, name) in [(&prog.strategy_id, "strategy"), (&prog.asset_class, "asset_class"), (&prog.timeframe, "timeframe")] {
        if field.is_empty() {
            errors.push(ParseError::new(1, "missing_clause", format!("no `{name}` clause")).with_fix(format!("add a `{name} …` line")));
        }
    }
    if prog.entries.is_empty() {
        errors.push(ParseError::new(1, "missing_clause", "no `entry` clause — a strategy that never enters is not a strategy").with_fix("add `entry long when …`"));
    }
    if prog.exits.is_empty() {
        errors.push(
            ParseError::new(1, "missing_exits", "no `exit` clause. Silence and delegation used to look the same; now the choice is stated")
                .with_fix("add a real exit (`exit stop 2 %`), or `exit risk_layer_only` if the risk layer's limits are the whole exit logic"),
        );
    }
    if prog.exits.iter().any(|x| x.kind == ExitKind::RiskLayerOnly) && prog.exits.len() > 1 {
        errors.push(
            ParseError::new(1, "exits_contradict", "`exit risk_layer_only` says there are no exit rules, and another `exit` clause is one")
                .with_fix("delete `exit risk_layer_only`, or delete the other exit clauses"),
        );
    }

    errors.sort_by_key(|e| e.line);
    if errors.is_empty() {
        Ok(prog)
    } else {
        Err(errors)
    }
}

fn take_ident(line: usize, rest: &str, clause: &str) -> Result<String, ParseError> {
    let v = rest.trim();
    if v.is_empty() || v.split_whitespace().count() != 1 {
        return Err(ParseError::new(line, "malformed_clause", format!("`{clause}` takes exactly one name")).with_fix(format!("write `{clause} <name>`")));
    }
    Ok(v.to_string())
}

/// `param fast int 7 range 2 50`
fn parse_param(line: usize, rest: &str, comments: Vec<String>) -> Result<ParamDecl, ParseError> {
    let w: Vec<&str> = rest.split_whitespace().collect();
    let bad = || {
        ParseError::new(line, "malformed_param", "a param needs a name, a type, a default and a range")
            .with_fix("write `param <name> int|number <default> range <lo> <hi>`")
    };
    if w.len() != 6 || w[4..].len() != 2 || w[3] != "range" {
        return Err(bad());
    }
    if !matches!(w[1], "int" | "number") {
        return Err(ParseError::new(line, "malformed_param", format!("`{}` is not a param type", w[1])).with_fix("the types are `int` and `number`"));
    }
    for v in [w[2], w[4], w[5]] {
        if v.parse::<f64>().is_err() {
            return Err(ParseError::new(line, "malformed_param", format!("`{v}` is not a number")).with_fix("defaults and ranges are plain numbers"));
        }
    }
    let (lo, hi) = (w[4].parse::<f64>().unwrap_or(0.0), w[5].parse::<f64>().unwrap_or(0.0));
    let d = w[2].parse::<f64>().unwrap_or(0.0);
    if lo > hi {
        return Err(ParseError::new(line, "param_range_inverted", format!("range {lo} {hi} is empty")).with_fix("write the low bound first"));
    }
    if d < lo || d > hi {
        return Err(
            ParseError::new(line, "param_default_outside_range", format!("default {d} is outside [{lo}, {hi}]")).with_fix("move the default inside the range, or widen the range")
        );
    }
    Ok(ParamDecl { name: w[0].to_string(), ptype: w[1].to_string(), default: w[2].to_string(), min: w[4].to_string(), max: w[5].to_string(), comments })
}

/// `input trend = ema(close, 50)`
fn parse_input(line: usize, rest: &str, comments: Vec<String>, prog: &Program) -> Result<Binding, ParseError> {
    let Some((name, expr)) = rest.split_once('=') else {
        return Err(ParseError::new(line, "malformed_input", "an input binds a name to an expression").with_fix("write `input <name> = <expression>`"));
    };
    let name = name.trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ParseError::new(line, "malformed_input", format!("`{name}` is not a name")).with_fix("names are letters, digits and underscores"));
    }
    if prog.inputs.iter().any(|b| b.name == name) {
        return Err(ParseError::new(line, "duplicate_input", format!("`{name}` is already bound")).with_fix("give this one a different name"));
    }
    Ok(Binding { name, expr: parse_expr(line, expr.trim(), prog)?, comments })
}

/// `entry long when <cond>`
fn parse_entry(line: usize, rest: &str, comments: Vec<String>, prog: &Program) -> Result<Entry, ParseError> {
    let bad = || ParseError::new(line, "malformed_entry", "an entry names a side and a condition").with_fix("write `entry long when <condition>` or `entry short when <condition>`");
    let (side_word, after) = rest.split_once(char::is_whitespace).ok_or_else(bad)?;
    let side = match side_word {
        "long" => Side::Long,
        "short" => Side::Short,
        other => {
            return Err(ParseError::new(line, "malformed_entry", format!("`{other}` is not a side")).with_fix("the sides are `long` and `short`"));
        }
    };
    let cond = after.trim().strip_prefix("when").ok_or_else(bad)?.trim();
    if cond.len() > MAX_INLINE_EXPR {
        return Err(
            ParseError::new(line, "expression_too_long", format!("the condition is {} characters; the limit is {MAX_INLINE_EXPR}", cond.len()))
                .with_fix("bind part of it as an input: `input <name> = <part>`, then reference `<name>` here"),
        );
    }
    let when = parse_expr(line, cond, prog)?;
    check_crossing_placement(line, &when)?;
    Ok(Entry { side, when, comments })
}

/// `crossed_above` compiles to a plain comparison and takes its crossing meaning
/// from the signal node's rising edge (see the module docs). Inside `or` or
/// `not` the rising edge is no longer the crossing's, so the clause would mean
/// something other than it says — refused rather than compiled into a different
/// strategy. Inside `and` the rising edge is the conjunction's, which is the
/// usual intent ("cross, while the filter holds") and is allowed.
fn check_crossing_placement(line: usize, e: &Expr) -> Result<(), ParseError> {
    fn has_cross(e: &Expr) -> bool {
        match e {
            Expr::CrossedAbove(..) | Expr::CrossedBelow(..) => true,
            Expr::Not(a) | Expr::Neg(a) | Expr::Rising(a, _) | Expr::Falling(a, _) => has_cross(a),
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Arith { lhs: a, rhs: b, .. } | Expr::Cmp { lhs: a, rhs: b, .. } => has_cross(a) || has_cross(b),
            Expr::Between(x, a, b) => has_cross(x) || has_cross(a) || has_cross(b),
            _ => false,
        }
    }
    fn walk(e: &Expr, line: usize) -> Result<(), ParseError> {
        match e {
            Expr::Or(a, b) if has_cross(a) || has_cross(b) => Err(ParseError::new(
                line,
                "crossing_under_or",
                "`crossed_above`/`crossed_below` inside an `or` would not mean a crossing: the rising edge that makes it a crossing is taken on the whole entry condition",
            )
            .with_fix("write one `entry` clause per alternative, so each crossing owns its own edge")),
            Expr::Not(a) if has_cross(a) => Err(ParseError::new(line, "crossing_under_not", "`not crossed_above(…)` is not a crossing downwards")
                .with_fix("write `crossed_below(…)` instead")),
            Expr::And(a, b) => {
                walk(a, line)?;
                walk(b, line)
            }
            _ => Ok(()),
        }
    }
    walk(e, line)
}

/// `size fixed 0.01`
fn parse_size(line: usize, rest: &str, comments: Vec<String>) -> Result<Size, ParseError> {
    let w: Vec<&str> = rest.split_whitespace().collect();
    if w.len() != 2 || w[0] != "fixed" {
        return Err(
            ParseError::new(line, "unsupported_size", "v2.0 sizes a position as a fixed quantity")
                .with_fix("write `size fixed <quantity>`; equity_fraction, vol_target and kelly need equity at decision time, which the canonical order schema has no field for")
        );
    }
    if w[1].parse::<f64>().map(|v| v <= 0.0).unwrap_or(true) {
        return Err(ParseError::new(line, "unsupported_size", format!("`{}` is not a positive quantity", w[1])).with_fix("write a positive decimal, e.g. `size fixed 0.01`"));
    }
    Ok(Size { kind: "fixed".into(), value: w[1].to_string(), comments })
}

/// `exit stop 2 %` · `exit time 48 bars` · `exit on signal flatten` · `exit risk_layer_only`
fn parse_exit(line: usize, rest: &str, comments: Vec<String>) -> Result<ExitClause, ParseError> {
    let w: Vec<&str> = rest.split_whitespace().collect();
    let kind = match w.as_slice() {
        ["risk_layer_only"] => ExitKind::RiskLayerOnly,
        ["on", "signal", name] => ExitKind::OnSignal((*name).to_string()),
        ["time", n, "bars"] => {
            let bars = n.parse::<u32>().map_err(|_| {
                ParseError::new(line, "malformed_exit", format!("`{n}` is not a whole number of bars")).with_fix("write `exit time 48 bars`")
            })?;
            if bars == 0 {
                return Err(ParseError::new(line, "exit_distance_not_positive", "a time exit of 0 bars closes on the entry bar").with_fix("use at least 1 bar"));
            }
            ExitKind::Time(bars)
        }
        [word @ ("stop" | "target" | "trail"), v, unit] => {
            let pct = v.parse::<f64>().map_err(|_| {
                ParseError::new(line, "malformed_exit", format!("`{v}` is not a number")).with_fix(format!("write `exit {word} 1.5 %`"))
            })?;
            match *unit {
                "%" => {}
                "atr" | "sigma" => {
                    return Err(ParseError::new(
                        line,
                        "unit_not_representable",
                        format!("`{unit}` distances cannot be stored: an exit rule at definition_version 1.3 carries a fraction of the entry price and nothing else (ADR-P6-01, frozen)"),
                    )
                    .with_fix(format!("write the distance in percent — `exit {word} 1.5 %` — or express the volatility scaling as a swept `param`")));
                }
                other => {
                    return Err(ParseError::new(line, "unknown_unit", format!("`{other}` is not a unit")).with_fix("the unit of an exit distance is `%`"));
                }
            }
            if pct <= 0.0 || pct >= 100.0 {
                return Err(
                    ParseError::new(line, "exit_distance_not_positive", format!("{pct} % is outside (0, 100)"))
                        .with_fix("a stop at 0 % fires on the entry bar and one at 100 % never fires; write something in between"),
                );
            }
            match *word {
                "stop" => ExitKind::Stop((*v).to_string()),
                "target" => ExitKind::Target((*v).to_string()),
                _ => ExitKind::Trail((*v).to_string()),
            }
        }
        [word @ ("stop" | "target" | "trail"), v] => {
            return Err(
                ParseError::new(line, "distance_without_unit", format!("`exit {word} {v}` has no unit"))
                    .with_fix(format!("write `exit {word} {v} %` — the unit is never implied, because `1.5` could be 1.5 percent or 1.5 times anything")),
            );
        }
        _ => {
            return Err(ParseError::new(line, "malformed_exit", format!("`exit {rest}` is not an exit clause")).with_fix(
                "the exits are: `exit stop <n> %`, `exit target <n> %`, `exit trail <n> %`, `exit time <n> bars`, `exit on signal <name>`, `exit risk_layer_only`",
            ));
        }
    };
    Ok(ExitClause { kind, comments })
}

/// `risk max_position 0.5`
fn parse_risk(line: usize, rest: &str, comments: Vec<String>) -> Result<RiskItem, ParseError> {
    let w: Vec<&str> = rest.split_whitespace().collect();
    const ITEMS: [&str; 3] = ["max_position", "max_orders_per_min", "max_daily_loss"];
    if w.len() != 2 || !ITEMS.contains(&w[0]) {
        return Err(ParseError::new(line, "malformed_risk", "a risk clause names one limit and its value")
            .with_fix(format!("the limits are {}; write `risk max_position 0.5`", ITEMS.join(", "))));
    }
    if w[1].parse::<f64>().is_err() {
        return Err(ParseError::new(line, "malformed_risk", format!("`{}` is not a number", w[1])).with_fix("risk limits are plain numbers"));
    }
    Ok(RiskItem { name: w[0].to_string(), value: w[1].to_string(), comments })
}

// ── expression parser ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

fn lex(line: usize, s: &str) -> Result<Vec<Tok>, ParseError> {
    let ch: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && ch.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            while i < ch.len() && (ch[i].is_ascii_digit() || ch[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(ch[start..i].iter().collect()));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < ch.len() && (ch[i].is_ascii_alphanumeric() || ch[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(ch[start..i].iter().collect()));
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if "<>=!+-*/".contains(c) {
            let two: String = ch[i..(i + 2).min(ch.len())].iter().collect();
            if ["<=", ">=", "==", "!="].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if c == '=' || c == '!' {
                return Err(ParseError::new(line, "bad_operator", format!("`{c}` on its own is not an operator"))
                    .with_fix("equality is `==` and inequality is `!=`; SLv2 has no assignment inside an expression"));
            } else {
                out.push(Tok::Op(c.to_string()));
                i += 1;
            }
        } else {
            return Err(ParseError::new(line, "bad_character", format!("`{c}` has no meaning in an expression"))
                .with_fix("expressions use numbers, feature calls, the bar fields, comparisons, and `and`/`or`/`not`"));
        }
    }
    Ok(out)
}

struct P<'a> {
    toks: Vec<Tok>,
    pos: usize,
    line: usize,
    prog: &'a Program,
}

/// Parse an expression or condition.
fn parse_expr(line: usize, s: &str, prog: &Program) -> Result<Expr, ParseError> {
    if s.is_empty() {
        return Err(ParseError::new(line, "empty_expression", "there is no expression here").with_fix("write a condition, e.g. `rsi(close, 14) < 30`"));
    }
    let mut p = P { toks: lex(line, s)?, pos: 0, line, prog };
    let e = p.or_expr()?;
    if p.pos != p.toks.len() {
        return Err(ParseError::new(line, "trailing_tokens", format!("`{s}` has more after the expression ends")).with_fix("check for a missing operator or an extra bracket"));
    }
    Ok(e)
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat_ident(&mut self, want: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(i)) if i == want) {
            self.pos += 1;
            return true;
        }
        false
    }

    fn err(&self, code: &'static str, msg: impl Into<String>) -> ParseError {
        ParseError::new(self.line, code, msg)
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.and_expr()?;
        while self.eat_ident("or") {
            lhs = Expr::Or(Box::new(lhs), Box::new(self.and_expr()?));
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.not_expr()?;
        while self.eat_ident("and") {
            lhs = Expr::And(Box::new(lhs), Box::new(self.not_expr()?));
        }
        Ok(lhs)
    }

    fn not_expr(&mut self) -> Result<Expr, ParseError> {
        if self.eat_ident("not") {
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.cmp_expr()
    }

    fn cmp_expr(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.sum()?;
        let op = match self.peek() {
            Some(Tok::Op(o)) => match o.as_str() {
                ">" => Some(CmpOp::Gt),
                "<" => Some(CmpOp::Lt),
                ">=" => Some(CmpOp::Ge),
                "<=" => Some(CmpOp::Le),
                "==" => Some(CmpOp::Eq),
                "!=" => Some(CmpOp::Ne),
                _ => None,
            },
            _ => None,
        };
        let Some(op) = op else { return Ok(lhs) };
        self.pos += 1;
        Ok(Expr::Cmp { op, lhs: Box::new(lhs), rhs: Box::new(self.sum()?) })
    }

    fn sum(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.product()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if o == "+" => ArithOp::Add,
                Some(Tok::Op(o)) if o == "-" => ArithOp::Sub,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            lhs = Expr::Arith { op, lhs: Box::new(lhs), rhs: Box::new(self.product()?) };
        }
    }

    fn product(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if o == "*" => ArithOp::Mul,
                Some(Tok::Op(o)) if o == "/" => ArithOp::Div,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            lhs = Expr::Arith { op, lhs: Box::new(lhs), rhs: Box::new(self.unary()?) };
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::Op(o)) if o == "-") {
            self.pos += 1;
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), ParseError> {
        if self.peek() == Some(t) {
            self.pos += 1;
            return Ok(());
        }
        Err(self.err("malformed_call", format!("expected {what}")).with_fix("check the brackets and commas"))
    }

    fn arg(&mut self) -> Result<Expr, ParseError> {
        self.or_expr()
    }

    fn int_arg(&mut self, fname: &str) -> Result<usize, ParseError> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) if !n.contains('.') => {
                self.pos += 1;
                n.parse().map_err(|_| self.err("malformed_call", format!("`{n}` is not a whole number")))
            }
            other => Err(self
                .err("malformed_call", format!("`{fname}` wants a whole number here, got {other:?}"))
                .with_fix(format!("write `{fname}(close, 20)`"))),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let e = self.or_expr()?;
                self.expect(&Tok::RParen, "a closing bracket")?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                let called = self.peek() == Some(&Tok::LParen);
                if !called {
                    return self.bare_ident(&name);
                }
                self.pos += 1;
                let e = self.call(&name)?;
                self.expect(&Tok::RParen, format!("a closing bracket for `{name}`").as_str())?;
                Ok(e)
            }
            other => Err(self.err("malformed_expression", format!("expected a value, found {other:?}"))),
        }
    }

    fn bare_ident(&mut self, name: &str) -> Result<Expr, ParseError> {
        if ["open", "high", "low", "close", "volume"].contains(&name) {
            return Ok(Expr::Bar(name.to_string()));
        }
        if self.prog.inputs.iter().any(|b| b.name == name) {
            return Ok(Expr::Input(name.to_string()));
        }
        if self.prog.params.iter().any(|p| p.name == name) {
            return Ok(Expr::Param(name.to_string()));
        }
        // A nullary catalogued family, e.g. `obv`, `hour_sin`, `utc_session_us`.
        if let Some(r) = registry::resolve(name) {
            if r.params().is_empty() {
                return Ok(Expr::Feature { family: r.family.name.to_string(), params: vec![] });
            }
        }
        if let Some(f) = registry::family(name) {
            return Err(self
                .err("feature_needs_parameters", format!("`{name}` takes {} parameter(s)", f.params.len()))
                .with_fix(format!("write `{}`", f.example().replace('_', "(close, ").replacen("(close, ", "(close, ", 1))));
        }
        Err(unknown_feature(self.line, name))
    }

    #[allow(clippy::too_many_lines)]
    fn call(&mut self, name: &str) -> Result<Expr, ParseError> {
        match name {
            "crossed_above" | "crossed_below" => {
                let a = self.arg()?;
                self.expect(&Tok::Comma, "a comma")?;
                let b = self.arg()?;
                Ok(if name == "crossed_above" { Expr::CrossedAbove(Box::new(a), Box::new(b)) } else { Expr::CrossedBelow(Box::new(a), Box::new(b)) })
            }
            "between" => {
                let x = self.arg()?;
                self.expect(&Tok::Comma, "a comma")?;
                let lo = self.arg()?;
                self.expect(&Tok::Comma, "a comma")?;
                let hi = self.arg()?;
                Ok(Expr::Between(Box::new(x), Box::new(lo), Box::new(hi)))
            }
            "rising" | "falling" => {
                let x = self.arg()?;
                self.expect(&Tok::Comma, "a comma")?;
                let n = self.int_arg(name)?;
                if !matches!(x, Expr::Bar(ref f) if f == "close") {
                    return Err(self
                        .err("rising_needs_close", format!("`{name}` can only be taken of `close`"))
                        .with_fix("write `rising(close, 5)`; a lagged copy of an arbitrary feature is not a catalogued feature, so nothing could declare its lookback"));
                }
                if registry::resolve(&format!("lag_{n}")).is_none() {
                    return Err(self.err("rising_needs_close", format!("a lag of {n} bars is outside the catalogue")).with_fix("use a smaller n"));
                }
                Ok(if name == "rising" { Expr::Rising(Box::new(x), n) } else { Expr::Falling(Box::new(x), n) })
            }
            "param" => match self.peek().cloned() {
                Some(Tok::Ident(p)) => {
                    self.pos += 1;
                    if !self.prog.params.iter().any(|d| d.name == p) {
                        return Err(self
                            .err("unknown_param", format!("`{p}` is not a declared parameter"))
                            .with_fix(format!("declare it first: `param {p} number 1 range 0 10`")));
                    }
                    Ok(Expr::Param(p))
                }
                _ => Err(self.err("malformed_call", "`param` names a declared parameter").with_fix("write `param(fast)`")),
            },
            "abs" | "min" | "max" | "clip" | "log" | "exp" | "sqrt" | "sign" => Err(self
                .err("function_not_representable", format!("`{name}` has no form in the canonical expression grammar, which is frozen"))
                .with_fix("rewrite the rule without it — a two-sided band is `between(x, lo, hi)`, and a clamp is usually a comparison — or compute it as a catalogued feature")),
            "flat" | "long" | "short" | "bars_in_trade" | "unrealized_return" | "last_exit_reason" | "position" | "entry_price" => Err(self
                .err("state_predicate_in_condition", format!("`{name}` is position state, and an entry condition is a function of the bar alone"))
                .with_fix("put the position-dependent part in an `exit` clause: stop, target, trail and time are evaluated against position state by the runtime")),
            _ => {
                // A catalogued family: `ema(close, 7)`.
                let Some(f) = registry::family(name) else {
                    return Err(unknown_feature(self.line, name));
                };
                let src = match self.peek().cloned() {
                    Some(Tok::Ident(s)) => {
                        self.pos += 1;
                        s
                    }
                    _ => {
                        return Err(self
                            .err("malformed_call", format!("`{name}` names its source first"))
                            .with_fix(format!("write `{name}(close, …)`")));
                    }
                };
                if src != "close" {
                    return Err(self
                        .err("source_not_close", format!("`{name}({src}, …)` reads a source other than the close"))
                        .with_fix("every catalogued family is defined on the close; families that need the high or the low have their own names (atr, donchian_upper, parkinson_vol)"));
                }
                let mut params = Vec::new();
                for i in 0..f.params.len() {
                    self.expect(&Tok::Comma, "a comma")?;
                    let v = self.int_arg(name)?;
                    let spec = f.params[i];
                    if v < spec.min || v > spec.max {
                        return Err(self
                            .err("parameter_out_of_range", format!("`{name}` parameter `{}` is {v}, outside [{}, {}]", spec.name, spec.min, spec.max))
                            .with_fix(format!("choose a value in [{}, {}]", spec.min, spec.max)));
                    }
                    params.push(v);
                }
                if f.params.is_empty() {
                    return Err(self.err("malformed_call", format!("`{name}` takes no parameters")).with_fix(format!("write `{name}` on its own")));
                }
                Ok(Expr::Feature { family: f.alias_of.unwrap_or(f.name).to_string(), params })
            }
        }
    }
}

/// The `unknown_feature` refusal, with the catalogue's nearest names.
fn unknown_feature(line: usize, name: &str) -> ParseError {
    let near: Vec<String> = registry::suggestions(name)
        .iter()
        .map(|f| if f.params.is_empty() { f.name.to_string() } else { format!("{}(close, {})", f.name, f.example_params().iter().map(usize::to_string).collect::<Vec<_>>().join(", ")) })
        .collect();
    let fix = if near.is_empty() {
        "call list_features to see what exists".to_string()
    } else {
        format!("did you mean {}? list_features(query=\"{name}\")", near.join(", "))
    };
    ParseError::new(line, "unknown_feature", format!("`{name}` is not a feature, an input or a parameter")).with_fix(fix)
}
