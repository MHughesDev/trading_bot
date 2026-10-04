//! Replaying a paper journal through the engine the backtest builds (Set U
//! U-8.1; ADR-0047 decision 2; QA-1).
//!
//! Sharing code makes "backtest and paper decide the same way" true; this makes
//! it *measured*. A paper deployment journals every decision with what the
//! engine saw (`strategy_decision_journal`, 0144). Here those recorded inputs —
//! the bars, the portfolio views, the model answers it actually received — are
//! fed to an engine compiled the way a backtest compiles it
//! ([`compile_for_run`](crate::runner::compile_for_run)), and every decision
//! must come out byte-identical. The first one that does not is reported with
//! its sequence number and the first field that differs.
//!
//! # What a replay needs, and why the journal carries it
//!
//! An engine's indicators remember: an EMA is a function of every bar since the
//! engine started, and a [`Checkpoint`] holds no indicator state (indicators are
//! rebuilt from bars on restore). So a decision can only be reproduced from the
//! moment its engine was built. The paper runner therefore journals:
//!
//! - a [`Genesis`] on the first record after each start — the warm-up bars it
//!   built the engine from, and the checkpoint when it restored one;
//! - on every record, the events the engine consumed since the previous record
//!   *without* deciding ([`JournalInputs::preceding`]) — a bar of a secondary
//!   input, a fill that armed nothing — because each one advanced its state.
//!
//! A *segment* is a genesis and the records after it, up to the next genesis.
//! Records with no genesis before them (written before this was journaled, or
//! whose genesis aged out) are reported as unreplayable, never as a match.
//!
//! [`JournalInputs`] and [`Genesis`] are the one definition of that record: the
//! runner writes them and this module reads them, and the runner builds its
//! engine through [`Genesis::build`] — the same call a replay makes.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strategy_engine::plan::{InputIdx, PRIMARY};
use strategy_engine::{
    Bar, Checkpoint, Engine, EngineError, Event, InstrumentSpec, ModelResponse, Plan,
    PortfolioView, RestoreError, Step,
};

/// One journal record's `inputs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalInputs {
    /// The event that produced this decision.
    pub event: Event,
    /// The account the engine saw with it.
    pub view: PortfolioView,
    pub spec: InstrumentSpec,
    /// Events consumed since the previous record without a decision, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preceding: Vec<Consumed>,
    /// How the engine was built — on the first record after a start only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genesis: Option<Genesis>,
}

/// An event the engine consumed without deciding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Consumed {
    pub event: Event,
    pub view: PortfolioView,
}

/// How a runner built its engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Genesis {
    /// The checkpoint it restored from; `None` for a fresh warm-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    /// The stored bars it was given, per input, as given.
    pub history: Vec<(InputIdx, Bar)>,
}

impl Genesis {
    /// Build the engine: restore from the checkpoint over `history`, or warm a
    /// fresh engine on all of it — the bars in the order they closed, other
    /// inputs before the primary at an equal instant, with an empty account.
    ///
    /// # Errors
    /// A checkpoint from another plan, or an engine error.
    pub fn build(&self, plan: Arc<Plan>, spec: &InstrumentSpec) -> Result<Engine, RestoreError> {
        if let Some(cp) = &self.checkpoint {
            return Engine::restore(plan, cp, self.history.iter().copied(), spec);
        }
        let mut engine = Engine::new(plan)?;
        let mut bars = self.history.clone();
        bars.sort_by_key(|(i, b)| (b.close_ns, *i == PRIMARY, *i));
        let view = PortfolioView::default();
        for (input, bar) in bars {
            engine.on_event(
                Event::Bar {
                    input,
                    bar,
                    warmup: true,
                },
                &view,
                spec,
            )?;
        }
        Ok(engine)
    }
}

/// One journal record, as the replay reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedStep {
    pub seq: i64,
    /// The recorded trace's digest.
    pub digest: String,
    /// A [`JournalInputs`], as stored.
    pub inputs: Value,
    /// The recorded [`DecisionTrace`](strategy_engine::DecisionTrace), as stored.
    pub trace: Value,
}

/// The first decision that came out differently.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Divergence {
    /// The journal record.
    pub seq: i64,
    /// The first differing field of the trace, as a path (`values.f`,
    /// `actions[0].qty`), or what differed about the step itself.
    pub field: String,
    pub recorded: Value,
    pub replayed: Value,
}

