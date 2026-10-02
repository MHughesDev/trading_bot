//! Graph → plan (FEAT-007 §5, ADR-0045).
//!
//! The only typechecker for strategies. It reads three registries — operators
//! (`domain::strategy_graph::ops`), indicators (DATA-006), model contracts — and
//! nothing else defines what a strategy may say. It reports **every** problem it
//! can find in one pass, each as a teachable [`Diagnostic`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use domain::model_def::contract::{FeatureSource, InputShape};
use domain::strategy_def::params::{self, ParamValues};
use domain::strategy_graph::graph::{Arg, InlineNode, InstrumentRef, RefPath, Sink};
use domain::strategy_graph::intent::{self, DistanceUnit, TradeIntent};
use domain::strategy_graph::ops::{self, OpSpec, Ty, Warmup};
use domain::strategy_graph::{Num, StrategyGraph, StrategyKind, Value, ValueType};

use crate::diagnostic::{nearest, Diagnostic};
use crate::plan::{
    BarField, CompiledRule, CompiledSelect, InputIdx, ModelArg, ModelCall, NodeKind, Operand, Plan,
    PlanInput, PlanNode, Slot, StateField, PRIMARY,
};
use crate::registry::Registries;

/// Compile a graph with parameter `overrides` (empty for defaults).
///
/// # Errors
/// Every problem found, in the order they appear in the graph.
pub fn compile(
    graph: &StrategyGraph,
    reg: &dyn Registries,
    overrides: &ParamValues,
) -> Result<Plan, Vec<Diagnostic>> {
    let mut c = Compiler::new(graph, reg);
    c.run(overrides);
    if c.diags.is_empty() {
        Ok(c.finish())
    } else {
        Err(c.diags)
    }
}

/// A node before typechecking: the graph's nodes plus every inline node lifted
/// out with an id of its own.
#[derive(Clone, Debug)]
struct RawNode {
    id: String,
    op: String,
    args: BTreeMap<String, Arg>,
}

/// A compile-time type: a value type, or a feature window (which only flows into
/// a model input and never occupies a value slot).
#[derive(Clone, Debug, PartialEq)]
enum CType {
    Value(ValueType),
    Window {
        input: InputIdx,
        columns: Vec<String>,
        lookback: u32,
    },
}

/// Type variables bound while checking one node.
#[derive(Default)]
struct Subst {
    types: BTreeMap<char, ValueType>,
    labels: BTreeMap<char, Vec<String>>,
}

struct Compiler<'a> {
    graph: &'a StrategyGraph,
    reg: &'a dyn Registries,
    diags: Vec<Diagnostic>,
    params: ParamValues,
    inputs: Vec<PlanInput>,
    raw: Vec<RawNode>,
    /// raw node id → index in `raw`.
    raw_index: BTreeMap<String, usize>,
    /// compiled nodes in topological order.
    nodes: Vec<PlanNode>,
    /// node id → its slot in `nodes`, once compiled.
    slots: BTreeMap<String, Slot>,
    /// `features(...)` nodes: id → window type. They have no slot.
    windows: BTreeMap<String, CType>,
    rules: Vec<CompiledRule>,
    select: Option<CompiledSelect>,
    atr_columns: BTreeMap<u32, usize>,
}

impl<'a> Compiler<'a> {
    fn new(graph: &'a StrategyGraph, reg: &'a dyn Registries) -> Self {
        let mut inputs = vec![PlanInput {
            id: "$self".into(),
            instrument: InstrumentRef::Bound,
            timeframe: graph.timeframe,
            columns: Vec::new(),
            history_bars: 0,
        }];
        for d in &graph.inputs {
            inputs.push(PlanInput {
                id: d.id.clone(),
                instrument: d.instrument.clone(),
                timeframe: d.timeframe,
                columns: Vec::new(),
                history_bars: 0,
            });
        }
        Self {
            graph,
            reg,
            diags: Vec::new(),
            params: ParamValues::new(),
            inputs,
            raw: Vec::new(),
            raw_index: BTreeMap::new(),
            nodes: Vec::new(),
            slots: BTreeMap::new(),
            windows: BTreeMap::new(),
            rules: Vec::new(),
            select: None,
            atr_columns: BTreeMap::new(),
        }
    }

