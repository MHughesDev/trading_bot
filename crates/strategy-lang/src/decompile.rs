//! The canonical [`StrategyDefinition`] → SLv2 text.
//!
//! # Why this direction exists at all
//!
//! Every stored strategy needs a readable form (S-5.7). The dossier, the human
//! reviewer and the model on its next turn all read a strategy, and none of them
//! reads `{"nodes":[{"id":"e0_cond","type":"condition","expr":"((feature('ema_7')
//! > feature('ema_21')) * (feature('rsi_14') < 60))"}]}` as an idea. But the
//! decompiler earns its place for a second and larger reason: it is what makes
//! the text and the JSON **one object** rather than two representations that
//! drift. `compile(parse(decompile(d))) == d` is a theorem you can run, and
//! `tests/round_trip.rs` runs it.
//!
//! # Refusing is part of being total
//!
//! A decompiler that always returns *a* string is worse than one that sometimes
//! refuses, because the string it returns for a definition it does not
//! understand is a lie that recompiles to something else. So this returns a
//! [`Diagnostic`] for anything SLv2 has no text for — an AI node, two actions
//! with different sizes, a declared feature nothing uses — naming what it was.
//! The set it accepts is exactly the set the round trip is proven over.
//!
//! # What it canonicalises, and what that costs
//!
//! Node identifiers (`e0_cond`, `x1_sig`) and exit-signal names (`x0`) are
//! *generated* by [`crate::compile()`] from clause order; they are not carried in
//! the text, because a model should not have to invent an identifier to write a
//! strategy. A stored definition using different names therefore decompiles
//! correctly and recompiles with the canonical names — the behaviour is
//! identical and the JSON is not byte-identical.
//! `decompiling_a_legacy_definition_canonicalises_its_node_ids` pins exactly
//! that boundary, so the round-trip property is stated over canonical
//! definitions rather than quietly assumed over all of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use domain::strategy_def::{
    actions::{ActionKind, SizeMode},
    exits::{DistanceUnit, ExitDistance, ExitPolicy, ExitRule},
    nodes::NodeKind,
    params::{ParamSpec, Scale},
    StrategyDefinition,
};
use serde_json::Value;

use crate::ast::{BinOp, Expr, UnOp};
use crate::catalogue::Catalogue;
use crate::compile::{
    shift_point, BARS_LANE_PREFIX, DEFAULT_ASSET_CLASS, FEATURES_LANE, TARGET_VERSION,
};
use crate::diagnostic::{Code, Diagnostic, Span};

/// Where a decompilation diagnostic points. There is no source text, so every
/// span is the same one: the alternative would be a nullable span, and then a
/// diagnostic type with a hole in it for every caller to handle.
const NO_SOURCE: Span = Span::new(0, 0, 0, 0);

/// Render a stored definition as SLv2, using the engine's feature registry.
///
/// # Errors
///
/// The first thing about this definition SLv2 has no text for.
pub fn decompile(def: &StrategyDefinition) -> Result<String, Diagnostic> {
    decompile_with(def, &Catalogue::engine())
}

/// Render a stored definition as SLv2 against a caller-supplied catalogue.
///
/// # Errors
///
/// The first thing about this definition SLv2 has no text for.
pub fn decompile_with(def: &StrategyDefinition, cat: &Catalogue) -> Result<String, Diagnostic> {
    let plan = Plan::read(def, cat)?;
    Ok(plan.render(def, cat))
}

fn refuse(code: Code, message: impl Into<String>, fix: impl Into<String>) -> Diagnostic {
    Diagnostic::semantic(code, NO_SOURCE, message)
        .with_fix(fix)
        .with_see("guide#json")
}

/// One entry, already read back as SLv2 syntax.
struct EntryPlan {
    side: crate::ast::Side,
    when: Expr,
}

struct Plan {
    timeframe: String,
    instrument: String,
    entries: Vec<EntryPlan>,
    /// `emit` → the condition, for `exit when` rules.
    exit_conditions: HashMap<String, Expr>,
    /// Feature name → the input name it will be bound to, for features read
    /// more than once.
    bindings: BTreeMap<String, String>,
    used_features: BTreeSet<String>,
}

