//! `RunKind::PositionSeries`: a Layer 1 run as a `RunExecutor` (Set M M-1.5; FEAT-005 §4.2).
//!
//! [`crate::position_series`] holds the pure parts (the series, the truncation test, the
//! evaluator). This is the half that makes one reachable from the platform's run path:
//!
//! * [`Layer1Artifact`] is what the `layer1_run` job stores as a `position_series`
//!   artifact: the validated target series **and the bars it was decided on**, the
//!   execution policy and the code's hash. It has no P&L field and `deny_unknown_fields`
//!   (ST-02), so an artifact a client edited to carry one does not load.
//! * [`PositionRunExecutor`] is a [`RunExecutor`]: given a [`RunConfig`] and a
//!   [`TrialTicket`] it evaluates the artifact and returns a [`RunResult`] with trades,
//!   an equity curve, per-bar exposure and the standard metrics. It cannot be reached
//!   without a ticket, so a Layer 1 run is a *counted* trial registered in the ledger
//!   before it computes (INV-16), exactly like every other run.
//! * Gate 0's truncation flag is applied **inside** the executor
//!   ([`gates::position_leakage::reject`](crate::gates::position_leakage::reject)), so a
//!   result that came back without its verdict is rejected rather than reported as ok.
//!
//! # `strategy_version`
//!
//! `layer1.<code sha256, 16 hex>.<artifact sha256, 16 hex>`: the code snapshot and the
//! series it produced are both in the run's identity, so two runs of the same file on
//! different data (or a changed file giving the same series) are distinguishable.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use ledger::TrialTicket;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::gates::position_leakage;
use crate::position_series::{
    evaluate_with_impact, Bar, Costs, EvalConfig, EvalError, Fill, PositionRun, PositionSeries,
    SqrtImpact, TruncationVerdict,
};
use crate::run::executor::map_sim_result;
use crate::run::{
    ComputeCost, DataSlice, EvalResolution, ParamMap, RunConfig, RunConfigBuilder, RunExecutor,
    RunResult, RunStatus, Side, Trade, ENGINE_VERSION,
};

/// The `strategy_ref` prefix of a Layer 1 run.
pub const STRATEGY_REF_PREFIX: &str = "layer1:";
/// The sizing reference of a Layer 1 run: the series carries its own sizes.
pub const SIZING_REF: &str = "sizing:position_series";

/// A bar as the artifact stores it: the close stamp, the open and the close.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBar {
    pub ts_ns: i64,
    pub open: f64,
    pub close: f64,
}

impl From<ArtifactBar> for Bar {
    fn from(b: ArtifactBar) -> Self {
        Self { ts_ns: b.ts_ns, open: b.open, close: b.close }
    }
}

/// What the `layer1_run` job stores. Content-addressed by [`Self::sha256`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer1Artifact {
    /// sha256 (hex) of the source file the series was produced by.
    pub code_sha256: String,
    pub instrument: String,
    pub timeframe: String,
    pub series: PositionSeries,
    pub bars: Vec<ArtifactBar>,
    pub initial_equity: f64,
    pub fee_bps: f64,
    pub half_spread_bps: f64,
    /// What `RunConfig::cost_model_ref` must say for this artifact: the model it was priced under.
    pub cost_model_ref: String,
    /// What `RunConfig::data_snapshot` must say: the point-in-time horizon the bars were read at.
    pub data_snapshot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lot_size: Option<f64>,
    /// Square-root impact charged on every fill, when a cost model supplied one (M-2.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact: Option<SqrtImpact>,
    /// The parameter values the function ran with.
    pub params: ParamMap,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ArtifactError {
    #[error("the artifact does not parse: {0}")]
    Parse(String),
    #[error("timeframe {0:?} has no RunConfig resolution; Layer 1 supports 1m, 5m, 10m, 15m, 30m, 1h and 1d")]
    Timeframe(String),
    #[error("the artifact has no bars")]
    NoBars,
}

