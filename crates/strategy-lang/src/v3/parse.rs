//! SLv3 text → the canonical graph (Set U U-7.1; ADR-0045).
//!
//! The parser builds the graph's wire JSON and lets `domain` deserialise it, so
//! the structural rules a graph must meet are checked in exactly one place — the
//! same place a graph that arrives as JSON or through builder calls is checked.
//!
//! Two value notations share one expression grammar:
//!
//! - **node values** — a node's arguments, a rule's `when`, the selection's
//!   score, filter and attributes: a bare name is a *reference* (`close`, `f`,
//!   `vol.sigma`, `$fast`, `state.flat`), a quoted string is a *literal*
//!   (`{"str": …}`), `op(…)` is an inline node, and `a < b`, `a and b`,
//!   `-x`, `a + b` are the comparison, logic and arithmetic operators.
//! - **data values** — intents, params, inputs, the universe, triggers and risk:
//!   `tag(field: value, …)` is an object tagged by the field it sits in
//!   (`size:` → `mode`, `style:`/`via:` → `type`, `unit:` → `unit`, an intent
//!   or `entry:` → `kind`), `{field: value}` an untagged object, a quoted string
//!   a plain string. A call in any other field is an inline node, and its
//!   arguments are node values again.

use domain::strategy_graph::ops;
use domain::strategy_graph::StrategyGraph;
use serde_json::{json, Map, Value};

use super::lex::{lex, Tok, TokKind};

/// Why a text is not a strategy. One at a time: the first place the text stops
/// making sense, with what was expected there and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: u32,
    pub col: u32,
    pub message: String,
    pub fix: String,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {} — {}",
            self.line, self.col, self.message, self.fix
        )
    }
}

/// Words that begin a statement, and so can never end an expression.
pub const KEYWORDS: [&str; 18] = [
    "strategy",
    "kind",
    "asset_class",
    "timeframe",
    "description",
    "param",
    "constraint",
    "input",
    "universe",
    "let",
    "rule",
    "when",
    "otherwise",
    "select",
    "triggers",
    "risk",
    "and",
    "or",
];

/// The comparison and arithmetic operators, as written and as registry ops.
pub const INFIX: [(&str, &str); 10] = [
    ("<", "lt"),
    (">", "gt"),
    ("<=", "le"),
    (">=", "ge"),
    ("==", "eq"),
    ("!=", "ne"),
    ("+", "add"),
    ("-", "sub"),
    ("*", "mul"),
    ("/", "div"),
];

/// The tag key of an object written `tag(…)` in data field `field`.
#[must_use]
pub fn tag_of(field: &str) -> Option<&'static str> {
    match field {
        "size" => Some("mode"),
        "style" | "via" => Some("type"),
        "unit" => Some("unit"),
        "entry" => Some("kind"),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Node,
    Data,
}

struct P {
    toks: Vec<Tok>,
    pos: usize,
}

