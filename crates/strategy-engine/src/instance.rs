//! One instrument's evaluation state, and the one routine that evaluates a plan's
//! nodes against it (U-1.4, U-6.2).
//!
//! A trading [`Engine`](crate::Engine) holds one [`Instance`]; a
//! [`SelectionEngine`](crate::selection::SelectionEngine) holds one per universe
//! member. Both evaluate nodes through [`Instance::pass`], so an indicator, a
//! comparison, a stateful operator or a model input means the same thing in a
//! screener as in the strategy that trades its result — there is no second
//! evaluator to drift.
//!
//! What differs is only what sits *around* a pass: a trading engine matches rules
//! after it; a selection engine fills the cross-sectional nodes between passes,
//! because a `cs_*` value needs every member's input before any member has it.

use std::collections::BTreeMap;

use domain::strategy_graph::{MissingReason, Value};

use crate::engine::EngineError;
use crate::eval::{self, ArgVal};
use crate::history::{Bar, History};
use crate::models::{ModelInput, ModelRequest};
use crate::plan::{BarField, InputIdx, ModelArg, NodeKind, Operand, Plan, Slot, StateField};
use crate::portfolio::PortfolioView;
use crate::temporal::Temporal;

/// What a pass needs to know about the tick it belongs to.
pub(crate) struct TickCtx<'a> {
    pub(crate) plan: &'a Plan,
    /// This instance's tick: stateful operators step once per tick.
    pub(crate) tick: u64,
    pub(crate) at_ns: i64,
    pub(crate) view: &'a PortfolioView,
    /// Warm-up ticks build history and state but never call a model.
    pub(crate) warmup: bool,
}

/// Histories, stateful-operator state and the last close per input — everything
/// that carries from one tick to the next for one instrument.
pub(crate) struct Instance {
    pub(crate) histories: Vec<History>,
    pub(crate) temporal: Vec<Option<Temporal>>,
    /// The close of the last bar seen on each input — where a checkpoint's
    /// history replay stops (U-4.4).
    pub(crate) last_close: Vec<Option<i64>>,
    /// The environment's macro and fundamentals rows (P-3.6). Empty until an
    /// [`Event::External`](crate::Event::External) arrives.
    pub(crate) external: std::sync::Arc<crate::external::ExternalData>,
}

