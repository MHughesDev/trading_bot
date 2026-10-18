//! S-10.8: a stored result re-runs under a reproduction ticket, spends no trial,
//! and reports bit-equality or a named difference (AGENT-013 §6.9, §8.4;
//! ADR-P2-17, ADR-S10-18).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use backtest::receipt::{Reproduction, ReproSpec};
use backtest::run::executor::{daily_curve, map_sim_result};
use backtest::run::{
    Backtest, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore, RunConfig,
    RunConfigBuilder, RunDispatch, RunExecutor, RunResult, ENGINE_VERSION,
};
use chrono::{TimeZone, Utc};
use ledger::{DispatchContext, InMemoryLedger, TrialLedger, TrialTicket};

/// An executor whose answer and receipt the test controls.
struct Controlled {
    /// Added to every equity point: moves the number without moving the receipt.
    nudge: Arc<AtomicU32>,
    /// The calendar version the receipt reports.
    calendar: Arc<AtomicU32>,
    with_receipt: bool,
    ran_under_reproduction: Arc<AtomicU32>,
}

impl RunExecutor for Controlled {
    fn execute(&self, cfg: &RunConfig, ticket: &TrialTicket) -> RunResult {
        if ticket.reproduces().is_some() {
            self.ran_under_reproduction.fetch_add(1, Ordering::SeqCst);
        }
        let bump = f64::from(self.nudge.load(Ordering::SeqCst)) * 0.37;
        map_sim_result(
            cfg,
            daily_curve(&[100.0, 101.0 + bump, 99.5, 103.0 + bump * 2.0, 104.0]),
            vec![],
            vec![],
            ComputeCost::default(),
            ENGINE_VERSION,
        )
    }

    fn receipt(&self, _cfg: &RunConfig) -> Option<ReproSpec> {
        self.with_receipt.then(|| ReproSpec {
            dataset_hash: "sha256:data".into(),
            feature_registry_version: "features@1".into(),
            calendar_version: format!("sessions@{}", self.calendar.load(Ordering::SeqCst)),
            engine_version: ENGINE_VERSION.into(),
            strategy_version_hash: "sha256:strategy".into(),
            cost_model_version: "fees:v1".into(),
            seed_digest: "sha256:seed".into(),
            harness_version: "backtest-test".into(),
        })
    }
}

struct Rig {
    bt: Backtest<InMemoryRunStore, Controlled, Arc<InMemoryLedger>>,
    ledger: Arc<InMemoryLedger>,
    nudge: Arc<AtomicU32>,
    calendar: Arc<AtomicU32>,
    reproduced: Arc<AtomicU32>,
}

fn rig(with_receipt: bool) -> Rig {
    let nudge = Arc::new(AtomicU32::new(0));
    let calendar = Arc::new(AtomicU32::new(1));
    let reproduced = Arc::new(AtomicU32::new(0));
    let ledger = Arc::new(InMemoryLedger::new());
    let bt = Backtest::new(
        InMemoryRunStore::new(),
        Controlled {
            nudge: nudge.clone(),
            calendar: calendar.clone(),
            with_receipt,
            ran_under_reproduction: reproduced.clone(),
        },
        ledger.clone(),
    );
    Rig {
        bt,
        ledger,
        nudge,
        calendar,
        reproduced,
    }
}

fn cfg() -> RunConfig {
    let slice = DataSlice::new(
        "u",
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
        EvalResolution::Day1,
    );
    RunConfigBuilder::new("s", "v", slice, "c", "z", "snap")
        .seed(3)
        .build()
}

fn ctx() -> DispatchContext {
    DispatchContext::human("tenant-a", "tester", 0.1).with_experiment("exp")
}

fn record(r: &Rig) -> RunConfig {
    let c = cfg();
    let ctx = ctx();
    r.bt.run(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
    c
}

#[test]
fn a_recorded_run_reproduces_bit_identical_and_spends_no_trial() {
    let r = rig(true);
    let c = record(&r);
    let trials = r.ledger.trial_count("tenant-a");
    let rows = r.ledger.rows("tenant-a");

    let report = r.bt.reproduce("tenant-a", &c.run_id, "tester").unwrap();

    assert_eq!(report.reproduction, Reproduction::BitIdentical);
    assert!(!report.receipt_missing);
    assert_eq!(r.reproduced.load(Ordering::SeqCst), 1, "the executor ran, under a reproduction ticket");
    assert_eq!(r.ledger.trial_count("tenant-a"), trials, "a reproduction is not a look");
    assert_eq!(r.ledger.rows("tenant-a"), rows, "the hash chain did not move");
    let recorded = r.ledger.reproductions("tenant-a");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1.outcome, "bit_identical");
}

#[test]
fn a_deliberate_calendar_bump_is_a_named_difference_and_not_a_defect() {
    let r = rig(true);
    let c = record(&r);
    r.calendar.store(2, Ordering::SeqCst);

    let report = r.bt.reproduce("tenant-a", &c.run_id, "tester").unwrap();

    match &report.reproduction {
        Reproduction::ReceiptChanged { differences } => {
            assert_eq!(differences.len(), 1, "{differences:?}");
            assert_eq!(differences[0].field, "calendar_version");
            assert_eq!(differences[0].was, "sessions@1");
            assert_eq!(differences[0].now, "sessions@2");
        }
        other => panic!("expected a changed receipt, got {other:?}"),
    }
    assert!(!report.reproduction.is_platform_defect());
    assert_eq!(r.ledger.reproductions("tenant-a")[0].1.outcome, "receipt_changed");
}

#[test]
fn a_changed_number_under_an_unchanged_receipt_is_drift_and_a_defect() {
    let r = rig(true);
    let c = record(&r);
    r.nudge.store(1, Ordering::SeqCst);

    let report = r.bt.reproduce("tenant-a", &c.run_id, "tester").unwrap();

    assert!(
        matches!(report.reproduction, Reproduction::DriftedUnderTheSameReceipt { .. }),
        "{:?}",
        report.reproduction
    );
    assert!(report.reproduction.is_platform_defect());
    let rec = &r.ledger.reproductions("tenant-a")[0].1;
    assert_eq!(rec.outcome, "drifted");
    assert!(rec.was_value.is_some() && rec.now_value.is_some());
}

#[test]
fn an_original_with_no_receipt_is_reported_as_such_and_not_as_agreement() {
    let r = rig(false);
    let c = record(&r);
    let report = r.bt.reproduce("tenant-a", &c.run_id, "tester").unwrap();
    assert!(report.receipt_missing, "the outcome rests on the number alone and says so");
    assert!(r.ledger.reproductions("tenant-a")[0].1.receipt_missing());
}

#[test]
fn a_run_this_process_never_executed_cannot_be_reproduced() {
    let r = rig(true);
    let c = cfg();
    assert!(r.bt.reproduce("tenant-a", &c.run_id, "tester").is_err());
    assert!(r.ledger.reproductions("tenant-a").is_empty());
}