    fn diag(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    fn run(&mut self, overrides: &ParamValues) {
        match params::resolve_declared(&self.graph.params, &self.graph.constraints, overrides) {
            Ok(p) => self.params = p,
            Err(e) => self.diag(Diagnostic::new(
                "params",
                "params",
                e.to_string(),
                "every parameter needs a value inside its declared range, and every constraint must hold",
            )),
        }
        self.lift_nodes();
        let Some(order) = self.topological_order() else {
            return;
        };
        for i in order {
            let raw = self.raw[i].clone();
            self.compile_node(&raw);
        }
        match &self.graph.sink {
            Sink::Decide(d) => {
                if self.graph.kind != StrategyKind::Trading {
                    return;
                }
                for (i, rule) in d.rules.iter().enumerate() {
                    self.compile_rule(i, rule);
                }
            }
            Sink::Select { select, universe } => self.compile_select(select, universe),
        }
    }

    // ── 1. lift inline nodes ─────────────────────────────────────────────────

    fn lift_nodes(&mut self) {
        let nodes = self.graph.nodes.clone();
        for n in &nodes {
            let args = self.lift_args(&n.id, &n.args);
            self.push_raw(RawNode {
                id: n.id.clone(),
                op: n.op.clone(),
                args,
            });
        }
    }

    fn push_raw(&mut self, n: RawNode) {
        self.raw_index.insert(n.id.clone(), self.raw.len());
        self.raw.push(n);
    }

    fn lift_args(&mut self, owner: &str, args: &BTreeMap<String, Arg>) -> BTreeMap<String, Arg> {
        args.iter()
            .map(|(port, a)| (port.clone(), self.lift_arg(&format!("{owner}~{port}"), a)))
            .collect()
    }

    /// Replace inline nodes in `a` with references to lifted nodes named from `at`.
    fn lift_arg(&mut self, at: &str, a: &Arg) -> Arg {
        match a {
            Arg::Inline(n) => {
                let InlineNode { op, args } = n.as_ref();
                let args = self.lift_args(at, args);
                self.push_raw(RawNode {
                    id: at.to_string(),
                    op: op.clone(),
                    args,
                });
                Arg::Ref(at.to_string())
            }
            Arg::List(xs) => Arg::List(
                xs.iter()
                    .enumerate()
                    .map(|(i, x)| self.lift_arg(&format!("{at}~{i}"), x))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    // ── 2. order ─────────────────────────────────────────────────────────────

    /// The node a reference string points at, if it points at a node.
    fn node_of_ref(&self, s: &str) -> Option<usize> {
        if let Some(&i) = self.raw_index.get(s) {
            return Some(i);
        }
        match RefPath::parse(s).ok()? {
            RefPath::Name { name, .. } => self.raw_index.get(&name).copied(),
            _ => None,
        }
    }

    /// Kahn's algorithm, ties broken by declaration order so the plan — and so
    /// every trace — is deterministic. `None` after reporting a cycle.
    fn topological_order(&mut self) -> Option<Vec<usize>> {
        let n = self.raw.len();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (i, node) in self.raw.iter().enumerate() {
            let mut refs = Vec::new();
            node.args.values().for_each(|a| a.references(&mut refs));
            for r in refs {
                if let Some(j) = self.node_of_ref(&r) {
                    deps[i].insert(j);
                }
            }
        }
        let mut indegree: Vec<usize> = deps.iter().map(BTreeSet::len).collect();
        let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, ds) in deps.iter().enumerate() {
            for &d in ds {
                users[d].push(i);
            }
        }
        let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(&i) = ready.iter().next() {
            ready.remove(&i);
            order.push(i);
            for &u in &users[i] {
                indegree[u] -= 1;
                if indegree[u] == 0 {
                    ready.insert(u);
                }
            }
        }
        if order.len() == n {
            return Some(order);
        }
        // Name one cycle: walk dependencies from any unordered node until a
        // node repeats.
        let done: BTreeSet<usize> = order.iter().copied().collect();
        let start = (0..n).find(|i| !done.contains(i)).unwrap_or(0);
        let mut path = vec![start];
        let mut at = start;
        loop {
            let next = deps[at].iter().copied().find(|d| !done.contains(d));
            let Some(next) = next else { break };
            if let Some(pos) = path.iter().position(|&p| p == next) {
                let cycle: Vec<String> = path[pos..]
                    .iter()
                    .map(|&p| self.raw[p].id.clone())
                    .collect();
                self.diag(
                    Diagnostic::new(
                        "cycle",
                        cycle[0].clone(),
                        format!("these nodes depend on each other in a loop: {}", cycle.join(" → ")),
                        "a value cannot depend on itself this tick; to use last tick's value, wrap one edge in `prev(x, 1)`",
                    )
                    .example("{\"op\":\"prev\",\"args\":{\"x\":\"signal\",\"n\":1}}"),
                );
                return None;
            }
            path.push(next);
            at = next;
        }
        self.diag(Diagnostic::new(
            "cycle",
            self.raw[start].id.clone(),
            "this node is part of a dependency loop",
            "break the loop with `prev(x, 1)`",
        ));
        None
    }

    // ── 3. nodes ─────────────────────────────────────────────────────────────

    fn compile_node(&mut self, raw: &RawNode) {
        let Some(spec) = ops::op(&raw.op) else {
            let names: Vec<&str> = ops::ops().iter().map(|o| o.name).collect();
            let mut d = Diagnostic::new(
                "unknown_op",
                &raw.id,
                format!("there is no operator '{}'", raw.op),
                nearest(&raw.op, names.iter().copied()).map_or_else(
                    || "see the operator catalogue for what exists".to_string(),
                    |n| format!("did you mean `{n}`?"),
                ),
            );
            if raw.op.starts_with("ema") || raw.op.starts_with("rsi") || raw.op.contains('_') {
                d = d.example("{\"op\":\"feature\",\"args\":{\"name\":{\"str\":\"rsi_14\"}}}");
            }
            self.diag(d);
            return;
        };
        if !spec.allowed_in(self.graph.kind) {
            self.diag(Diagnostic::new(
                "cross_sectional_in_trading",
                &raw.id,
                format!("`{}` compares instruments across a universe, and a trading strategy sees one instrument", spec.name),
                "move the ranking into a selection strategy and read its result with `selection_attr`",
            ));
            return;
        }
        match spec.name {
            "parse" => self.not_yet(raw, "U-5.6 (LLM outputs as typed values)"),
            "selection_attr" => self.not_yet(raw, "U-6.3 (running a selection)"),
            "feature" => self.compile_feature(raw, spec),
            "bar" => self.compile_bar(raw, spec),
            "hour" | "weekday" => self.compile_clock(raw, spec),
            "features" => self.compile_window(raw, spec),
            "model" => self.compile_model(raw),
            _ => {
                if let Some(lowered) = self.lowered_indicator(raw, spec) {
                    self.compile_feature_named(raw, lowered.0, &lowered.1);
                } else {
                    self.compile_op(raw, spec);
                }
            }
        }
    }

    fn not_yet(&mut self, raw: &RawNode, when: &str) {
        self.diag(Diagnostic::new(
            "not_yet_available",
            &raw.id,
            format!("`{}` is specified (FEAT-007) but not built yet", raw.op),
            format!("it arrives with Set U task {when}; until then express this another way"),
        ));
    }

    fn push_node(&mut self, id: &str, kind: NodeKind, ty: ValueType, warmup: u32) -> Slot {
        let slot = self.nodes.len();
        self.nodes.push(PlanNode {
            id: id.to_string(),
            kind,
            ty,
            warmup,
        });
        self.slots.insert(id.to_string(), slot);
        slot
    }

    /// The column index of indicator `name` on `input`, registering it.
    fn column(&mut self, input: InputIdx, name: &str, lookback: u32) -> usize {
        let inp = &mut self.inputs[input];
        inp.history_bars = inp.history_bars.max(lookback);
        if let Some(i) = inp.columns.iter().position(|c| c == name) {
            return i;
        }
        inp.columns.push(name.to_string());
        inp.columns.len() - 1
    }

    /// A string literal (or enum parameter) with `{{param}}` interpolated.
    fn literal_text(&mut self, at: &str, port: &str, a: &Arg) -> Option<String> {
        let raw = match a {
            Arg::Str(s) => s.str.clone(),
            Arg::Ref(r) if r.starts_with('$') => match self.params.get(&r[1..]) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => {
                    self.unknown_param(at, &r[1..]);
                    return None;
                }
            },
            _ => {
                self.diag(
                    Diagnostic::new(
                        "literal_required",
                        at,
                        format!("`{port}` must be a text literal"),
                        "write it as {\"str\": \"…\"}",
                    )
                    .example("{\"str\": \"rsi_14\"}"),
                );
                return None;
            }
        };
        self.interpolate(at, &raw)
    }

    fn interpolate(&mut self, at: &str, s: &str) -> Option<String> {
        let mut out = String::new();
        let mut rest = s;
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after = &rest[open + 2..];
            let Some(close) = after.find("}}") else {
                out.push_str(&rest[open..]);
                return Some(out);
            };
            let name = after[..close].trim();
            if let Some(v) = self.params.get(name) {
                out.push_str(&params::literal(v));
            } else {
                self.unknown_param(at, name);
                return None;
            }
            rest = &after[close + 2..];
        }
        out.push_str(rest);
        Some(out)
    }

    fn unknown_param(&mut self, at: &str, name: &str) {
        let names: Vec<String> = self.graph.params.keys().cloned().collect();
        self.diag(Diagnostic::new(
            "unknown_param",
            at,
            format!("there is no parameter '{name}'"),
            nearest(name, names.iter().map(String::as_str)).map_or_else(
                || "declare it under `params`".to_string(),
                |n| format!("did you mean `${n}`?"),
            ),
        ));
    }

    /// A literal number (or numeric parameter).
    fn literal_num(&mut self, at: &str, port: &str, a: &Arg) -> Option<f64> {
        match a {
            Arg::Num(x) => Some(*x),
            Arg::Ref(r) if r.starts_with('$') => {
                match self.params.get(&r[1..]).and_then(serde_json::Value::as_f64) {
                    Some(x) => Some(x),
                    None if self.params.contains_key(&r[1..]) => {
                        self.diag(Diagnostic::new(
                            "literal_required",
                            at,
                            format!("parameter `{r}` is not a number, and `{port}` needs one"),
                            "use an int or float parameter",
                        ));
                        None
                    }
                    None => {
                        self.unknown_param(at, &r[1..]);
                        None
                    }
                }
            }
            _ => {
                self.diag(Diagnostic::new(
                    "literal_required",
                    at,
                    format!("`{port}` must be a number fixed before the run, not a value computed each tick"),
                    "write a number or a `$param`",
                ));
                None
            }
        }
    }

    /// A positive whole number of ticks or bars.
    fn literal_count(&mut self, at: &str, port: &str, a: &Arg) -> Option<u32> {
        let x = self.literal_num(at, port, a)?;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::float_cmp
        )]
        if x >= 1.0 && x.fract() == 0.0 && x <= f64::from(u32::MAX) {
            return Some(x as u32);
        }
        self.diag(Diagnostic::new(
            "bad_count",
            at,
            format!("`{port}` is {x}; it counts bars, so it must be a whole number of at least 1"),
            "use 1, 2, 3, …",
        ));
        None
    }

    fn input_named(&mut self, at: &str, name: &str) -> Option<InputIdx> {
        if let Some(i) = self.inputs.iter().position(|inp| inp.id == name) {
            return Some(i);
        }
        let ids: Vec<String> = self.inputs.iter().skip(1).map(|i| i.id.clone()).collect();
        self.diag(Diagnostic::new(
            "unknown_input",
            at,
            format!("there is no declared input '{name}'"),
            nearest(name, ids.iter().map(String::as_str)).map_or_else(
                || "declare it under `inputs` with an instrument and timeframe".to_string(),
                |n| format!("did you mean `{n}`?"),
            ),
        ));
        None
    }

    fn compile_feature(&mut self, raw: &RawNode, spec: &OpSpec) {
        self.check_ports(raw, spec);
        let Some(a) = raw.args.get("name") else {
            return;
        };
        let Some(name) = self.literal_text(&raw.id, "name", a) else {
            return;
        };
        let input = match raw.args.get("input") {
            Some(a) => match self.literal_text(&raw.id, "input", a) {
                Some(n) => match self.input_named(&raw.id, &n) {
                    Some(i) => i,
                    None => return,
                },
                None => return,
            },
            None => PRIMARY,
        };
        self.compile_feature_named(raw, input, &name);
    }

    fn compile_feature_named(&mut self, raw: &RawNode, input: InputIdx, name: &str) {
        let Some(lookback) = self.reg.feature_lookback(name) else {
            let known = features::feature_sets::known_features();
            self.diag(
                Diagnostic::new(
                    "unknown_feature",
                    &raw.id,
                    format!("there is no indicator '{name}'"),
                    nearest(name, known.iter().copied()).map_or_else(
                        || {
                            "indicator names are family_params, e.g. rsi_14, ema_20, atr_14"
                                .to_string()
                        },
                        |n| format!("did you mean `{n}`?"),
                    ),
                )
                .example("{\"op\":\"feature\",\"args\":{\"name\":{\"str\":\"rsi_14\"}}}"),
            );
            return;
        };
        let column = self.column(input, name, lookback);
        // Ticks of warm-up only count on the primary input, whose bars are the
        // ticks; other inputs are preloaded to their own history.
        let warmup = if input == PRIMARY { lookback } else { 0 };
        self.push_node(
            &raw.id,
            NodeKind::Feature {
                input,
                column,
                name: name.to_string(),
                lookback,
            },
            ValueType::Num,
            warmup,
        );
    }

    /// `ema`/`rolling_*` of a bar's close with a literal period is the DATA-006
    /// indicator of the same name — computed by the one indicator engine
    /// (ADR-0029), not by a second implementation here.
    fn lowered_indicator(&mut self, raw: &RawNode, spec: &OpSpec) -> Option<(InputIdx, String)> {
        let family = match spec.name {
            "ema" | "rolling_mean" | "rolling_std" | "rolling_min" | "rolling_max"
            | "rolling_sum" => spec.name,
            _ => return None,
        };
        let x = raw.args.get("x")?.reference().ok()??;
        let input = match x {
            RefPath::Name {
                ref name,
                field: None,
            } if name == "close" && !self.raw_index.contains_key(name) => PRIMARY,
            RefPath::Name {
                ref name,
                field: Some(ref f),
            } if f == "close" && !self.raw_index.contains_key(name) => {
                self.inputs.iter().position(|i| &i.id == name)?
            }
            _ => return None,
        };
        let n = match raw.args.get("n")? {
            Arg::Num(x) => *x,
            Arg::Ref(r) if r.starts_with('$') => self.params.get(&r[1..])?.as_f64()?,
            _ => return None,
        };
        #[allow(
            clippy::float_cmp,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        if n < 1.0 || n.fract() != 0.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let name = format!("{family}_{}", n as u32);
        self.reg.feature_lookback(&name).map(|_| (input, name))
    }

    fn compile_bar(&mut self, raw: &RawNode, spec: &OpSpec) {
        self.check_ports(raw, spec);
        let Some(a) = raw.args.get("field") else {
            return;
        };
        let Some(f) = self.literal_text(&raw.id, "field", a) else {
            return;
        };
        let Some(field) = BarField::parse(&f) else {
            self.diag(Diagnostic::new(
                "unknown_bar_field",
                &raw.id,
                format!("'{f}' is not a bar field"),
                "use open, high, low, close or volume",
            ));
            return;
        };
        let input = match raw.args.get("input") {
            Some(a) => match self
                .literal_text(&raw.id, "input", a)
                .and_then(|n| self.input_named(&raw.id, &n))
            {
                Some(i) => i,
                None => return,
            },
            None => PRIMARY,
        };
        let inp = &mut self.inputs[input];
        inp.history_bars = inp.history_bars.max(1);
        self.push_node(&raw.id, NodeKind::Bar { input, field }, ValueType::Num, 0);
    }

    fn compile_clock(&mut self, raw: &RawNode, spec: &OpSpec) {
        self.check_ports(raw, spec);
        let kind = if spec.name == "hour" {
            NodeKind::Hour
        } else {
            NodeKind::Weekday
        };
        self.push_node(&raw.id, kind, ValueType::Num, 0);
    }

    fn compile_window(&mut self, raw: &RawNode, spec: &OpSpec) {
        self.check_ports(raw, spec);
        let (Some(set_arg), Some(lb_arg)) = (raw.args.get("set"), raw.args.get("lookback")) else {
            return;
        };
        let Some(set) = self.literal_text(&raw.id, "set", set_arg) else {
            return;
        };
        let Some(lookback) = self.literal_count(&raw.id, "lookback", lb_arg) else {
            return;
        };
        let input = match raw.args.get("timeframe") {
            Some(a) => match self.literal_text(&raw.id, "timeframe", a) {
                Some(tf) => match self.input_at_timeframe(&raw.id, &tf) {
                    Some(i) => i,
                    None => return,
                },
                None => return,
            },
            None => PRIMARY,
        };
        let Some(columns) = self.feature_list(&raw.id, &FeatureSource::Set { name: set }) else {
            return;
        };
        self.windows.insert(
            raw.id.clone(),
            CType::Window {
                input,
                columns,
                lookback,
            },
        );
    }

    /// The input bound to this instance at `tf`: the primary, or a declared
    /// `Bound` input at that timeframe.
    fn input_at_timeframe(&mut self, at: &str, tf: &str) -> Option<InputIdx> {
        if let Some(i) = self
            .inputs
            .iter()
            .position(|inp| inp.instrument == InstrumentRef::Bound && inp.timeframe.key() == tf)
        {
            return Some(i);
        }
        self.diag(
            Diagnostic::new(
                "undeclared_timeframe",
                at,
                format!("nothing supplies this instrument's {tf} bars"),
                format!("declare an input for the bound instrument at {tf}"),
            )
            .example(format!("{{\"id\":\"self_{tf}\",\"instrument\":{{\"kind\":\"bound\"}},\"timeframe\":\"{tf}\"}}")),
        );
        None
    }

    /// The feature names a source names, each checked against the indicator
    /// registry. A model input the engine cannot compute is a compile error —
    /// never a zero sent in its place (FEAT-007 §1 defect 4).
    fn feature_list(&mut self, at: &str, src: &FeatureSource) -> Option<Vec<String>> {
        let names = match src {
            FeatureSource::Named { features } => features.clone(),
            FeatureSource::Set { name } => {
                if let Some(f) = self.reg.feature_set(name) {
                    f
                } else {
                    self.diag(Diagnostic::new(
                        "unknown_feature_set",
                        at,
                        format!("there is no feature set '{name}'"),
                        "list the feature sets in the catalogue, or name the features explicitly",
                    ));
                    return None;
                }
            }
        };
        let unknown: Vec<String> = names
            .iter()
            .filter(|n| self.reg.feature_lookback(n).is_none())
            .cloned()
            .collect();
        if !unknown.is_empty() {
            self.diag(Diagnostic::new(
                "uncomputable_feature",
                at,
                format!("the engine cannot compute {}: {}", if unknown.len() == 1 { "this feature" } else { "these features" }, unknown.join(", ")),
                "a model's inputs must all be DATA-006 indicators the engine computes; retrain on computable features or add the family",
            ));
            return None;
        }
        Some(names)
    }

    /// Ports the spec does not declare (and that are not open inputs) and
    /// required ports that are absent.
    fn check_ports(&mut self, raw: &RawNode, spec: &OpSpec) {
        for p in spec.inputs {
            if !p.optional && !raw.args.contains_key(p.name) {
                let ports: Vec<&str> = spec.inputs.iter().map(|p| p.name).collect();
                self.diag(Diagnostic::new(
                    "missing_arg",
                    &raw.id,
                    format!("`{}` needs `{}`", spec.name, p.name),
                    format!("`{}` takes {}", spec.name, ports.join(", ")),
                ));
            }
        }
        if spec.open_inputs {
            return;
        }
        for k in raw.args.keys() {
            if spec.port(k).is_none() {
                let ports: Vec<&str> = spec.inputs.iter().map(|p| p.name).collect();
                self.diag(Diagnostic::new(
                    "unknown_arg",
                    &raw.id,
                    format!("`{}` has no argument `{k}`", spec.name),
                    nearest(k, ports.iter().copied()).map_or_else(
                        || format!("`{}` takes {}", spec.name, ports.join(", ")),
                        |n| format!("did you mean `{n}`?"),
                    ),
                ));
            }
        }
    }

    /// Compile a wire argument to an operand and its type.
    fn operand(&mut self, at: &str, a: &Arg) -> Option<(Operand, CType)> {
        match a {
            Arg::Num(x) => {
                if let Some(n) = Num::new(*x) {
                    Some((Operand::Const(Value::Num(n)), CType::Value(ValueType::Num)))
                } else {
                    self.diag(Diagnostic::new(
                        "bad_number",
                        at,
                        format!("{x} is not a finite number"),
                        "use a finite number",
                    ));
                    None
                }
            }
            Arg::Bool(b) => Some((
                Operand::Const(Value::Bool(*b)),
                CType::Value(ValueType::Bool),
            )),
            Arg::Str(s) => {
                let text = self.interpolate(at, &s.str)?;
                Some((
                    Operand::Const(Value::Text(text)),
                    CType::Value(ValueType::Text),
                ))
            }
            Arg::List(xs) => {
                let mut ops_ = Vec::new();
                let mut ty: Option<ValueType> = None;
                for x in xs {
                    let (o, t) = self.operand(at, x)?;
                    let CType::Value(t) = t else {
                        self.window_misuse(at);
                        return None;
                    };
                    if let Some(prev) = &ty {
                        if !compatible(prev, &t) {
                            self.diag(Diagnostic::new(
                                "mixed_list",
                                at,
                                format!("a list mixes {prev} and {t}"),
                                "every element of a list must have the same type",
                            ));
                            return None;
                        }
                    }
                    ty = Some(t);
                    ops_.push(o);
                }
                // A list's type is recorded as a record of its element type
                // under "*"; the port checker unwraps it.
                let elem = ty.unwrap_or(ValueType::Num);
                Some((
                    Operand::List(ops_),
                    CType::Value(ValueType::Record {
                        fields: BTreeMap::from([("*".to_string(), elem)]),
                    }),
                ))
            }
            Arg::Inline(_) => {
                // Lifted before typechecking; an inline here is a compiler bug.
                self.diag(Diagnostic::new(
                    "internal",
                    at,
                    "an inline node was not lifted",
                    "report this",
                ));
                None
            }
            Arg::Ref(s) => self.resolve_ref(at, s),
        }
    }

    fn window_misuse(&mut self, at: &str) {
        self.diag(Diagnostic::new(
            "window_misuse",
            at,
            "a `features(...)` window is not a value; it can only be a model input",
            "pass it as one of a model's inputs",
        ));
    }

    fn resolve_ref(&mut self, at: &str, s: &str) -> Option<(Operand, CType)> {
        // Lifted inline nodes and plain node ids.
        if let Some(&slot) = self.slots.get(s) {
            return Some((
                Operand::Slot(slot),
                CType::Value(self.nodes[slot].ty.clone()),
            ));
        }
        if let Some(w) = self.windows.get(s) {
            return Some((Operand::Const(Value::Bool(false)), w.clone()));
        }
        let path = match RefPath::parse(s) {
            Ok(p) => p,
            Err(e) => {
                self.diag(Diagnostic::new(
                    "bad_ref",
                    at,
                    e.to_string(),
                    "write `node`, `node.field`, `$param` or `state.field`",
                ));
                return None;
            }
        };
        match path {
            RefPath::Param(p) => match self.params.get(&p).cloned() {
                Some(serde_json::Value::String(t)) => Some((
                    Operand::Const(Value::Text(t)),
                    CType::Value(ValueType::Text),
                )),
                Some(v) => {
                    let x = v.as_f64().and_then(Num::new)?;
                    Some((Operand::Const(Value::Num(x)), CType::Value(ValueType::Num)))
                }
                None => {
                    self.unknown_param(at, &p);
                    None
                }
            },
            RefPath::State(f) => {
                if let Some(field) = StateField::parse(&f) {
                    Some((Operand::State(field), CType::Value(field.value_type())))
                } else {
                    let names: Vec<&str> = ops::STATE_FIELDS.iter().map(|(n, _)| *n).collect();
                    self.diag(Diagnostic::new(
                        "unknown_state",
                        at,
                        format!("there is no `state.{f}`"),
                        nearest(&f, names.iter().copied()).map_or_else(
                            || format!("state fields: {}", names.join(", ")),
                            |n| format!("did you mean `state.{n}`?"),
                        ),
                    ));
                    None
                }
            }
            RefPath::Name { name, field } => {
                if let Some(&slot) = self.slots.get(&name) {
                    let ty = self.nodes[slot].ty.clone();
                    return match field {
                        None => Some((Operand::Slot(slot), CType::Value(ty))),
                        Some(f) => {
                            if let ValueType::Record { fields } = &ty {
                                if let Some(ft) = fields.get(&f) {
                                    return Some((
                                        Operand::Field(slot, f),
                                        CType::Value(ft.clone()),
                                    ));
                                }
                                let names: Vec<String> = fields.keys().cloned().collect();
                                self.diag(Diagnostic::new(
                                    "unknown_field",
                                    at,
                                    format!("`{name}` has no field `{f}`"),
                                    nearest(&f, names.iter().map(String::as_str)).map_or_else(
                                        || format!("its fields are {}", names.join(", ")),
                                        |n| format!("did you mean `{name}.{n}`?"),
                                    ),
                                ));
                            } else {
                                self.diag(Diagnostic::new(
                                    "not_a_record",
                                    at,
                                    format!("`{name}` is a {ty}, which has no fields"),
                                    format!("refer to it as `{name}`"),
                                ));
                            }
                            None
                        }
                    };
                }
                if self.raw_index.contains_key(&name) {
                    // A node that exists but failed to compile: its own
                    // diagnostic already explains why.
                    return None;
                }
                if field.is_none() {
                    if let Some(bf) = BarField::parse(&name) {
                        let inp = &mut self.inputs[PRIMARY];
                        inp.history_bars = inp.history_bars.max(1);
                        return Some((Operand::Bar(PRIMARY, bf), CType::Value(ValueType::Num)));
                    }
                }
                if let Some(f) = &field {
                    if let Some(input) = self.inputs.iter().position(|i| i.id == name) {
                        if let Some(bf) = BarField::parse(f) {
                            let inp = &mut self.inputs[input];
                            inp.history_bars = inp.history_bars.max(1);
                            return Some((Operand::Bar(input, bf), CType::Value(ValueType::Num)));
                        }
                    }
                }
                let mut names: Vec<String> = self
                    .raw_index
                    .keys()
                    .filter(|k| !k.contains('~'))
                    .cloned()
                    .collect();
                names.extend(ops::BAR_FIELDS.iter().map(|s| (*s).to_string()));
                self.diag(Diagnostic::new(
                    "unknown_ref",
                    at,
                    format!("nothing is called `{s}`"),
                    nearest(&name, names.iter().map(String::as_str)).map_or_else(
                        || "refer to a node id, a bar field (open high low close volume), `$param` or `state.field`".to_string(),
                        |n| format!("did you mean `{n}`?"),
                    ),
                ));
                None
            }
        }
    }

    /// Check one argument against one port's signature type, binding variables.
    fn unify(&mut self, at: &str, port: &str, want: Ty, got: &CType, subst: &mut Subst) -> bool {
        let CType::Value(got) = got else {
            if want == Ty::Window {
                return true;
            }
            self.window_misuse(at);
            return false;
        };
        let ok = match want {
            Ty::Num => *got == ValueType::Num,
            Ty::Bool => *got == ValueType::Bool,
            Ty::Text => *got == ValueType::Text,
            Ty::Var(v) => match subst.types.get(&v) {
                Some(bound) if compatible(bound, got) => true,
                Some(_) => false,
                None => {
                    subst.types.insert(v, got.clone());
                    true
                }
            },
            Ty::CatOf(v) => match got {
                ValueType::Cat { labels } => bind_labels(subst, v, labels),
                _ => false,
            },
            Ty::ProbsOf(v) => match got {
                ValueType::Probs { labels } => bind_labels(subst, v, labels),
                _ => false,
            },
            Ty::AnyDist => matches!(got, ValueType::Dist { .. }),
            Ty::AnyVec => matches!(got, ValueType::Vec { .. }),
            Ty::AnyRecord => matches!(got, ValueType::Record { .. }),
            Ty::NumList | Ty::BoolList => {
                let elem = if want == Ty::NumList {
                    ValueType::Num
                } else {
                    ValueType::Bool
                };
                matches!(got, ValueType::Record { fields } if fields.get("*") == Some(&elem))
            }
            Ty::Window => false,
            Ty::FromContract | Ty::FromSchema => true,
        };
        if !ok {
            self.diag(Diagnostic::new(
                "type_mismatch",
                at,
                format!(
                    "`{port}` expects {}, and this is {got}",
                    describe(want, subst)
                ),
                match (want, got) {
                    (Ty::Bool, ValueType::Num) => {
                        "compare it to make a condition, e.g. `gt(x, 0)`".to_string()
                    }
                    (Ty::Num, ValueType::Bool) => {
                        "a condition is not a number; use `if(cond, 1, 0)` if you mean one"
                            .to_string()
                    }
                    (Ty::Num, ValueType::Record { .. }) => {
                        "pick a field of the model's output, e.g. `vol.sigma`".to_string()
                    }
                    _ => format!("wire a {} into `{port}`", describe(want, subst)),
                },
            ));
        }
        ok
    }

    fn output_type(spec: &OpSpec, subst: &Subst) -> ValueType {
        match spec.output {
            Ty::Num => ValueType::Num,
            Ty::Bool => ValueType::Bool,
            Ty::Text => ValueType::Text,
            Ty::Var(v) => subst.types.get(&v).cloned().unwrap_or(ValueType::Num),
            Ty::CatOf(v) => ValueType::Cat {
                labels: subst.labels.get(&v).cloned().unwrap_or_default(),
            },
            Ty::ProbsOf(v) => ValueType::Probs {
                labels: subst.labels.get(&v).cloned().unwrap_or_default(),
            },
            Ty::AnyDist => ValueType::Dist { levels: Vec::new() },
            Ty::AnyVec => ValueType::Vec { dim: None },
            _ => ValueType::Record {
                fields: BTreeMap::new(),
            },
        }
    }

    /// The generic path: every port typechecked, every literal read.
    #[allow(clippy::too_many_lines)]
    fn compile_op(&mut self, raw: &RawNode, spec: &'static OpSpec) {
        self.check_ports(raw, spec);
        let mut subst = Subst::default();
        let mut args: BTreeMap<String, Operand> = BTreeMap::new();
        let mut in_warmup: u32 = 0;
        let mut own_warmup: u32 = match spec.warmup {
            Warmup::Fixed(n) => n,
            _ => 0,
        };
        let mut ok = true;
        for port in spec.inputs {
            let Some(a) = raw.args.get(port.name) else {
                ok &= port.optional;
                continue;
            };
            if port.literal {
                match port.ty {
                    Ty::Num => {
                        let is_count = matches!(spec.warmup, Warmup::FromPort(p) if p == port.name)
                            || matches!((spec.name, port.name), ("vote", "k"));
                        if is_count {
                            if let Some(n) = self.literal_count(&raw.id, port.name, a) {
                                if matches!(spec.warmup, Warmup::FromPort(p) if p == port.name) {
                                    own_warmup = n;
                                }
                                // `f64::from(u32)` is always finite, so this `Value::num` is
                                // always a `num`.
                                args.insert(
                                    port.name.into(),
                                    Operand::Const(Value::num(f64::from(n), port.name)),
                                );
                            } else {
                                ok = false;
                            }
                        } else if let Some(x) = self.literal_num(&raw.id, port.name, a) {
                            if !self.check_literal_range(raw, spec, port.name, x) {
                                ok = false;
                            }
                            match Num::new(x) {
                                Some(n) => {
                                    args.insert(port.name.into(), Operand::Const(Value::Num(n)));
                                }
                                None => ok = false,
                            }
                        } else {
                            ok = false;
                        }
                    }
                    Ty::Text => match self.literal_text(&raw.id, port.name, a) {
                        Some(t) => {
                            if !self.check_label(raw, spec, port.name, &t, &subst) {
                                ok = false;
                            }
                            args.insert(port.name.into(), Operand::Const(Value::Text(t)));
                        }
                        None => ok = false,
                    },
                    Ty::NumList => {
                        if let Arg::List(xs) = a {
                            let mut vals = Vec::new();
                            for x in xs {
                                if let Some(n) = x.literal_num().and_then(Num::new) {
                                    vals.push(Operand::Const(Value::Num(n)));
                                } else {
                                    self.diag(Diagnostic::new(
                                        "literal_required",
                                        &raw.id,
                                        format!("`{}` must be a list of numbers", port.name),
                                        "e.g. [0.5, 0.3, 0.2]",
                                    ));
                                    ok = false;
                                }
                            }
                            args.insert(port.name.into(), Operand::List(vals));
                        } else {
                            self.diag(Diagnostic::new(
                                "literal_required",
                                &raw.id,
                                format!("`{}` must be a list of numbers", port.name),
                                "e.g. [0.5, 0.5]",
                            ));
                            ok = false;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            let Some((operand, ty)) = self.operand(&raw.id, a) else {
                ok = false;
                continue;
            };
            if let Operand::Slot(s) | Operand::Field(s, _) = &operand {
                in_warmup = in_warmup.max(self.nodes[*s].warmup);
            }
            if !self.unify(&raw.id, port.name, port.ty, &ty, &mut subst) {
                ok = false;
            }
            args.insert(port.name.into(), operand);
        }
        // switch: every other argument is a case, keyed by label.
        if spec.name == "switch" {
            let labels = subst.labels.get(&'L').cloned().unwrap_or_default();
            for (k, a) in &raw.args {
                if k == "on" || k == "default" {
                    continue;
                }
                if !labels.is_empty() && !labels.contains(k) {
                    self.diag(Diagnostic::new(
                        "unknown_label",
                        &raw.id,
                        format!("`{k}` is not one of the labels {}", labels.join(", ")),
                        "name each case after a label of `on`",
                    ));
                    ok = false;
                    continue;
                }
                let Some((operand, ty)) = self.operand(&raw.id, a) else {
                    ok = false;
                    continue;
                };
                if let Operand::Slot(s) | Operand::Field(s, _) = &operand {
                    in_warmup = in_warmup.max(self.nodes[*s].warmup);
                }
                if !self.unify(&raw.id, k, Ty::Var('T'), &ty, &mut subst) {
                    ok = false;
                }
                args.insert(k.clone(), operand);
            }
        }
        if spec.name == "weighted_mean" {
            if let (Some(Operand::List(xs)), Some(Operand::List(ws))) =
                (args.get("xs"), args.get("weights"))
            {
                if xs.len() != ws.len() || xs.is_empty() {
                    self.diag(Diagnostic::new(
                        "weights_length",
                        &raw.id,
                        format!("{} values and {} weights", xs.len(), ws.len()),
                        "give one weight per value",
                    ));
                    ok = false;
                }
            }
        }
        if !ok {
            return;
        }
        let ty = Self::output_type(spec, &subst);
        let warmup = in_warmup + if spec.stateful { own_warmup } else { 0 };
        self.push_node(&raw.id, NodeKind::Op { op: spec, args }, ty, warmup);
    }

    fn check_literal_range(&mut self, raw: &RawNode, spec: &OpSpec, port: &str, x: f64) -> bool {
        let bad = match (spec.name, port) {
            ("quantile", "q") => !(x > 0.0 && x < 1.0),
            ("round", "digits") => !(0.0..=12.0).contains(&x) || x.fract() != 0.0,
            _ => false,
        };
        if bad {
            self.diag(Diagnostic::new(
                "bad_literal",
                &raw.id,
                format!("`{port}` = {x} is out of range for `{}`", spec.name),
                match (spec.name, port) {
                    ("quantile", _) => "a quantile level is strictly between 0 and 1, e.g. 0.25",
                    _ => "round to between 0 and 12 decimal places",
                },
            ));
        }
        !bad
    }

    /// A label literal must be one of the declared labels, when they are declared.
    fn check_label(
        &mut self,
        raw: &RawNode,
        spec: &OpSpec,
        port: &str,
        label: &str,
        subst: &Subst,
    ) -> bool {
        if port != "label" {
            return true;
        }
        let Some(labels) = subst.labels.get(&'L') else {
            return true;
        };
        if labels.is_empty() || labels.iter().any(|l| l == label) {
            return true;
        }
        self.diag(Diagnostic::new(
            "unknown_label",
            &raw.id,
            format!("'{label}' is not one of {}", labels.join(", ")),
            nearest(label, labels.iter().map(String::as_str)).map_or_else(
                || format!("`{}` needs one of the declared labels", spec.name),
                |n| format!("did you mean '{n}'?"),
            ),
        ));
        false
    }

    // ── models ───────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn compile_model(&mut self, raw: &RawNode) {
        let Some(ref_arg) = raw.args.get("ref") else {
            self.diag(
                Diagnostic::new(
                    "missing_arg",
                    &raw.id,
                    "`model` needs `ref` — which model, at which version",
                    "name the model",
                )
                .example("{\"str\": \"vol_forecaster@12\"}"),
            );
            return;
        };
        let Some(reference) = self.literal_text(&raw.id, "ref", ref_arg) else {
            return;
        };
        let Some(resolved) = self.reg.model(&reference) else {
            self.diag(Diagnostic::new(
                "unknown_model",
                &raw.id,
                format!("no model answers to '{reference}'"),
                "use `name@version` or `name@alias` for a model in the registry; list them with the models catalogue",
            ));
            return;
        };
        let contract = Arc::new(resolved.contract);
        let mut inputs = BTreeMap::new();
        let mut warmup: u32 = 0;
        let mut ok = true;
        for ci in &contract.inputs {
            let arg = raw.args.get(&ci.name);
            match (&ci.shape, arg) {
                (InputShape::Row { features }, None) => {
                    let Some(names) = self.feature_list(&raw.id, features) else {
                        ok = false;
                        continue;
                    };
                    let columns = self.columns_for(PRIMARY, &names);
                    warmup = warmup.max(self.input_lookback(PRIMARY, &names));
                    inputs.insert(
                        ci.name.clone(),
                        ModelArg::Row {
                            input: PRIMARY,
                            columns,
                        },
                    );
                }
                (
                    InputShape::Window {
                        features,
                        lookback,
                        timeframe,
                    },
                    None,
                ) => {
                    let input = match timeframe {
                        Some(tf) => {
                            if let Some(i) = self.input_at_timeframe(&raw.id, tf.key()) {
                                i
                            } else {
                                ok = false;
                                continue;
                            }
                        }
                        None => PRIMARY,
                    };
                    let Some(names) = self.feature_list(&raw.id, features) else {
                        ok = false;
                        continue;
                    };
                    warmup = warmup.max(self.add_window(input, &names, *lookback));
                    let columns = self.columns_for(input, &names);
                    inputs.insert(
                        ci.name.clone(),
                        ModelArg::Window {
                            input,
                            columns,
                            lookback: *lookback,
                        },
                    );
                }
                (InputShape::Row { .. } | InputShape::Window { .. }, Some(a)) => {
                    // An explicit `features(...)` window overrides the contract's.
                    let Some((_, ty)) = self.operand(&raw.id, a) else {
                        ok = false;
                        continue;
                    };
                    let CType::Window {
                        input,
                        columns,
                        lookback,
                    } = ty
                    else {
                        self.diag(Diagnostic::new(
                            "type_mismatch",
                            &raw.id,
                            format!("model input `{}` takes feature data, not a single value", ci.name),
                            "pass a `features(set, lookback)` window, or leave it out to use the model's own",
                        ));
                        ok = false;
                        continue;
                    };
                    let lookback = if matches!(ci.shape, InputShape::Row { .. }) {
                        1
                    } else {
                        lookback
                    };
                    warmup = warmup.max(self.add_window(input, &columns, lookback));
                    let cols = self.columns_for(input, &columns);
                    inputs.insert(
                        ci.name.clone(),
                        if lookback == 1 && matches!(ci.shape, InputShape::Row { .. }) {
                            ModelArg::Row {
                                input,
                                columns: cols,
                            }
                        } else {
                            ModelArg::Window {
                                input,
                                columns: cols,
                                lookback,
                            }
                        },
                    );
                }
                (InputShape::Scalar { value_type }, Some(a)) => {
                    let Some((operand, ty)) = self.operand(&raw.id, a) else {
                        ok = false;
                        continue;
                    };
                    match &ty {
                        CType::Value(t) if compatible(value_type, t) => {}
                        _ => {
                            let got = match &ty {
                                CType::Value(t) => t.label(),
                                CType::Window { .. } => "a feature window".into(),
                            };
                            self.diag(Diagnostic::new(
                                "model_input_type",
                                &raw.id,
                                format!("model '{reference}' input `{}` takes {value_type}, and this is {got}", ci.name),
                                format!("wire a {value_type} into `{}`", ci.name),
                            ));
                            ok = false;
                            continue;
                        }
                    }
                    if let Operand::Slot(s) | Operand::Field(s, _) = &operand {
                        warmup = warmup.max(self.nodes[*s].warmup);
                    }
                    inputs.insert(ci.name.clone(), ModelArg::Value(operand));
                }
                (InputShape::Text { .. }, Some(a)) => {
                    let Some((operand, ty)) = self.operand(&raw.id, a) else {
                        ok = false;
                        continue;
                    };
                    if ty != CType::Value(ValueType::Text) {
                        self.diag(Diagnostic::new(
                            "model_input_type",
                            &raw.id,
                            format!("model '{reference}' input `{}` takes text", ci.name),
                            "wire a text value into it",
                        ));
                        ok = false;
                        continue;
                    }
                    inputs.insert(ci.name.clone(), ModelArg::Value(operand));
                }
                (InputShape::CrossSection { .. }, _) => {
                    self.not_yet(raw, "U-6.2 (cross-sectional model inputs)");
                    ok = false;
                }
                (_, None) if ci.optional => {}
                (_, None) => {
                    self.diag(Diagnostic::new(
                        "missing_model_input",
                        &raw.id,
                        format!("model '{reference}' needs input `{}`", ci.name),
                        format!("wire a value into `{}`", ci.name),
                    ));
                    ok = false;
                }
            }
        }
        for k in raw.args.keys() {
            if k != "ref" && contract.input(k).is_none() {
                let names: Vec<&str> = contract.inputs.iter().map(|i| i.name.as_str()).collect();
                self.diag(Diagnostic::new(
                    "unknown_model_input",
                    &raw.id,
                    format!("model '{reference}' has no input `{k}`"),
                    nearest(k, names.iter().copied()).map_or_else(
                        || {
                            format!(
                                "its inputs are {}",
                                if names.is_empty() {
                                    "none".into()
                                } else {
                                    names.join(", ")
                                }
                            )
                        },
                        |n| format!("did you mean `{n}`?"),
                    ),
                ));
                ok = false;
            }
        }
        if !ok {
            return;
        }
        let ty = contract.output_type();
        self.push_node(
            &raw.id,
            NodeKind::Model(ModelCall {
                reference,
                model: resolved.model,
                version: resolved.version,
                contract,
                inputs,
            }),
            ty,
            warmup,
        );
    }

    fn columns_for(&mut self, input: InputIdx, names: &[String]) -> Vec<(String, usize)> {
        names
            .iter()
            .map(|n| {
                let lb = self.reg.feature_lookback(n).unwrap_or(1);
                (n.clone(), self.column(input, n, lb))
            })
            .collect()
    }

    fn input_lookback(&self, input: InputIdx, names: &[String]) -> u32 {
        let lb = names
            .iter()
            .filter_map(|n| self.reg.feature_lookback(n))
            .max()
            .unwrap_or(1);
        if input == PRIMARY {
            lb
        } else {
            0
        }
    }

    /// Register a window over `names` on `input`; its warm-up in primary ticks.
    fn add_window(&mut self, input: InputIdx, names: &[String], lookback: u32) -> u32 {
        let lb = names
            .iter()
            .filter_map(|n| self.reg.feature_lookback(n))
            .max()
            .unwrap_or(1);
        let need = lb + lookback.saturating_sub(1);
        let inp = &mut self.inputs[input];
        inp.history_bars = inp.history_bars.max(need);
        if input == PRIMARY {
            need
        } else {
            0
        }
    }

    // ── 4. sinks ─────────────────────────────────────────────────────────────

    fn compile_rule(&mut self, i: usize, rule: &domain::strategy_graph::graph::Rule) {
        let at = format!(
            "rule[{i}]{}",
            rule.name
                .as_ref()
                .map_or(String::new(), |n| format!(" ({n})"))
        );
        let when_arg = self.lift_arg(&format!("rule{i}~when"), &rule.when);
        self.compile_lifted_tail();
        let Some((when, ty)) = self.operand(&at, &when_arg) else {
            return;
        };
        if ty != CType::Value(ValueType::Bool) {
            let got = match &ty {
                CType::Value(t) => t.label(),
                CType::Window { .. } => "a feature window".into(),
            };
            self.diag(Diagnostic::new(
                "type_mismatch",
                &at,
                format!("`when` must be a condition (bool), and this is {got}"),
                "compare it, e.g. `gt(x, 0)`, or combine conditions with `and`",
            ));
            return;
        }
        if let Err(e) = intent::check_literals(&rule.then) {
            self.diag(Diagnostic::new(
                "bad_intent",
                &at,
                e.to_string(),
                "fix the literal",
            ));
            return;
        }
        let mut template = rule.then.clone();
        let mut operands = Vec::new();
        let mut atr: Vec<u32> = Vec::new();
        let mut failed = false;
        let mut k = 0usize;
        // Distances in ATR need the ATR indicator on the primary input.
        visit_distance_units(&template, &mut |u| {
            if let DistanceUnit::Atr { n } = u {
                atr.push(*n);
            }
        });
        let mut lifted: Vec<(usize, Arg)> = Vec::new();
        visit_args_mut(&mut template, &mut |a| {
            if a.literal_num().is_some() {
                return;
            }
            lifted.push((k, a.clone()));
            *a = Arg::Ref(format!("#{k}"));
            k += 1;
        });
        for (j, a) in lifted {
            let a = self.lift_arg(&format!("rule{i}~then~{j}"), &a);
            self.compile_lifted_tail();
            match self.operand(&at, &a) {
                Some((o, CType::Value(ValueType::Num))) => operands.push(o),
                Some((_, t)) => {
                    let got = match &t {
                        CType::Value(t) => t.label(),
                        CType::Window { .. } => "a feature window".into(),
                    };
                    self.diag(Diagnostic::new(
                        "type_mismatch",
                        &at,
                        format!("every number in an intent must be a num, and one is {got}"),
                        "wire a number — a size, a price, a distance",
                    ));
                    failed = true;
                }
                None => failed = true,
            }
        }
        for n in atr {
            let name = format!("atr_{n}");
            if let Some(lb) = self.reg.feature_lookback(&name) {
                let col = self.column(PRIMARY, &name, lb);
                self.atr_columns.insert(n, col);
            } else {
                self.diag(Diagnostic::new(
                    "unknown_feature",
                    &at,
                    format!("ATR({n}) is not available"),
                    "use a period the indicator registry has, e.g. 14",
                ));
                failed = true;
            }
        }
        if failed {
            return;
        }
        self.rules.push(CompiledRule {
            index: i,
            name: rule.name.clone(),
            group: rule.group.clone(),
            when,
            template,
            operands,
        });
    }

    /// Compile any raw nodes lifted after the main pass (inline nodes inside
    /// rules and the select sink). They can only depend on already-compiled
    /// nodes or on each other in lifting order.
    fn compile_lifted_tail(&mut self) {
        let pending: Vec<RawNode> = self
            .raw
            .iter()
            .filter(|r| {
                !self.slots.contains_key(&r.id)
                    && !self.windows.contains_key(&r.id)
                    && r.id.contains('~')
                    && (r.id.starts_with("rule") || r.id.starts_with("select"))
            })
            .cloned()
            .collect();
        // Lifted inner nodes are pushed before their parents, so compiling in
        // lifting order respects their dependencies.
        for r in pending {
            if !self.slots.contains_key(&r.id) && !self.windows.contains_key(&r.id) {
                self.compile_node(&r);
            }
        }
    }

    fn compile_select(
        &mut self,
        s: &domain::strategy_graph::graph::Select,
        universe: &domain::strategy_graph::graph::UniverseRef,
    ) {
        let score_arg = self.lift_arg("select~score", &s.score);
        let filter_arg = s.filter.as_ref().map(|f| self.lift_arg("select~where", f));
        let attr_args: Vec<(String, Arg)> = s
            .attributes
            .iter()
            .map(|(k, a)| (k.clone(), self.lift_arg(&format!("select~attr~{k}"), a)))
            .collect();
        self.compile_lifted_tail();
        let score = match self.operand("select.score", &score_arg) {
            Some((o, CType::Value(ValueType::Num))) => Some(o),
            Some(_) => {
                self.diag(Diagnostic::new(
                    "type_mismatch",
                    "select.score",
                    "the score must be a number",
                    "rank by a num, e.g. a momentum z-score",
                ));
                None
            }
            None => None,
        };
        let filter = match filter_arg {
            Some(f) => match self.operand("select.where", &f) {
                Some((o, CType::Value(ValueType::Bool))) => Some(Some(o)),
                Some(_) => {
                    self.diag(Diagnostic::new(
                        "type_mismatch",
                        "select.where",
                        "the filter must be a condition (bool)",
                        "e.g. `gt(liq, 0.5)`",
                    ));
                    None
                }
                None => None,
            },
            None => Some(None),
        };
        let mut attributes = BTreeMap::new();
        for (k, a) in attr_args {
            if let Some((o, CType::Value(_))) = self.operand(&format!("select.attributes.{k}"), &a)
            {
                attributes.insert(k, o);
            }
        }
        if let (Some(score), Some(filter)) = (score, filter) {
            self.select = Some(CompiledSelect {
                score,
                filter,
                top_n: s.top_n,
                order: s.order,
                attributes,
                universe: universe.clone(),
            });
        }
    }

    fn finish(self) -> Plan {
        let warmup_ticks = self
            .nodes
            .iter()
            .map(|n| n.warmup)
            .max()
            .unwrap_or(0)
            .max(self.inputs[PRIMARY].history_bars);
        let revision = self
            .graph
            .revision()
            .map(|r| r.as_str().to_string())
            .unwrap_or_default();
        Plan {
            kind: self.graph.kind,
            name: self.graph.name.clone(),
            revision,
            timeframe: self.graph.timeframe,
            triggers: self.graph.triggers.clone(),
            params: self.params,
            inputs: self.inputs,
            nodes: self.nodes,
            rules: self.rules,
            select: self.select,
            atr_columns: self.atr_columns,
            warmup_ticks,
            risk: self.graph.risk.clone(),
        }
    }
}

fn bind_labels(subst: &mut Subst, v: char, labels: &[String]) -> bool {
    match subst.labels.get(&v) {
        Some(bound) if bound.is_empty() || labels.is_empty() => true,
        Some(bound) => bound == labels,
        None => {
            subst.labels.insert(v, labels.to_vec());
            true
        }
    }
}

/// Structural compatibility: equal, or a side whose labels/dimension are
/// undeclared.
pub(crate) fn compatible(a: &ValueType, b: &ValueType) -> bool {
    match (a, b) {
        (ValueType::Cat { labels: x }, ValueType::Cat { labels: y })
        | (ValueType::Probs { labels: x }, ValueType::Probs { labels: y }) => {
            x.is_empty() || y.is_empty() || x == y
        }
        (ValueType::Vec { dim: x }, ValueType::Vec { dim: y }) => {
            x.is_none() || y.is_none() || x == y
        }
        (ValueType::Dist { levels: x }, ValueType::Dist { levels: y }) => {
            x.is_empty() || y.is_empty() || x == y
        }
        _ => a == b,
    }
}

fn describe(t: Ty, subst: &Subst) -> String {
    match t {
        Ty::Num => "a num".into(),
        Ty::Bool => "a condition (bool)".into(),
        Ty::Text => "text".into(),
        Ty::Var(v) => subst
            .types
            .get(&v)
            .map_or_else(|| "a value".into(), |t| format!("a {t}")),
        Ty::CatOf(_) => "a category".into(),
        Ty::ProbsOf(_) => "class probabilities".into(),
        Ty::AnyDist => "a distribution".into(),
        Ty::AnyVec => "a vector".into(),
        Ty::AnyRecord => "a record".into(),
        Ty::NumList => "a list of nums".into(),
        Ty::BoolList => "a list of conditions".into(),
        Ty::Window => "a feature window".into(),
        Ty::FromContract | Ty::FromSchema => "a model output".into(),
    }
}

/// Every numeric `Arg` in an intent, mutably — sizes, prices, triggers,
/// distances, fractions.
pub(crate) fn visit_args_mut(i: &mut TradeIntent, f: &mut dyn FnMut(&mut Arg)) {
    use domain::strategy_graph::intent::{Entry, OrderStyle, SizeSpec};
    fn size(s: &mut SizeSpec, f: &mut dyn FnMut(&mut Arg)) {
        match s {
            SizeSpec::Units { x } | SizeSpec::Notional { x } | SizeSpec::EquityFraction { x } => {
                f(x);
            }
            SizeSpec::Risk { fraction, stop } => {
                f(fraction);
                dist(stop, f);
            }
            SizeSpec::VolTarget { target, vol } => {
                f(target);
                f(vol);
            }
        }
    }
    fn dist(d: &mut domain::strategy_graph::intent::Distance, f: &mut dyn FnMut(&mut Arg)) {
        f(&mut d.amount);
        if let DistanceUnit::Sigma { of } = &mut d.unit {
            f(of);
        }
    }
    fn style(s: &mut OrderStyle, f: &mut dyn FnMut(&mut Arg)) {
        match s {
            OrderStyle::Market => {}
            OrderStyle::Limit { price } => f(price),
            OrderStyle::Stop { trigger } => f(trigger),
            OrderStyle::StopLimit { trigger, price } => {
                f(trigger);
                f(price);
            }
            OrderStyle::TrailingStop { trail } => dist(trail, f),
        }
    }
    fn order(o: &mut domain::strategy_graph::intent::OrderSpec, f: &mut dyn FnMut(&mut Arg)) {
        size(&mut o.size, f);
        style(&mut o.style, f);
    }
    match i {
        TradeIntent::Hold | TradeIntent::Cancel { .. } => {}
        TradeIntent::Target { size: s, via }
        | TradeIntent::Add { size: s, via }
        | TradeIntent::Reduce { size: s, via } => {
            size(s, f);
            if let Some(v) = via {
                style(v, f);
            }
        }
        TradeIntent::Order(o) => order(o, f),
        TradeIntent::Bracket(b) => {
            match &mut b.entry {
                Entry::Target { size: s, via } => {
                    size(s, f);
                    if let Some(v) = via {
                        style(v, f);
                    }
                }
                Entry::Order(o) => order(o, f),
            }
            dist(&mut b.stop, f);
            if let Some(d) = &mut b.take_profit {
                dist(d, f);
            }
            if let Some(d) = &mut b.trailing {
                dist(d, f);
            }
        }
        TradeIntent::Protect(p) => {
            for d in [&mut p.stop, &mut p.take_profit, &mut p.trailing]
                .into_iter()
                .flatten()
            {
                dist(d, f);
            }
        }
        TradeIntent::Close { fraction, via } => {
            f(fraction);
            if let Some(v) = via {
                style(v, f);
            }
        }
    }
}

/// Every distance unit in an intent.
pub(crate) fn visit_distance_units(i: &TradeIntent, f: &mut dyn FnMut(&DistanceUnit)) {
    let mut clone = i.clone();
    // Reuse the mutable visitor's traversal by collecting units on a copy.
    let mut units: Vec<DistanceUnit> = Vec::new();
    collect_units(&mut clone, &mut units);
    for u in &units {
        f(u);
    }
}

fn collect_units(i: &mut TradeIntent, out: &mut Vec<DistanceUnit>) {
    use domain::strategy_graph::intent::{Entry, OrderStyle, SizeSpec};
    let mut d = |x: &domain::strategy_graph::intent::Distance| out.push(x.unit.clone());
    let style_d = |s: &OrderStyle, d: &mut dyn FnMut(&domain::strategy_graph::intent::Distance)| {
        if let OrderStyle::TrailingStop { trail } = s {
            d(trail);
        }
    };
    let size_d = |s: &SizeSpec, d: &mut dyn FnMut(&domain::strategy_graph::intent::Distance)| {
        if let SizeSpec::Risk { stop, .. } = s {
            d(stop);
        }
    };
    match i {
        TradeIntent::Hold | TradeIntent::Cancel { .. } => {}
        TradeIntent::Target { size, via }
        | TradeIntent::Add { size, via }
        | TradeIntent::Reduce { size, via } => {
            size_d(size, &mut d);
            if let Some(v) = via {
                style_d(v, &mut d);
            }
        }
        TradeIntent::Close { via, .. } => {
            if let Some(v) = via {
                style_d(v, &mut d);
            }
        }
        TradeIntent::Order(o) => {
            size_d(&o.size, &mut d);
            style_d(&o.style, &mut d);
        }
        TradeIntent::Bracket(b) => {
            match &b.entry {
                Entry::Target { size, via } => {
                    size_d(size, &mut d);
                    if let Some(v) = via {
                        style_d(v, &mut d);
                    }
                }
                Entry::Order(o) => {
                    size_d(&o.size, &mut d);
                    style_d(&o.style, &mut d);
                }
            }
            d(&b.stop);
            if let Some(x) = &b.take_profit {
                d(x);
            }
            if let Some(x) = &b.trailing {
                d(x);
            }
        }
        TradeIntent::Protect(p) => {
            for x in [&p.stop, &p.take_profit, &p.trailing].into_iter().flatten() {
                d(x);
            }
        }
    }
}
