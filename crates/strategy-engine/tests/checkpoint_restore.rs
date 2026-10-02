//! Set U U-4.4: a restored engine continues exactly where the checkpointed one
//! would have — same values, same rule outcomes, same order tags — and a
//! checkpoint from another plan is refused.

mod common;

use std::sync::Arc;

use common::*;
use domain::strategy_def::params::ParamValues;
use strategy_engine::{compile, Engine, Event, RestoreError, StdRegistries, Step};

fn ema_cross() -> domain::strategy_graph::StrategyGraph {
    trading(
        serde_json::json!([
          {"id":"f","op":"ema","args":{"x":"close","n":5}},
          {"id":"s","op":"ema","args":{"x":"close","n":13}},
          {"id":"up","op":"crossed_above","args":{"a":"f","b":"s"}},
          {"id":"dn","op":"crossed_below","args":{"a":"f","b":"s"}}
        ]),
        serde_json::json!([
          {"when":"up","then":{"kind":"target","size":{"mode":"units","x":1}}},
          {"when":"dn","then":{"kind":"target","size":{"mode":"units","x":0}}}
        ]),
    )
}

fn step(e: &mut Engine, i: usize, close: f64) -> Option<String> {
    match e
        .on_event(
            Event::Bar {
                input: 0,
                bar: bar(i, close),
                warmup: false,
            },
            &flat(),
            &spec(),
        )
        .unwrap()
    {
        Step::Decided(t) => Some(serde_json::to_string(&*t).unwrap()),
        Step::Idle => None,
        Step::NeedModels(_) => panic!("no models in this plan"),
    }
}

#[test]
fn a_restored_engine_decides_exactly_as_the_one_that_never_stopped() {
    let plan = Arc::new(compile(&ema_cross(), &StdRegistries::new(), &ParamValues::new()).unwrap());
    let cs = closes(160);
    let k = 90;

    let mut straight = Engine::new(Arc::clone(&plan)).unwrap();
    let mut before = Engine::new(Arc::clone(&plan)).unwrap();
    for (i, c) in cs.iter().enumerate().take(k) {
        step(&mut straight, i, *c);
        step(&mut before, i, *c);
    }
    let cp = before.checkpoint().unwrap();
    // Through JSON, as the store keeps it.
    let cp: strategy_engine::Checkpoint =
        serde_json::from_str(&serde_json::to_string(&cp).unwrap()).unwrap();
    drop(before); // the process dies

    // Restore from every stored bar — including bars after the checkpoint,
    // which restore must ignore.
    let history = cs.iter().enumerate().map(|(i, c)| (0usize, bar(i, *c)));
    let mut restored = Engine::restore(Arc::clone(&plan), &cp, history, &spec()).unwrap();
    assert_eq!(restored.tick(), straight.tick());

    let mut decisions = 0;
    for (i, c) in cs.iter().enumerate().skip(k) {
        let a = step(&mut straight, i, *c);
        let b = step(&mut restored, i, *c);
        assert_eq!(a, b, "decision at bar {i} differs after restore");
        if a.as_deref().is_some_and(|s| s.contains("\"submit\"")) {
            decisions += 1;
        }
    }
    assert!(
        decisions > 0,
        "the comparison covered orders, with their tags"
    );
}

#[test]
fn a_checkpoint_from_another_plan_is_refused() {
    let plan = Arc::new(compile(&ema_cross(), &StdRegistries::new(), &ParamValues::new()).unwrap());
    let mut e = Engine::new(Arc::clone(&plan)).unwrap();
    step(&mut e, 0, 100.0);
    let mut cp = e.checkpoint().unwrap();
    cp.revision = "0".repeat(64);
    let err = Engine::restore(plan, &cp, std::iter::empty(), &spec())
        .err()
        .unwrap();
    assert!(matches!(err, RestoreError::PlanMismatch { .. }), "{err}");
}
