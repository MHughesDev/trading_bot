//! `BatchRun`: N members over one data load, each its own counted trial (Set M M-3.1; BT-08).
//!
//! A sweep of N parameter sets over one window should read the bars **once**, not N times, and
//! it must not turn N looks into one: every member is its own [`RunConfig`], its own `run_id`
//! and its own registered ledger trial (INV-1), through the same `Backtest::run_traced` path a
//! single run takes, so a member's result is *identical* to its single run's.
//!
//! What this module does and does not claim. It proves, on a fixture, that members equal their
//! single-run results and that the ledger counts every member. The "at least 5x faster than 200
//! single runs" clause of the task is a statement about **data-load cost**, and the load that
//! costs is the ClickHouse read the platform worker does: [`BarSource`] is that seam. The
//! measured speedup against a real source is not claimed here (see the task note); the
//! amortisation is structural: [`run_batch`] calls [`BarSource::load`] once, and
//! [`run_singly`] calls it once per member, and a test counts the calls.

use std::sync::Arc;

use ledger::{DispatchContext, LedgerError, TrialLedger};

use crate::position_run::{ArtifactBar, Layer1Artifact, PositionRunExecutor};
use crate::position_series::{PositionSeries, TruncationVerdict};
use crate::run::{Backtest, InMemoryRunStore, ParamMap, RunDispatch, RunResult};

/// Where bars come from. On the platform: the point-in-time `BarStore` read.
pub trait BarSource {
    fn load(&self) -> Vec<ArtifactBar>;
}

/// One member of a batch: its parameters and the series those parameters produced.
#[derive(Clone, Debug)]
pub struct BatchMember {
    pub params: ParamMap,
    pub series: PositionSeries,
}

/// What every member shares: everything but the series and the parameters.
#[derive(Clone, Debug)]
pub struct BatchBase {
    pub code_sha256: String,
    pub instrument: String,
    pub timeframe: String,
    pub initial_equity: f64,
    pub fee_bps: f64,
    pub half_spread_bps: f64,
    pub cost_model_ref: String,
    pub data_snapshot: String,
    pub seed: u64,
}

fn artifact_for(base: &BatchBase, bars: &[ArtifactBar], m: &BatchMember) -> Layer1Artifact {
    Layer1Artifact {
        code_sha256: base.code_sha256.clone(),
        instrument: base.instrument.clone(),
        timeframe: base.timeframe.clone(),
        series: m.series.clone(),
        bars: bars.to_vec(),
        initial_equity: base.initial_equity,
        fee_bps: base.fee_bps,
        half_spread_bps: base.half_spread_bps,
        cost_model_ref: base.cost_model_ref.clone(),
        data_snapshot: base.data_snapshot.clone(),
        lot_size: None,
        impact: None,
        params: m.params.clone(),
        seed: base.seed,
    }
}

fn run_one<L: TrialLedger + Clone>(
    ledger: &L,
    ctx: &DispatchContext,
    artifact: Layer1Artifact,
    verdict: &TruncationVerdict,
) -> Result<RunResult, LedgerError> {
    let a = Arc::new(artifact);
    let cfg = a.run_config().map_err(|e| LedgerError::Backend(e.to_string()))?;
    let bt = Backtest::new(InMemoryRunStore::new(), PositionRunExecutor::new(a, Some(verdict.clone())), ledger.clone());
    bt.run_traced(&RunDispatch::new(ctx, &cfg, 1.0)).map(|(r, _)| r)
}

/// Run every member over **one** load of the bars.
///
/// # Errors
/// The first ledger refusal; members before it have already been registered and run.
pub fn run_batch<L: TrialLedger + Clone, S: BarSource>(
    ledger: &L,
    ctx: &DispatchContext,
    source: &S,
    base: &BatchBase,
    members: &[BatchMember],
    verdict: &TruncationVerdict,
) -> Result<Vec<RunResult>, LedgerError> {
    let bars = source.load();
    members.iter().map(|m| run_one(ledger, ctx, artifact_for(base, &bars, m), verdict)).collect()
}