type R<T> = Result<T, SyntaxError>;

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    fn peek_at(&self, k: usize) -> &TokKind {
        &self.toks[(self.pos + k).min(self.toks.len() - 1)].kind
    }

    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, message: impl Into<String>, fix: impl Into<String>) -> R<T> {
        let t = self.peek();
        Err(SyntaxError {
            line: t.line,
            col: t.col,
            message: message.into(),
            fix: fix.into(),
        })
    }

    fn describe(k: &TokKind) -> String {
        match k {
            TokKind::Ident(s) | TokKind::Number(s) | TokKind::Word(s) => format!("`{s}`"),
            TokKind::Str(s) => format!("\"{s}\""),
            TokKind::Sym(s) => format!("`{s}`"),
            TokKind::Eof => "the end of the text".into(),
        }
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(&self.peek().kind, TokKind::Sym(x) if *x == s)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(&self.peek().kind, TokKind::Ident(x) if x == w)
    }

    fn sym(&mut self, s: &str, fix: &str) -> R<()> {
        if self.is_sym(s) {
            self.next();
            Ok(())
        } else {
            let got = Self::describe(&self.peek().kind);
            self.err(format!("expected `{s}`, found {got}"), fix)
        }
    }

    fn word(&mut self, w: &str, fix: &str) -> R<()> {
        if self.is_word(w) {
            self.next();
            Ok(())
        } else {
            let got = Self::describe(&self.peek().kind);
            self.err(format!("expected `{w}`, found {got}"), fix)
        }
    }

    fn ident(&mut self, what: &str) -> R<String> {
        match self.peek().kind.clone() {
            TokKind::Ident(s) if !s.starts_with('$') => {
                self.next();
                Ok(s)
            }
            other => {
                let got = Self::describe(&other);
                self.err(
                    format!("expected {what}, found {got}"),
                    format!("write {what} as letters, digits and `_`"),
                )
            }
        }
    }

    /// A name that may also be written as a string, for names that are not
    /// identifiers (`strategy "my strategy"`).
    fn name_or_string(&mut self, what: &str) -> R<String> {
        match self.peek().kind.clone() {
            TokKind::Str(s) => {
                self.next();
                Ok(s)
            }
            TokKind::Word(s) => {
                self.next();
                Ok(s)
            }
            _ => self.ident(what),
        }
    }

    fn string(&mut self, what: &str) -> R<String> {
        match self.peek().kind.clone() {
            TokKind::Str(s) => {
                self.next();
                Ok(s)
            }
            other => {
                let got = Self::describe(&other);
                self.err(
                    format!("expected {what} in double quotes, found {got}"),
                    "quote it: \"…\"".to_string(),
                )
            }
        }
    }

    // ── expressions ──────────────────────────────────────────────────────────

    fn expr(&mut self, mode: Mode) -> R<Value> {
        let mut left = self.and_expr(mode)?;
        while self.is_word("or") {
            self.next();
            let right = self.and_expr(mode)?;
            left = json!({"op": "or", "args": {"a": left, "b": right}});
        }
        Ok(left)
    }

    fn and_expr(&mut self, mode: Mode) -> R<Value> {
        let mut left = self.not_expr(mode)?;
        while self.is_word("and") {
            self.next();
            let right = self.not_expr(mode)?;
            left = json!({"op": "and", "args": {"a": left, "b": right}});
        }
        Ok(left)
    }

    fn not_expr(&mut self, mode: Mode) -> R<Value> {
        if self.is_word("not") && !matches!(self.peek_at(1), TokKind::Sym("(")) {
            self.next();
            let x = self.not_expr(mode)?;
            return Ok(json!({"op": "not", "args": {"x": x}}));
        }
        self.cmp(mode)
    }

    fn infix_op(&self, set: &[&str]) -> Option<&'static str> {
        match &self.peek().kind {
            TokKind::Sym(s) if set.contains(s) => {
                INFIX.iter().find(|(w, _)| w == s).map(|(_, o)| *o)
            }
            _ => None,
        }
    }

    fn cmp(&mut self, mode: Mode) -> R<Value> {
        let left = self.additive(mode)?;
        if let Some(op) = self.infix_op(&["<", ">", "<=", ">=", "==", "!="]) {
            self.next();
            let right = self.additive(mode)?;
            if self.infix_op(&["<", ">", "<=", ">=", "==", "!="]).is_some() {
                return self.err(
                    "comparisons do not chain",
                    "write `a < b and b < c`, or use `between(x, lo, hi)`",
                );
            }
            return Ok(json!({"op": op, "args": {"a": left, "b": right}}));
        }
        Ok(left)
    }

    fn additive(&mut self, mode: Mode) -> R<Value> {
        let mut left = self.multiplicative(mode)?;
        while let Some(op) = self.infix_op(&["+", "-"]) {
            self.next();
            let right = self.multiplicative(mode)?;
            left = json!({"op": op, "args": {"a": left, "b": right}});
        }
        Ok(left)
    }

    fn multiplicative(&mut self, mode: Mode) -> R<Value> {
        let mut left = self.unary(mode)?;
        while let Some(op) = self.infix_op(&["*", "/"]) {
            self.next();
            let right = self.unary(mode)?;
            left = json!({"op": op, "args": {"a": left, "b": right}});
        }
        Ok(left)
    }

    fn unary(&mut self, mode: Mode) -> R<Value> {
        if self.is_sym("-") {
            self.next();
            // A minus on a number literal is a negative number, not `neg`.
            if let TokKind::Number(s) = self.peek().kind.clone() {
                self.next();
                return Ok(Self::number(&format!("-{s}"), mode));
            }
            let x = self.unary(mode)?;
            return Ok(json!({"op": "neg", "args": {"x": x}}));
        }
        self.primary(mode, None)
    }

    fn number(s: &str, mode: Mode) -> Value {
        let integral = !s.contains(['.', 'e', 'E']);
        if mode == Mode::Data && integral {
            if let Ok(i) = s.parse::<i64>() {
                return json!(i);
            }
        }
        // Validated by the lexer; a node value is always a float (`Arg::Num`).
        json!(s.parse::<f64>().unwrap_or(0.0))
    }

    /// One operand. `tag` is the tag key when this value sits in a tagged data
    /// field.
    fn primary(&mut self, mode: Mode, tag: Option<&'static str>) -> R<Value> {
        let t = self.peek().clone();
        match t.kind {
            TokKind::Number(s) => {
                self.next();
                Ok(Self::number(&s, mode))
            }
            TokKind::Str(s) => {
                self.next();
                Ok(match mode {
                    Mode::Node => json!({"str": s}),
                    Mode::Data => json!(s),
                })
            }
            TokKind::Word(s) => {
                self.next();
                Ok(json!(s))
            }
            TokKind::Sym("(") => {
                self.next();
                let v = self.expr(mode)?;
                self.sym(")", "close the parenthesis")?;
                Ok(v)
            }
            TokKind::Sym("[") => {
                self.next();
                let mut xs = Vec::new();
                while !self.is_sym("]") {
                    xs.push(self.value(mode, None)?);
                    if !self.is_sym("]") {
                        self.sym(",", "separate list items with `,`")?;
                    }
                }
                self.next();
                Ok(Value::Array(xs))
            }
            TokKind::Sym("{") if mode == Mode::Data => {
                self.next();
                let obj = self.fields(None, "}")?;
                Ok(Value::Object(obj))
            }
            TokKind::Ident(name) => {
                // In a tagged field the name is the tag, and a tag may be any
                // name — `risk(…)` is a size mode, `asset_class(…)` a universe.
                if let (Mode::Data, Some(key)) = (mode, tag) {
                    self.next();
                    if self.is_sym("(") {
                        return self.tagged(key, &name);
                    }
                    return Ok(json!({ key: name }));
                }
                if name == "true" || name == "false" {
                    self.next();
                    return Ok(json!(name == "true"));
                }
                if KEYWORDS.contains(&name.as_str()) || name == "not" {
                    return self.err(
                        format!("`{name}` is a keyword and cannot be a value here"),
                        "a statement started before the value did: check the line above",
                    );
                }
                self.next();
                if self.is_sym("(") {
                    return self.call(&name);
                }
                let mut path = name;
                if self.is_sym(".") {
                    self.next();
                    let field = self.ident("a field name after `.`")?;
                    path = format!("{path}.{field}");
                }
                Ok(json!(path))
            }
            other => {
                let got = Self::describe(&other);
                self.err(
                    format!("expected a value, found {got}"),
                    "a value is a number, a name, a \"string\", a call `op(…)` or a list `[…]`",
                )
            }
        }
    }

    /// One value — an expression — in `mode`.
    fn value(&mut self, mode: Mode, field: Option<&str>) -> R<Value> {
        if mode == Mode::Data {
            if let Some(key) = field.and_then(tag_of) {
                return self.primary(mode, Some(key));
            }
        }
        self.expr(mode)
    }

    /// `name: value, …` up to `close`; the value of each field is a data value
    /// tagged by that field's name.
    fn fields(&mut self, tag: Option<(&str, &str)>, close: &str) -> R<Map<String, Value>> {
        let mut obj = Map::new();
        if let Some((k, v)) = tag {
            obj.insert(k.into(), json!(v));
        }
        while !self.is_sym(close) {
            let key = self.ident("a field name")?;
            self.sym(":", "write `field: value`")?;
            let v = self.value(Mode::Data, Some(&key))?;
            if obj.insert(key.clone(), v).is_some() {
                return self.err(format!("`{key}` is given twice"), "give each field once");
            }
            if !self.is_sym(close) {
                self.sym(",", "separate fields with `,`")?;
            }
        }
        self.next();
        Ok(obj)
    }

    fn tagged(&mut self, key: &str, tag: &str) -> R<Value> {
        self.sym("(", "")?;
        Ok(Value::Object(self.fields(Some((key, tag)), ")")?))
    }

    /// `op(positional…, port: value…)` — an inline node. Positional arguments
    /// bind to the registry's ports in order.
    fn call(&mut self, op: &str) -> R<Value> {
        let spec = ops::op(op);
        self.sym("(", "")?;
        let mut args = Map::new();
        let mut positional = 0usize;
        let mut named = false;
        while !self.is_sym(")") {
            let is_named = matches!(self.peek().kind, TokKind::Ident(_))
                && matches!(self.peek_at(1), TokKind::Sym(":"));
            if is_named {
                named = true;
                let port = self.ident("a port name")?;
                self.next(); // `:`
                let v = self.expr(Mode::Node)?;
                if args.insert(port.clone(), v).is_some() {
                    return self.err(format!("`{port}` is given twice"), "give each port once");
                }
            } else {
                if named {
                    return self.err(
                        "a positional argument after a named one",
                        "put positional arguments first, in the operator's port order",
                    );
                }
                let Some(spec) = spec else {
                    let names: Vec<&str> = ops::ops().iter().map(|o| o.name).collect();
                    let near = nearest(op, &names);
                    return self.err(
                        format!("there is no operator `{op}`"),
                        near.map_or_else(
                            || "see the operator catalogue for what exists".to_string(),
                            |n| format!("did you mean `{n}`?"),
                        ),
                    );
                };
                let Some(port) = spec.inputs.get(positional) else {
                    return self.err(
                        format!("`{op}` takes {} positional argument(s)", spec.inputs.len()),
                        "name the extra arguments: `port: value`",
                    );
                };
                let v = self.expr(Mode::Node)?;
                args.insert(port.name.to_string(), v);
                positional += 1;
            }
            if !self.is_sym(")") {
                self.sym(",", "separate arguments with `,`")?;
            }
        }
        self.next();
        Ok(if args.is_empty() {
            json!({"op": op})
        } else {
            json!({"op": op, "args": args})
        })
    }

    // ── statements ───────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn strategy(&mut self) -> R<Value> {
        self.word("strategy", "a strategy starts `strategy <name> kind <trading|selection> asset_class <class> timeframe <tf>`")?;
        let name = self.name_or_string("the strategy's name")?;
        let mut kind = None;
        let mut asset_class = None;
        let mut timeframe = None;
        loop {
            if self.is_word("kind") {
                self.next();
                kind = Some(self.ident("`trading` or `selection`")?);
            } else if self.is_word("asset_class") {
                self.next();
                asset_class = Some(self.name_or_string("an asset class")?);
            } else if self.is_word("timeframe") {
                self.next();
                timeframe = Some(self.name_or_string("a timeframe such as `1h`")?);
            } else {
                break;
            }
        }
        let header =
            "the header names all three: `kind trading asset_class crypto_spot_cex timeframe 1h`";
        let Some(kind) = kind else {
            return self.err("the header has no `kind`", header);
        };
        let Some(asset_class) = asset_class else {
            return self.err("the header has no `asset_class`", header);
        };
        let Some(timeframe) = timeframe else {
            return self.err("the header has no `timeframe`", header);
        };
        let mut g = Map::new();
        g.insert("format".into(), json!(domain::strategy_graph::FORMAT));
        g.insert("kind".into(), json!(kind));
        g.insert("name".into(), json!(name));
        g.insert("asset_class".into(), json!(asset_class));
        g.insert("timeframe".into(), json!(timeframe));
        let mut params = Map::new();
        let mut constraints = Vec::new();
        let mut inputs = Vec::new();
        let mut nodes = Vec::new();
        let mut rules = Vec::new();
        let mut otherwise = None;
        let mut select: Option<Map<String, Value>> = None;
        loop {
            let t = self.peek().clone();
            let TokKind::Ident(w) = &t.kind else {
                if t.kind == TokKind::Eof {
                    break;
                }
                let got = Self::describe(&t.kind);
                return self.err(
                    format!("expected a statement, found {got}"),
                    "statements start with `let`, `when`, `rule`, `param`, `input`, `select`, …",
                );
            };
            match w.as_str() {
                "description" => {
                    self.next();
                    let d = self.string("the description")?;
                    g.insert("description".into(), json!(d));
                }
                "param" => {
                    self.next();
                    let name = self.ident("the parameter's name")?;
                    self.sym(":", "write `param fast: int(default: 12, min: 5, max: 50)`")?;
                    let spec = self.primary(Mode::Data, Some("type"))?;
                    if params.insert(name.clone(), spec).is_some() {
                        return self.err(
                            format!("parameter `{name}` is declared twice"),
                            "declare it once",
                        );
                    }
                }
                "constraint" => {
                    self.next();
                    constraints.push(json!(self.string("the constraint")?));
                }
                "input" => {
                    self.next();
                    let id = self.ident("the input's name")?;
                    self.sym(
                        ":",
                        "write `input btc_1d: fixed(instrument: \"BTC-USD\") 1d`",
                    )?;
                    let instrument = self.primary(Mode::Data, Some("kind"))?;
                    let tf = self.name_or_string("the input's timeframe")?;
                    inputs.push(json!({"id": id, "instrument": instrument, "timeframe": tf}));
                }
                "universe" => {
                    self.next();
                    let u = self.primary(Mode::Data, Some("kind"))?;
                    g.insert("universe".into(), u);
                }
                "let" => {
                    self.next();
                    let id = self.ident("the node's name")?;
                    self.sym("=", "write `let name = op(…)`")?;
                    let v = self.expr(Mode::Node)?;
                    let Value::Object(mut n) = v else {
                        return self.err(
                            format!("`{id}` must be bound to an operator, not a bare value"),
                            "wrap the value in an operator, e.g. `let x = add(y, 0)`; a reference to another node needs no `let`",
                        );
                    };
                    if !n.contains_key("op") {
                        return self.err(
                            format!("`{id}` must be bound to an operator"),
                            "write `let name = op(…)`",
                        );
                    }
                    n.insert("id".into(), json!(id));
                    nodes.push(Value::Object(n));
                }
                "rule" | "when" => {
                    let mut rule = Map::new();
                    if w == "rule" {
                        self.next();
                        if matches!(self.peek().kind, TokKind::Str(_))
                            || (matches!(self.peek().kind, TokKind::Ident(_))
                                && !self.is_word("group")
                                && !self.is_word("when"))
                        {
                            rule.insert(
                                "name".into(),
                                json!(self.name_or_string("the rule's name")?),
                            );
                        }
                        if self.is_word("group") {
                            self.next();
                            rule.insert("group".into(), json!(self.ident("the rule's group")?));
                        }
                    }
                    self.word("when", "a rule is `when <condition> -> <intent>`")?;
                    rule.insert("when".into(), self.expr(Mode::Node)?);
                    self.sym("->", "a rule is `when <condition> -> <intent>`")?;
                    rule.insert("then".into(), self.primary(Mode::Data, Some("kind"))?);
                    rules.push(Value::Object(rule));
                }
                "otherwise" => {
                    self.next();
                    otherwise = Some(self.ident("`hold`")?);
                }
                "select" => {
                    self.next();
                    let mut s = Map::new();
                    s.insert("score".into(), self.expr(Mode::Node)?);
                    let mut attributes = Map::new();
                    loop {
                        if self.is_word("where") {
                            self.next();
                            s.insert("where".into(), self.expr(Mode::Node)?);
                        } else if self.is_word("top") {
                            self.next();
                            match self.peek().kind.clone() {
                                TokKind::Number(n) => {
                                    self.next();
                                    s.insert("top_n".into(), Self::number(&n, Mode::Data));
                                }
                                other => {
                                    let got = Self::describe(&other);
                                    return self.err(
                                        format!("expected a count after `top`, found {got}"),
                                        "write `top 10`",
                                    );
                                }
                            }
                        } else if self.is_word("order") {
                            self.next();
                            s.insert("order".into(), json!(self.ident("`asc` or `desc`")?));
                        } else if self.is_word("attribute") {
                            self.next();
                            let k = self.ident("the attribute's name")?;
                            self.sym("=", "write `attribute weight = <value>`")?;
                            attributes.insert(k, self.expr(Mode::Node)?);
                        } else {
                            break;
                        }
                    }
                    if !attributes.is_empty() {
                        s.insert("attributes".into(), Value::Object(attributes));
                    }
                    select = Some(s);
                }
                "triggers" | "risk" => {
                    let key = w.clone();
                    self.next();
                    self.sym("{", "write `{field: value, …}`")?;
                    let obj = self.fields(None, "}")?;
                    g.insert(key, Value::Object(obj));
                }
                other => {
                    let known = [
                        "description",
                        "param",
                        "constraint",
                        "input",
                        "universe",
                        "let",
                        "rule",
                        "when",
                        "otherwise",
                        "select",
                        "triggers",
                        "risk",
                    ];
                    let near = nearest(other, &known);
                    return self.err(
                        format!("`{other}` does not start a statement"),
                        near.map_or_else(
                            || "statements start with `let`, `when`, `rule`, `param`, `input`, `select`, …".to_string(),
                            |n| format!("did you mean `{n}`?"),
                        ),
                    );
                }
            }
        }
        if !params.is_empty() {
            g.insert("params".into(), Value::Object(params));
        }
        if !constraints.is_empty() {
            g.insert("constraints".into(), Value::Array(constraints));
        }
        if !inputs.is_empty() {
            g.insert("inputs".into(), Value::Array(inputs));
        }
        g.insert("nodes".into(), Value::Array(nodes));
        if kind == "trading" || !rules.is_empty() || otherwise.is_some() {
            let mut d = Map::new();
            d.insert("rules".into(), Value::Array(rules));
            d.insert(
                "otherwise".into(),
                json!(otherwise.unwrap_or_else(|| "hold".into())),
            );
            g.insert("decide".into(), Value::Object(d));
        }
        if let Some(s) = select {
            g.insert("select".into(), Value::Object(s));
        }
        Ok(Value::Object(g))
    }
}

