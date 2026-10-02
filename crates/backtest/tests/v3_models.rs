//! Set U U-5.4 acceptance: models answer backtests point-in-time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use backtest::models::{
    MemoryCache, PitModels, PredictionSource, RegistrySnapshot, RunModels, VersionEntry,
};
use backtest::run::{
    ComputeCost, DataSlice, EvalResolution, ParamMap, RunConfigBuilder, RunStatus, UnsafeFlags,
};
use backtest::runner::{self, V3Run};
use backtest::store::LoadedBar;
use chrono::{DateTime, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use domain::instrument::AssetClass;
use domain::model_def::contract::{ContractOutput, CostClass, IoContract, PitSpec};
use domain::strategy_graph::{StrategyGraph, Value, ValueType};
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::{InstrumentSpec, ModelOutcome, ModelRequest, VenueCaps};

const HOUR: i64 = 3_600_000_000_000;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
}

fn ns(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap()
}

fn bars(n: usize) -> Vec<LoadedBar> {
    (0..n)
        .map(|i| {
            let c = 100.0 + 5.0 * (i as f64 / 9.0).sin();
            let d = |x: f64| Decimal::from_f64_retain((x * 100.0).round() / 100.0).unwrap();
            let open_ns = ns(t0()) + i as i64 * HOUR;
            LoadedBar {
                ts_ns: open_ns + HOUR,
                open_ns,
                knowledge_ns: open_ns + HOUR,
                open: d(c),
                high: d(c + 0.5),
                low: d(c - 0.5),
                close: d(c),
                volume: dec!(1000),
                trade_count: 1,
                quality_flags: QualityFlags::default(),
            }
        })
        .collect()
}

fn contract() -> IoContract {
    IoContract {
        inputs: vec![],
        outputs: vec![ContractOutput {
            name: "f".into(),
            value_type: ValueType::Num,
        }],
        horizon: Some("1h".into()),
        pit: PitSpec {
            walk_forward: false,
            embargo: Some("1h".into()),
        },
        cost_class: CostClass::Cheap,
        timeout_ms: 100,
    }
}

fn registry(trained_through: Option<DateTime<Utc>>) -> Arc<RegistrySnapshot> {
    Arc::new(RegistrySnapshot::new().with_version(VersionEntry {
        model: "sizer".into(),
        version: 1,
        kind: "regressor".into(),
        contract: Some(contract()),
        trained_through,
        artifact_uri: None,
        artifact_sha256: None,
    }))
}

fn graph() -> StrategyGraph {
    serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "model_sized",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"m","op":"model","args":{"ref":{"str":"sizer@1"}}},
        {"id":"go","op":"ge","args":{"a":"close","b":0}}
      ],
      "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":"m.f"}}}]}
    }))
    .unwrap()
}

/// A sidecar stand-in that counts its calls and answers `f = 1`.
#[derive(Clone, Default)]
struct Counting(Arc<AtomicU64>);

impl PredictionSource for Counting {
    fn predict(&mut self, _e: &VersionEntry, _req: &ModelRequest) -> ModelOutcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        ModelOutcome::Answered {
            outputs: [("f".to_string(), Value::Num(1.0.try_into().unwrap()))]
                .into_iter()
                .collect(),
        }
    }
}

fn v3_run(reg: &RegistrySnapshot, start_ns: i64) -> V3Run {
    let plan = runner::compile_for_run(&graph(), &ParamMap::new(), reg).unwrap();
    V3Run {
        plan: Arc::new(plan),
        instrument: "ETH-USD".into(),
        asset_class: AssetClass::CryptoSpotCex,
        bars: vec![bars(120)],
        start_ns,
        spec: InstrumentSpec {
            instrument: "ETH-USD".into(),
            lot_size: dec!(0.0001),
            tick_size: dec!(0.01),
            min_qty: dec!(0.0001),
            caps: VenueCaps::all(),
        },
        instrument_limits: InstrumentLimits {
            tick_size: dec!(0.01),
            lot_size: Decimal::ZERO,
        },
        limits: GlobalRiskLimits::default(),
        initial_balance: dec!(10_000),
        fill_policy: FillPolicy::NextBarOpen,
        strategy_id: "model_sized".into(),
        sims: SimulatorSet::realistic(),
    }
}

fn cfg(optimistic: bool) -> backtest::run::RunConfig {
    let slice = DataSlice::new(
        "ETH-USD",
        t0(),
        t0() + chrono::Duration::hours(120),
        EvalResolution::Hour1,
    );
    RunConfigBuilder::new("model_sized", "rev", slice, "cost:paper", "sz", "snap")
        .disable_protection(UnsafeFlags {
            models_optimistic: optimistic,
            ..Default::default()
        })
        .build()
}

