//! Cross-sectional evaluation: the engine for selection strategies (Set U
//! U-6.2; FEAT-007 §4, §5.3; ADR-0040).
//!
//! A selection strategy answers *which assets should we be in?* It is the same
//! compiled graph as a trading strategy, evaluated once per universe member —
//! each member through the same [`Instance`] routine a trading engine uses, with
//! its own bars, indicators and stateful-operator state — and then compared
//! across members by the `cs_*` operators and ranked by the `select` sink.
//!
//! # One evaluation
//!
//! 1. Every member in the universe advances its own tick and evaluates every node
//!    it can. A `cs_*` node waits: it needs every member's input first.
//! 2. When every member has a `cs_*` node's input, the node is computed across
//!    them and written into each member's values; the members' passes resume
//!    from there. Model requests from every member go out together.
//! 3. When every node of every member has a value, the sink ranks the members:
//!    the filter, then the score, then `top_n`. Every member is accounted for —
//!    selected, or excluded with its reason. A member whose score is missing is
//!    excluded *with that reason*, never ranked as zero.
//!
//! # Order does not matter
//!
//! Members are kept and visited in instrument order, so the order a caller lists
//! the universe in cannot change a single value: the cross-sectional sums run in
//! one order, ties in the ranking are broken by instrument, and the excluded list
//! is sorted. Two environments asking about the same universe at the same instant
//! get the same selection, byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::DateTime;
use domain::strategy_graph::graph::SortOrder;
use domain::strategy_graph::selection::{Excluded, ExclusionReason, SelectedAsset, Selection};
use domain::strategy_graph::{MissingReason, StrategyKind, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::engine::EngineError;
use crate::history::Bar;
use crate::instance::{Instance, TickCtx};
use crate::models::{self, ModelRequest, ModelResponse};
use crate::plan::{InputIdx, NodeKind, Operand, Plan, Slot};
use crate::portfolio::PortfolioView;

/// What a selection engine needs next.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionStep {
    /// A warm-up evaluation: state advanced, nothing selected.
    Idle,
    /// Answer these, then call [`SelectionEngine::resume`].
    NeedModels(Vec<ModelRequest>),
    Selected(Box<SelectionRun>),
}

/// One evaluation's result and everything that produced it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectionRun {
    pub selection: Selection,
    /// Member → node id → value, at this evaluation.
    pub values: BTreeMap<String, BTreeMap<String, Value>>,
    pub model_requests: Vec<ModelRequest>,
    pub model_responses: Vec<ModelResponse>,
}

impl SelectionRun {
    /// SHA-256 of the canonical serialisation — what a journal stores and what
    /// two environments compare.
    #[must_use]
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(&bytes))
    }
}

struct Member {
    inst: Instance,
    tick: u64,
}

struct Pending {
    at_ns: i64,
    warmup: bool,
    /// Sorted, distinct.
    universe: Vec<String>,
    values: BTreeMap<String, Vec<Option<Value>>>,
    /// Request key → (member, slot).
    waiting: BTreeMap<String, (String, Slot)>,
    requests: Vec<ModelRequest>,
    responses: Vec<ModelResponse>,
}

/// One running selection strategy over a universe.
pub struct SelectionEngine {
    plan: Arc<Plan>,
    members: BTreeMap<String, Member>,
    pending: Option<Pending>,
    /// A selection reads no account; every `state.*` is refused at compile time,
    /// and this flat view is what the shared pass is handed.
    view: PortfolioView,
}

/// How a member's model request is named, so its answer finds its way back.
fn request_key(member: &str, node: &str) -> String {
    format!("{member}::{node}")
}

impl SelectionEngine {
    /// Build an engine for a compiled selection plan.
    ///
    /// # Errors
    /// A trading plan, or an indicator the features crate cannot construct.
    pub fn new(plan: Arc<Plan>) -> Result<Self, EngineError> {
        if plan.kind != StrategyKind::Selection || plan.select.is_none() {
            return Err(EngineError::TradingPlan);
        }
        // Fail now, not on the first member's first bar.
        Instance::new(&plan)?;
        Ok(Self {
            plan,
            members: BTreeMap::new(),
            pending: None,
            view: PortfolioView::default(),
        })
    }

