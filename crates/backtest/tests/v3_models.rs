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
        adapter: None,
        transform: None,
        folds: Vec::new(),
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

// ── U-5.5: the prediction series ─────────────────────────────────────────────

fn row_contract() -> IoContract {
    use domain::model_def::contract::{ContractInput, FeatureSource, InputShape};
    IoContract {
        inputs: vec![ContractInput {
            name: "features".into(),
            shape: InputShape::Row {
                features: FeatureSource::Named {
                    features: vec!["rsi_14".into()],
                },
            },
            optional: false,
        }],
        ..contract()
    }
}

#[test]
fn a_series_covers_exactly_the_out_of_sample_window_and_a_backtest_then_asks_nothing() {
    // Past the input's warm-up: `rsi_14` needs its history before the engine
    // can assemble a request at all, so the training end sits after it and the
    // test sees both refused in-sample decisions and answered ones.
    let warm = features::runtime::lookback_bars("rsi_14").unwrap();
    let through = t0() + chrono::Duration::hours(i64::from(warm) + 15);
    let reg = Arc::new(RegistrySnapshot::new().with_version(VersionEntry {
        model: "sizer".into(),
        version: 1,
        kind: "regressor".into(),
        contract: Some(row_contract()),
        trained_through: Some(through),
        artifact_uri: None,
        artifact_sha256: None,
        adapter: None,
        transform: None,
        folds: Vec::new(),
    }));
    let cache = Arc::new(MemoryCache::default());

    // The job: fill the series over the whole window.
    let filled = Counting::default();
    let mut series_models = PitModels::new(Arc::clone(&reg), filled.clone(), cache.clone(), false);
    let report = runner::predict_series(
        "sizer@1",
        &reg,
        "ETH-USD",
        "1h",
        AssetClass::CryptoSpotCex,
        bars(120),
        ns(t0()),
        &mut series_models,
    )
    .unwrap();
    let cutoff = ns(through) + HOUR;
    let keys = cache.keys();
    assert_eq!(
        keys.len() as u64,
        report.answered,
        "one row per answered decision"
    );
    assert!(
        keys.iter().all(|k| k.at_ns > cutoff),
        "nothing inside the training window"
    );
    let mut instants: Vec<i64> = keys.iter().map(|k| k.at_ns).collect();
    instants.dedup();
    assert_eq!(instants.len(), keys.len(), "no decision twice: no overlap");
    let after_cutoff = bars(120).iter().filter(|b| b.ts_ns > cutoff).count() as u64;
    assert_eq!(
        report.answered, after_cutoff,
        "every out-of-sample bar is covered"
    );
    assert!(
        report.before_training_end > 0,
        "and the in-sample ones were refused, not written"
    );

    // A strategy backtest over the same window reads the series.
    let strategy: StrategyGraph = serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "uses_series",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"m","op":"model","args":{"ref":{"str":"sizer@1"}}},
        {"id":"go","op":"gt","args":{"a":"m.f","b":0}}
      ],
      "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":1}}}]}
    }))
    .unwrap();
    let plan = runner::compile_for_run(&strategy, &ParamMap::new(), reg.as_ref()).unwrap();
    let mut r = v3_run(&reg, ns(t0()));
    r.plan = Arc::new(plan);
    let asked = Counting::default();
    let mut models = PitModels::new(Arc::clone(&reg), asked.clone(), cache, false);
    let out = runner::run(r, &mut models).unwrap();
    assert_eq!(
        asked.0.load(Ordering::SeqCst),
        0,
        "the backtest made no sidecar calls"
    );
    assert!(
        !out.trades.is_empty() || out.manifest.journal.iter().any(|j| j.actions > 0),
        "and it traded on the series"
    );
}

// ── U-5.7: a transform answers backtests ────────────────────────────────────

#[test]
fn a_transform_has_no_training_end_to_be_held_to_and_answers_every_decision() {
    let reg = Arc::new(RegistrySnapshot::new().with_version(VersionEntry {
        model: "sizer".into(),
        version: 1,
        kind: "transform".into(),
        contract: Some(contract()),
        trained_through: None,
        artifact_uri: None,
        artifact_sha256: None,
        adapter: None,
        transform: Some(serde_json::json!({"language": "python", "entrypoint": "f", "source": "def f(i):\n    return {'f': 1.0}\n"})),
        folds: Vec::new(),
    }));
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
        out.manifest.decisions,
        "asked at every decision"
    );
    assert!(models
        .consumption()
        .iter()
        .all(backtest::models::ModelConsumption::point_in_time));
    let result = runner::finish(&cfg(false), out, ComputeCost::default(), &models);
    assert_eq!(
        result.status,
        RunStatus::Ok,
        "Gate 0 has nothing to object to"
    );
}