impl Layer1Artifact {
    /// The canonical bytes: what is stored and what the hash is over.
    ///
    /// # Panics
    /// Never: the type is plain data.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a Layer1Artifact is plain data")
    }

    /// Parse stored bytes. A `pnl` or `equity` field is an error (ST-02).
    ///
    /// # Errors
    /// Bytes that are not an artifact, or one carrying a field it has no place for.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArtifactError> {
        serde_json::from_slice(bytes).map_err(|e| ArtifactError::Parse(e.to_string()))
    }

    #[must_use]
    pub fn sha256(&self) -> String {
        hex::encode(Sha256::digest(self.to_bytes()))
    }

    /// `layer1.<code 16>.<artifact 16>`.
    #[must_use]
    pub fn strategy_version(&self) -> String {
        format!(
            "layer1.{}.{}",
            &self.code_sha256[..16.min(self.code_sha256.len())],
            &self.sha256()[..16]
        )
    }

    /// The resolution a timeframe key maps to.
    ///
    /// # Errors
    /// A timeframe `RunConfig` cannot express.
    pub fn resolution(&self) -> Result<EvalResolution, ArtifactError> {
        resolution_of(&self.timeframe)
    }

    /// The config this artifact is run under. The slice is the artifact's bar span.
    ///
    /// # Errors
    /// An unsupported timeframe or an artifact with no bars.
    pub fn run_config(&self) -> Result<RunConfig, ArtifactError> {
        let first = self.bars.first().ok_or(ArtifactError::NoBars)?;
        let last = self.bars.last().ok_or(ArtifactError::NoBars)?;
        let at = |ns: i64| Utc.timestamp_nanos(ns);
        let slice = DataSlice::new(self.instrument.clone(), at(first.ts_ns), at(last.ts_ns), self.resolution()?);
        Ok(RunConfigBuilder::new(
            format!("{STRATEGY_REF_PREFIX}{}", self.instrument),
            self.strategy_version(),
            slice,
            self.cost_model_ref.clone(),
            SIZING_REF,
            self.data_snapshot.clone(),
        )
        .params(self.params.clone())
        .seed(self.seed)
        .build())
    }
}

/// `1h` to [`EvalResolution`].
///
/// # Errors
/// A timeframe with no resolution.
pub fn resolution_of(timeframe: &str) -> Result<EvalResolution, ArtifactError> {
    Ok(match timeframe {
        "1m" => EvalResolution::Min1,
        "5m" => EvalResolution::Min5,
        "10m" => EvalResolution::Min10,
        "15m" => EvalResolution::Min15,
        "30m" => EvalResolution::Min30,
        "1h" => EvalResolution::Hour1,
        "1d" => EvalResolution::Day1,
        other => return Err(ArtifactError::Timeframe(other.to_string())),
    })
}

/// Whether a config belongs to this executor.
#[must_use]
pub fn is_layer1(cfg: &RunConfig) -> bool {
    cfg.strategy_ref.starts_with(STRATEGY_REF_PREFIX)
}

fn decimal(x: f64) -> Decimal {
    Decimal::from_f64(x).unwrap_or(Decimal::ZERO)
}

fn at(ns: i64) -> DateTime<Utc> {
    Utc.timestamp_nanos(ns)
}

/// A trade's costs by leg (BT-10: costs on a trade equal the sum of its legs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TradeLegs {
    pub fee: f64,
    pub spread: f64,
    pub impact: f64,
}

impl TradeLegs {
    #[must_use]
    pub fn total(&self) -> f64 {
        self.fee + self.spread + self.impact
    }
    fn scaled(self, k: f64) -> Self {
        Self { fee: self.fee * k, spread: self.spread * k, impact: self.impact * k }
    }
    fn plus(self, o: Self) -> Self {
        Self { fee: self.fee + o.fee, spread: self.spread + o.spread, impact: self.impact + o.impact }
    }
    fn minus(self, o: Self) -> Self {
        Self { fee: self.fee - o.fee, spread: self.spread - o.spread, impact: self.impact - o.impact }
    }
}

/// Round trips out of a fill list.
///
/// A trade opens when the position leaves flat and closes when it returns to flat; a fill
/// that flips the position closes the old trade at that price and opens the new one with
/// the remainder. Costs are attributed to the trade the fill belongs to (a flipping fill's
/// cost is split by quantity). MAE and MFE are measured on bar closes between entry and exit.
#[must_use]
pub fn trades_from_fills(symbol: &str, fills: &[Fill], bars: &[Bar]) -> Vec<Trade> {
    trades_and_legs(symbol, fills, bars).into_iter().map(|(t, _)| t).collect()
}