/// What a segment's replay found.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Every decision byte-identical.
    Identical { decisions: usize },
    Diverged(Divergence),
    /// Could not be replayed, and why — never counted as a match.
    Unreplayable { reason: String },
}

/// One segment: a genesis and the records up to the next.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SegmentReport {
    pub first_seq: i64,
    pub last_seq: i64,
    #[serde(flatten)]
    pub verdict: Verdict,
}

/// Replay a deployment's journal, in `seq` order, through engines built from
/// `plan`. One report per segment.
#[must_use]
pub fn replay(plan: &Arc<Plan>, steps: &[RecordedStep]) -> Vec<SegmentReport> {
    let mut reports = Vec::new();
    let mut start = 0;
    while start < steps.len() {
        let end = steps[start + 1..]
            .iter()
            .position(has_genesis)
            .map_or(steps.len(), |p| start + 1 + p);
        let segment = &steps[start..end];
        let verdict = if has_genesis(&segment[0]) {
            replay_segment(plan, segment)
        } else {
            Verdict::Unreplayable {
                reason: "no genesis before these records: written before replay inputs were \
                         journaled, or the genesis aged out of retention"
                    .into(),
            }
        };
        reports.push(SegmentReport {
            first_seq: segment[0].seq,
            last_seq: segment[segment.len() - 1].seq,
            verdict,
        });
        start = end;
    }
    reports
}

fn has_genesis(s: &RecordedStep) -> bool {
    s.inputs.get("genesis").is_some_and(|g| !g.is_null())
}

fn diverged(seq: i64, field: impl Into<String>, recorded: Value, replayed: Value) -> Verdict {
    Verdict::Diverged(Divergence {
        seq,
        field: field.into(),
        recorded,
        replayed,
    })
}

fn engine_error(seq: i64, e: &EngineError) -> Verdict {
    diverged(
        seq,
        "engine",
        Value::String("a step".into()),
        Value::String(e.to_string()),
    )
}

fn replay_segment(plan: &Arc<Plan>, steps: &[RecordedStep]) -> Verdict {
    let mut engine: Option<Engine> = None;
    for s in steps {
        let inputs: JournalInputs = match serde_json::from_value(s.inputs.clone()) {
            Ok(i) => i,
            Err(e) => {
                return Verdict::Unreplayable {
                    reason: format!("record {}: inputs do not parse: {e}", s.seq),
                }
            }
        };
        let recorded_responses: Vec<ModelResponse> = match s
            .trace
            .get("model_responses")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
        {
            Ok(r) => r.unwrap_or_default(),
            Err(e) => {
                return Verdict::Unreplayable {
                    reason: format!("record {}: model answers do not parse: {e}", s.seq),
                }
            }
        };
        let engine = match (&mut engine, &inputs.genesis) {
            (Some(e), _) => e,
            (None, Some(g)) => match g.build(Arc::clone(plan), &inputs.spec) {
                Ok(e) => engine.insert(e),
                Err(e) => {
                    return diverged(
                        s.seq,
                        "genesis",
                        Value::String("an engine".into()),
                        Value::String(e.to_string()),
                    )
                }
            },
            (None, None) => {
                return Verdict::Unreplayable {
                    reason: format!("record {}: a segment must begin with a genesis", s.seq),
                }
            }
        };
        for (i, c) in inputs.preceding.iter().enumerate() {
            match engine.on_event(c.event.clone(), &c.view, &inputs.spec) {
                Ok(Step::Idle) => {}
                Ok(_) => {
                    return diverged(
                        s.seq,
                        format!("preceding[{i}]"),
                        Value::String("no decision".into()),
                        Value::String("a decision".into()),
                    )
                }
                Err(e) => return engine_error(s.seq, &e),
            }
        }
        let mut step = match engine.on_event(inputs.event.clone(), &inputs.view, &inputs.spec) {
            Ok(st) => st,
            Err(e) => return engine_error(s.seq, &e),
        };
        while let Step::NeedModels(requests) = step {
            let asked: BTreeSet<&str> = requests.iter().map(|r| r.node.as_str()).collect();
            // The answers the paper engine received, in the order it got them;
            // a request it got no answer to stays unanswered here too.
            let answers: Vec<ModelResponse> = recorded_responses
                .iter()
                .filter(|r| asked.contains(r.node.as_str()))
                .cloned()
                .collect();
            step = match engine.resume(answers) {
                Ok(st) => st,
                Err(e) => return engine_error(s.seq, &e),
            };
        }
        let Step::Decided(trace) = step else {
            return diverged(
                s.seq,
                "decided",
                Value::String("a decision".into()),
                Value::String("no decision".into()),
            );
        };
        if trace.digest() != s.digest {
            let replayed = serde_json::to_value(&*trace).unwrap_or_default();
            let (field, recorded, replayed) = first_difference(&s.trace, &replayed, String::new())
                .unwrap_or_else(|| {
                    (
                        "digest".into(),
                        Value::String(s.digest.clone()),
                        Value::String(trace.digest()),
                    )
                });
            return diverged(s.seq, field, recorded, replayed);
        }
    }
    Verdict::Identical {
        decisions: steps.len(),
    }
}

