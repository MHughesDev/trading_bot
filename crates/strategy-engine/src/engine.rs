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
use crate::history::Bar;
use crate::instance::{Instance, TickCtx};
use crate::models::{self, ModelRequest, ModelResponse};
use crate::plan::{InputIdx, NodeKind, Plan, Slot, PRIMARY};
use crate::planner::{self, Planned, PlannerState};
use crate::portfolio::{Fill, InstrumentSpec, OrderClosed, PortfolioView};
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
    /// The environment's macro and fundamentals rows for the plan's external
    /// indicators (P-3.6), replacing any delivered before. Read point-in-time
    /// by each bar's close; never decides on its own.
    External(crate::external::ExternalData),
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
    #[error("a selection strategy is evaluated cross-sectionally over its universe: run it in a `SelectionEngine`, not an `Engine`")]
    SelectionPlan,
    #[error("a trading strategy decides for one instrument: run it in an `Engine`, not a `SelectionEngine`")]
    TradingPlan,
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
    /// Histories, stateful-operator state and last closes — the same per-
    /// instrument state a selection engine keeps for each member.
    pub(crate) inst: Instance,
    pub(crate) planner: PlannerState,
    pub(crate) tick: u64,
    pending: Option<Pending>,
}

impl Engine {
    /// Build an engine for a compiled plan.
    ///
    /// # Errors
    /// A selection plan (those run in a
    /// [`SelectionEngine`](crate::selection::SelectionEngine)), or an indicator
    /// the features crate cannot construct.
    pub fn new(plan: Arc<Plan>) -> Result<Self, EngineError> {
        if plan.kind == StrategyKind::Selection {
            return Err(EngineError::SelectionPlan);
        }
        let inst = Instance::new(&plan)?;
        Ok(Self {
            plan,
            inst,
            planner: PlannerState::default(),
            tick: 0,
            pending: None,
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
                if !self.inst.push(input, bar) {
                    return Ok(Step::Idle);
                }
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
            Event::External(data) => {
                self.inst.external = Arc::new(data);
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
        let ctx = TickCtx {
            plan: &plan,
            tick: self.tick,
            at_ns: p.at_ns,
            view: &p.view,
            warmup,
        };
        let made = {
            let waiting = &p.waiting;
            self.inst.pass(
                &ctx,
                &mut p.values,
                &|slot| waiting.values().any(|s| *s == slot),
                &|id| id.to_string(),
            )
        };
        let mut new_requests = Vec::with_capacity(made.len());
        for (slot, req) in made {
            p.waiting.insert(req.node.clone(), slot);
            new_requests.push(req);
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
                .inst
                .operand_value(&plan, &rule.when, &lookup, &p.view)
                .unwrap_or(Value::Bool(false));
            match when {
                Value::Bool(true) => {
                    taken.insert(rule.group.as_str());
                    let operand = |k: usize| {
                        rule.operands
                            .get(k)
                            .and_then(|o| self.inst.operand_value(&plan, o, &lookup, &p.view))
                            .unwrap_or(Value::Missing(MissingReason::Undefined {
                                op: format!("#{k}"),
                            }))
                    };
                    let atr = |n: u32| match plan.atr_columns.get(&n) {
                        Some(&c) => self.inst.histories[PRIMARY].latest(c).map_or_else(
                            || {
                                Value::Missing(MissingReason::WarmingUp {
                                    needed: features::runtime::lookback_bars(&format!("atr_{n}"))
                                        .unwrap_or(n),
                                    have: u32::try_from(self.inst.histories[PRIMARY].bars_seen)
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
}