impl Instance {
    /// Fresh state for `plan`.
    ///
    /// # Errors
    /// An indicator the features crate cannot construct.
    pub(crate) fn new(plan: &Plan) -> Result<Self, EngineError> {
        let mut histories = Vec::with_capacity(plan.inputs.len());
        for (i, input) in plan.inputs.iter().enumerate() {
            let mut impls = Vec::with_capacity(input.columns.len());
            let mut rows_cap = 1usize;
            for name in &input.columns {
                let bar_secs = input.timeframe.seconds();
                let f = match features::external_features::resolve_external(name, bar_secs) {
                    Ok(Some(f)) => crate::history::Column::External(f),
                    Ok(None) => crate::history::Column::Windowed(
                        features::runtime::feature(name)
                            .map_err(|e| EngineError::Feature(name.clone(), e.to_string()))?,
                    ),
                    Err(e) => return Err(EngineError::Feature(name.clone(), e.to_string())),
                };
                rows_cap = rows_cap.max(f.lookback_bars() as usize);
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
        Ok(Self {
            histories,
            temporal,
            last_close: vec![None; plan.inputs.len()],
            external: std::sync::Arc::default(),
        })
    }

    /// Append a closed bar to one input. `false` if the plan has no such input.
    pub(crate) fn push(&mut self, input: InputIdx, bar: Bar) -> bool {
        let Some(h) = self.histories.get_mut(input) else {
            return false;
        };
        h.push(bar, &self.external);
        self.last_close[input] = Some(bar.close_ns);
        true
    }

    /// Evaluate every node that can be evaluated now, in plan order.
    ///
    /// A node is skipped while it already has a value, while `waiting(slot)`, or
    /// while one of its inputs has none — a later pass picks it up. A
    /// cross-sectional node is never evaluated here: the selection engine fills
    /// it from every member at once. Returns the model requests this pass made,
    /// each with the slot it answers; `request_node` names a request (a trading
    /// engine uses the node id, a selection engine qualifies it by member).
    pub(crate) fn pass(
        &mut self,
        ctx: &TickCtx<'_>,
        values: &mut [Option<Value>],
        waiting: &dyn Fn(Slot) -> bool,
        request_node: &dyn Fn(&str) -> String,
    ) -> Vec<(Slot, ModelRequest)> {
        let plan = ctx.plan;
        let mut requests = Vec::new();
        for (slot, node) in plan.nodes.iter().enumerate() {
            if values[slot].is_some() || waiting(slot) {
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
                    values[slot] = Some(match h.latest(*column) {
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
                    values[slot] = Some(self.bar_value(plan, *input, *field));
                }
                NodeKind::Hour => {
                    let secs = ctx.at_ns.div_euclid(1_000_000_000);
                    #[allow(clippy::cast_precision_loss)]
                    let h = (secs.div_euclid(3_600)).rem_euclid(24) as f64;
                    values[slot] = Some(Value::num(h, "hour"));
                }
                NodeKind::Weekday => {
                    let days = ctx.at_ns.div_euclid(86_400_000_000_000);
                    // 1970-01-01 was a Thursday; Monday = 0.
                    #[allow(clippy::cast_precision_loss)]
                    let d = (days + 3).rem_euclid(7) as f64;
                    values[slot] = Some(Value::num(d, "weekday"));
                }
                NodeKind::SelectionAttr { selection, attr } => {
                    values[slot] = Some(selection_attr(ctx.view, selection, attr));
                }
                NodeKind::Op { op, .. } if op.cross_sectional => {}
                NodeKind::Op { op, args } => {
                    let mut vals: BTreeMap<String, ArgVal> = BTreeMap::new();
                    let mut ready = true;
                    for (port, operand) in args {
                        if let Some(v) = self.arg_value(plan, operand, values, ctx.view) {
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
                        state.step(op.name, ctx.tick, &get)
                    } else {
                        let ports: Vec<&str> = op.inputs.iter().map(|p| p.name).collect();
                        eval::eval(op.name, &ports, &vals)
                    };
                    values[slot] = Some(v);
                }
                NodeKind::Model(call) => {
                    if ctx.warmup {
                        values[slot] = Some(Value::Missing(MissingReason::WarmingUp {
                            needed: node.warmup,
                            have: u32::try_from(ctx.tick).unwrap_or(u32::MAX),
                        }));
                        continue;
                    }
                    let mut inputs = BTreeMap::new();
                    let mut ready = true;
                    let mut missing: Option<MissingReason> = None;
                    for (name, arg) in &call.inputs {
                        match self.model_input(plan, arg, values, ctx.view) {
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
                        values[slot] = Some(Value::Missing(reason));
                        continue;
                    }
                    requests.push((
                        slot,
                        ModelRequest {
                            node: request_node(&node.id),
                            reference: call.reference.clone(),
                            model: call.model.clone(),
                            version: call.version,
                            at_ns: ctx.at_ns,
                            inputs,
                        },
                    ));
                }
            }
        }
        requests
    }

    pub(crate) fn bar_value(&self, plan: &Plan, input: InputIdx, field: BarField) -> Value {
        let Some(b) = self.histories.get(input).and_then(|h| h.last) else {
            return Value::Missing(MissingReason::NoData {
                what: format!(
                    "a closed bar on {}",
                    plan.inputs.get(input).map_or("?", |i| i.id.as_str())
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

    pub(crate) fn state_value(view: &PortfolioView, f: StateField) -> Value {
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
    pub(crate) fn operand_value(
        &self,
        plan: &Plan,
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
            Operand::Bar(input, field) => self.bar_value(plan, *input, *field),
            Operand::List(_) => {
                // Lists are only read through `arg_value`; as a single value a
                // list has no meaning.
                Value::Missing(MissingReason::Undefined {
                    op: "a list read as one value".into(),
                })
            }
        })
    }

    fn arg_value(
        &self,
        plan: &Plan,
        o: &Operand,
        values: &[Option<Value>],
        view: &PortfolioView,
    ) -> Option<ArgVal> {
        match o {
            Operand::List(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.operand_value(plan, x, values, view)?);
                }
                Some(ArgVal::Many(out))
            }
            other => self
                .operand_value(plan, other, values, view)
                .map(ArgVal::One),
        }
    }

    /// Assemble one model input. `Ok(None)`: depends on a node not yet
    /// evaluated. `Err`: cannot be assembled now, with why.
    fn model_input(
        &self,
        plan: &Plan,
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
            ModelArg::Value(o) => match self.operand_value(plan, o, values, view) {
                None => Ok(None),
                Some(Value::Missing(r)) => Err(r),
                Some(v) => Ok(Some(ModelInput::Value { value: v })),
            },
        }
    }
}

/// `selection_attr` (U-6.6): the attribute this member's selection gave its
/// instrument — missing, with the reason, when the instance is not bound to a
/// selection, is bound to another one, or was given no such attribute.
fn selection_attr(view: &PortfolioView, selection: &str, attr: &str) -> Value {
    let missing = |what: String| Value::Missing(MissingReason::NoData { what });
    match &view.selection {
        None => missing(format!(
            "selection attribute `{attr}`: this instance is not bound to a selection"
        )),
        Some(s) if s.name != selection && s.revision != selection => missing(format!(
            "selection attribute `{attr}`: this instance trades for selection `{}`, not `{selection}`",
            s.name
        )),
        // Back through the decimal's own digits: `to_f64` is not exact, and the
        // attribute must read as the number the selection computed.
        Some(s) => match s.attributes.get(attr).and_then(|d| d.to_string().parse::<f64>().ok()) {
            Some(x) => Value::num(x, attr),
            None => missing(format!(
                "selection attribute `{attr}`: selection `{}` gave this member no `{attr}`",
                s.name
            )),
        },
    }
}
