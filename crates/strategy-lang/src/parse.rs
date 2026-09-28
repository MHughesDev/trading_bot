//! SLv2 text → a syntax tree, or every reason it is not one.
//!
//! # The parser reports every clause, not the first
//!
//! A model pays for a round trip out of the same budget as its research, so a
//! parser that stops at the first error charges it once per mistake. Clauses are
//! newline-terminated (see [`crate::lex`]), which gives a recovery point that
//! needs no heuristics: on an error the parser records it, skips to the next
//! line, and carries on. Three malformed clauses come back as three
//! diagnostics.
//!
//! # The two legibility rules are here, not in the compiler
//!
//! AGENT-011 §3 makes `inputs` mandatory "for anything used twice" and refuses
//! "an expression longer than 120 characters inline". Both are rules about the
//! *text* — they are checked by measuring source spans, and nothing about the
//! canonical form is involved — so they live with the text. They are also the
//! two rules that exist for the reader rather than for the machine: a rule that
//! compiles fine and reads badly is exactly the rule a compiler forgets to
//! enforce.
//!
//! ## What "an expression" means for the 120-character rule
//!
//! The rule applies to each **maximal non-logical subexpression**, not to the
//! whole condition. Two reasons. It keeps the rule's purpose — one unreadable
//! monster must be given a name — while leaving a conjunction of four legible
//! filters alone, which is the shape the research protocol asks for and the
//! shape the blocking profile (AGENT-011 §4) reads clause by clause. And it
//! keeps [`crate::decompile()`] total: a stored strategy with five short filters
//! must have a text form, and a whole-condition rule would refuse to parse the
//! text this crate had just written.

use std::collections::HashMap;

// The bounds the feature engine computes a rolling statistic over, read from
// `domain` rather than spelled again: a second copy of `2..=100_000` in the
// parser would drift from the one the validator and the runtime read.
use domain::strategy_def::exits::{MAX_UNIT_PERIOD, MIN_UNIT_PERIOD};

use crate::ast::{
    Anchor, Binding, ConstraintClause, Distance, Entry, Exit, ExitKind, Expr, Num, OrderClause,
    OrderType, ParamDecl, ParamKind, RiskItem, RiskKind, Side, SizeClause, SizeKind, Strategy,
    Unit,
};
use crate::diagnostic::{Code, Diagnostic, Expected, Span};
use crate::lex::{lex, Comment, Spanned, Tok};

/// The longest expression that may be written inline rather than bound to a
/// name (AGENT-011 §3).
pub const MAX_INLINE_EXPR: usize = 120;

/// The clause keywords, in the order the guide introduces them. Used for the
/// `expected` list on an unknown clause, so the answer to "what can I write
/// here" is always in the error.
pub const CLAUSES: &[&str] = &[
    "strategy",
    "version",
    "asset_class",
    "trust",
    "param",
    "input",
    "entry",
    "size",
    "exit",
    "order",
    "risk",
    "constraint",
];

/// Parse SLv2 text.
///
/// # Errors
///
/// Returns every diagnostic the text produced, in source order. The list is
/// never empty when it is returned.
pub fn parse(source: &str) -> Result<Strategy, Vec<Diagnostic>> {
    let lexed = lex(source)?;
    let mut p = Parser {
        source,
        tokens: lexed.tokens,
        comments: lexed.comments,
        pos: 0,
        errors: Vec::new(),
        next_comment: 0,
    };
    let strategy = p.program();
    if p.errors.is_empty() {
        let mut strategy = strategy.expect("no errors implies a strategy");
        let lints = lint(source, &strategy);
        if lints.is_empty() {
            strategy.constraints.shrink_to_fit();
            Ok(strategy)
        } else {
            Err(lints)
        }
    } else {
        Err(p.errors)
    }
}