impl Plan {
    fn read(def: &StrategyDefinition, cat: &Catalogue) -> Result<Self, Diagnostic> {
        // ── lanes ────────────────────────────────────────────────────────────
        let mut timeframe = None;
        let mut declared: Option<Vec<String>> = None;
        let mut instrument = None;
        for input in &def.inputs {
            if let Some(tf) = input.lane.strip_prefix(BARS_LANE_PREFIX) {
                timeframe = Some(tf.to_string());
                instrument = Some(input.instrument.clone());
            } else if input.lane == FEATURES_LANE {
                declared = Some(input.features.clone());
                instrument.get_or_insert_with(|| input.instrument.clone());
            } else {
                return Err(refuse(
                    Code::NotExpressible,
                    format!("this definition reads the lane `{}`", input.lane),
                    format!(
                        "SLv2 declares its lanes from the header and its features from the \
                         expressions; a strategy on `{}` has no text form",
                        input.lane
                    ),
                ));
            }
        }
        let (Some(timeframe), Some(instrument)) = (timeframe, instrument) else {
            return Err(refuse(
                Code::NotExpressible,
                "this definition subscribes to no bar lane, so it has no timeframe",
                format!("add an input on `{BARS_LANE_PREFIX}1h`"),
            ));
        };

        // ── nodes ────────────────────────────────────────────────────────────
        let mut conditions: HashMap<&str, &str> = HashMap::new();
        for node in &def.nodes {
            match &node.kind {
                NodeKind::Condition { expr } => {
                    conditions.insert(node.id.as_str(), expr.as_str());
                }
                NodeKind::Signal { .. } => {}
                other => {
                    return Err(refuse(
                        Code::NotExpressible,
                        format!(
                            "node `{}` is a {} node, and SLv2 v2.0 has entries, exits, sizing \
                             and risk only",
                            node.id,
                            node_kind_name(other)
                        ),
                        "read it as JSON; model, universe and LLM nodes are the MCP builder's, \
                         not the language's (AGENT-011 §9)",
                    ))
                }
            }
        }

        let mut entries = Vec::new();
        let mut exit_conditions = HashMap::new();
        let mut referenced: BTreeSet<&str> = BTreeSet::new();
        for node in &def.nodes {
            let NodeKind::Signal { when, emit } = &node.kind else {
                continue;
            };
            let Some(expr) = conditions.get(when.as_str()) else {
                return Err(refuse(
                    Code::NotExpressible,
                    format!(
                        "signal `{}` watches `{when}`, which is not a condition node",
                        node.id
                    ),
                    "every signal in SLv2 watches the condition its own `entry` or `exit when` \
                     clause produced",
                ));
            };
            referenced.insert(when.as_str());
            let parsed = crate::parse::expr_from_str(expr).map_err(|d| {
                refuse(
                    Code::BadStoredExpression,
                    format!("node `{when}` does not parse: {}", d.message),
                    "the stored expression must be in the frozen v1 grammar",
                )
            })?;
            let readable = deencode(&parsed);
            match emit.as_str() {
                "long" => entries.push(EntryPlan {
                    side: crate::ast::Side::Long,
                    when: readable,
                }),
                "short" => entries.push(EntryPlan {
                    side: crate::ast::Side::Short,
                    when: readable,
                }),
                other => {
                    exit_conditions.insert(other.to_string(), readable);
                }
            }
        }
        if let Some(dead) = conditions.keys().find(|k| !referenced.contains(*k)) {
            return Err(refuse(
                Code::NotExpressible,
                format!("condition node `{dead}` is not watched by any signal"),
                "delete it, or add the signal that reads it; SLv2 has no text for a condition \
                 nothing uses",
            ));
        }

        // ── actions ──────────────────────────────────────────────────────────
        let mut order: Option<(&SizeMode, &str)> = None;
        for action in &def.actions {
            let ActionKind::PlaceOrder { order: spec } = &action.kind;
            let want_side = match action.on_signal.as_str() {
                "long" => domain::order::Side::Buy,
                "short" => domain::order::Side::Sell,
                other => {
                    return Err(refuse(
                        Code::NotExpressible,
                        format!("an action fires on the signal `{other}`"),
                        "SLv2's entries emit `long` and `short`; other signal names belong to \
                         the JSON form",
                    ))
                }
            };
            if spec.side != want_side {
                return Err(refuse(
                    Code::NotExpressible,
                    format!(
                        "the `{}` signal places a {:?} order",
                        action.on_signal, spec.side
                    ),
                    "an `entry long` buys and an `entry short` sells; a crossed pair has no \
                     text form",
                ));
            }
            match order {
                None => order = Some((&spec.size_mode, spec.size.as_str())),
                Some((mode, size)) if mode == &spec.size_mode && size == spec.size => {}
                Some(_) => {
                    return Err(refuse(
                        Code::NotExpressible,
                        "the actions in this definition are sized differently",
                        "SLv2 has one `size` clause for the strategy; per-signal sizing has no \
                         text form",
                    ))
                }
            }
        }
        if let Some((SizeMode::Model { node_ref }, _)) = order {
            return Err(refuse(
                Code::NotExpressible,
                format!("sizing comes from the model node `{node_ref}`"),
                "model-driven sizing is a JSON-only feature until SLv2 gains `size model(…)`",
            ));
        }
        if order.is_none() && !entries.is_empty() {
            return Err(refuse(
                Code::NotExpressible,
                "this definition has entry signals and no action to size them",
                "add a `place_order` action, or remove the signals",
            ));
        }

        // ── exits ────────────────────────────────────────────────────────────
        if let Some(ExitPolicy::Rules { rules }) = &def.exits {
            let mut seen = 0usize;
            for rule in rules {
                if let ExitRule::OnSignal { emit } = rule {
                    if !exit_conditions.contains_key(emit) {
                        return Err(refuse(
                            Code::NotExpressible,
                            format!("the exit rule watches `{emit}`, which no signal emits"),
                            "an `exit when` clause compiles to a signal; this rule has none",
                        ));
                    }
                    seen += 1;
                }
            }
            if seen != exit_conditions.len() {
                return Err(refuse(
                    Code::NotExpressible,
                    "a signal fires that neither an entry nor an exit rule reads",
                    "every non-entry signal in SLv2 comes from an `exit when` clause",
                ));
            }
        } else if !exit_conditions.is_empty() {
            return Err(refuse(
                Code::NotExpressible,
                "a non-entry signal fires and the exit policy does not read it",
                "every non-entry signal in SLv2 comes from an `exit when` clause",
            ));
        }

        // ── features ─────────────────────────────────────────────────────────
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in entries
            .iter()
            .map(|e| &e.when)
            .chain(exit_conditions.values())
        {
            count_features(e, &mut counts);
        }
        let used_features: BTreeSet<String> = counts.keys().cloned().collect();
        let declared: BTreeSet<String> = declared.unwrap_or_default().into_iter().collect();
        if declared != used_features {
            let missing: Vec<&String> = used_features.difference(&declared).collect();
            let extra: Vec<&String> = declared.difference(&used_features).collect();
            return Err(refuse(
                Code::NotCanonical,
                format!(
                    "the declared features and the features the rules read disagree: \
                     used-but-undeclared {missing:?}, declared-but-unused {extra:?}"
                ),
                "a definition must declare exactly the features it reads; an undeclared one \
                 fails inside the feature engine after a trial is spent",
            ));
        }

        // Anything read twice gets a name, because the parser requires it —
        // see `parse::used_twice`. The name is the feature name, which is
        // already an identifier and already unique.
        let taken: BTreeSet<&str> = def.parameters.keys().map(String::as_str).collect();
        let mut bindings = BTreeMap::new();
        for (feature, n) in &counts {
            if *n < 2 {
                continue;
            }
            let mut name = identifier_for(feature);
            let mut suffix = 2;
            while taken.contains(name.as_str()) || bindings.values().any(|v| v == &name) {
                name = format!("{}_{suffix}", identifier_for(feature));
                suffix += 1;
            }
            bindings.insert(feature.clone(), name);
        }
        let _ = cat;

        Ok(Self {
            timeframe,
            instrument,
            entries,
            exit_conditions,
            bindings,
            used_features,
        })
    }