// ── U-5.8: one model per walk-forward fold ──────────────────────────────────

/// A sidecar stand-in that records which artifact answered at which instant.
#[derive(Clone, Default)]
#[allow(clippy::type_complexity)]
struct WhoAnswered(Arc<std::sync::Mutex<Vec<(i64, String, DateTime<Utc>)>>>);

impl PredictionSource for WhoAnswered {
    fn predict(&mut self, e: &VersionEntry, req: &ModelRequest) -> ModelOutcome {
        self.0.lock().unwrap().push((
            req.at_ns,
            e.artifact_uri.clone().unwrap_or_default(),
            e.trained_through
                .expect("an answering model has a training end"),
        ));
        Counting::default().predict(e, req)
    }
}

#[test]
fn each_fold_answers_its_own_test_window_and_none_answers_before_it_was_trained() {
    use backtest::models::FoldArtifact;
    let at = |h: i64| t0() + chrono::Duration::hours(h);
    let fold = |index: u32, h: i64| FoldArtifact {
        index,
        trained_through: at(h),
        artifact_uri: format!("file:///fold/{index}"),
        artifact_sha256: format!("{index:064}"),
    };
    let mut entry = registry(Some(at(100))).pinned("sizer", 1).unwrap().clone();
    entry.artifact_uri = Some("file:///version".into());
    entry.artifact_sha256 = Some("f".repeat(64));
    entry.folds = vec![fold(0, 20), fold(1, 50), fold(2, 80)];
    let reg = Arc::new(RegistrySnapshot::new().with_version(entry));

    let who = WhoAnswered::default();
    let mut models = PitModels::new(
        Arc::clone(&reg),
        who.clone(),
        Arc::new(MemoryCache::default()),
        false,
    );
    let out = runner::run(v3_run(&reg, ns(t0())), &mut models).unwrap();
    let answers = who.0.lock().unwrap().clone();

    // Every training end, the version's own last.
    let ends = [
        (20, "file:///fold/0"),
        (50, "file:///fold/1"),
        (80, "file:///fold/2"),
        (100, "file:///version"),
    ];
    for (at_ns, uri, through) in &answers {
        let through_ns = ns(*through);
        assert!(
            through_ns + HOUR < *at_ns,
            "{uri} (trained through {through}) answered at {at_ns}: inside its own training"
        );
        // And it was the latest one allowed: no later model was also eligible.
        let latest = ends
            .iter()
            .rfind(|(h, _)| ns(at(*h)) + HOUR < *at_ns)
            .unwrap();
        assert_eq!(uri, latest.1, "at {at_ns}");
    }
    for (_, uri) in ends {
        assert!(
            answers.iter().any(|(_, u, _)| u == uri),
            "{uri} answered its window"
        );
    }
    // Every decision after fold 0 may answer is answered — a fold switches in
    // as soon as the embargo allows, never a bar late (which would hold).
    let open = ns(at(20)) + HOUR;
    assert_eq!(
        answers.len(),
        bars(120).iter().filter(|b| b.ts_ns > open).count(),
        "every decision after {open} answered"
    );
    // Before the first fold's training end nothing could answer: those held.
    assert!(
        out.manifest
            .holds_by_reason
            .get("model_unavailable")
            .copied()
            .unwrap_or(0)
            > 0
    );
    assert!(
        answers.iter().all(|(t, _, _)| *t > ns(at(20)) + HOUR),
        "nothing answered before fold 0 was trained"
    );

    // The manifest names each fold that answered, and Gate 0 passes.
    let uses = models.fold_uses();
    assert_eq!(uses.len(), 4, "{uses:?}");
    assert!(uses
        .iter()
        .any(|u| u.fold.starts_with("fold 1, trained through")));
    assert_eq!(
        uses.iter().map(|u| u.answered).sum::<u64>(),
        answers.len() as u64
    );
    assert!(models
        .consumption()
        .iter()
        .all(backtest::models::ModelConsumption::point_in_time));
    let result = runner::finish(&cfg(false), out, ComputeCost::default(), &models);
    assert_eq!(result.status, RunStatus::Ok);
}