/// [`trades_from_fills`] with each trade's cost legs beside it.
#[must_use]
#[allow(clippy::float_cmp)] // exact: side is +/-1.0 and signum() returns +/-1.0
pub fn trades_and_legs(symbol: &str, fills: &[Fill], bars: &[Bar]) -> Vec<(Trade, TradeLegs)> {
    struct Open {
        side: f64,
        qty: f64,
        avg: f64,
        entry_ns: i64,
        legs: TradeLegs,
    }
    let mut trades = Vec::new();
    let mut cur: Option<Open> = None;
    let excursions = |entry_ns: i64, exit_ns: i64, entry_px: f64, side: f64| {
        let (mut mae, mut mfe) = (0.0_f64, 0.0_f64);
        for b in bars.iter().filter(|b| b.ts_ns >= entry_ns && b.ts_ns <= exit_ns) {
            let r = side * (b.close / entry_px - 1.0);
            mae = mae.min(r);
            mfe = mfe.max(r);
        }
        (mae, mfe)
    };
    let close = |o: &Open, qty_closed: f64, exit_px: f64, exit_ns: i64, extra: TradeLegs| -> (Trade, TradeLegs) {
        let legs = o.legs.scaled(qty_closed / o.qty).plus(extra);
        let gross = o.side * (exit_px - o.avg) * qty_closed;
        let (mae, mfe) = excursions(o.entry_ns, exit_ns, o.avg, o.side);
        (
            Trade {
                symbol: symbol.to_string(),
                side: if o.side > 0.0 { Side::Long } else { Side::Short },
                entry_time: at(o.entry_ns),
                exit_time: at(exit_ns),
                entry_price: decimal(o.avg),
                exit_price: decimal(exit_px),
                qty: decimal(qty_closed),
                mae,
                mfe,
                holding_period_secs: (exit_ns - o.entry_ns) / 1_000_000_000,
                costs_paid: decimal(legs.total()),
                // The spread and impact are already in the fill prices, so only the fee comes off the gross.
                pnl: decimal(gross - legs.fee),
            },
            legs,
        )
    };
    for f in fills {
        let mut remaining = f.qty;
        let mut left_legs = TradeLegs { fee: f.fee, spread: f.spread_cost, impact: f.impact_cost };
        while remaining != 0.0 {
            match cur.take() {
                None => {
                    cur = Some(Open {
                        side: remaining.signum(),
                        qty: remaining.abs(),
                        avg: f.price,
                        entry_ns: f.filled_bar_ns,
                        legs: left_legs,
                    });
                    remaining = 0.0;
                }
                Some(mut o) if remaining.signum() == o.side => {
                    let add = remaining.abs();
                    o.avg = (o.avg * o.qty + f.price * add) / (o.qty + add);
                    o.qty += add;
                    o.legs = o.legs.plus(left_legs);
                    cur = Some(o);
                    remaining = 0.0;
                }
                Some(o) => {
                    let reduce = remaining.abs().min(o.qty);
                    let share = left_legs.scaled(reduce / remaining.abs());
                    trades.push(close(&o, reduce, f.price, f.filled_bar_ns, share));
                    left_legs = left_legs.minus(share);
                    let left = o.qty - reduce;
                    remaining += o.side * reduce; // moves toward zero
                    if left > 1e-12 {
                        cur = Some(Open { qty: left, legs: o.legs.scaled(left / o.qty), ..o });
                    } else {
                        remaining = if remaining.abs() < 1e-12 { 0.0 } else { remaining };
                    }
                }
            }
        }
    }
    trades
}

/// Net exposure per bar (position value over equity), from the fills and the marked equity.
#[must_use]
pub fn exposures(fills: &[Fill], bars: &[Bar], equity: &[(i64, f64)]) -> Vec<(i64, f64)> {
    let mut qty = 0.0;
    let mut fi = 0;
    bars.iter()
        .zip(equity)
        .map(|(b, (_, eq))| {
            while fi < fills.len() && fills[fi].filled_bar_ns <= b.ts_ns {
                qty += fills[fi].qty;
                fi += 1;
            }
            (b.ts_ns, if *eq > 0.0 { qty * b.close / eq } else { 0.0 })
        })
        .collect()
}

/// Evaluates a Layer 1 artifact under a ticket.
pub struct PositionRunExecutor {
    artifact: Arc<Layer1Artifact>,
    truncation: Option<TruncationVerdict>,
}

impl PositionRunExecutor {
    /// `truncation` is the verdict the job produced. `None` is *not* a pass: the result is
    /// rejected with `leakage.position_truncation_not_run`.
    #[must_use]
    pub fn new(artifact: Arc<Layer1Artifact>, truncation: Option<TruncationVerdict>) -> Self {
        Self { artifact, truncation }
    }