    fn render(&self, def: &StrategyDefinition, cat: &Catalogue) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "strategy {} on {} timeframe {}\n",
            def.strategy_id,
            render_instrument(&self.instrument),
            self.timeframe
        ));
        if def.definition_version != TARGET_VERSION {
            out.push_str(&format!("version {}\n", def.definition_version));
        }
        if def.asset_class != DEFAULT_ASSET_CLASS {
            out.push_str(&format!("asset_class {}\n", def.asset_class));
        }
        if def.min_trust_tier != domain::trust::TrustTier::CentralizedExchange {
            let tier = serde_json::to_value(def.min_trust_tier)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "centralized_exchange".into());
            out.push_str(&format!("trust {tier}\n"));
        }

        for (name, spec) in &def.parameters {
            out.push_str(&render_param(name, spec));
        }
        for c in &def.constraints {
            out.push_str(&format!("constraint {c}\n"));
        }

        // Bindings are emitted in the order their feature names sort, which is
        // the order the canonical `features` list is already in.
        for (feature, name) in &self.bindings {
            out.push_str(&format!(
                "input {name} = {}\n",
                render_feature(feature, cat)
            ));
        }

        for entry in &self.entries {
            out.push_str(&format!(
                "entry {} when {}\n",
                entry.side.as_str(),
                self.render_expr(&entry.when, cat, 0)
            ));
        }

        if let Some(action) = def.actions.first() {
            let ActionKind::PlaceOrder { order } = &action.kind;
            let word = match order.size_mode {
                SizeMode::Fixed => "fixed",
                SizeMode::PercentOfBalance => "equity_fraction",
                SizeMode::RiskUnit => "risk_per_trade",
                SizeMode::Model { .. } => unreachable!("refused in Plan::read"),
            };
            out.push_str(&format!("size {word} {}\n", order.size));
        }

        match &def.exits {
            None => {}
            Some(ExitPolicy::RiskLayerOnly) => out.push_str("exit none\n"),
            Some(ExitPolicy::Rules { rules }) => {
                for rule in rules {
                    out.push_str(&self.render_exit(rule, cat));
                }
            }
        }

        let risk = serde_json::to_value(&def.risk_overrides).unwrap_or(Value::Null);
        for (key, clause) in [
            ("max_position", "max_position"),
            ("max_order_rate_per_minute", "max_orders_per_min"),
            ("max_order_rate_per_second", "max_orders_per_sec"),
        ] {
            if let Some(v) = risk.get(key) {
                let rendered = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push_str(&format!("risk {clause} {rendered}\n"));
            }
        }

        let _ = &self.used_features;
        out
    }

    /// `2 %`, `1.5 atr(14)`, `2.5 sigma(20)` — the distance as its author wrote
    /// it.
    ///
    /// The percent form shifts the point because the canonical document stores a
    /// fraction and the grammar reads a percentage; the scaled forms do not,
    /// because a multiplier is already the number the text carries. Rendering
    /// the period is not decoration: `atr(14)` and `atr(50)` are different
    /// distances, and a decompiler that dropped it would write text that
    /// compiles to a different strategy.
    /// An exit distance as its author writes it, unit included.
    ///
    /// The point shift belongs to `fraction_of_entry` alone: `0.02` is written
    /// `2 %`, and that conversion is the *unit's*, not the number's. A scaled
    /// distance is already in its unit — `1.5 atr(14)` means one and a half
    /// average true ranges — so shifting it would turn a stop into a different
    /// stop, which is exactly the bug this replaced.
    ///
    /// The period is never elided. `atr(14)` and `atr(50)` are different
    /// distances, and the grammar has no bare `atr` to fall back to (the unit
    /// takes a lookback and refuses without one), so a renderer that dropped it
    /// would write text the parser rejects.
    fn render_distance(d: &ExitDistance) -> String {
        let magnitude = d.magnitude.normalize().to_string();
        match d.unit {
            DistanceUnit::FractionOfEntry => format!("{} %", shift_point(&magnitude, 2)),
            DistanceUnit::Atr { period } => format!("{magnitude} atr({period})"),
            DistanceUnit::Sigma { period } => format!("{magnitude} sigma({period})"),
        }
    }

    fn render_exit(&self, rule: &ExitRule, cat: &Catalogue) -> String {
        match rule {
            ExitRule::StopLoss { distance }
            | ExitRule::TakeProfit { distance }
            | ExitRule::TrailingStop { distance } => {
                let (word, anchor) = match rule {
                    ExitRule::StopLoss { .. } => ("stop", "entry"),
                    ExitRule::TakeProfit { .. } => ("target", "entry"),
                    _ => ("trailing", "high_water"),
                };
                // Read from the typed `ExitDistance`, not from
                // `serde_json::to_value(rule)["fraction"]`. That key still
                // exists — `magnitude` is serialized under it so stored
                // documents keep their wire form (ADR-0010) — but reading it as
                // a *fraction* was true only until ADR-D-09 gave the distance a
                // unit. At `atr` the same number is a multiplier, so the old
                // code shifted the point on it and wrote `150 %` over
                // `1.5 atr(14)`: a decompiler that silently changes what a stop
                // means, which is worse than one that refuses.
                format!(
                    "exit {word} {} from {anchor}\n",
                    Self::render_distance(distance)
                )
            }
            ExitRule::MaxHoldingBars { bars } => format!("exit time {bars} bars\n"),
            ExitRule::OnSignal { emit } => {
                let cond = self
                    .exit_conditions
                    .get(emit)
                    .expect("Plan::read checked every OnSignal has a condition");
                format!("exit when {}\n", self.render_expr(cond, cat, 0))
            }
        }
    }

    fn render_expr(&self, e: &Expr, cat: &Catalogue, parent: u8) -> String {
        render_expr(e, cat, &self.bindings, parent)
    }
}