/// The first path at which `a` and `b` differ, depth first in key order, with
/// the two values there.
#[must_use]
pub fn first_difference(a: &Value, b: &Value, path: String) -> Option<(String, Value, Value)> {
    let join = |k: &str| {
        if path.is_empty() {
            k.to_string()
        } else {
            format!("{path}.{k}")
        }
    };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            keys.into_iter().find_map(|k| {
                let null = Value::Null;
                first_difference(x.get(k).unwrap_or(&null), y.get(k).unwrap_or(&null), join(k))
            })
        }
        (Value::Array(x), Value::Array(y)) => (0..x.len().max(y.len())).find_map(|i| {
            let null = Value::Null;
            first_difference(
                x.get(i).unwrap_or(&null),
                y.get(i).unwrap_or(&null),
                format!("{path}[{i}]"),
            )
        }),
        (Value::Number(x), Value::Number(y)) if x.as_f64() == y.as_f64() => None,
        _ if a == b => None,
        _ => Some((
            if path.is_empty() { "trace".into() } else { path },
            a.clone(),
            b.clone(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_first_difference_is_named_by_its_path() {
        let a = json!({"values": {"f": {"num": 1.0}, "s": 2}, "actions": [{"qty": "1"}]});
        let b = json!({"values": {"f": {"num": 1.5}, "s": 2}, "actions": [{"qty": "2"}]});
        let (field, x, y) = first_difference(&a, &b, String::new()).unwrap();
        assert_eq!(field, "actions[0].qty");
        assert_eq!((x, y), (json!("1"), json!("2")));
        let b = json!({"values": {"f": {"num": 1.0}, "s": 2}, "actions": [{"qty": "1"}]});
        assert_eq!(first_difference(&a, &b, String::new()), None);
        // An integer and the same float are the same number.
        assert_eq!(first_difference(&json!(5), &json!(5.0), String::new()), None);
    }

    /// A recorded input must read back as exactly the number the engine saw,
    /// or replay drifts in the last digit. serde_json does this only with its
    /// `float_roundtrip` feature (workspace Cargo.toml).
    #[test]
    fn every_recorded_number_reads_back_as_the_same_f64() {
        let mut x: f64 = 103.830_589_889_641_16;
        for i in 0..20_000 {
            x = (x * 1.000_37 + f64::from(i) * 1e-7).fract() * 1e3 + 1.0;
            let text = serde_json::to_string(&x).unwrap();
            let back: f64 = serde_json::from_str(&text).unwrap();
            assert_eq!(back.to_bits(), x.to_bits(), "{text}");
        }
    }

    #[test]
    fn records_before_any_genesis_are_unreplayable_not_a_match() {
        let plan = Arc::new(
            strategy_engine::compile(
                &serde_json::from_value(json!({
                  "format": "tbot.strategy/3", "kind": "trading", "name": "x",
                  "asset_class": "crypto_spot_cex", "timeframe": "1m",
                  "nodes": [], "decide": {"rules": []}
                }))
                .unwrap(),
                &strategy_engine::StdRegistries::new(),
                &domain::strategy_def::params::ParamValues::new(),
            )
            .unwrap(),
        );
        let step = RecordedStep {
            seq: 1,
            digest: "0".repeat(64),
            inputs: json!({"event": {"event": "timer", "at_ns": 0}}),
            trace: json!({}),
        };
        let r = replay(&plan, &[step]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0].verdict, Verdict::Unreplayable { .. }), "{r:?}");
    }
}