    #[must_use]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Whether an evaluation is waiting for model answers.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn member(&mut self, instrument: &str) -> Result<&mut Member, EngineError> {
        if !self.members.contains_key(instrument) {
            let inst = Instance::new(&self.plan)?;
            self.members
                .insert(instrument.to_string(), Member { inst, tick: 0 });
        }
        Ok(self.members.get_mut(instrument).expect("just inserted"))
    }

    /// A bar closed on one of `instrument`'s inputs. Bars only build history;
    /// nothing is evaluated until [`Self::evaluate`].
    ///
    /// # Errors
    /// An evaluation is waiting for model answers.
    pub fn on_bar(
        &mut self,
        instrument: &str,
        input: InputIdx,
        bar: Bar,
    ) -> Result<(), EngineError> {
        if self.pending.is_some() {
            return Err(EngineError::TickPending);
        }
        self.member(instrument)?.inst.push(input, bar);
        Ok(())
    }

    /// Evaluate the strategy over `universe` at `at_ns`.
    ///
    /// The universe is the membership at that instant; a member that has never
    /// had a bar is evaluated like any other and excluded for having no data.
    /// `warmup` evaluations advance every member's stateful operators and call
    /// no model.
    ///
    /// # Errors
    /// An evaluation already waiting, or an engine defect.
    pub fn evaluate(
        &mut self,
        at_ns: i64,
        universe: &[String],
        warmup: bool,
    ) -> Result<SelectionStep, EngineError> {
        if self.pending.is_some() {
            return Err(EngineError::TickPending);
        }
        let universe: Vec<String> = universe
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let n = self.plan.nodes.len();
        let mut values = BTreeMap::new();
        for m in &universe {
            self.member(m)?.tick += 1;
            values.insert(m.clone(), vec![None; n]);
        }
        let p = Pending {
            at_ns,
            warmup,
            universe,
            values,
            waiting: BTreeMap::new(),
            requests: Vec::new(),
            responses: Vec::new(),
        };
        self.run(p)
    }

    /// Answer the outstanding model requests and continue the evaluation.
    ///
    /// # Errors
    /// No evaluation waiting, or a response nobody asked for.
    pub fn resume(&mut self, responses: Vec<ModelResponse>) -> Result<SelectionStep, EngineError> {
        let mut p = self.pending.take().ok_or(EngineError::NothingPending)?;
        for r in responses {
            let Some((member, slot)) = p.waiting.remove(&r.node) else {
                self.pending = Some(p);
                return Err(EngineError::UnexpectedResponse(r.node));
            };
            let NodeKind::Model(call) = &self.plan.nodes[slot].kind else {
                unreachable!("only model nodes wait");
            };
            p.values
                .get_mut(&member)
                .expect("a member of this evaluation")[slot] = Some(models::response_value(
                &call.reference,
                &call.contract,
                &r.outcome,
            ));
            p.responses.push(r);
        }
        // An unanswered request is a model that did not answer: missing, never
        // waited on for ever.
        for (_, (member, slot)) in std::mem::take(&mut p.waiting) {
            let NodeKind::Model(call) = &self.plan.nodes[slot].kind else {
                continue;
            };
            p.values
                .get_mut(&member)
                .expect("a member of this evaluation")[slot] =
                Some(Value::Missing(MissingReason::ModelTimeout {
                    model: call.reference.clone(),
                    timeout_ms: call.contract.timeout_ms,
                }));
        }
        self.run(p)
    }

    fn run(&mut self, mut p: Pending) -> Result<SelectionStep, EngineError> {
        let plan = Arc::clone(&self.plan);
        loop {
            let mut made = Vec::new();
            for m in &p.universe {
                let member = self.members.get_mut(m).expect("created by evaluate");
                let ctx = TickCtx {
                    plan: &plan,
                    tick: member.tick,
                    at_ns: p.at_ns,
                    view: &self.view,
                    warmup: p.warmup,
                };
                let waiting = &p.waiting;
                let vals = p.values.get_mut(m).expect("one row per member");
                for (slot, req) in member.inst.pass(
                    &ctx,
                    vals,
                    &|slot| waiting.values().any(|(w, s)| w == m && *s == slot),
                    &|node| request_key(m, node),
                ) {
                    made.push((m.clone(), slot, req));
                }
            }
            let progressed = self.cross_sections(&mut p);
            if !made.is_empty() {
                let mut out = Vec::with_capacity(made.len());
                for (m, slot, req) in made {
                    p.waiting.insert(req.node.clone(), (m, slot));
                    out.push(req);
                }
                p.requests.extend(out.iter().cloned());
                self.pending = Some(p);
                return Ok(SelectionStep::NeedModels(out));
            }
            if !progressed {
                break;
            }
        }
        let unevaluated = p
            .values
            .values()
            .map(|v| v.iter().filter(|x| x.is_none()).count())
            .sum::<usize>();
        if unevaluated > 0 {
            return Err(EngineError::Stalled(unevaluated));
        }
        if p.warmup {
            return Ok(SelectionStep::Idle);
        }
        Ok(SelectionStep::Selected(Box::new(self.select(p))))
    }

    /// Compute every `cs_*` node whose input every member now has. Returns
    /// whether any was computed.
    fn cross_sections(&self, p: &mut Pending) -> bool {
        let plan = &self.plan;
        let mut progressed = false;
        for (slot, node) in plan.nodes.iter().enumerate() {
            let NodeKind::Op { op, args } = &node.kind else {
                continue;
            };
            if !op.cross_sectional {
                continue;
            }
            if p.values.values().all(|v| v[slot].is_some()) {
                continue;
            }
            let port = op.inputs.first().map_or("x", |p| p.name);
            let Some(operand) = args.get(port) else {
                continue;
            };
            let mut inputs = Vec::with_capacity(p.universe.len());
            for m in &p.universe {
                let inst = &self.members[m].inst;
                match inst.operand_value(plan, operand, &p.values[m], &self.view) {
                    Some(v) => inputs.push(v),
                    None => break,
                }
            }
            if inputs.len() < p.universe.len() {
                continue; // some member has not reached it yet
            }
            for (m, v) in p.universe.iter().zip(cross_section(op.name, &inputs)) {
                p.values.get_mut(m).expect("one row per member")[slot] = Some(v);
            }
            progressed = true;
        }
        progressed
    }

    fn select(&self, p: Pending) -> SelectionRun {
        let plan = &self.plan;
        let sink = plan.select.as_ref().expect("checked in new");
        let read = |m: &str, o: &Operand| -> Value {
            self.members[m]
                .inst
                .operand_value(plan, o, &p.values[m], &self.view)
                .unwrap_or(Value::Missing(MissingReason::Undefined {
                    op: "unevaluated".into(),
                }))
        };
        let mut candidates: Vec<(f64, &String)> = Vec::new();
        let mut excluded: Vec<Excluded> = Vec::new();
        for m in &p.universe {
            let qualifies = match sink.filter.as_ref().map(|f| read(m, f)) {
                None | Some(Value::Bool(true)) => true,
                Some(Value::Missing(reason)) => {
                    excluded.push(Excluded {
                        instrument: m.clone(),
                        why: ExclusionReason::Missing { reason },
                    });
                    false
                }
                Some(_) => {
                    excluded.push(Excluded {
                        instrument: m.clone(),
                        why: ExclusionReason::Filtered,
                    });
                    false
                }
            };
            if !qualifies {
                continue;
            }
            match read(m, &sink.score) {
                Value::Num(x) => candidates.push((x.get(), m)),
                Value::Missing(reason) => excluded.push(Excluded {
                    instrument: m.clone(),
                    why: ExclusionReason::Missing { reason },
                }),
                other => excluded.push(Excluded {
                    instrument: m.clone(),
                    why: ExclusionReason::Missing {
                        reason: MissingReason::Undefined {
                            op: format!("a score that is not a number ({other:?})"),
                        },
                    },
                }),
            }
        }
        // Best first; ties broken by instrument so the order is total.
        candidates.sort_by(|(a, ia), (b, ib)| {
            let by_score = match sink.order {
                SortOrder::Desc => b.total_cmp(a),
                SortOrder::Asc => a.total_cmp(b),
            };
            by_score.then_with(|| ia.cmp(ib))
        });
        let mut selected = Vec::new();
        for (i, (score, m)) in candidates.into_iter().enumerate() {
            let rank = u32::try_from(i + 1).unwrap_or(u32::MAX);
            if sink.top_n.is_some_and(|n| rank > n) {
                excluded.push(Excluded {
                    instrument: m.clone(),
                    why: ExclusionReason::BelowTopN { rank, score },
                });
                continue;
            }
            let attributes = sink
                .attributes
                .iter()
                .map(|(k, o)| (k.clone(), read(m, o)))
                .collect();
            selected.push(SelectedAsset {
                instrument: m.clone(),
                rank,
                score,
                attributes,
            });
        }
        excluded.sort_by(|a, b| a.instrument.cmp(&b.instrument));
        let values = p
            .universe
            .iter()
            .map(|m| {
                let row = plan
                    .nodes
                    .iter()
                    .zip(&p.values[m])
                    .map(|(n, v)| {
                        (
                            n.id.clone(),
                            v.clone()
                                .unwrap_or(Value::Missing(MissingReason::Undefined {
                                    op: "unevaluated".into(),
                                })),
                        )
                    })
                    .collect();
                (m.clone(), row)
            })
            .collect();
        SelectionRun {
            selection: Selection {
                revision: plan.revision.clone(),
                as_of: DateTime::from_timestamp_nanos(p.at_ns),
                universe_size: u32::try_from(p.universe.len()).unwrap_or(u32::MAX),
                selected,
                excluded,
            },
            values,
            model_requests: p.requests,
            model_responses: p.responses,
        }
    }
}