/// Render an SLv2 expression back as text.
///
/// Free rather than a method on [`Plan`] because [`crate::diff`] and
/// [`crate::block`] both have to name a clause the way its author reads it, and
/// a second renderer in either of them would be a second answer to "how is this
/// expression written" — the two-implementations shape this crate has already
/// removed once from `catalogue.rs`.
///
/// `bindings` maps a feature name to the `input` name standing for it; pass an
/// empty map to render every feature call in full.
pub(crate) fn render_expr(
    e: &Expr,
    cat: &Catalogue,
    bindings: &BTreeMap<String, String>,
    parent: u8,
) -> String {
    let (text, prec) = match e {
        Expr::Number(n) => (n.raw.clone(), 8),
        Expr::Bar { field, .. } => (field.as_str().to_string(), 8),
        Expr::Name { name, .. } => (name.clone(), 8),
        Expr::Text { value, .. } => (format!("'{value}'"), 8),
        Expr::Call { name, args, .. } => match (name.as_str(), args.as_slice()) {
            ("feature", [Expr::Text { value, .. }]) => {
                let text = bindings
                    .get(value)
                    .cloned()
                    .unwrap_or_else(|| render_feature(value, cat));
                (text, 8)
            }
            ("bar", [Expr::Text { value, .. }]) => (value.clone(), 8),
            ("param", [Expr::Text { value, .. }]) => (value.clone(), 8),
            _ => {
                let rendered: Vec<String> = args
                    .iter()
                    .map(|a| render_expr(a, cat, bindings, 0))
                    .collect();
                (format!("{name}({})", rendered.join(", ")), 8)
            }
        },
        Expr::Unary { op, rhs, .. } => match op {
            UnOp::Not => (format!("not {}", render_expr(rhs, cat, bindings, 3)), 3),
            UnOp::Neg => (format!("-{}", render_expr(rhs, cat, bindings, 7)), 7),
        },
        Expr::Binary { op, lhs, rhs, .. } => {
            let prec = precedence(*op);
            let text = format!(
                "{} {} {}",
                render_expr(lhs, cat, bindings, prec),
                op.as_str(),
                render_expr(rhs, cat, bindings, prec + 1)
            );
            (text, prec)
        }
    };
    if prec < parent {
        format!("({text})")
    } else {
        text
    }
}