/// The closest name, when it is close enough to be a typo.
fn nearest<'a>(word: &str, names: &[&'a str]) -> Option<&'a str> {
    fn dist(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.iter().enumerate() {
            let mut cur = vec![i + 1; b.len() + 1];
            for (j, cb) in b.iter().enumerate() {
                cur[j + 1] = (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1);
            }
            prev = cur;
        }
        prev[b.len()]
    }
    names
        .iter()
        .map(|n| (dist(word, n), *n))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map(|(_, n)| n)
}

/// SLv3 text → the graph's wire JSON, without structural checks.
///
/// # Errors
/// The first place the text stops being SLv3.
pub fn parse_to_json(src: &str) -> Result<Value, SyntaxError> {
    let toks = lex(src).map_err(|e| SyntaxError {
        line: e.line,
        col: e.col,
        message: e.message,
        fix: "remove or quote it".into(),
    })?;
    let mut p = P { toks, pos: 0 };
    p.strategy()
}

/// One SLv3 node expression — `ema(close, $fast) > 3` — as graph JSON.
///
/// # Errors
/// Text that is not one whole expression.
pub fn parse_expr(src: &str) -> Result<Value, SyntaxError> {
    let toks = lex(src).map_err(|e| SyntaxError {
        line: e.line,
        col: e.col,
        message: e.message,
        fix: "remove or quote it".into(),
    })?;
    let mut p = P { toks, pos: 0 };
    let v = p.expr(Mode::Node)?;
    if p.peek().kind != TokKind::Eof {
        let got = P::describe(&p.peek().kind);
        return p.err(
            format!("expected the end of the expression, found {got}"),
            "one expression only",
        );
    }
    Ok(v)
}

/// SLv3 text → the canonical graph.
///
/// # Errors
/// A syntax error, or a graph `domain` refuses structurally (wrong sink for its
/// kind, a duplicate node, an unknown timeframe…), named.
pub fn parse(src: &str) -> Result<StrategyGraph, SyntaxError> {
    let v = parse_to_json(src)?;
    serde_json::from_value(v).map_err(|e| SyntaxError {
        line: 1,
        col: 1,
        message: format!("the text is SLv3 but not a strategy: {e}"),
        fix: "a trading strategy ends in `when … -> …` rules; a selection strategy in `universe …` and `select …`".into(),
    })
}
