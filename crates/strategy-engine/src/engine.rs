//! The engine (FEAT-007 §3, ADR-0039).
//!
//! Events in — a closed bar, a fill, an order closing, a scheduled timer — and
//! either nothing, a request for model answers, or a decision out. No I/O, no
//! clock, no async: the decision instant comes from the event, the account from
//! the [`PortfolioView`] passed with it, and model answers through
//! [`Engine::resume`]. Which environment is driving it, the engine cannot tell.
//!
//! # One tick
//!
//! 1. Advance the tick counter and evaluate the plan's nodes in order. A model
//!    node whose inputs are all present becomes a [`ModelRequest`]; anything
//!    depending on it waits.
//! 2. If requests were made, return [`Step::NeedModels`]. The environment answers
//!    with [`Engine::resume`], and evaluation continues from where it stopped —
//!    nodes already evaluated are never evaluated again, so stateful operators
//!    advance exactly once per tick.
//! 3. When every node has a value: match rules, resolve intents (a missing value
//!    → hold), plan orders, and return the [`DecisionTrace`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use domain::strategy_graph::{MissingReason, StrategyKind, Value};
use serde::{Deserialize, Serialize};

use crate::decide::{self, RuleOutcome};
use crate::eval::{self, ArgVal};
use crate::history::{Bar, History};
use crate::models::{self, ModelInput, ModelRequest, ModelResponse};
use crate::plan::{
    BarField, InputIdx, ModelArg, NodeKind, Operand, Plan, Slot, StateField, PRIMARY,
};
use crate::planner::{self, Planned, PlannerState};
use crate::portfolio::{Fill, InstrumentSpec, OrderClosed, PortfolioView};
use crate::temporal::Temporal;
use crate::trace::{Cause, DecisionTrace};

/// Something that happened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A bar closed on one of the plan's inputs. `warmup` bars build history and
    /// stateful operators but never decide or call a model.
    Bar {
        input: InputIdx,
        bar: Bar,
        #[serde(default)]
        warmup: bool,
    },
    Fill(Fill),
    OrderClosed(OrderClosed),
    /// A scheduled trigger fired.
    Timer {
        at_ns: i64,
    },
}

/// What the engine needs next.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Nothing to decide on this event.
    Idle,
    /// Answer these, then call [`Engine::resume`].
    NeedModels(Vec<ModelRequest>),
    Decided(Box<DecisionTrace>),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("selection strategies are evaluated cross-sectionally, which arrives with Set U Phase 6; this engine runs trading strategies")]
    SelectionNotYet,
    #[error("indicator '{0}' could not be constructed: {1}")]
    Feature(String, String),
    #[error("an event arrived while a tick is waiting for model answers; call `resume` first")]
    TickPending,
    #[error("`resume` was called with no tick waiting for model answers")]
    NothingPending,
    #[error("no node is waiting for an answer from '{0}'")]
    UnexpectedResponse(String),
    #[error("evaluation stalled: {0} node(s) can never be evaluated — an engine defect")]
    Stalled(usize),
}

struct Pending {
    at_ns: i64,
    cause: Cause,
    view: PortfolioView,
    spec: InstrumentSpec,
    values: Vec<Option<Value>>,
    /// node id → its outstanding request.
    waiting: BTreeMap<String, Slot>,
    requests: Vec<ModelRequest>,
    responses: Vec<ModelResponse>,
    pre: Planned,
}

/// One running strategy instance on one instrument.
pub struct Engine {
    pub(crate) plan: Arc<Plan>,
    histories: Vec<History>,
    temporal: Vec<Option<Temporal>>,
    pub(crate) planner: PlannerState,
    pub(crate) tick: u64,
    pending: Option<Pending>,
    /// The close of the last bar seen on each input — where a checkpoint's
    /// history replay stops (U-4.4).
    pub(crate) last_close: Vec<Option<i64>>,
}

