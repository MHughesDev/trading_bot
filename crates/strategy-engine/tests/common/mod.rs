//! Shared fixtures for the engine's end-to-end tests.
#![allow(dead_code, clippy::needless_pass_by_value)]

use std::collections::BTreeMap;
use std::sync::Arc;

use domain::model_def::contract::{
    ContractInput, ContractOutput, CostClass, FeatureSource, InputShape, IoContract, PitSpec,
};
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::{StrategyGraph, Value, ValueType};
use rust_decimal_macros::dec;
use strategy_engine::{
    compile, Bar, Engine, Event, InstrumentSpec, PortfolioView, ResolvedModel, StdRegistries, Step,
    VenueCaps,
};

pub fn graph(v: serde_json::Value) -> StrategyGraph {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("graph does not parse: {e}\n{v:#}"))
}

/// A trading graph around `nodes` and `rules`.
#[allow(clippy::needless_pass_by_value)]
pub fn trading(nodes: serde_json::Value, rules: serde_json::Value) -> StrategyGraph {
    graph(serde_json::json!({
        "format": "tbot.strategy/3",
        "kind": "trading",
        "name": "t",
        "asset_class": "crypto_spot_cex",
        "timeframe": "1h",
        "nodes": nodes,
        "decide": {"rules": rules}
    }))
}

pub fn contract(inputs: Vec<ContractInput>, outputs: Vec<(&str, ValueType)>) -> IoContract {
    IoContract {
        inputs,
        outputs: outputs
            .into_iter()
            .map(|(n, t)| ContractOutput {
                name: n.into(),
                value_type: t,
            })
            .collect(),
        horizon: Some("1h".into()),
        pit: PitSpec::default(),
        cost_class: CostClass::Cheap,
        timeout_ms: 100,
    }
}

pub fn row_input(name: &str, features: &[&str]) -> ContractInput {
    ContractInput {
        name: name.into(),
        shape: InputShape::Row {
            features: FeatureSource::Named {
                features: features.iter().map(|s| (*s).to_string()).collect(),
            },
        },
        optional: false,
    }
}

pub fn scalar_input(name: &str, t: ValueType) -> ContractInput {
    ContractInput {
        name: name.into(),
        shape: InputShape::Scalar { value_type: t },
        optional: false,
    }
}

pub fn model(reference: &str, c: IoContract) -> (String, ResolvedModel) {
    (
        reference.to_string(),
        ResolvedModel {
            model: reference.split('@').next().unwrap_or(reference).to_string(),
            version: 1,
            contract: c,
        },
    )
}

pub fn registries(models: Vec<(String, ResolvedModel)>) -> StdRegistries {
    models
        .into_iter()
        .fold(StdRegistries::new(), |r, (k, m)| r.with_model(&k, m))
}

pub fn engine(g: &StrategyGraph, reg: &StdRegistries) -> Engine {
    let plan =
        compile(g, reg, &ParamValues::new()).unwrap_or_else(|d| panic!("does not compile: {d:#?}"));
    Engine::new(Arc::new(plan)).expect("engine builds")
}

pub fn spec() -> InstrumentSpec {
    InstrumentSpec {
        instrument: "BTC-USD".into(),
        lot_size: dec!(0.0001),
        tick_size: dec!(0.01),
        min_qty: dec!(0.0001),
        caps: VenueCaps::all(),
    }
}

pub fn flat() -> PortfolioView {
    PortfolioView {
        equity: dec!(10000),
        cash: dec!(10000),
        mark: Some(dec!(100)),
        ..Default::default()
    }
}

pub const HOUR_NS: i64 = 3_600_000_000_000;

/// A deterministic, oscillating close series (no randomness: same every run).
pub fn closes(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = i as f64;
            100.0 + 10.0 * (x / 7.0).sin() + 4.0 * (x / 3.0).cos() + x * 0.05
        })
        .collect()
}

pub fn bar(i: usize, close: f64) -> Bar {
    Bar {
        close_ns: (i as i64 + 1) * HOUR_NS,
        open: close - 0.5,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1000.0 + (i % 5) as f64,
    }
}

/// Feed `closes` as primary bars; answer every model request with `answer`.
pub fn drive(
    e: &mut Engine,
    closes: &[f64],
    answer: &dyn Fn(&strategy_engine::ModelRequest) -> strategy_engine::ModelOutcome,
) -> Vec<Step> {
    let mut out = Vec::new();
    for (i, c) in closes.iter().enumerate() {
        let mut step = e
            .on_event(
                Event::Bar {
                    input: 0,
                    bar: bar(i, *c),
                    warmup: false,
                },
                &flat(),
                &spec(),
            )
            .unwrap();
        while let Step::NeedModels(reqs) = &step {
            let responses = reqs
                .iter()
                .map(|r| strategy_engine::ModelResponse {
                    node: r.node.clone(),
                    outcome: answer(r),
                })
                .collect();
            step = e.resume(responses).unwrap();
        }
        out.push(step);
    }
    out
}

pub fn answered(fields: Vec<(&str, Value)>) -> strategy_engine::ModelOutcome {
    strategy_engine::ModelOutcome::Answered {
        outputs: fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<BTreeMap<_, _>>(),
    }
}