    fn failed(cfg: &RunConfig, why: &EvalError) -> RunResult {
        let mut r = map_sim_result(cfg, Vec::new(), Vec::new(), Vec::new(), ComputeCost::default(), ENGINE_VERSION);
        r.status = RunStatus::Failed(ledger::TerminalReason::DataError);
        r.integrity_flags.push(crate::run::Flag {
            code: "position_series.invalid".into(),
            detail: why.to_string(),
        });
        r
    }

    /// The evaluation, separated from the config plumbing so a test can read the numbers.
    ///
    /// # Errors
    /// An invalid series or bars.
    pub fn evaluate(&self) -> Result<PositionRun, EvalError> {
        self.evaluate_scaled(0)
    }

    /// As [`Self::evaluate`] with every cost leg scaled by `cost_scale_bp` hundredths of a
    /// percent of itself (`+2000` is +20 %): what a cost-perturbation replicate varies.
    ///
    /// # Errors
    /// An invalid series or bars.
    pub fn evaluate_scaled(&self, cost_scale_bp: i32) -> Result<PositionRun, EvalError> {
        let a = &self.artifact;
        let k = 1.0 + f64::from(cost_scale_bp) / 10_000.0;
        let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
        evaluate_with_impact(
            &a.series,
            &bars,
            &EvalConfig {
                initial_equity: a.initial_equity,
                costs: Costs { fee_bps: a.fee_bps * k, half_spread_bps: a.half_spread_bps * k },
                lot_size: a.lot_size,
            },
            a.impact.map(|i| SqrtImpact { y: i.y * k, ..i }),
        )
    }
}

impl PositionRunExecutor {
    /// Every `RunConfig` field either changes this run or **refuses it by name** (BT-05): an
    /// executor that ignores a field produces a result whose hash differs from its twin's while
    /// the numbers do not, which is the defect FEAT-005 §2 found. The table is
    /// `tests/runconfig_fields_are_honoured.rs`.
    ///
    /// # Errors
    /// The first field of `cfg` that disagrees with the artifact or asks for something a
    /// position-series run cannot do, named.
    pub fn check_config(&self, cfg: &RunConfig) -> Result<(), (&'static str, String)> {
        let a = &self.artifact;
        let want = a.strategy_version();
        if cfg.strategy_version != want {
            return Err(("strategy_version", format!("version_mismatch: the config says {} and the artifact is {want}", cfg.strategy_version)));
        }
        if cfg.params != a.params {
            return Err(("params", "the config's params are not the ones the series was produced with".into()));
        }
        if cfg.seed != a.seed {
            return Err(("seed", format!("the config's seed {} is not the series' {}", cfg.seed, a.seed)));
        }
        if cfg.cost_model_ref != a.cost_model_ref {
            return Err(("cost_model_ref", format!("{} is not the model the artifact was priced under ({})", cfg.cost_model_ref, a.cost_model_ref)));
        }
        if cfg.data_snapshot != a.data_snapshot {
            return Err(("data_snapshot", format!("{} is not the horizon the bars were read at ({})", cfg.data_snapshot, a.data_snapshot)));
        }
        if cfg.sizing_ref != SIZING_REF {
            return Err(("sizing_ref", format!("a position series carries its own sizes; {} is not {SIZING_REF}", cfg.sizing_ref)));
        }
        if cfg.fill_model != crate::run::FillModel::NextBarOpen {
            return Err(("fill_model", "a position series fills at the next bar's open and no other model is implemented".into()));
        }
        if cfg.fill_delay_bars != 0 {
            return Err(("fill_delay_bars", "extra fill delay is not implemented for position series; refusing rather than ignoring it".into()));
        }
        if cfg.null_world.is_some() || cfg.target.is_some() {
            return Err(("null_world/target", "a position-series run has no null world and no deployment target".into()));
        }
        let expected = a.run_config().map_err(|e| ("data_slice", e.to_string()))?;
        if cfg.data_slice != expected.data_slice {
            return Err(("data_slice", "the config's slice is not the span of the bars the series was decided on".into()));
        }
        Ok(())
    }
}

