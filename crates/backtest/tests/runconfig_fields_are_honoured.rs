//! Every `RunConfig` field changes the run or refuses it by name (Set M M-0.2, BT-05).
//!
//! FEAT-005 §2 found, on 2026-09-10, fields whose hash differed while the result did not: the
//! executor read the config and ignored them. The defence has two halves, and both are here:
//!
//! 1. **The table cannot go stale.** The struct's field list is read out of the source of
//!    `RunConfig` and compared with the rows below, so a field added without a row fails this
//!    test, and a row for a field that no longer exists fails it too.
//! 2. **Each row is observed.** Change only that field and run the same fixture; the result is
//!    either different or a refusal that names the field. A field that is inert for a run kind
//!    says so in the table, with the reason.
//!
//! What this covers is the **position-series executor**, which was written to this standard
//! (`PositionRunExecutor::check_config`). The other two executors (`SimRunExecutor`, which
//! needs Postgres and ClickHouse, and the v3 runner) have `fill_delay_bars` proved by
//! `fill_delay.rs` and the rest unverified; the plan says so (M-0.2 is `PARTIAL`).

use std::collections::BTreeSet;
use std::sync::Arc;

use backtest::position_run::{ArtifactBar, Layer1Artifact, PositionRunExecutor};
use backtest::position_series::{PositionPoint, PositionSeries, PositionUnits, TruncationVerdict};
use backtest::run::{
    Backtest, FillModel, InMemoryRunStore, ParamMap, RunConfig, RunDispatch, RunResult, RunStatus,
};
use ledger::{DispatchContext, InMemoryLedger};

fn artifact() -> Layer1Artifact {
    let bars: Vec<ArtifactBar> = (0..120)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let p = 100.0 + (f64::from(i) * 0.37).sin() * 4.0;
            ArtifactBar {
                ts_ns: (i64::from(i) + 1) * 3_600_000_000_000,
                open: p,
                close: p + 0.2,
            }
        })
        .collect();
    let points = bars
        .iter()
        .enumerate()
        .map(|(i, b)| PositionPoint::new(b.ts_ns, if (i / 15) % 2 == 0 { 1.0 } else { 0.0 }))
        .collect();
    let mut params = ParamMap::new();
    params.insert("lb".into(), serde_json::json!(5));
    Layer1Artifact {
        code_sha256: "ab".repeat(32),
        instrument: "BTC-USD".into(),
        timeframe: "1h".into(),
        series: PositionSeries {
            points,
            position_units: PositionUnits::Fraction,
            allow_short: false,
        },
        bars,
        initial_equity: 10_000.0,
        fee_bps: 10.0,
        half_spread_bps: 2.0,
        cost_model_ref: "cost:flat".into(),
        data_snapshot: "snap-1".into(),
        lot_size: None,
        impact: None,
        params,
        seed: 7,
    }
}

fn run(a: &Arc<Layer1Artifact>, cfg: &RunConfig) -> RunResult {
    let bt = Backtest::new(
        InMemoryRunStore::new(),
        PositionRunExecutor::new(a.clone(), Some(TruncationVerdict::Pass { cuts_checked: 8 })),
        InMemoryLedger::new(),
    );
    let ctx = DispatchContext::human("tenant", "m-0.2", 0.1).with_experiment("exp");
    bt.run_traced(&RunDispatch::new(&ctx, cfg, 1.0)).unwrap().0
}

