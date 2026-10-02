//! Set U U-3.1: a mock environment drives a full decision cycle through the
//! ports — bar in, model asked and answered, decision journaled, orders handed
//! to the venue, the venue's refusal fed back — with nothing environment-
//! specific in the engine.

mod common;

use std::cell::Cell;

use common::*;
use domain::order::Side;
use domain::strategy_graph::{Value, ValueType};
use rust_decimal_macros::dec;
use strategy_engine::env::{MemoryJournal, ModelProvider, VenuePort};
use strategy_engine::planner::OrderAction;
use strategy_engine::{
    cycle, Event, InstrumentSpec, ModelOutcome, ModelRequest, OrderClosed, OrderClosedReason,
    PortfolioView,
};

struct Models {
    asked: Vec<String>,
}

impl ModelProvider for Models {
    fn answer(&mut self, req: &ModelRequest) -> ModelOutcome {
        self.asked.push(req.reference.clone());
        answered(vec![("f", Value::Num(2.0.try_into().unwrap()))])
    }
}

/// Records what it was asked to do and refuses every order, as a risk gate
/// would.
struct Refusing {
    acted: Vec<OrderAction>,
    views: Cell<u32>,
}

impl VenuePort for Refusing {
    fn view(&self) -> PortfolioView {
        self.views.set(self.views.get() + 1);
        flat()
    }
    fn spec(&self) -> InstrumentSpec {
        spec()
    }
    fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> Vec<Event> {
        self.acted.extend_from_slice(actions);
        actions
            .iter()
            .filter_map(|a| match a {
                OrderAction::Submit(o) => Some(Event::OrderClosed(OrderClosed {
                    tag: o.tag.clone(),
                    reason: OrderClosedReason::Rejected,
                    at_ns,
                })),
                OrderAction::Cancel { .. } => None,
            })
            .collect()
    }
}

#[test]
fn a_mock_environment_drives_one_full_decision_cycle() {
    let reg = registries(vec![model(
        "sizer@1",
        contract(vec![], vec![("f", ValueType::Num)]),
    )]);
    let g = trading(
        serde_json::json!([
          {"id":"m","op":"model","args":{"ref":{"str":"sizer@1"}}},
          {"id":"go","op":"ge","args":{"a":"close","b":0}}
        ]),
        serde_json::json!([
          {"when":"go","then":{"kind":"target","size":{"mode":"units","x":"m.f"}}}
        ]),
    );
    let mut engine = engine(&g, &reg);
    let mut models = Models { asked: Vec::new() };
    let mut venue = Refusing {
        acted: Vec::new(),
        views: Cell::new(0),
    };
    let mut journal = MemoryJournal::default();

    let decided = cycle(
        &mut engine,
        Event::Bar {
            input: 0,
            bar: bar(0, 100.0),
            warmup: false,
        },
        &mut models,
        &mut venue,
        &mut journal,
    )
    .expect("cycle runs");

    assert_eq!(
        models.asked,
        vec!["sizer@1".to_string()],
        "the model was asked once"
    );
    assert_eq!(decided.len(), 1);
    assert_eq!(journal.traces, decided, "every decision is journaled");
    let [OrderAction::Submit(order)] = venue.acted.as_slice() else {
        panic!("one order expected, got {:?}", venue.acted);
    };
    assert_eq!(
        (order.side, order.qty),
        (Side::Buy, dec!(2)),
        "sized by the model's answer"
    );
    // The refusal went back through the engine: one view for the bar, one for
    // the OrderClosed event the venue raised.
    assert_eq!(venue.views.get(), 2);
}