/// Render an expression with every feature call written out in full.
///
/// This is the form a diff and a blocking profile name a clause in: an `input`
/// name means nothing to a reader who cannot see the binding, and two
/// definitions hoist different features into inputs depending on how often each
/// is read — so comparing un-inlined text would report a move nobody made.
#[must_use]
pub fn expr_to_text(e: &Expr, cat: &Catalogue) -> String {
    render_expr(e, cat, &BTreeMap::new(), 0)
}

const fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le | BinOp::Eq | BinOp::Ne => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div => 6,
    }
}

fn node_kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Condition { .. } => "condition",
        NodeKind::Signal { .. } => "signal",
        NodeKind::DataSource { .. } => "data_source",
        NodeKind::Rank { .. } => "rank",
        NodeKind::Filter { .. } => "filter",
        NodeKind::TakeTopN { .. } => "take_top_n",
        NodeKind::SurfaceAction { .. } => "surface_action",
        NodeKind::ModelForecast { .. } => "model_forecast",
        NodeKind::Inference { .. } => "inference",
        NodeKind::Sizing { .. } => "sizing",
        NodeKind::Decision { .. } => "decision",
        NodeKind::ModelRank { .. } => "model_rank",
        NodeKind::LlmInference { .. } => "llm_inference",
        NodeKind::Combine { .. } => "combine",
    }
}