/// The same members as N independent runs, each loading its own bars: the baseline a batch is
/// measured against, and what a member's result must equal.
///
/// # Errors
/// The first ledger refusal.
pub fn run_singly<L: TrialLedger + Clone, S: BarSource>(
    ledger: &L,
    ctx: &DispatchContext,
    source: &S,
    base: &BatchBase,
    members: &[BatchMember],
    verdict: &TruncationVerdict,
) -> Result<Vec<RunResult>, LedgerError> {
    members
        .iter()
        .map(|m| run_one(ledger, ctx, artifact_for(base, &source.load(), m), verdict))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_series::{PositionPoint, PositionUnits};
    use ledger::InMemoryLedger;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Source {
        loads: AtomicUsize,
    }
    impl BarSource for Source {
        fn load(&self) -> Vec<ArtifactBar> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            (0..120)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let p = 100.0 + (f64::from(i) * 0.25).sin() * 5.0;
                    ArtifactBar { ts_ns: (i64::from(i) + 1) * 3_600_000_000_000, open: p, close: p + 0.1 }
                })
                .collect()
        }
    }

    fn base() -> BatchBase {
        BatchBase {
            code_sha256: "ab".repeat(32),
            instrument: "BTC-USD".into(),
            timeframe: "1h".into(),
            initial_equity: 10_000.0,
            fee_bps: 10.0,
            half_spread_bps: 2.0,
            cost_model_ref: "cost:flat".into(),
            data_snapshot: "s".into(),
            seed: 1,
        }
    }

    fn members(n: usize, bars: &[ArtifactBar]) -> Vec<BatchMember> {
        (0..n)
            .map(|k| {
                let mut params = ParamMap::new();
                params.insert("period".into(), serde_json::json!(k));
                let points = bars
                    .iter()
                    .enumerate()
                    .map(|(i, b)| PositionPoint::new(b.ts_ns, if (i + k) % (6 + k % 5) < 3 { 1.0 } else { 0.0 }))
                    .collect();
                BatchMember { params, series: PositionSeries { points, position_units: PositionUnits::Fraction, allow_short: false } }
            })
            .collect()
    }

    fn setup(n: usize) -> (Source, Vec<BatchMember>) {
        let source = Source { loads: AtomicUsize::new(0) };
        let ms = members(n, &source.load());
        source.loads.store(0, Ordering::SeqCst);
        (source, ms)
    }

    fn ctx() -> DispatchContext {
        DispatchContext::human("tenant", "m-3.1", 0.1).with_experiment("exp")
    }

    fn pass() -> TruncationVerdict {
        TruncationVerdict::Pass { cuts_checked: 8 }
    }

    #[test]
    fn a_batch_loads_the_bars_once_and_singles_load_them_once_per_member() {
        let (source, ms) = setup(40);
        run_batch(&std::sync::Arc::new(InMemoryLedger::new()), &ctx(), &source, &base(), &ms, &pass()).unwrap();
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
        source.loads.store(0, Ordering::SeqCst);
        run_singly(&std::sync::Arc::new(InMemoryLedger::new()), &ctx(), &source, &base(), &ms, &pass()).unwrap();
        assert_eq!(source.loads.load(Ordering::SeqCst), 40);
    }

    #[test]
    fn member_results_equal_their_single_run_results_bit_for_bit() {
        let (source, ms) = setup(25);
        let batch = run_batch(&std::sync::Arc::new(InMemoryLedger::new()), &ctx(), &source, &base(), &ms, &pass()).unwrap();
        let single = run_singly(&std::sync::Arc::new(InMemoryLedger::new()), &ctx(), &source, &base(), &ms, &pass()).unwrap();
        assert_eq!(batch.len(), single.len());
        for (b, s) in batch.iter().zip(&single) {
            assert_eq!(b.run_id, s.run_id);
            let bits = |r: &RunResult| r.equity_curve.iter().map(|(t, e)| (*t, e.to_bits())).collect::<Vec<_>>();
            assert_eq!(bits(b), bits(s));
            assert_eq!(b.trades, s.trades);
        }
    }

    #[test]
    fn every_member_is_its_own_run_id_and_its_own_counted_trial() {
        let (source, ms) = setup(30);
        let ledger = std::sync::Arc::new(InMemoryLedger::new());
        let out = run_batch(&ledger, &ctx(), &source, &base(), &ms, &pass()).unwrap();
        let ids: std::collections::BTreeSet<_> = out.iter().map(|r| r.run_id.as_str().to_string()).collect();
        assert_eq!(ids.len(), 30, "30 members, 30 distinct configs");
        assert_eq!(ledger.trial_count("tenant"), 30, "a batch of 30 adds exactly 30 trials (INV-1)");
    }
}