impl Engine {
    /// Build an engine for a compiled plan.
    ///
    /// # Errors
    /// A selection plan (Phase 6), or an indicator the features crate cannot
    /// construct.
    pub fn new(plan: Arc<Plan>) -> Result<Self, EngineError> {
        if plan.kind == StrategyKind::Selection {
            return Err(EngineError::SelectionNotYet);
        }
        let mut histories = Vec::with_capacity(plan.inputs.len());
        for (i, input) in plan.inputs.iter().enumerate() {
            let mut impls = Vec::with_capacity(input.columns.len());
            let mut rows_cap = 1usize;
            for name in &input.columns {
                let f = features::runtime::feature(name)
                    .map_err(|e| EngineError::Feature(name.clone(), e.to_string()))?;
                rows_cap = rows_cap.max(f.def().lookback_bars as usize);
                impls.push(f);
            }
            let cols_cap = plan
                .nodes
                .iter()
                .filter_map(|n| match &n.kind {
                    NodeKind::Model(m) => Some(m.inputs.values()),
                    _ => None,
                })
                .flatten()
                .filter_map(|a| match a {
                    ModelArg::Window {
                        input, lookback, ..
                    } if *input == i => Some(*lookback as usize),
                    _ => None,
                })
                .max()
                .unwrap_or(1);
            histories.push(History::new(impls, rows_cap, cols_cap));
        }
        let temporal = plan
            .nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Op { op, args } if op.stateful => {
                    let period = args.get("n").and_then(|o| match o {
                        Operand::Const(v) => v.as_num(),
                        _ => None,
                    });
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Some(Temporal::new(op.name, period.unwrap_or(0.0) as u32))
                }
                _ => None,
            })
            .collect();
        let inputs = plan.inputs.len();
        Ok(Self {
            plan,
            histories,
            temporal,
            planner: PlannerState::default(),
            tick: 0,
            pending: None,
            last_close: vec![None; inputs],
        })
    }

    #[must_use]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Whether a tick is waiting for model answers.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Decision ticks so far.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Handle one event.
    ///
    /// # Errors
    /// [`EngineError::TickPending`] if a tick is waiting for model answers.
    pub fn on_event(
        &mut self,
        event: Event,
        view: &PortfolioView,
        spec: &InstrumentSpec,
    ) -> Result<Step, EngineError> {
        if self.pending.is_some() {
            return Err(EngineError::TickPending);
        }
        match event {
            Event::Bar { input, bar, warmup } => {
                let Some(h) = self.histories.get_mut(input) else {
                    return Ok(Step::Idle);
                };
                h.push(bar);
                self.last_close[input] = Some(bar.close_ns);
                if input != PRIMARY || !self.plan.triggers.bar_close {
                    return Ok(Step::Idle);
                }
                self.begin(
                    bar.close_ns,
                    Cause::BarClose,
                    view,
                    spec,
                    warmup,
                    Planned::default(),
                )
            }
            Event::Fill(fill) => {
                let pre = planner::on_fill(&fill, view, spec, &mut self.planner, self.tick);
                if self.plan.triggers.on_fill {
                    let tag = fill.tag.clone();
                    return self.begin(fill.at_ns, Cause::Fill { tag }, view, spec, false, pre);
                }
                Ok(self.mechanical(fill.at_ns, Cause::Fill { tag: fill.tag }, pre))
            }
            Event::OrderClosed(closed) => {
                planner::on_closed(&closed.tag, &mut self.planner);
                if self.plan.triggers.on_fill {
                    return self.begin(
                        closed.at_ns,
                        Cause::OrderClosed { tag: closed.tag },
                        view,
                        spec,
                        false,
                        Planned::default(),
                    );
                }
                Ok(Step::Idle)
            }
            Event::Timer { at_ns } => {
                if self.plan.triggers.schedule.is_none() {
                    return Ok(Step::Idle);
                }
                self.begin(
                    at_ns,
                    Cause::Schedule,
                    view,
                    spec,
                    false,
                    Planned::default(),
                )
            }
        }
    }

    /// Answer the outstanding model requests and continue the tick.
    ///
    /// # Errors
    /// No tick waiting, or a response for a node that asked for nothing.
    pub fn resume(&mut self, responses: Vec<ModelResponse>) -> Result<Step, EngineError> {
        let mut p = self.pending.take().ok_or(EngineError::NothingPending)?;
        for r in responses {
            let Some(slot) = p.waiting.remove(&r.node) else {
                self.pending = Some(p);
                return Err(EngineError::UnexpectedResponse(r.node));
            };
            let NodeKind::Model(call) = &self.plan.nodes[slot].kind else {
                unreachable!("only model nodes wait");
            };
            p.values[slot] = Some(models::response_value(
                &call.reference,
                &call.contract,
                &r.outcome,
            ));
            p.responses.push(r);
        }
        // Any request the environment did not answer is a model that did not
        // answer: missing, never waited on for ever.
        for (node, slot) in std::mem::take(&mut p.waiting) {
            let NodeKind::Model(call) = &self.plan.nodes[slot].kind else {
                continue;
            };
            p.values[slot] = Some(Value::Missing(MissingReason::ModelTimeout {
                model: call.reference.clone(),
                timeout_ms: call.contract.timeout_ms,
            }));
            let _ = node;
        }
        self.evaluate(p, false)
    }

    fn mechanical(&self, at_ns: i64, cause: Cause, pre: Planned) -> Step {
        if pre.actions.is_empty() && pre.refusals.is_empty() {
            return Step::Idle;
        }
        Step::Decided(Box::new(DecisionTrace {
            at_ns,
            tick: self.tick,
            cause,
            values: BTreeMap::new(),
            model_requests: Vec::new(),
            model_responses: Vec::new(),
            rules: BTreeMap::new(),
            intents: Vec::new(),
            actions: pre.actions,
            refusals: pre.refusals,
        }))
    }

    fn begin(
        &mut self,
        at_ns: i64,
        cause: Cause,
        view: &PortfolioView,
        spec: &InstrumentSpec,
        warmup: bool,
        pre: Planned,
    ) -> Result<Step, EngineError> {
        self.tick += 1;
        let p = Pending {
            at_ns,
            cause,
            view: view.clone(),
            spec: spec.clone(),
            values: vec![None; self.plan.nodes.len()],
            waiting: BTreeMap::new(),
            requests: Vec::new(),
            responses: Vec::new(),
            pre,
        };
        self.evaluate(p, warmup)
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&mut self, mut p: Pending, warmup: bool) -> Result<Step, EngineError> {
        let plan = Arc::clone(&self.plan);
        let mut new_requests = Vec::new();
        for (slot, node) in plan.nodes.iter().enumerate() {
            if p.values[slot].is_some() || p.waiting.contains_key(&node.id) {
                continue;
            }
            match &node.kind {
                NodeKind::Feature {
                    input,
                    column,
                    lookback,
                    name,
                } => {
                    let h = &self.histories[*input];
                    p.values[slot] = Some(match h.latest(*column) {
                        Some(x) => Value::num(x, name),
                        None if h.bars_seen < u64::from(*lookback) => {
                            Value::Missing(MissingReason::WarmingUp {
                                needed: *lookback,
                                have: u32::try_from(h.bars_seen).unwrap_or(u32::MAX),
                            })
                        }
                        None => Value::Missing(MissingReason::Undefined { op: name.clone() }),
                    });
                }
                NodeKind::Bar { input, field } => {
                    p.values[slot] = Some(self.bar_value(*input, *field));
                }
                NodeKind::Hour => {
                    let secs = p.at_ns.div_euclid(1_000_000_000);
                    #[allow(clippy::cast_precision_loss)]
                    let h = (secs.div_euclid(3_600)).rem_euclid(24) as f64;
                    p.values[slot] = Some(Value::num(h, "hour"));
                }
                NodeKind::Weekday => {
                    let days = p.at_ns.div_euclid(86_400_000_000_000);
                    // 1970-01-01 was a Thursday; Monday = 0.
                    #[allow(clippy::cast_precision_loss)]
                    let d = (days + 3).rem_euclid(7) as f64;
                    p.values[slot] = Some(Value::num(d, "weekday"));
                }
                NodeKind::Op { op, args } => {
                    let mut vals: BTreeMap<String, ArgVal> = BTreeMap::new();
                    let mut ready = true;
                    for (port, operand) in args {
                        if let Some(v) = self.arg_value(operand, &p.values, &p.view) {
                            vals.insert(port.clone(), v);
                        } else {
                            ready = false;
                            break;
                        }
                    }
                    if !ready {
                        continue;
                    }
                    let v = if let Some(state) = self.temporal[slot].as_mut() {
                        let get = |port: &str| match vals.get(port) {
                            Some(ArgVal::One(v)) => v.clone(),
                            _ => Value::Missing(MissingReason::Undefined {
                                op: format!("{} has no {port}", op.name),
                            }),
                        };
                        state.step(op.name, self.tick, &get)
                    } else {
                        let ports: Vec<&str> = op.inputs.iter().map(|p| p.name).collect();
                        eval::eval(op.name, &ports, &vals)
                    };
                    p.values[slot] = Some(v);
                }
                NodeKind::Model(call) => {
                    if warmup {
                        p.values[slot] = Some(Value::Missing(MissingReason::WarmingUp {
                            needed: node.warmup,
                            have: u32::try_from(self.tick).unwrap_or(u32::MAX),
                        }));
                        continue;
                    }
                    let mut inputs = BTreeMap::new();
                    let mut ready = true;
                    let mut missing: Option<MissingReason> = None;
                    for (name, arg) in &call.inputs {
                        match self.model_input(name, arg, &p.values, &p.view) {
                            Ok(Some(input)) => {
                                inputs.insert(name.clone(), input);
                            }
                            Ok(None) => {
                                ready = false;
                                break;
                            }
                            Err(reason) => {
                                let optional =
                                    call.contract.input(name).is_some_and(|c| c.optional);
                                if !optional {
                                    missing = Some(reason);
                                    break;
                                }
                            }
                        }
                    }
                    if !ready {
                        continue;
                    }
                    if let Some(reason) = missing {
                        // An input the engine cannot assemble: no call, no
                        // zero in its place.
                        p.values[slot] = Some(Value::Missing(reason));
                        continue;
                    }
                    let req = ModelRequest {
                        node: node.id.clone(),
                        reference: call.reference.clone(),
                        model: call.model.clone(),
                        version: call.version,
                        at_ns: p.at_ns,
                        inputs,
                    };
                    p.waiting.insert(node.id.clone(), slot);
                    new_requests.push(req);
                }
            }
        }
        if !new_requests.is_empty() {
            p.requests.extend(new_requests.iter().cloned());
            self.pending = Some(p);
            return Ok(Step::NeedModels(new_requests));
        }
        let unevaluated = p.values.iter().filter(|v| v.is_none()).count();
        if unevaluated > 0 {
            return Err(EngineError::Stalled(unevaluated));
        }
        if warmup {
            return Ok(Step::Idle);
        }
        Ok(Step::Decided(Box::new(self.decide(p))))
    }

    fn decide(&mut self, p: Pending) -> DecisionTrace {
        let plan = Arc::clone(&self.plan);
        let values: Vec<Value> = p
            .values
            .into_iter()
            .map(|v| {
                v.unwrap_or(Value::Missing(MissingReason::Undefined {
                    op: "unevaluated".into(),
                }))
            })
            .collect();
        let lookup: Vec<Option<Value>> = values.iter().cloned().map(Some).collect();
        let mut outcomes = BTreeMap::new();
        let mut intents = Vec::new();
        let mut taken: BTreeSet<&str> = BTreeSet::new();
        for rule in &plan.rules {
            if taken.contains(rule.group.as_str()) {
                outcomes.insert(rule.index, RuleOutcome::Shadowed);
                continue;
            }
            let when = self
                .operand_value(&rule.when, &lookup, &p.view)
                .unwrap_or(Value::Bool(false));
            match when {
                Value::Bool(true) => {
                    taken.insert(rule.group.as_str());
                    let operand = |k: usize| {
                        rule.operands
                            .get(k)
                            .and_then(|o| self.operand_value(o, &lookup, &p.view))
                            .unwrap_or(Value::Missing(MissingReason::Undefined {
                                op: format!("#{k}"),
                            }))
                    };
                    let atr = |n: u32| match plan.atr_columns.get(&n) {
                        Some(&c) => self.histories[PRIMARY].latest(c).map_or_else(
                            || {
                                Value::Missing(MissingReason::WarmingUp {
                                    needed: features::runtime::lookback_bars(&format!("atr_{n}"))
                                        .unwrap_or(n),
                                    have: u32::try_from(self.histories[PRIMARY].bars_seen)
                                        .unwrap_or(u32::MAX),
                                })
                            },
                            |x| Value::num(x, "atr"),
                        ),
                        None => Value::Missing(MissingReason::Undefined {
                            op: format!("atr_{n} not compiled"),
                        }),
                    };
                    match decide::resolve(&rule.template, &operand, &atr) {
                        Ok(ri) => {
                            outcomes.insert(rule.index, RuleOutcome::Fired);
                            intents.push((rule.index, ri));
                        }
                        Err(reasons) => {
                            outcomes.insert(rule.index, RuleOutcome::Held { reasons });
                        }
                    }
                }
                Value::Missing(reason) => {
                    outcomes.insert(rule.index, RuleOutcome::Unknown { reason });
                }
                _ => {
                    outcomes.insert(rule.index, RuleOutcome::NotMatched);
                }
            }
        }
        let mut planned = p.pre;
        let more = planner::plan(&intents, &p.view, &p.spec, &mut self.planner, self.tick);
        planned.actions.extend(more.actions);
        planned.refusals.extend(more.refusals);
        let named: BTreeMap<String, Value> = plan
            .nodes
            .iter()
            .zip(values)
            .map(|(n, v)| (n.id.clone(), v))
            .collect();
        DecisionTrace {
            at_ns: p.at_ns,
            tick: self.tick,
            cause: p.cause,
            values: named,
            model_requests: p.requests,
            model_responses: p.responses,
            rules: outcomes,
            intents,
            actions: planned.actions,
            refusals: planned.refusals,
        }
    }

    fn bar_value(&self, input: InputIdx, field: BarField) -> Value {
        let Some(b) = self.histories.get(input).and_then(|h| h.last) else {
            return Value::Missing(MissingReason::NoData {
                what: format!(
                    "a closed bar on {}",
                    self.plan.inputs.get(input).map_or("?", |i| i.id.as_str())
                ),
            });
        };
        let x = match field {
            BarField::Open => b.open,
            BarField::High => b.high,
            BarField::Low => b.low,
            BarField::Close => b.close,
            BarField::Volume => b.volume,
        };
        Value::num(x, field.name())
    }

    fn state_value(view: &PortfolioView, f: StateField) -> Value {
        use rust_decimal::prelude::ToPrimitive;
        let num = |d: rust_decimal::Decimal, what: &str| {
            d.to_f64().map_or_else(
                || Value::Missing(MissingReason::Undefined { op: what.into() }),
                |x| Value::num(x, what),
            )
        };
        let opt = |d: Option<rust_decimal::Decimal>, what: &str| match d {
            Some(d) => num(d, what),
            None => Value::Missing(MissingReason::NoData {
                what: format!("state.{what}"),
            }),
        };
        match f {
            StateField::Flat => Value::Bool(view.position.is_zero()),
            StateField::Long => {
                Value::Bool(view.position.is_sign_positive() && !view.position.is_zero())
            }
            StateField::Short => {
                Value::Bool(view.position.is_sign_negative() && !view.position.is_zero())
            }
            StateField::Position => num(view.position, "position"),
            StateField::PositionFraction => view.position_fraction().map_or_else(
                || {
                    Value::Missing(MissingReason::NoData {
                        what: "state.position_fraction (needs a mark and equity)".into(),
                    })
                },
                |x| Value::num(x, "position_fraction"),
            ),
            StateField::EntryPrice => opt(view.avg_entry_price, "entry_price"),
            StateField::BarsInPosition => {
                Value::num(f64::from(view.bars_in_position), "bars_in_position")
            }
            StateField::UnrealizedReturn => view.unrealized_return().map_or_else(
                || {
                    Value::Missing(MissingReason::NoData {
                        what: "state.unrealized_return (flat, or no entry price or mark)".into(),
                    })
                },
                |x| Value::num(x, "unrealized_return"),
            ),
            StateField::RealizedPnlToday => num(view.realized_pnl_today, "realized_pnl_today"),
            StateField::Equity => num(view.equity, "equity"),
            StateField::Cash => num(view.cash, "cash"),
            #[allow(clippy::cast_precision_loss)]
            StateField::OpenOrders => Value::num(view.open_orders.len() as f64, "open_orders"),
            StateField::Mark => opt(view.mark, "mark"),
        }
    }

    /// An operand's value; `None` if it reads a slot not yet evaluated this tick.
    fn operand_value(
        &self,
        o: &Operand,
        values: &[Option<Value>],
        view: &PortfolioView,
    ) -> Option<Value> {
        Some(match o {
            Operand::Slot(s) => values[*s].clone()?,
            Operand::Field(s, f) => match values[*s].clone()? {
                Value::Record(mut r) => {
                    r.remove(f)
                        .unwrap_or(Value::Missing(MissingReason::Undefined {
                            op: format!("no field {f}"),
                        }))
                }
                missing @ Value::Missing(_) => missing,
                _ => Value::Missing(MissingReason::Undefined {
                    op: format!("field {f} of a non-record"),
                }),
            },
            Operand::Const(v) => v.clone(),
            Operand::State(f) => Self::state_value(view, *f),
            Operand::Bar(input, field) => self.bar_value(*input, *field),
            Operand::List(xs) => {
                // Lists are only read through `arg_value`; as a single value a
                // list has no meaning.
                let _ = xs;
                Value::Missing(MissingReason::Undefined {
                    op: "a list read as one value".into(),
                })
            }
        })
    }

    fn arg_value(
        &self,
        o: &Operand,
        values: &[Option<Value>],
        view: &PortfolioView,
    ) -> Option<ArgVal> {
        match o {
            Operand::List(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.operand_value(x, values, view)?);
                }
                Some(ArgVal::Many(out))
            }
            other => self.operand_value(other, values, view).map(ArgVal::One),
        }
    }

    /// Assemble one model input. `Ok(None)`: depends on a node not yet
    /// evaluated. `Err`: cannot be assembled now, with why.
    fn model_input(
        &self,
        name: &str,
        arg: &ModelArg,
        values: &[Option<Value>],
        view: &PortfolioView,
    ) -> Result<Option<ModelInput>, MissingReason> {
        match arg {
            ModelArg::Row { input, columns } => {
                let h = &self.histories[*input];
                let mut row = BTreeMap::new();
                for (feature, col) in columns {
                    match h.latest(*col) {
                        Some(x) => {
                            row.insert(feature.clone(), x);
                        }
                        None => {
                            return Err(MissingReason::WarmingUp {
                                needed: features::runtime::lookback_bars(feature).unwrap_or(1),
                                have: u32::try_from(h.bars_seen).unwrap_or(u32::MAX),
                            })
                        }
                    }
                }
                Ok(Some(ModelInput::Row { values: row }))
            }
            ModelArg::Window {
                input,
                columns,
                lookback,
            } => {
                let h = &self.histories[*input];
                let n = *lookback as usize;
                let mut cols = Vec::with_capacity(columns.len());
                for (feature, col) in columns {
                    match h.window(*col, n) {
                        Some(xs) => cols.push((feature.clone(), xs)),
                        None => {
                            return Err(MissingReason::WarmingUp {
                                needed: features::runtime::lookback_bars(feature).unwrap_or(1)
                                    + lookback.saturating_sub(1),
                                have: u32::try_from(h.bars_seen).unwrap_or(u32::MAX),
                            })
                        }
                    }
                }
                let rows = (0..n)
                    .map(|i| cols.iter().map(|(_, xs)| xs[i]).collect())
                    .collect();
                Ok(Some(ModelInput::Window {
                    features: cols.into_iter().map(|(f, _)| f).collect(),
                    rows,
                }))
            }
            ModelArg::Value(o) => match self.operand_value(o, values, view) {
                None => Ok(None),
                Some(Value::Missing(r)) => Err(r),
                Some(v) => {
                    let _ = name;
                    Ok(Some(ModelInput::Value { value: v }))
                }
            },
        }
    }
}