/// How a field is proved.
enum Proof {
    /// Changing it refuses the run, with a flag naming the field.
    RefusesNaming(&'static str),
    /// Changing it changes the result (here: the carried `unsafe` bit).
    ChangesResult,
    /// Changing it changes the equity curve (a cost scale).
    ChangesEquity,
    /// Inert for this run kind, and why.
    Inert(&'static str),
}

fn table() -> Vec<(&'static str, Proof)> {
    vec![
        ("run_id", Proof::Inert("derived: the builder sets it from every other field")),
        ("strategy_ref", Proof::Inert("a label for the instrument; the series is identified by strategy_version")),
        ("strategy_version", Proof::RefusesNaming("strategy_version")),
        ("params", Proof::RefusesNaming("params")),
        ("data_slice", Proof::RefusesNaming("data_slice")),
        ("cost_model_ref", Proof::RefusesNaming("cost_model_ref")),
        ("fill_model", Proof::RefusesNaming("fill_model")),
        ("sizing_ref", Proof::RefusesNaming("sizing_ref")),
        ("seed", Proof::RefusesNaming("seed")),
        ("data_snapshot", Proof::RefusesNaming("data_snapshot")),
        ("unsafe_", Proof::ChangesResult),
        ("unsafe_flags", Proof::Inert("recorded with `unsafe_`; it changes the run_id and the stored flags, not the arithmetic")),
        ("null_world", Proof::RefusesNaming("null_world")),
        ("target", Proof::RefusesNaming("target")),
        ("fill_delay_bars", Proof::RefusesNaming("fill_delay_bars")),
        ("cost_scale_bp", Proof::ChangesEquity),
    ]
}

/// The field names of `pub struct RunConfig`, read from its source.
fn struct_fields() -> BTreeSet<String> {
    let src = include_str!("../src/run/config.rs");
    let start = src
        .find("pub struct RunConfig {")
        .expect("RunConfig is declared in config.rs");
    let body = &src[start..];
    let end = body.find("\n}").expect("the struct closes");
    body[..end]
        .lines()
        .skip(1)
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split(':').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_table_has_a_row_for_every_field_and_no_row_for_a_field_that_is_gone() {
    let fields = struct_fields();
    let rows: BTreeSet<String> = table().iter().map(|(n, _)| (*n).to_string()).collect();
    assert_eq!(
        fields, rows,
        "RunConfig's fields and this table disagree: add a row (and a proof) for a new field"
    );
}

#[test]
fn each_field_changes_the_run_or_refuses_it_by_name() {
    let a = Arc::new(artifact());
    let base = a.run_config().expect("the artifact has a config");
    let baseline = run(&a, &base);
    assert_eq!(
        baseline.status,
        RunStatus::Ok,
        "{:?}",
        baseline.integrity_flags
    );

    let mutate = |field: &str, cfg: &mut RunConfig| match field {
        "strategy_version" => cfg.strategy_version.push_str("-edited"),
        "params" => {
            cfg.params.insert("lb".into(), serde_json::json!(6));
        }
        "data_slice" => cfg.data_slice.end += chrono::Duration::days(1),
        "cost_model_ref" => cfg.cost_model_ref = "cost:other".into(),
        "fill_model" => cfg.fill_model = FillModel::CurrentClose,
        "sizing_ref" => cfg.sizing_ref = "sizing:fixed".into(),
        "seed" => cfg.seed += 1,
        "data_snapshot" => cfg.data_snapshot = "snap-2".into(),
        "unsafe_" => cfg.unsafe_ = true,
        "null_world" => {
            cfg.null_world = Some(backtest::run::NullWorld {
                null: backtest::nulls::Null::new(
                    backtest::nulls::NullKind::BlockPermutation,
                    backtest::nulls::NullParams::default(),
                )
                .unwrap(),
                seed: 1,
            });
        }
        "target" => {
            cfg.target = Some(
                domain::strategy_graph::deployment::DeploymentTarget::Instrument {
                    instrument: "BTC-USD".into(),
                },
            );
        }
        "fill_delay_bars" => cfg.fill_delay_bars = 1,
        other => panic!("no mutation for {other}"),
    };

    for (field, proof) in table() {
        match proof {
            Proof::Inert(_) => {}
            Proof::RefusesNaming(name) => {
                let mut cfg = base.clone();
                mutate(field, &mut cfg);
                let r = run(&a, &cfg);
                assert!(
                    matches!(r.status, RunStatus::Failed(_)),
                    "{field}: expected a refusal, got {:?}",
                    r.status
                );
                assert!(
                    r.integrity_flags
                        .iter()
                        .any(|f| f.code.starts_with("layer1.config.")
                            && f.code.contains(&name.replace('/', "_"))),
                    "{field}: the refusal must name it: {:?}",
                    r.integrity_flags
                );
            }
            Proof::ChangesEquity => {
                let mut cfg = base.clone();
                cfg.cost_scale_bp = 5_000;
                let r = run(&a, &cfg);
                assert_eq!(r.status, RunStatus::Ok);
                assert!(
                    r.equity_curve.last().unwrap().1 < baseline.equity_curve.last().unwrap().1,
                    "{field}: dearer costs, lower equity"
                );
                assert_ne!(cfg.compute_id(), base.run_id);
            }
            Proof::ChangesResult => {
                let mut cfg = base.clone();
                mutate(field, &mut cfg);
                let r = run(&a, &cfg);
                assert_ne!(
                    r.unsafe_, baseline.unsafe_,
                    "{field}: the result carries the changed bit"
                );
            }
        }
    }
}

#[test]
fn a_field_that_is_inert_says_why_and_the_honoured_ones_outnumber_them() {
    let inert: Vec<_> = table()
        .into_iter()
        .filter(|(_, p)| matches!(p, Proof::Inert(_)))
        .collect();
    assert!(
        inert.len() <= 3,
        "inert fields need a reason each, and there should be few"
    );
    for (_, p) in inert {
        if let Proof::Inert(why) = p {
            assert!(why.len() > 20, "a reason, not a shrug");
        }
    }
}

#[test]
fn an_unchanged_config_is_accepted_so_the_refusals_above_are_about_the_one_field() {
    let a = Arc::new(artifact());
    let cfg = a.run_config().unwrap();
    assert_eq!(run(&a, &cfg).status, RunStatus::Ok);
}