/// One `cs_*` operator over every member's input, in member order.
///
/// Absence follows the engine's rule: a member whose input is missing gets a
/// missing output with the input's own reason, and is not counted in anyone
/// else's statistic. Statistics run over the members that have a value.
/// `cs_rank` and `cs_pct_rank` share mid-ranks for ties, the same convention as
/// the over-time `pct_rank`; `cs_zscore` uses the sample standard deviation, as
/// the over-time `zscore` does.
#[must_use]
pub fn cross_section(op: &str, inputs: &[Value]) -> Vec<Value> {
    if op == "cs_count" {
        #[allow(clippy::cast_precision_loss)]
        let k = inputs.iter().filter(|v| **v == Value::Bool(true)).count() as f64;
        return vec![Value::num(k, op); inputs.len()];
    }
    let xs: Vec<Option<f64>> = inputs.iter().map(Value::as_num).collect();
    let present: Vec<f64> = xs.iter().flatten().copied().collect();
    #[allow(clippy::cast_precision_loss)]
    let n = present.len() as f64;
    let own_absence = |i: usize| -> Value {
        match &inputs[i] {
            Value::Missing(r) => Value::Missing(r.clone()),
            other => Value::Missing(MissingReason::Undefined {
                op: format!("{op} of a non-number ({other:?})"),
            }),
        }
    };
    let none_present = || {
        Value::Missing(MissingReason::NoData {
            what: format!("{op}: no member of the universe has a value"),
        })
    };
    let mean = present.iter().sum::<f64>() / n;
    match op {
        "cs_mean" | "cs_median" => {
            if present.is_empty() {
                return vec![none_present(); inputs.len()];
            }
            let v = if op == "cs_mean" {
                mean
            } else {
                let mut s = present.clone();
                s.sort_by(f64::total_cmp);
                let k = s.len();
                if k % 2 == 1 {
                    s[k / 2]
                } else {
                    (s[k / 2 - 1] + s[k / 2]) / 2.0
                }
            };
            vec![Value::num(v, op); inputs.len()]
        }
        "cs_rank" | "cs_pct_rank" => (0..inputs.len())
            .map(|i| {
                let Some(x) = xs[i] else {
                    return own_absence(i);
                };
                #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
                let (below, equal, above) = (
                    present.iter().filter(|y| **y < x).count() as f64,
                    present.iter().filter(|y| **y == x).count() as f64,
                    present.iter().filter(|y| **y > x).count() as f64,
                );
                let r = if op == "cs_rank" {
                    1.0 + above + (equal - 1.0) / 2.0
                } else if n <= 1.0 {
                    0.5
                } else {
                    (below + (equal - 1.0) / 2.0) / (n - 1.0)
                };
                Value::num(r, op)
            })
            .collect(),
        "cs_zscore" => {
            let sd = (present.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            (0..inputs.len())
                .map(|i| {
                    let Some(x) = xs[i] else {
                        return own_absence(i);
                    };
                    if present.len() < 2 {
                        Value::Missing(MissingReason::Undefined {
                            op: "cs_zscore over fewer than two members".into(),
                        })
                    } else if sd == 0.0 {
                        Value::Missing(MissingReason::Undefined {
                            op: "cs_zscore of a universe where every member is equal".into(),
                        })
                    } else {
                        Value::num((x - mean) / sd, op)
                    }
                })
                .collect()
        }
        other => vec![
            Value::Missing(MissingReason::Undefined {
                op: format!("{other} is not a cross-sectional operator"),
            });
            inputs.len()
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(xs: &[f64]) -> Vec<Value> {
        xs.iter().map(|x| Value::num(*x, "t")).collect()
    }

    fn get(v: &[Value]) -> Vec<f64> {
        v.iter().map(|x| x.as_num().expect("present")).collect()
    }

    #[test]
    fn rank_and_zscore_match_a_reference_computation() {
        // Reference: scipy.stats.rankdata(-x, 'average') and
        // (x - mean) / std(x, ddof=1).
        let x = [3.0, 1.0, 4.0, 1.0, 5.0];
        assert_eq!(
            get(&cross_section("cs_rank", &nums(&x))),
            [3.0, 4.5, 2.0, 4.5, 1.0]
        );
        let z = get(&cross_section("cs_zscore", &nums(&x)));
        let mean = 2.8_f64;
        let sd = ((0.04 + 3.24 + 1.44 + 3.24 + 4.84) / 4.0_f64).sqrt();
        for (i, xi) in x.iter().enumerate() {
            assert!((z[i] - (xi - mean) / sd).abs() < 1e-12, "{z:?}");
        }
        assert_eq!(
            get(&cross_section("cs_pct_rank", &nums(&x))),
            [0.5, 0.125, 0.75, 0.125, 1.0]
        );
        assert_eq!(get(&cross_section("cs_mean", &nums(&x))), [2.8; 5]);
        assert_eq!(get(&cross_section("cs_median", &nums(&x))), [3.0; 5]);
        assert_eq!(
            get(&cross_section("cs_median", &nums(&[1.0, 2.0, 3.0, 10.0]))),
            [2.5; 4]
        );
    }

    #[test]
    fn a_missing_member_keeps_its_reason_and_is_not_counted() {
        let gone = MissingReason::WarmingUp {
            needed: 20,
            have: 3,
        };
        let mut xs = nums(&[1.0, 2.0, 3.0]);
        xs.insert(1, Value::Missing(gone.clone()));
        let r = cross_section("cs_rank", &xs);
        assert_eq!(r[1], Value::Missing(gone.clone()), "never ranked as zero");
        assert_eq!(
            [r[0].as_num(), r[2].as_num(), r[3].as_num()],
            [Some(3.0), Some(2.0), Some(1.0)],
            "ranked among the three that have a value"
        );
        assert_eq!(cross_section("cs_mean", &xs)[0].as_num(), Some(2.0));
        let conds = vec![
            Value::Bool(true),
            Value::Missing(gone),
            Value::Bool(false),
            Value::Bool(true),
        ];
        assert_eq!(cross_section("cs_count", &conds)[0].as_num(), Some(2.0));
    }

    #[test]
    fn degenerate_universes_are_named_not_guessed() {
        assert!(matches!(
            cross_section("cs_zscore", &nums(&[1.0]))[0],
            Value::Missing(MissingReason::Undefined { .. })
        ));
        assert!(matches!(
            cross_section("cs_zscore", &nums(&[2.0, 2.0]))[0],
            Value::Missing(MissingReason::Undefined { .. })
        ));
        assert!(matches!(
            cross_section(
                "cs_mean",
                &[Value::Missing(MissingReason::NoData { what: "x".into() })]
            )[0],
            Value::Missing(MissingReason::NoData { .. })
        ));
        assert_eq!(
            cross_section("cs_pct_rank", &nums(&[7.0]))[0].as_num(),
            Some(0.5)
        );
    }
}