/// Parse one expression on its own, with no surrounding clause.
///
/// This is how [`crate::decompile()`] reads a stored `Condition` node back: the v1
/// expression grammar is a subset of SLv2's, so the same parser reads both and
/// there is no second expression grammar in the repository to drift from this
/// one.
///
/// # Errors
///
/// The first reason the text is not one expression.
pub fn expr_from_str(source: &str) -> Result<crate::ast::Expr, Diagnostic> {
    let lexed = lex(source).map_err(|mut e| e.remove(0))?;
    let mut p = Parser {
        source,
        tokens: lexed.tokens,
        comments: Vec::new(),
        pos: 0,
        errors: Vec::new(),
        next_comment: 0,
    };
    let e = p.expr()?;
    match p.peek() {
        Tok::Newline | Tok::Eof => Ok(e),
        other => {
            let other = other.clone();
            Err(p.unexpected(&other, Expected::one("the end of the expression")))
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Spanned>,
    comments: Vec<Comment>,
    pos: usize,
    errors: Vec<Diagnostic>,
    next_comment: usize,
}

type Parsed<T> = Result<T, Diagnostic>;

impl<'a> Parser<'a> {
    // ── token access ─────────────────────────────────────────────────────────

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn bump(&mut self) -> Spanned {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn at_ident(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == word)
    }

    fn eat_ident(&mut self, expected: Expected) -> Parsed<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                let span = self.bump().span;
                Ok((s, span))
            }
            other => Err(self.unexpected(&other, expected)),
        }
    }

    fn expect_kw(&mut self, word: &'static str) -> Parsed<Span> {
        if self.at_ident(word) {
            Ok(self.bump().span)
        } else {
            let found = self.peek().clone();
            Err(self.unexpected(&found, Expected::one(format!("`{word}`"))))
        }
    }

    fn expect(&mut self, tok: &Tok, expected: Expected) -> Parsed<Span> {
        if self.peek() == tok {
            Ok(self.bump().span)
        } else {
            let found = self.peek().clone();
            Err(self.unexpected(&found, expected))
        }
    }

    fn eat_num(&mut self, expected: Expected) -> Parsed<Num> {
        match self.peek().clone() {
            Tok::Num(raw) => {
                let span = self.bump().span;
                Ok(Num { raw, span })
            }
            Tok::Minus => {
                let minus = self.bump().span;
                match self.peek().clone() {
                    Tok::Num(raw) => {
                        let span = self.bump().span;
                        Ok(Num {
                            raw: format!("-{raw}"),
                            span: minus.to(span),
                        })
                    }
                    other => Err(self.unexpected(&other, expected)),
                }
            }
            other => Err(self.unexpected(&other, expected)),
        }
    }

    fn unexpected(&self, found: &Tok, expected: Expected) -> Diagnostic {
        Diagnostic::syntax(
            Code::UnexpectedToken,
            self.span(),
            format!("found {}, expected {}", found.describe(), expected.joined()),
            expected,
        )
    }

    fn end_of_clause(&mut self) -> Parsed<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::Eof => Ok(()),
            other => {
                let other = other.clone();
                Err(self
                    .unexpected(&other, Expected::one("the end of the line"))
                    .with_fix("every clause is one line; start a new line for the next one"))
            }
        }
    }

    /// Advance past the rest of the current line, so one bad clause costs one
    /// diagnostic rather than derailing every clause after it.
    fn recover(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Newline => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// The comments written above the line a clause starts on.
    fn take_comments(&mut self, clause_line: u32) -> Vec<String> {
        let mut out = Vec::new();
        while self.next_comment < self.comments.len() {
            let c = &self.comments[self.next_comment];
            if c.line < clause_line && !c.trailing {
                out.push(c.text.clone());
                self.next_comment += 1;
            } else if c.line < clause_line {
                // A comment trailing an earlier clause: consumed, not attached.
                self.next_comment += 1;
            } else {
                break;
            }
        }
        out
    }

    // ── the program ──────────────────────────────────────────────────────────

    fn program(&mut self) -> Option<Strategy> {
        let mut strategy = match self.header() {
            Ok(s) => s,
            Err(d) => {
                self.errors.push(d);
                return None;
            }
        };

        while !matches!(self.peek(), Tok::Eof) {
            let line = self.span().line;
            let comments = self.take_comments(line);
            if let Err(d) = self.clause(&mut strategy, comments) {
                self.errors.push(d);
                self.recover();
            }
        }
        Some(strategy)
    }

    fn header(&mut self) -> Parsed<Strategy> {
        let line = self.span().line;
        let header_comments = self.take_comments(line);
        if !self.at_ident("strategy") {
            let found = self.peek().clone();
            return Err(Diagnostic::syntax(
                Code::MissingHeader,
                self.span(),
                format!(
                    "a strategy starts with its header; found {}",
                    found.describe()
                ),
                Expected::one("`strategy <name> on <instrument> timeframe <tf>`"),
            )
            .with_fix("write `strategy my_idea on $bound_at_init timeframe 1h` first")
            .with_see("guide#grammar.header"));
        }
        let start = self.bump().span;
        let (name, name_span) = self.eat_ident(Expected::one("a name for the strategy"))?;
        self.expect_kw("on")?;
        let instrument = match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                s
            }
            Tok::Str(s) => {
                self.bump();
                s
            }
            other => {
                return Err(self
                    .unexpected(
                        &other,
                        Expected::any([
                            "`$bound_at_init`, so the definition is reusable",
                            "a quoted instrument such as `'BTC-USD'`",
                        ])
                        .expect("two alternatives"),
                    )
                    .with_see("guide#grammar.header"))
            }
        };
        self.expect_kw("timeframe")?;
        let (timeframe, tf_span) = match self.peek().clone() {
            Tok::Word(s) | Tok::Ident(s) => {
                let sp = self.bump().span;
                (s, sp)
            }
            other => {
                return Err(self.unexpected(
                    &other,
                    Expected::one("a timeframe such as `1m`, `5m`, `1h` or `1d`"),
                ))
            }
        };
        self.end_of_clause()?;

        Ok(Strategy {
            name,
            name_span,
            instrument,
            timeframe,
            header_span: start.to(tf_span),
            header_comments,
            version: None,
            asset_class: None,
            trust: None,
            params: Vec::new(),
            inputs: Vec::new(),
            entries: Vec::new(),
            size: None,
            exits: Vec::new(),
            orders: Vec::new(),
            risk: Vec::new(),
            constraints: Vec::new(),
        })
    }

    fn clause(&mut self, s: &mut Strategy, comments: Vec<String>) -> Parsed<()> {
        let Tok::Ident(word) = self.peek().clone() else {
            let found = self.peek().clone();
            return Err(self
                .unexpected(
                    &found,
                    Expected::any(CLAUSES.iter().map(|c| format!("`{c}`")))
                        .expect("CLAUSES is not empty"),
                )
                .with_fix("every clause begins with its keyword"));
        };

        match word.as_str() {
            "strategy" => Err(Diagnostic::semantic(
                Code::UnknownClause,
                self.span(),
                "a second `strategy` header: SLv2 describes one strategy per document",
            )
            .with_fix("delete this line, or split the document in two")),
            "version" => {
                self.bump();
                let n = self.eat_num(Expected::one("a definition version such as `1.3`"))?;
                s.version = Some((n.raw, n.span));
                self.end_of_clause()
            }
            "asset_class" => {
                self.bump();
                let (v, sp) =
                    self.eat_ident(Expected::one("an asset class such as `crypto_spot_cex`"))?;
                s.asset_class = Some((v, sp));
                self.end_of_clause()
            }
            "trust" => {
                self.bump();
                let (v, sp) = self.eat_ident(Expected::one(
                    "a trust tier such as `centralized_exchange` or `regulated`",
                ))?;
                s.trust = Some((v, sp));
                self.end_of_clause()
            }
            "param" => {
                let d = self.param_decl(comments)?;
                s.params.push(d);
                Ok(())
            }
            "input" => {
                let b = self.binding(comments)?;
                s.inputs.push(b);
                Ok(())
            }
            "entry" => {
                let e = self.entry(comments)?;
                s.entries.push(e);
                Ok(())
            }
            "size" => {
                let c = self.size(comments)?;
                s.size = Some(c);
                Ok(())
            }
            "exit" => {
                let e = self.exit(comments)?;
                s.exits.push(e);
                Ok(())
            }
            "order" => {
                let o = self.order(comments)?;
                s.orders.push(o);
                Ok(())
            }
            "risk" => {
                let r = self.risk(comments)?;
                s.risk.push(r);
                Ok(())
            }
            "constraint" => {
                let c = self.constraint(comments)?;
                s.constraints.push(c);
                Ok(())
            }
            other => Err(Diagnostic::syntax(
                Code::UnknownClause,
                self.span(),
                format!("`{other}` is not a clause keyword"),
                Expected::any(CLAUSES.iter().map(|c| format!("`{c}`")))
                    .expect("CLAUSES is not empty"),
            )
            .with_fix("every clause begins with one of the keywords above")
            .with_see("guide#grammar")),
        }
    }

    // ── clauses ──────────────────────────────────────────────────────────────

    fn param_decl(&mut self, comments: Vec<String>) -> Parsed<ParamDecl> {
        let start = self.expect_kw("param")?;
        let (name, _) = self.eat_ident(Expected::one("a parameter name"))?;
        self.expect(&Tok::Colon, Expected::one("`:` then the parameter's type"))?;
        let (ty, ty_span) = self.eat_ident(
            Expected::any(["`int`", "`float`", "`enum`"]).expect("three alternatives"),
        )?;

        let kind = match ty.as_str() {
            "int" | "float" => {
                self.expect(&Tok::LBracket, Expected::one("`[` then the range"))?;
                let min = self.eat_num(Expected::one("the lowest value the sweep may use"))?;
                self.expect(&Tok::DotDot, Expected::one("`..`"))?;
                let max = self.eat_num(Expected::one("the highest value the sweep may use"))?;
                let mut step = None;
                let mut log = false;
                if self.at_ident("step") {
                    self.bump();
                    step = Some(self.eat_num(Expected::one("the sweep step"))?);
                } else if self.at_ident("log") {
                    self.bump();
                    log = true;
                }
                self.expect(&Tok::RBracket, Expected::one("`]`"))?;
                self.expect(&Tok::Assign, Expected::one("`=` then the default"))?;
                let default = self.eat_num(Expected::one("the default value"))?;
                if ty == "int" {
                    ParamKind::Int {
                        min,
                        max,
                        step,
                        default,
                    }
                } else {
                    ParamKind::Float {
                        min,
                        max,
                        log,
                        default,
                    }
                }
            }
            "enum" => {
                self.expect(&Tok::LBracket, Expected::one("`[` then the choices"))?;
                let mut choices = Vec::new();
                loop {
                    let (c, _) = self.eat_ident(Expected::one("a choice"))?;
                    choices.push(c);
                    if matches!(self.peek(), Tok::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
                self.expect(&Tok::RBracket, Expected::one("`]`"))?;
                self.expect(&Tok::Assign, Expected::one("`=` then the default"))?;
                let (default, _) = self.eat_ident(Expected::one("one of the choices"))?;
                ParamKind::Enum { choices, default }
            }
            other => {
                return Err(Diagnostic::syntax(
                    Code::UnexpectedToken,
                    ty_span,
                    format!("`{other}` is not a parameter type"),
                    Expected::any(["`int`", "`float`", "`enum`"]).expect("three alternatives"),
                )
                .with_fix("write `param n: int[5..50] = 20`")
                .with_see("guide#params"))
            }
        };
        let end = self.span();
        self.end_of_clause()?;
        Ok(ParamDecl {
            name,
            kind,
            span: start.to(end),
            comments,
        })
    }

    fn binding(&mut self, comments: Vec<String>) -> Parsed<Binding> {
        let start = self.expect_kw("input")?;
        let (name, name_span) = self.eat_ident(Expected::one("a name for this input"))?;
        self.expect(
            &Tok::Assign,
            Expected::one("`=` then the expression this name stands for"),
        )?;
        let expr = self.expr()?;
        let end = expr.span();
        self.end_of_clause()?;
        Ok(Binding {
            name,
            name_span,
            expr,
            span: start.to(end),
            comments,
        })
    }

    fn entry(&mut self, comments: Vec<String>) -> Parsed<Entry> {
        let start = self.expect_kw("entry")?;
        let side = if self.at_ident("long") {
            self.bump();
            Side::Long
        } else if self.at_ident("short") {
            self.bump();
            Side::Short
        } else {
            let found = self.peek().clone();
            return Err(self.unexpected(
                &found,
                Expected::any(["`long`", "`short`"]).expect("two alternatives"),
            ));
        };
        self.expect_kw("when")?;
        let when = self.expr()?;
        let mut cooldown = None;
        let mut end = when.span();
        if self.at_ident("cooldown") {
            let kw = self.bump().span;
            let n = self.eat_num(Expected::one("a number of bars"))?;
            let bars = self.expect_kw("bars")?;
            end = bars;
            cooldown = Some((n, kw.to(bars)));
        }
        self.end_of_clause()?;
        Ok(Entry {
            side,
            when,
            cooldown,
            span: start.to(end),
            comments,
        })
    }

    fn size(&mut self, comments: Vec<String>) -> Parsed<SizeClause> {
        let start = self.expect_kw("size")?;
        let (word, word_span) = self.eat_ident(
            Expected::any(["`fixed`", "`equity_fraction`", "`risk_per_trade`"])
                .expect("three alternatives"),
        )?;
        let kind = match word.as_str() {
            "fixed" => SizeKind::Fixed(self.eat_num(Expected::one("a quantity"))?),
            "equity_fraction" => {
                SizeKind::EquityFraction(self.eat_num(Expected::one("a fraction of equity"))?)
            }
            "risk_per_trade" => {
                SizeKind::RiskPerTrade(self.eat_num(Expected::one("a risk fraction"))?)
            }
            _ => {
                // Parsed, not refused here: FEAT-004 §3.1 really does contain
                // `vol_target` and `kelly`, and "the canonical form has no
                // `SizeMode` for it" is a more useful answer than "unexpected
                // token".
                let rest = match self.peek() {
                    Tok::Num(_) => Some(self.eat_num(Expected::one("a number"))?),
                    _ => None,
                };
                SizeKind::Other { word, rest }
            }
        };
        let end = self.span();
        self.end_of_clause()?;
        Ok(SizeClause {
            kind,
            span: start.to(end).to(word_span),
            comments,
        })
    }

    fn exit(&mut self, comments: Vec<String>) -> Parsed<Exit> {
        let start = self.expect_kw("exit")?;
        let (word, word_span) = self.eat_ident(
            Expected::any([
                "`stop`",
                "`target`",
                "`trailing`",
                "`time`",
                "`when`",
                "`none`",
            ])
            .expect("six alternatives"),
        )?;
        let (kind, end) = match word.as_str() {
            "none" => (ExitKind::None, word_span),
            "stop" | "target" | "trailing" => {
                let d = self.distance()?;
                let span = d.span;
                let k = match word.as_str() {
                    "stop" => ExitKind::Stop(d),
                    "target" => ExitKind::Target(d),
                    _ => ExitKind::Trailing(d),
                };
                (k, span)
            }
            "time" => {
                let n = self.eat_num(Expected::one("a number of bars"))?;
                let bars = self.expect_kw("bars")?;
                (ExitKind::Time(n), bars)
            }
            "when" => {
                let e = self.expr()?;
                let span = e.span();
                (ExitKind::When(e), span)
            }
            other => {
                return Err(Diagnostic::syntax(
                    Code::UnexpectedToken,
                    word_span,
                    format!("`{other}` is not a kind of exit"),
                    Expected::any([
                        "`stop`",
                        "`target`",
                        "`trailing`",
                        "`time`",
                        "`when`",
                        "`none`",
                    ])
                    .expect("six alternatives"),
                )
                .with_see("guide#exits"))
            }
        };
        self.end_of_clause()?;
        Ok(Exit {
            kind,
            span: start.to(end),
            comments,
        })
    }

    /// `2 % from entry`, `1.5 atr(14) from entry`. The unit is not optional and
    /// the diagnostic for a missing one names all three (AGENT-011 §3; S-5.5
    /// acceptance).
    ///
    /// The two scaled units take a lookback in parentheses, and it is not
    /// optional either. `atr` alone is the shape the compiler used to refuse
    /// wholesale: the schema has `DistanceUnit::Atr { period }` (ADR-D-09) and
    /// there was no syntax for the period, so the only honest answers were to
    /// refuse the unit or to invent a lookback. It takes one now, and the
    /// bare-keyword form is refused by name instead of guessed at.
    fn distance(&mut self) -> Parsed<Distance> {
        let units =
            || Expected::any(Unit::WORDS.iter().map(|w| format!("`{w}`"))).expect("three units");
        // `1.5atr` arrives as one `Word` (see `lex`): the number and its unit
        // were written with no space. Saying exactly that is a one-character
        // edit for the author; "expected a distance" is a puzzle.
        if let Tok::Word(w) = self.peek().clone() {
            if w.starts_with(|c: char| c.is_ascii_digit()) {
                let split = w.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(w.len());
                let (number, unit) = w.split_at(split);
                return Err(Diagnostic::syntax(
                    Code::UnitNeedsSpace,
                    self.span(),
                    format!("`{w}` runs a number into its unit"),
                    units(),
                )
                .with_fix(format!("put a space between them, as in `{number} {unit}`"))
                .with_see("guide#exits.units"));
            }
        }
        let value = self.eat_num(Expected::one("a distance"))?;
        let (unit, unit_span) = match self.peek().clone() {
            Tok::Percent => (Unit::Percent, self.bump().span),
            Tok::Ident(w) if w == "atr" || w == "sigma" => {
                let word_span = self.bump().span;
                let (period, period_span) = self.unit_period(&w, word_span)?;
                let unit = if w == "atr" {
                    Unit::Atr { period }
                } else {
                    Unit::Sigma { period }
                };
                (unit, word_span.to(period_span))
            }
            _ => {
                return Err(Diagnostic::syntax(
                    Code::DistanceWithoutUnit,
                    value.span,
                    format!("the distance `{}` has no unit", value.raw),
                    units(),
                )
                .with_fix(format!(
                    "write `{v} %`, `{v} atr(14)` or `{v} sigma(20)` from entry",
                    v = value.raw
                ))
                .with_see("guide#exits.units"));
            }
        };
        self.expect_kw("from")?;
        let (anchor_word, anchor_span) =
            self.eat_ident(Expected::any(["`entry`", "`high_water`"]).expect("two alternatives"))?;
        let anchor = match anchor_word.as_str() {
            "entry" => Anchor::Entry,
            "high_water" => Anchor::HighWater,
            other => {
                return Err(Diagnostic::syntax(
                    Code::UnexpectedToken,
                    anchor_span,
                    format!("`{other}` is not something a distance is measured from"),
                    Expected::any(["`entry`", "`high_water`"]).expect("two alternatives"),
                )
                .with_see("guide#exits.units"))
            }
        };
        Ok(Distance {
            span: value.span.to(anchor_span),
            value,
            unit,
            unit_span,
            anchor,
        })
    }

    /// `(14)` after `atr` or `sigma`, which is not optional.
    ///
    /// Two refusals, and neither of them defaults:
    ///
    /// - **No parentheses at all.** `atr` names a family, not a measurement, and
    ///   the period is what selects `atr_14` over `atr_50`. A compiler that
    ///   supplied 14 here would store a lookback the author never wrote into a
    ///   definition that is then swept, hashed and cited.
    /// - **A period the engine will not compute.** `atr(0)` is not "no
    ///   lookback"; it is the feature name `atr_0`, which nothing serves. Caught
    ///   where it is written rather than at the first bar of the run.
    fn unit_period(&mut self, word: &str, word_span: Span) -> Parsed<(u32, Span)> {
        if self.peek() != &Tok::LParen {
            return Err(Diagnostic::syntax(
                Code::UnitNeedsPeriod,
                word_span,
                format!(
                    "`{word}` has no lookback, and `{word}` over 14 bars and `{word}` over 50 \
                     bars are different measurements"
                ),
                Expected::one("`(`"),
            )
            .with_fix(format!(
                "write the lookback in parentheses — `{word}(14)`. \
                 `list_features(query=\"{word}\")` says what the engine serves"
            ))
            .with_see("guide#exits.units"));
        }
        self.bump();
        let n = self.eat_num(Expected::one("a whole number of bars"))?;
        let close = self.expect(&Tok::RParen, Expected::one("`)`"))?;
        let period = match n.raw.parse::<u32>() {
            Ok(p) if (MIN_UNIT_PERIOD..=MAX_UNIT_PERIOD).contains(&p) => p,
            _ => {
                return Err(Diagnostic::semantic(
                    Code::BadFeatureParam,
                    n.span,
                    format!(
                        "`{}` is not a lookback `{word}` can be measured over: the feature \
                         engine computes a rolling statistic over {MIN_UNIT_PERIOD}..=\
                         {MAX_UNIT_PERIOD} whole bars",
                        n.raw
                    ),
                )
                .with_fix(format!("write a whole number in range, as in `{word}(14)`"))
                .with_see("guide#exits.units"));
            }
        };
        Ok((period, close))
    }

    fn risk(&mut self, comments: Vec<String>) -> Parsed<RiskItem> {
        let start = self.expect_kw("risk")?;
        let (word, word_span) = self.eat_ident(
            Expected::any(RiskKind::ALL.iter().map(|k| format!("`{}`", k.as_str())))
                .expect("four alternatives"),
        )?;
        let Some(kind) = RiskKind::parse(&word) else {
            return Err(Diagnostic::syntax(
                Code::UnexpectedToken,
                word_span,
                format!("`{word}` is not a risk limit"),
                Expected::any(RiskKind::ALL.iter().map(|k| format!("`{}`", k.as_str())))
                    .expect("four alternatives"),
            )
            .with_see("guide#risk"));
        };
        let value = self.eat_num(Expected::one("a limit"))?;
        let end = value.span;
        self.end_of_clause()?;
        Ok(RiskItem {
            kind,
            value,
            span: start.to(end),
            comments,
        })
    }

    /// `order market`, `order limit offset 0.1 % tif gtc` (FEAT-004 §3.1).
    ///
    /// Parsed in full and then judged by the compiler, which is the only place
    /// that knows what the canonical form can hold. Refusing the type here would
    /// put half the answer in the parser and report a *syntax* error for a
    /// clause whose syntax is right — and `order limit` is the shape a model
    /// reading FEAT-004 will write, so it is worth answering precisely.
    fn order(&mut self, comments: Vec<String>) -> Parsed<OrderClause> {
        let start = self.expect_kw("order")?;
        let types =
            || Expected::any(OrderType::ALL.iter().map(|t| format!("`{t}`"))).expect("four types");
        let (word, type_span) = self.eat_ident(types())?;
        let Some(order_type) = OrderType::parse(&word) else {
            return Err(Diagnostic::syntax(
                Code::UnexpectedToken,
                type_span,
                format!("`{word}` is not an order type"),
                types(),
            )
            .with_fix("the default is `market`, which is the one the platform can store")
            .with_see("guide#orders"));
        };
        let mut end = type_span;

        let mut offset = None;
        if self.at_ident("offset") {
            let kw = self.bump().span;
            let n = self.eat_num(Expected::one("an offset"))?;
            // The `%` is required for the same reason every other distance in
            // this language carries a unit: a bare `0.1` is a tenth of a percent
            // to one reader and a tenth of the price to the next.
            let pct = self.expect(
                &Tok::Percent,
                Expected::one("`%` — an order offset is a percentage of the reference price"),
            )?;
            end = pct;
            offset = Some((n, kw.to(pct)));
        }

        let mut tif = None;
        if self.at_ident("tif") {
            let kw = self.bump().span;
            let (word, word_span) = self.eat_ident(
                Expected::any(["`gtc`", "`ioc`", "`fok`", "`day`"]).expect("four alternatives"),
            )?;
            end = word_span;
            tif = Some((word, kw.to(word_span)));
        }

        self.end_of_clause()?;
        Ok(OrderClause {
            order_type,
            type_span,
            offset,
            tif,
            span: start.to(end),
            comments,
        })
    }

    /// `constraint param('fast') < param('slow')`.
    ///
    /// The text after the keyword is taken verbatim to the end of the line.
    /// `domain::strategy_def::params` stores a constraint as a string and parses
    /// it with its own tiny grammar; re-rendering it here would be a second
    /// grammar for one thing, which is the defect shape this repository keeps
    /// finding.
    fn constraint(&mut self, comments: Vec<String>) -> Parsed<ConstraintClause> {
        let start = self.expect_kw("constraint")?;
        let text_start = self.span().offset as usize;
        let mut end_span = start;
        while !matches!(self.peek(), Tok::Newline | Tok::Eof) {
            end_span = self.bump().span;
        }
        let text_end = (end_span.offset + end_span.len) as usize;
        if text_end <= text_start {
            return Err(Diagnostic::syntax(
                Code::UnexpectedToken,
                start,
                "a `constraint` clause with nothing in it",
                Expected::one("a comparison such as `param('fast') < param('slow')`"),
            )
            .with_see("guide#params.constraints"));
        }
        let text = self.source[text_start..text_end].trim().to_string();
        self.end_of_clause()?;
        Ok(ConstraintClause {
            text,
            span: start.to(end_span),
            comments,
        })
    }

    // ── expressions ──────────────────────────────────────────────────────────

    fn expr(&mut self) -> Parsed<Expr> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Parsed<Expr> {
        let mut lhs = self.and_expr()?;
        while self.at_ident("or") {
            self.bump();
            let rhs = self.and_expr()?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op: crate::ast::BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Parsed<Expr> {
        let mut lhs = self.not_expr()?;
        while self.at_ident("and") {
            self.bump();
            let rhs = self.not_expr()?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op: crate::ast::BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn not_expr(&mut self) -> Parsed<Expr> {
        if self.at_ident("not") {
            let kw = self.bump().span;
            let rhs = self.not_expr()?;
            let span = kw.to(rhs.span());
            return Ok(Expr::Unary {
                op: crate::ast::UnOp::Not,
                rhs: Box::new(rhs),
                span,
            });
        }
        self.cmp_expr()
    }

    fn cmp_expr(&mut self) -> Parsed<Expr> {
        let lhs = self.add_expr()?;
        let op = match self.peek() {
            Tok::Gt => crate::ast::BinOp::Gt,
            Tok::Lt => crate::ast::BinOp::Lt,
            Tok::Ge => crate::ast::BinOp::Ge,
            Tok::Le => crate::ast::BinOp::Le,
            Tok::EqEq => crate::ast::BinOp::Eq,
            Tok::Ne => crate::ast::BinOp::Ne,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.add_expr()?;
        let span = lhs.span().to(rhs.span());
        Ok(Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        })
    }

    fn add_expr(&mut self) -> Parsed<Expr> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => crate::ast::BinOp::Add,
                Tok::Minus => crate::ast::BinOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.mul_expr()?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn mul_expr(&mut self) -> Parsed<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => crate::ast::BinOp::Mul,
                Tok::Slash => crate::ast::BinOp::Div,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.unary()?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        if matches!(self.peek(), Tok::Minus) {
            let kw = self.bump().span;
            let rhs = self.unary()?;
            let span = kw.to(rhs.span());
            return Ok(Expr::Unary {
                op: crate::ast::UnOp::Neg,
                rhs: Box::new(rhs),
                span,
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Parsed<Expr> {
        match self.peek().clone() {
            Tok::Num(raw) => {
                let span = self.bump().span;
                Ok(Expr::Number(Num { raw, span }))
            }
            Tok::Str(value) => {
                let span = self.bump().span;
                Ok(Expr::Text { value, span })
            }
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Tok::RParen, Expected::one("`)`"))?;
                Ok(inner)
            }
            Tok::Ident(name) => {
                let name_span = self.bump().span;
                if matches!(self.peek(), Tok::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if matches!(self.peek(), Tok::Comma) {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    let close = self.expect(
                        &Tok::RParen,
                        Expected::any(["`)`", "`,` then another argument"])
                            .expect("two alternatives"),
                    )?;
                    return Ok(Expr::Call {
                        name,
                        args,
                        span: name_span.to(close),
                        name_span,
                    });
                }
                if let Some(field) = crate::ast::BarField::parse(&name) {
                    return Ok(Expr::Bar {
                        field,
                        span: name_span,
                    });
                }
                Ok(Expr::Name {
                    name,
                    span: name_span,
                })
            }
            other => Err(self
                .unexpected(
                    &other,
                    Expected::any([
                        "a number",
                        "a name",
                        "a feature call such as `ema(close, 20)`",
                        "`(`",
                    ])
                    .expect("four alternatives"),
                )
                .with_see("guide#grammar.expressions")),
        }
    }
}

// ── the two legibility rules ─────────────────────────────────────────────────

/// Every expression the author wrote inline, in source order, paired with the
/// clause it belongs to.
fn inline_exprs(s: &Strategy) -> Vec<&Expr> {
    let mut out: Vec<&Expr> = Vec::new();
    for e in &s.entries {
        out.push(&e.when);
    }
    for x in &s.exits {
        if let ExitKind::When(e) = &x.kind {
            out.push(e);
        }
    }
    out
}

fn lint(source: &str, s: &Strategy) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    too_long(source, s, &mut out);
    used_twice(source, s, &mut out);
    out.sort_by_key(|d| d.span.offset);
    out
}

/// One monster expression must be given a name. A conjunction of legible
/// filters is left alone — see the module docs for why the rule descends
/// through `and`, `or` and `not` rather than measuring the whole condition.
fn too_long(source: &str, s: &Strategy, out: &mut Vec<Diagnostic>) {
    fn visit(source: &str, e: &Expr, out: &mut Vec<Diagnostic>) {
        let logical = match e {
            Expr::Binary { op, .. } => op.is_logical(),
            Expr::Unary {
                op: crate::ast::UnOp::Not,
                ..
            } => true,
            _ => false,
        };
        if logical {
            match e {
                Expr::Binary { lhs, rhs, .. } => {
                    visit(source, lhs, out);
                    visit(source, rhs, out);
                }
                Expr::Unary { rhs, .. } => visit(source, rhs, out),
                _ => unreachable!("logical implies binary or unary"),
            }
            return;
        }
        let span = e.span();
        let text = source.get(span.range()).unwrap_or_default();
        let chars = text.chars().count();
        if chars > MAX_INLINE_EXPR {
            out.push(
                Diagnostic::semantic(
                    Code::ExpressionTooLong,
                    span,
                    format!(
                        "this expression is {chars} characters inline; the limit is {MAX_INLINE_EXPR}"
                    ),
                )
                .with_fix("bind it as an input, then reference it by name")
                .with_see("guide#inputs"),
            );
        }
    }
    for e in inline_exprs(s) {
        visit(source, e, out);
    }
}

/// `inputs` are mandatory for anything used twice (AGENT-011 §3).
///
/// "Anything" means a call: a repeated `close` is not a legibility problem and
/// a repeated `ema(close, 20)` is. Bindings are counted too, so writing the
/// expression once as an input and once inline is the same mistake as writing
/// it twice inline — and the fix names the input that already exists.
fn used_twice(source: &str, s: &Strategy, out: &mut Vec<Diagnostic>) {
    fn normalised(source: &str, span: Span) -> String {
        source
            .get(span.range())
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    let mut occurrences: HashMap<String, Vec<Span>> = HashMap::new();
    let mut bound: HashMap<String, String> = HashMap::new();

    let record = |e: &Expr, occ: &mut HashMap<String, Vec<Span>>| {
        e.walk(&mut |node| {
            if let Expr::Call { span, .. } = node {
                occ.entry(normalised(source, *span))
                    .or_default()
                    .push(*span);
            }
        });
    };

    for b in &s.inputs {
        bound.insert(normalised(source, b.expr.span()), b.name.clone());
        record(&b.expr, &mut occurrences);
    }
    for e in inline_exprs(s) {
        record(e, &mut occurrences);
    }

    let mut keys: Vec<&String> = occurrences.keys().collect();
    keys.sort();
    for key in keys {
        let spans = &occurrences[key];
        if spans.len() < 2 {
            continue;
        }
        let fix = bound.get(key).map_or_else(
            || format!("bind it once — `input my_name = {key}` — and use the name here"),
            |name| format!("you already bound this as `{name}`; write `{name}` here"),
        );
        for span in spans.iter().skip(1) {
            out.push(
                Diagnostic::semantic(
                    Code::ExpressionUsedTwice,
                    *span,
                    format!("`{key}` is written in {} places", spans.len()),
                )
                .with_fix(fix.clone())
                .with_see("guide#inputs"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREND: &str = "\
# a trend follower; the 60 gate keeps it out of blow-off tops
strategy ema_cross on $bound_at_init timeframe 1h
input fast = ema(close, 7)
input slow = ema(close, 21)
entry long when fast > slow and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
exit time 48 bars
risk max_position 0.5
";

    #[test]
    fn the_trend_skeleton_parses_into_the_clauses_it_declares() {
        let s = parse(TREND).expect("parses");
        assert_eq!(s.name, "ema_cross");
        assert_eq!(s.instrument, "$bound_at_init");
        assert_eq!(s.timeframe, "1h");
        assert_eq!(s.inputs.len(), 2);
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.exits.len(), 3);
        assert_eq!(s.risk.len(), 1);
        assert!(s.size.is_some());
    }

    #[test]
    fn a_comment_attaches_to_the_clause_below_it() {
        let s = parse(TREND).expect("parses");
        assert_eq!(
            s.header_comments,
            vec!["a trend follower; the 60 gate keeps it out of blow-off tops".to_string()]
        );
    }

    /// S-5.5 acceptance, first half.
    #[test]
    fn a_bare_number_in_a_distance_names_all_three_units() {
        let src = "strategy s on $bound_at_init timeframe 1h\nexit stop 1.5 from entry\n";
        let errs = parse(src).expect_err("refused");
        let d = errs
            .iter()
            .find(|d| d.code == Code::DistanceWithoutUnit)
            .expect("a distance_without_unit diagnostic");
        let expected: Vec<&str> = d.expected().expect("syntax error").items().collect();
        assert_eq!(expected, vec!["`%`", "`atr`", "`sigma`"]);
        assert!(d.fix.as_ref().unwrap().contains("1.5 atr"));
        assert_eq!(d.span.line, 2);
    }

    #[test]
    fn a_number_glued_to_its_unit_says_to_add_a_space() {
        let src = "strategy s on $bound_at_init timeframe 1h\nexit stop 1.5atr from entry\n";
        let errs = parse(src).expect_err("refused");
        let d = errs
            .iter()
            .find(|d| d.code == Code::UnitNeedsSpace)
            .expect("a unit_needs_space diagnostic");
        assert!(d.fix.as_ref().unwrap().contains("1.5 atr"), "{:?}", d.fix);
    }

    /// S-5.5 acceptance, second half.
    #[test]
    fn an_inline_expression_over_the_limit_says_to_bind_it() {
        let long = format!(
            "rolling_quantile(close, 30, 50) + rolling_quantile(close, 40, 50) + rolling_quantile(close, 50, 50) + rolling_quantile(close, 60, 50) > {}",
            1
        );
        assert!(
            long.chars().count() > MAX_INLINE_EXPR,
            "the fixture must exceed the limit to test it"
        );
        let src = format!("strategy s on $bound_at_init timeframe 1h\nentry long when {long}\n");
        let errs = parse(&src).expect_err("refused");
        let d = errs
            .iter()
            .find(|d| d.code == Code::ExpressionTooLong)
            .expect("an expression_too_long diagnostic");
        assert_eq!(
            d.fix.as_deref(),
            Some("bind it as an input, then reference it by name")
        );
    }

    /// The limit applies per non-logical operand, so a legible conjunction of
    /// short filters is accepted however many there are. Without this the
    /// decompiler could write text its own parser refuses.
    #[test]
    fn a_long_conjunction_of_short_filters_is_accepted() {
        let filters: Vec<String> = (10..30)
            .map(|n| format!("zscore(close, {n}) > 1"))
            .collect();
        let cond = filters.join(" and ");
        assert!(cond.chars().count() > MAX_INLINE_EXPR * 2);
        let src = format!("strategy s on $bound_at_init timeframe 1h\nentry long when {cond}\n");
        let s = parse(&src).expect("a conjunction of legible filters parses");
        assert_eq!(s.entries.len(), 1);
    }

    #[test]
    fn an_expression_written_twice_must_be_bound() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
entry long when ema(close, 20) > 100 and ema(close, 20) < 200
";
        let errs = parse(src).expect_err("refused");
        let d = errs
            .iter()
            .find(|d| d.code == Code::ExpressionUsedTwice)
            .expect("an expression_used_twice diagnostic");
        assert!(d.message.contains("in 2 places"), "{}", d.message);
        assert!(d.fix.as_ref().unwrap().contains("input my_name"));
    }

    #[test]
    fn writing_a_bound_expression_inline_points_at_the_binding() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
input trend = ema(close, 20)
entry long when ema(close, 20) > 100
";
        let errs = parse(src).expect_err("refused");
        let d = errs
            .iter()
            .find(|d| d.code == Code::ExpressionUsedTwice)
            .expect("an expression_used_twice diagnostic");
        assert_eq!(
            d.fix.as_deref(),
            Some("you already bound this as `trend`; write `trend` here")
        );
    }

    /// One bad clause costs one diagnostic. Without recovery the second and
    /// third errors below would be invisible until two more round trips.
    #[test]
    fn every_bad_clause_is_reported_not_just_the_first() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
exit stop 1.5 from entry
risk not_a_limit 3
wobble 1
";
        let errs = parse(src).expect_err("refused");
        assert_eq!(errs.len(), 3, "{errs:#?}");
        assert_eq!(errs[0].code, Code::DistanceWithoutUnit);
        assert_eq!(errs[1].code, Code::UnexpectedToken);
        assert_eq!(errs[2].code, Code::UnknownClause);
        assert_eq!(errs[2].span.line, 4);
    }

    #[test]
    fn a_document_without_a_header_says_what_a_header_looks_like() {
        let errs = parse("entry long when close > 1\n").expect_err("refused");
        assert_eq!(errs[0].code, Code::MissingHeader);
        assert!(errs[0].fix.as_ref().unwrap().contains("strategy my_idea"));
    }

    #[test]
    fn every_syntax_diagnostic_carries_a_non_empty_expected_list() {
        let sources = [
            "entry long when close > 1\n",
            "strategy s on $bound_at_init timeframe 1h\nexit stop 1.5 from entry\n",
            "strategy s on $bound_at_init timeframe 1h\nparam n: colour[1..2] = 1\n",
            "strategy s on $bound_at_init timeframe 1h\nentry sideways when close > 1\n",
        ];
        let mut seen = 0;
        for src in sources {
            for d in parse(src).expect_err("refused") {
                if let Some(expected) = d.expected() {
                    assert!(expected.items().count() >= 1);
                    seen += 1;
                }
            }
        }
        assert!(
            seen >= sources.len(),
            "the scan found {seen} syntax diagnostics"
        );
    }

    #[test]
    fn params_declare_their_type_range_step_and_default() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
param fast: int[5..50 step 2] = 7
param gate: float[0.005..0.08 log] = 0.02
param mode: enum[trail, fixed] = trail
constraint param('fast') < param('slow')
";
        let s = parse(src).expect("parses");
        assert_eq!(s.params.len(), 3);
        match &s.params[0].kind {
            ParamKind::Int {
                min,
                max,
                step,
                default,
            } => {
                assert_eq!(min.raw, "5");
                assert_eq!(max.raw, "50");
                assert_eq!(step.as_ref().unwrap().raw, "2");
                assert_eq!(default.raw, "7");
            }
            other => panic!("expected an int param, got {other:?}"),
        }
        match &s.params[1].kind {
            ParamKind::Float { log, .. } => assert!(*log),
            other => panic!("expected a float param, got {other:?}"),
        }
        assert_eq!(s.constraints[0].text, "param('fast') < param('slow')");
    }

    #[test]
    fn the_v1_expression_grammar_is_a_subset() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
entry long when feature('ema_7') > bar('close')
";
        let s = parse(src).expect("the v1 forms parse unchanged");
        match &s.entries[0].when {
            Expr::Binary { lhs, rhs, .. } => {
                assert!(matches!(**lhs, Expr::Call { .. }));
                assert!(matches!(**rhs, Expr::Call { .. }));
            }
            other => panic!("expected a comparison, got {other:?}"),
        }
    }
}