/// `ema_7` → `ema(close, 7)`; `ema_{{fast}}` → `ema(close, fast)`; anything the
/// catalogue does not know stays in the v1 form, which SLv2 also accepts.
///
/// That last fallback is what keeps the decompiler total over the stored
/// corpus: a definition naming a family this build has never heard of still has
/// a text form, and that form recompiles to the same name.
fn render_feature(name: &str, cat: &Catalogue) -> String {
    let Some((family, params)) = cat.resolve(name) else {
        return format!("feature('{name}')");
    };
    if params.is_empty() {
        return format!("{}()", family.name);
    }
    let args: Vec<String> = std::iter::once("close".to_string())
        .chain(params.iter().map(|p| {
            p.strip_prefix("{{")
                .and_then(|p| p.strip_suffix("}}"))
                .map_or_else(|| p.clone(), str::to_string)
        }))
        .collect();
    format!("{}({})", family.name, args.join(", "))
}

fn identifier_for(feature: &str) -> String {
    feature
        .replace("{{", "")
        .replace("}}", "")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn render_instrument(instrument: &str) -> String {
    let bare = !instrument.is_empty()
        && instrument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !instrument.starts_with(|c: char| c.is_ascii_digit());
    if bare {
        instrument.to_string()
    } else {
        format!("'{instrument}'")
    }
}

fn render_param(name: &str, spec: &ParamSpec) -> String {
    match spec {
        ParamSpec::Int {
            default,
            min,
            max,
            step,
        } => {
            let step = if *step == 1 {
                String::new()
            } else {
                format!(" step {step}")
            };
            format!("param {name}: int[{min}..{max}{step}] = {default}\n")
        }
        ParamSpec::Float {
            default,
            min,
            max,
            scale,
        } => {
            let log = if *scale == Scale::Log { " log" } else { "" };
            format!("param {name}: float[{min}..{max}{log}] = {default}\n")
        }
        ParamSpec::Enum { default, choices } => {
            format!("param {name}: enum[{}] = {default}\n", choices.join(", "))
        }
    }
}

fn count_features(e: &Expr, out: &mut BTreeMap<String, usize>) {
    e.walk(&mut |node| {
        if let Expr::Call { name, args, .. } = node {
            if name == "feature" {
                if let [Expr::Text { value, .. }] = args.as_slice() {
                    *out.entry(value.clone()).or_default() += 1;
                }
            }
        }
    });
}

// ── reading the boolean encoding back ────────────────────────────────────────

/// Whether this expression is one [`crate::compile`] would have produced in a
/// condition position, i.e. one that evaluates to exactly `0` or `1`.
pub(crate) fn is_encoded_bool(e: &Expr) -> bool {
    match e {
        Expr::Binary { op, lhs, rhs, .. } => {
            if op.is_comparison() {
                // `((A + B) > 0)` is itself a comparison; it is recognised as
                // an `or` by `deencode`, and either way it is a boolean.
                return true;
            }
            match op {
                BinOp::Mul => is_encoded_bool(lhs) && is_encoded_bool(rhs),
                BinOp::Sub => {
                    matches!(&**lhs, Expr::Number(n) if n.raw == "1") && is_encoded_bool(rhs)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Turn the arithmetic encoding back into `and`, `or` and `not`.
///
/// The recognisers are deliberately exact, not heuristic. `(A * B)` is an `and`
/// only when both sides are themselves boolean-shaped; `(x * 2)` where `x` is a
/// comparison is left as arithmetic, because a definition that really multiplies
/// a flag by two means that, and rewriting it as `and` would change it.
pub(crate) fn deencode(e: &Expr) -> Expr {
    match e {
        Expr::Binary { op, lhs, rhs, span } => {
            // `((A + B) > 0)` → `A or B`
            if *op == BinOp::Gt {
                if let (
                    Expr::Binary {
                        op: BinOp::Add,
                        lhs: a,
                        rhs: b,
                        ..
                    },
                    Expr::Number(zero),
                ) = (&**lhs, &**rhs)
                {
                    if zero.raw == "0" && is_encoded_bool(a) && is_encoded_bool(b) {
                        return Expr::Binary {
                            op: BinOp::Or,
                            lhs: Box::new(deencode(a)),
                            rhs: Box::new(deencode(b)),
                            span: *span,
                        };
                    }
                }
            }
            // `(A * B)` → `A and B`
            if *op == BinOp::Mul && is_encoded_bool(lhs) && is_encoded_bool(rhs) {
                return Expr::Binary {
                    op: BinOp::And,
                    lhs: Box::new(deencode(lhs)),
                    rhs: Box::new(deencode(rhs)),
                    span: *span,
                };
            }
            // `(1 - A)` → `not A`
            if *op == BinOp::Sub
                && matches!(&**lhs, Expr::Number(n) if n.raw == "1")
                && is_encoded_bool(rhs)
            {
                return Expr::Unary {
                    op: UnOp::Not,
                    rhs: Box::new(deencode(rhs)),
                    span: *span,
                };
            }
            Expr::Binary {
                op: *op,
                lhs: Box::new(deencode(lhs)),
                rhs: Box::new(deencode(rhs)),
                span: *span,
            }
        }
        Expr::Unary { op, rhs, span } => Expr::Unary {
            op: *op,
            rhs: Box::new(deencode(rhs)),
            span: *span,
        },
        Expr::Call {
            name,
            args,
            span,
            name_span,
        } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(deencode).collect(),
            span: *span,
            name_span: *name_span,
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::parse::parse;

    fn def_of(src: &str) -> StrategyDefinition {
        let ast = parse(src).unwrap_or_else(|e| panic!("parse: {e:#?}"));
        compile(&ast).unwrap_or_else(|e| panic!("compile: {e:#?}"))
    }

    #[test]
    fn the_trend_skeleton_reads_back_as_the_clauses_it_was_written_as() {
        let src = "\
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
        let text = decompile(&def_of(src)).expect("decompiles");
        assert_eq!(
            text,
            "\
strategy ema_cross on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
exit time 48 bars
risk max_position 0.5
"
        );
    }

    /// The names an author chose for inputs are not stored anywhere, so they
    /// cannot come back; the *features* do, and a feature read twice gets a
    /// name because the parser requires one.
    #[test]
    fn a_feature_read_twice_is_given_a_name_so_the_text_parses() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
entry long when between(zscore(close, 20), 0, 2)
size fixed 1
exit none
";
        let text = decompile(&def_of(src)).expect("decompiles");
        assert!(
            text.contains("input zscore_20 = zscore(close, 20)"),
            "{text}"
        );
        assert!(
            text.contains("entry long when zscore_20 >= 0 and zscore_20 <= 2"),
            "{text}"
        );
        parse(&text).expect("the decompiler's own output parses");
    }

    #[test]
    fn the_boolean_encoding_is_read_back_as_and_or_and_not() {
        let src = "\
strategy s on $bound_at_init timeframe 1h
entry long when not (close > 1 or close < 0) and volume > 10
size fixed 1
exit none
";
        let text = decompile(&def_of(src)).expect("decompiles");
        assert!(
            text.contains("entry long when not (close > 1 or close < 0) and volume > 10"),
            "{text}"
        );
    }

    /// Arithmetic that happens to look like the encoding is left alone. A
    /// definition that multiplies a flag by two means that.
    #[test]
    fn arithmetic_that_is_not_the_encoding_is_not_rewritten() {
        let parsed = crate::parse::expr_from_str("((bar('close') > 1) * 2)").expect("parses");
        let back = deencode(&parsed);
        match back {
            Expr::Binary { op, .. } => assert_eq!(op, BinOp::Mul, "left as arithmetic"),
            other => panic!("expected a product, got {other:?}"),
        }
    }

    #[test]
    fn an_ai_node_is_refused_by_the_name_of_its_kind() {
        let mut def = def_of(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit none
",
        );
        def.nodes.push(domain::strategy_def::nodes::Node {
            id: "m1".into(),
            kind: NodeKind::ModelForecast {
                model_ref: "price_forecaster".into(),
                alias: "production".into(),
                direction: "bullish".into(),
                min_confidence: 0.6,
                input: None,
            },
        });
        let d = decompile(&def).expect_err("refused");
        assert_eq!(d.code, Code::NotExpressible);
        assert!(d.message.contains("model_forecast"), "{}", d.message);
    }

    /// Declaring a feature nothing reads is a real defect — the runtime would
    /// subscribe to a feature no rule uses, and the reverse fails inside the
    /// engine after a trial is spent. Either way there is no honest text.
    #[test]
    fn a_definition_whose_declared_features_disagree_with_its_rules_is_refused() {
        let mut def = def_of(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 1
exit none
",
        );
        def.inputs[1].features.push("ema_200".into());
        let d = decompile(&def).expect_err("refused");
        assert_eq!(d.code, Code::NotCanonical);
        assert!(d.message.contains("ema_200"), "{}", d.message);
    }

    /// The canonicalisation boundary, stated rather than assumed: a definition
    /// whose node identifiers were chosen by hand still reads back, and
    /// recompiling gives it the generated names. Behaviour identical, JSON not
    /// byte-identical — which is why the round-trip property is stated over
    /// canonical definitions.
    #[test]
    fn decompiling_a_legacy_definition_canonicalises_its_node_ids() {
        let mut def = def_of(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit none
",
        );
        def.nodes[1] = domain::strategy_def::nodes::Node {
            id: "n2".into(),
            kind: NodeKind::Signal {
                when: "n1".into(),
                emit: "long".into(),
            },
        };
        def.nodes[0].id = "n1".into();

        let text = decompile(&def).expect("decompiles");
        let again = compile(&parse(&text).expect("parses")).expect("compiles");
        assert_ne!(again.nodes[0].id, def.nodes[0].id);
        assert_eq!(again.nodes[0].id, "e0_cond");
        // Everything that is not an identifier is unchanged.
        assert_eq!(again.actions, def.actions);
        assert_eq!(again.exits, def.exits);
        match (&again.nodes[0].kind, &def.nodes[0].kind) {
            (NodeKind::Condition { expr: a }, NodeKind::Condition { expr: b }) => {
                assert_eq!(a, b);
            }
            _ => panic!("expected two condition nodes"),
        }
    }

    #[test]
    fn a_feature_the_catalogue_does_not_know_keeps_its_v1_form() {
        assert_eq!(
            render_feature("frobnicate_12", &Catalogue::engine()),
            "feature('frobnicate_12')"
        );
        assert_eq!(
            render_feature("ema_{{fast}}", &Catalogue::engine()),
            "ema(close, fast)"
        );
        assert_eq!(render_feature("obv", &Catalogue::engine()), "obv()");
    }

    #[test]
    fn an_instrument_that_is_not_an_identifier_is_quoted() {
        assert_eq!(render_instrument("$bound_at_init"), "$bound_at_init");
        assert_eq!(render_instrument("BTC-USD"), "'BTC-USD'");
    }
}