impl RunExecutor for PositionRunExecutor {
    fn execute(&self, cfg: &RunConfig, _ticket: &TrialTicket) -> RunResult {
        let started = std::time::Instant::now();
        let a = &self.artifact;
        if let Err((field, why)) = self.check_config(cfg) {
            let mut r = map_sim_result(cfg, Vec::new(), Vec::new(), Vec::new(), ComputeCost::default(), ENGINE_VERSION);
            r.status = RunStatus::Failed(ledger::TerminalReason::IntegrityRejected);
            r.integrity_flags.push(crate::run::Flag { code: format!("layer1.config.{}", field.replace('/', "_")), detail: why });
            return r;
        }
        let run = match self.evaluate_scaled(cfg.cost_scale_bp) {
            Ok(r) => r,
            Err(e) => return Self::failed(cfg, &e),
        };
        let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
        let equity = run.equity.iter().map(|(ts, e)| (at(*ts), *e)).collect();
        let net_exposure = exposures(&run.fills, &bars, &run.equity)
            .into_iter()
            .map(|(ts, x)| (at(ts), x))
            .collect();
        let trades = trades_from_fills(&a.instrument, &run.fills, &bars);
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut result = map_sim_result(
            cfg,
            equity,
            net_exposure,
            trades,
            ComputeCost { wall_ms: elapsed, cpu_ms: elapsed },
            ENGINE_VERSION,
        );
        if run.ruined_at_ns.is_some() {
            result.integrity_flags.push(crate::run::Flag {
                code: "position_series.ruin".into(),
                detail: format!("equity reached zero at {}; trading stopped there", at(run.ruined_at_ns.unwrap_or(0)).to_rfc3339()),
            });
        }
        // Gate 0 (M-1.7): a missing or failing truncation verdict rejects the run.
        position_leakage::reject(&mut result, self.truncation.as_ref());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_series::{PositionPoint, PositionUnits, TruncationFailure};
    use crate::run::{Backtest, InMemoryRunStore};
    use ledger::{DispatchContext, InMemoryLedger};

    fn artifact(n: usize, target: impl Fn(usize) -> f64) -> Layer1Artifact {
        let bars: Vec<ArtifactBar> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let p = 100.0 + (i as f64 * 0.31).sin() * 4.0 + i as f64 * 0.02;
                ArtifactBar { ts_ns: (i64::try_from(i).unwrap() + 1) * 3_600_000_000_000, open: p, close: p * 1.001 }
            })
            .collect();
        let points = bars.iter().enumerate().map(|(i, b)| PositionPoint::new(b.ts_ns, target(i))).collect();
        Layer1Artifact {
            code_sha256: "ab".repeat(32),
            instrument: "BTC-USD".into(),
            timeframe: "1h".into(),
            series: PositionSeries { points, position_units: PositionUnits::Fraction, allow_short: true },
            bars,
            initial_equity: 10_000.0,
            fee_bps: 10.0,
            half_spread_bps: 2.0,
            cost_model_ref: "cost:flat".into(),
            data_snapshot: "snap-1".into(),
            lot_size: None,
            impact: None,
            params: ParamMap::new(),
            seed: 7,
        }
    }

    fn run_with(a: Layer1Artifact, truncation: Option<TruncationVerdict>) -> RunResult {
        let a = Arc::new(a);
        let cfg = a.run_config().unwrap();
        let bt = Backtest::new(InMemoryRunStore::new(), PositionRunExecutor::new(a, truncation), InMemoryLedger::new());
        let ctx = DispatchContext::human("tenant-test", "layer1", 0.1).with_experiment("exp-l1");
        let (result, _) = bt.run_traced(&crate::run::RunDispatch::new(&ctx, &cfg, 1.0)).unwrap();
        result
    }

    fn pass() -> TruncationVerdict {
        TruncationVerdict::Pass { cuts_checked: 8 }
    }

    #[test]
    fn a_layer1_run_is_a_registered_trial_that_produces_trades_equity_and_metrics() {
        let a = artifact(300, |i| if (i / 20) % 2 == 0 { 1.0 } else { -1.0 });
        let r = run_with(a, Some(pass()));
        assert_eq!(r.status, RunStatus::Ok, "{:?}", r.integrity_flags);
        assert_eq!(r.equity_curve.len(), 300);
        assert!(!r.trades.is_empty(), "a series that changes sign trades");
        assert_eq!(r.net_exposure.len(), 300);
    }

    #[test]
    fn a_run_whose_truncation_verdict_is_missing_or_failed_is_rejected_not_ok() {
        let missing = run_with(artifact(120, |_| 1.0), None);
        assert_eq!(missing.status, RunStatus::RejectedIntegrity);
        assert_eq!(missing.integrity_flags.last().unwrap().code, position_leakage::NOT_RUN_CODE);

        let failed = TruncationVerdict::Fail(TruncationFailure {
            cut_ns: 50 * 3_600_000_000_000,
            first_differing_ns: 40 * 3_600_000_000_000,
            full: Some(1.0),
            truncated: Some(0.0),
        });
        let r = run_with(artifact(120, |_| 1.0), Some(failed));
        assert_eq!(r.status, RunStatus::RejectedIntegrity);
        let flag = r.integrity_flags.last().unwrap();
        assert_eq!(flag.code, position_leakage::CODE);
        assert!(flag.detail.contains("1970-01-02T16:00:00"), "{}", flag.detail);
    }

    #[test]
    fn the_same_artifact_gives_the_same_run_id_and_the_same_numbers() {
        let a = artifact(200, |i| if i % 40 < 20 { 0.5 } else { 0.0 });
        let x = run_with(a.clone(), Some(pass()));
        let y = run_with(a, Some(pass()));
        assert_eq!(x.run_id, y.run_id);
        assert_eq!(x.equity_curve, y.equity_curve);
    }

    #[test]
    fn the_strategy_version_carries_the_code_hash_and_the_series_hash() {
        let a = artifact(60, |_| 1.0);
        let v = a.strategy_version();
        assert!(v.starts_with("layer1.abababababababab."), "{v}");
        let b = artifact(60, |i| if i == 10 { 0.0 } else { 1.0 });
        assert_ne!(a.strategy_version(), b.strategy_version(), "a different series is a different version");
        let mut c = artifact(60, |_| 1.0);
        c.code_sha256 = "cd".repeat(32);
        assert_ne!(a.strategy_version(), c.strategy_version(), "and so is different code");
    }

    #[test]
    fn an_artifact_carrying_a_pnl_field_does_not_load() {
        let a = artifact(30, |_| 1.0);
        let mut v: serde_json::Value = serde_json::from_slice(&a.to_bytes()).unwrap();
        v["pnl"] = serde_json::json!(123.0);
        assert!(matches!(Layer1Artifact::from_bytes(&serde_json::to_vec(&v).unwrap()), Err(ArtifactError::Parse(_))));
        let mut nested: serde_json::Value = serde_json::from_slice(&a.to_bytes()).unwrap();
        nested["series"]["equity"] = serde_json::json!([1.0]);
        assert!(Layer1Artifact::from_bytes(&serde_json::to_vec(&nested).unwrap()).is_err());
        assert_eq!(Layer1Artifact::from_bytes(&a.to_bytes()).unwrap(), a);
    }

    #[test]
    fn a_timeframe_with_no_resolution_is_refused() {
        let mut a = artifact(30, |_| 1.0);
        a.timeframe = "4h".into();
        assert!(matches!(a.run_config(), Err(ArtifactError::Timeframe(_))));
    }

    #[test]
    fn a_long_then_flat_series_makes_one_trade_whose_costs_and_pnl_are_consistent() {
        let a = artifact(100, |i| if (10..30).contains(&i) { 1.0 } else { 0.0 });
        let a = Arc::new(a);
        let ex = PositionRunExecutor::new(a.clone(), Some(pass()));
        let run = ex.evaluate().unwrap();
        let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
        let trades = trades_from_fills("BTC-USD", &run.fills, &bars);
        assert_eq!(trades.len(), 1);
        let t = &trades[0];
        assert_eq!(t.side, Side::Long);
        let fees_and_spread = run.total_fees + run.total_spread_cost;
        let paid: f64 = rust_decimal::prelude::ToPrimitive::to_f64(&t.costs_paid).unwrap();
        assert!((paid - fees_and_spread).abs() < 1e-6, "{paid} vs {fees_and_spread}");
        let pnl: f64 = rust_decimal::prelude::ToPrimitive::to_f64(&t.pnl).unwrap();
        assert!((run.final_equity - (a.initial_equity + pnl)).abs() < 1e-6, "equity {} pnl {pnl}", run.final_equity);
    }

    #[test]
    fn a_flip_closes_one_trade_and_opens_the_next() {
        let a = Arc::new(artifact(100, |i| if i < 30 { 1.0 } else { -1.0 }));
        let run = PositionRunExecutor::new(a.clone(), Some(pass())).evaluate().unwrap();
        let bars: Vec<Bar> = a.bars.iter().copied().map(Bar::from).collect();
        let trades = trades_from_fills("X", &run.fills, &bars);
        assert_eq!(trades.len(), 1, "the short is still open at the end: {trades:?}");
        assert_eq!(trades[0].side, Side::Long);
    }
}