#[test]
fn a_model_trained_on_the_whole_window_cannot_answer_inside_it() {
    // Trained through the window's last bar: no decision in it is after
    // trained_through + embargo.
    let reg = registry(Some(t0() + chrono::Duration::hours(120)));
    let calls = Counting::default();
    let mut models = PitModels::new(
        Arc::clone(&reg),
        calls.clone(),
        Arc::new(MemoryCache::default()),
        false,
    );
    let out = runner::run(v3_run(&reg, ns(t0())), &mut models).unwrap();

    assert_eq!(
        calls.0.load(Ordering::SeqCst),
        0,
        "the sidecar was never asked"
    );
    assert!(
        out.trades.is_empty(),
        "the strategy held rather than guess a size"
    );
    assert_eq!(
        out.manifest
            .holds_by_reason
            .get("model_unavailable")
            .copied(),
        Some(out.manifest.decisions),
        "every decision held, and the report counts why: {:?}",
        out.manifest.holds_by_reason
    );
    let result = runner::finish(&cfg(false), out, ComputeCost::default(), &models);
    assert_eq!(
        result.status,
        RunStatus::Ok,
        "holding is honest, not a failure"
    );
}

#[test]
fn a_model_with_an_unknown_training_end_answers_no_backtest() {
    let reg = registry(None);
    let calls = Counting::default();
    let mut models = PitModels::new(
        Arc::clone(&reg),
        calls.clone(),
        Arc::new(MemoryCache::default()),
        false,
    );
    runner::run(v3_run(&reg, ns(t0())), &mut models).unwrap();
    assert_eq!(calls.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_model_trained_midway_answers_only_after_its_training_end_plus_embargo() {
    let through = t0() + chrono::Duration::hours(60);
    let reg = registry(Some(through));
    let calls = Counting::default();
    let mut models = PitModels::new(
        Arc::clone(&reg),
        calls.clone(),
        Arc::new(MemoryCache::default()),
        false,
    );
    let out = runner::run(v3_run(&reg, ns(t0())), &mut models).unwrap();
    let cutoff = ns(through) + HOUR;
    for t in out.traces.iter().filter(|t| !t.model_responses.is_empty()) {
        let answered = matches!(t.model_responses[0].outcome, ModelOutcome::Answered { .. });
        assert_eq!(
            answered,
            t.at_ns > cutoff,
            "at {} (cutoff {cutoff})",
            t.at_ns
        );
    }
    assert!(models
        .consumption()
        .iter()
        .all(backtest::models::ModelConsumption::point_in_time));
    let uses = models.fold_uses();
    assert_eq!(uses.len(), 1);
    assert!(uses[0].answered > 0 && uses[0].fold.contains("trained through"));
}

#[test]
fn a_deliberately_leaky_run_fails_gate_0_naming_the_first_timestamp() {
    let reg = registry(Some(t0() + chrono::Duration::hours(120)));
    let mut models = PitModels::new(
        Arc::clone(&reg),
        Counting::default(),
        Arc::new(MemoryCache::default()),
        true,
    );
    let out = runner::run(v3_run(&reg, ns(t0())), &mut models).unwrap();
    let first = out
        .traces
        .iter()
        .find(|t| {
            t.model_responses
                .iter()
                .any(|r| matches!(r.outcome, ModelOutcome::Answered { .. }))
        })
        .expect("the optimistic run consumed answers")
        .at_ns;
    let config = cfg(true);
    assert!(config.unsafe_, "optimistic_models is a disabled protection");
    assert_ne!(
        config.run_id,
        cfg(false).run_id,
        "and part of the run's identity"
    );
    let result = runner::finish(&config, out, ComputeCost::default(), &models);
    assert_eq!(result.status, RunStatus::RejectedIntegrity);
    let flag = result
        .integrity_flags
        .iter()
        .find(|f| f.code == backtest::gates::model_leakage::CODE)
        .expect("Gate 0 names it");
    let first_at = DateTime::<Utc>::from_timestamp_nanos(first).to_rfc3339();
    assert!(
        flag.detail.contains(&first_at),
        "{} should name {first_at}",
        flag.detail
    );
    assert!(result
        .integrity_flags
        .iter()
        .any(|f| f.code == "models.optimistic"));
}

#[test]
fn repeating_a_run_reads_the_cache_and_calls_the_sidecar_zero_times() {
    let reg = registry(Some(t0() - chrono::Duration::hours(10)));
    let cache = Arc::new(MemoryCache::default());
    let calls = Counting::default();

    let mut first = PitModels::new(Arc::clone(&reg), calls.clone(), cache.clone(), false);
    let a = runner::run(v3_run(&reg, ns(t0())), &mut first).unwrap();
    let after_first = calls.0.load(Ordering::SeqCst);
    assert!(after_first > 0);

    let mut second = PitModels::new(Arc::clone(&reg), calls.clone(), cache, false);
    let b = runner::run(v3_run(&reg, ns(t0())), &mut second).unwrap();
    assert_eq!(
        calls.0.load(Ordering::SeqCst),
        after_first,
        "no sidecar calls the second time"
    );
    assert_eq!(second.source_calls(), 0);
    assert_eq!(
        a.manifest.journal_digest, b.manifest.journal_digest,
        "and the same decisions"
    );
}
