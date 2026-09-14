//! **Real execution** — the `market_simulator`-backed [`RunExecutor`] (Set K's
//! live leg, wired for FEAT-003).
//!
//! Turns one [`RunConfig`] into one [`RunResult`]:
//!
//! 1. `strategy_ref` → the stored definition (`strategy_definitions`).
//! 2. `params` → [`domain::strategy_def::params::materialize`] (typed
//!    parameters become literals; the simulator sees frozen v1.0 grammar).
//! 3. `data_slice.eval_resolution` → bar timeframe; the strategy's declared
//!    lane may narrow it (`derive_requirements`).
//! 4. `data_slice.universe_ref` → instrument id; venue/precisions from the
//!    `instruments` table when present, sensible defaults otherwise.
//! 5. Bars from ClickHouse with indicator warm-up lead-in. **No auto-collect**:
//!    a Run is a pure function of stored data (ADR-001); a missing history is
//!    a `Failed` Run with a reason, never a side effect.
//! 6. `run_simulation_detailed` → [`map_detailed_result`].
//!
//! The [`RunExecutor`] contract is synchronous (Studies iterate members in a
//! plain loop), so the async data path is bridged with a runtime [`Handle`].
//! Callers run Studies inside `spawn_blocking`; `block_in_place` also makes it
//! safe from a multi-thread worker.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context};
use chrono::Duration;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::{params, StrategyDefinition};
use ledger::TerminalReason;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio::runtime::Handle;

use crate::requirements::derive_requirements;
use crate::run::executor::map_detailed_result;
use crate::run::{ComputeCost, EvalResolution, RunConfig, RunExecutor, RunResult};
use crate::sim::{
    run_simulation_detailed, DetailedOutcome, InstrumentPrecisions, SimulationInputs,
};
use crate::store::BarStore;
use crate::types::TimeframeExt;
use nautilus_backtest::sdk::SimulationControl;

/// `produced_by` tag on every result this executor emits.
pub const EXECUTOR_TAG: &str = "SimRunExecutor@market_simulator";

/// The real executor. Cheap to clone-by-reference; hold it in an `Arc`/`Box`.
pub struct SimRunExecutor {
    handle: Handle,
    pg: PgPool,
    ch_url: String,
    initial_balance: Decimal,
}

/// Pass the loaded bars through a null generator.
///
/// The generator works in `f64` — null-world prices are synthetic by
/// construction and are not money (`nulls::generators` says so at the top of the
/// file). The bar's four timestamps, quality flags and provenance are carried
/// through unchanged: what the null destroys is the price path, not the clock.
///
/// Bars whose generated OHLC is not internally consistent (`low ≤ open,close ≤
/// high`) are dropped rather than repaired. A repaired bar is a third thing that
/// is neither the real world nor the null, and a p-value computed against it
/// means nothing.
fn apply_null_world(bars: &[crate::store::LoadedBar], nw: &crate::run::NullWorld) -> Vec<crate::store::LoadedBar> {
    use crate::nulls::{Bar as NullBar, NullData};
    use rust_decimal::prelude::ToPrimitive;

    let f = |d: Decimal| d.to_f64().unwrap_or(0.0);
    let data = NullData {
        bars: bars
            .iter()
            .map(|b| NullBar {
                ts_ns: b.ts_ns,
                open: f(b.open),
                high: f(b.high),
                low: f(b.low),
                close: f(b.close),
            })
            .collect(),
        // The forward returns the signal-decoupling nulls permute.
        forward_returns: bars
            .windows(2)
            .map(|w| {
                let (a, b) = (f(w[0].close), f(w[1].close));
                if a == 0.0 { 0.0 } else { b / a - 1.0 }
            })
            .chain(std::iter::once(0.0))
            .collect(),
        ..NullData::default()
    };

    let generated = nw.null.generate(&data, nw.seed);
    let dec = |x: f64| rust_decimal::Decimal::from_f64_retain(x).unwrap_or_default();
    generated
        .bars
        .iter()
        .zip(bars)
        .filter(|(g, _)| g.is_valid())
        .map(|(g, src)| crate::store::LoadedBar {
            open: dec(g.open),
            high: dec(g.high),
            low: dec(g.low),
            close: dec(g.close),
            ..src.clone()
        })
        .collect()
}

/// Default venue per asset class (mirrors the backtest API's routing).
fn default_venue(asset_class: &str) -> &'static str {
    match asset_class {
        "equity" | "etf" => "alpaca",
        "fx" => "oanda",
        "futures_expiring" => "cme",
        "option" => "opra",
        "prediction_market" => "kalshi",
        _ => "coinbase",
    }
}

/// `BASE-QUOTE` → `QUOTE`; anything else assumes USD.
fn quote_currency(instrument_id: &str) -> String {
    instrument_id
        .rsplit_once('-')
        .map_or_else(|| "USD".to_string(), |(_, q)| q.to_uppercase())
}

/// The bar timeframe a config's eval resolution runs on.
fn timeframe_for(res: EvalResolution) -> anyhow::Result<Timeframe> {
    Ok(match res {
        EvalResolution::Min1 => Timeframe::Minutes1,
        EvalResolution::Min5 => Timeframe::Minutes5,
        EvalResolution::Min15 => Timeframe::Minutes15,
        EvalResolution::Hour1 => Timeframe::Hours1,
        EvalResolution::Day1 => Timeframe::Daily,
        other => anyhow::bail!(
            "eval resolution {other:?} has no stored bar timeframe; use 1m, 5m, 15m, 1h or 1d"
        ),
    })
}

/// An execution failure that already knows how the trial ended.
///
/// Every `?` inside `execute_async` names its [`TerminalReason`] at the point the
/// failure happens, where what went wrong is actually known. The alternative —
/// one `anyhow::Error` out of the whole function and a reason reconstructed from
/// its message downstream — is what this replaces (ADR-P2-30): "bar load failed"
/// and "requirements: unknown feature" are not the same censoring, and no amount
/// of substring matching makes that judgement reliably.
struct ExecFailure {
    reason: TerminalReason,
    error: anyhow::Error,
}

impl ExecFailure {
    fn new(reason: TerminalReason, error: impl Into<anyhow::Error>) -> Self {
        Self { reason, error: error.into() }
    }
}

/// Attaches the terminal reason to a fallible step.
trait Because<T> {
    fn because(self, reason: TerminalReason) -> Result<T, ExecFailure>;
}

impl<T, E: Into<anyhow::Error>> Because<T> for Result<T, E> {
    fn because(self, reason: TerminalReason) -> Result<T, ExecFailure> {
        self.map_err(|e| ExecFailure::new(reason, e))
    }
}

impl SimRunExecutor {
    /// Build over the platform's runtime, Postgres pool and ClickHouse URL.
    #[must_use]
    pub fn new(handle: Handle, pg: PgPool, ch_url: impl Into<String>) -> Self {
        Self {
            handle,
            pg,
            ch_url: ch_url.into(),
            initial_balance: Decimal::from(10_000),
        }
    }

    /// Override the starting balance every Run is funded with.
    #[must_use]
    pub fn with_initial_balance(mut self, balance: Decimal) -> Self {
        self.initial_balance = balance;
        self
    }

    async fn resolve_definition(&self, slug: &str) -> anyhow::Result<StrategyDefinition> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
        )
        .bind(slug)
        .fetch_optional(&self.pg)
        .await
        .context("strategy lookup failed")?;
        let (json,) = row.ok_or_else(|| anyhow!("strategy '{slug}' not found"))?;
        serde_json::from_value(json)
            .with_context(|| format!("strategy '{slug}' is not a valid definition"))
    }

    /// Venue id and precisions from instrument metadata (defaults if unknown).
    async fn instrument_meta(
        &self,
        instrument_id: &str,
        asset_class: &str,
    ) -> (String, Option<InstrumentPrecisions>) {
        match storage::postgres::instruments::fetch_by_id(&self.pg, instrument_id).await {
            Ok(Some(inst)) => {
                let precisions = if inst.tick_size.is_zero() || inst.lot_size.is_zero() {
                    None
                } else {
                    let scale =
                        |d: Decimal| u8::try_from(d.normalize().scale()).unwrap_or(9).min(9);
                    Some(InstrumentPrecisions {
                        price: scale(inst.tick_size),
                        size: scale(inst.lot_size),
                    })
                };
                (inst.venue_id, precisions)
            }
            Ok(None) => (default_venue(asset_class).to_string(), None),
            Err(e) => {
                tracing::warn!(instrument_id, error = %e, "instrument lookup failed; using defaults");
                (default_venue(asset_class).to_string(), None)
            }
        }
    }

    async fn execute_async(&self, cfg: &RunConfig) -> Result<DetailedOutcome, ExecFailure> {
        let definition = self.resolve_definition(&cfg.strategy_ref).await.because(TerminalReason::DependencyFailure)?;
        let definition = params::materialize(&definition, &cfg.params)
            .map_err(|e| anyhow!("parameters: {e}"))
            .because(TerminalReason::IntegrityRejected)?;

        let requested = timeframe_for(cfg.data_slice.eval_resolution).because(TerminalReason::IntegrityRejected)?;
        let requirements = derive_requirements(&definition, requested)
            .map_err(|e| anyhow!("requirements: {e}"))
            .because(TerminalReason::IntegrityRejected)?;
        let timeframe = requirements.timeframe;
        let warmup_secs =
            i64::try_from(requirements.warmup_bars * timeframe.seconds()).unwrap_or(i64::MAX / 4);
        let start = cfg.data_slice.start;
        let end = cfg.data_slice.end;
        let data_from = start - Duration::seconds(warmup_secs);

        let instrument_id = cfg.data_slice.universe_ref.clone();
        let asset_class = definition.asset_class.clone();
        let (venue_id, precisions) = self.instrument_meta(&instrument_id, &asset_class).await;

        let store = BarStore::connect(&self.ch_url);
        let bars = store
            .load_bars(&instrument_id, timeframe, data_from, end)
            .await
            .context("bar load failed")
            .because(TerminalReason::DataError)?;
        if bars.is_empty() {
            return Err(ExecFailure::new(
                TerminalReason::DataError,
                anyhow!(
                    "no {} bars stored for {instrument_id} in [{}, {}) — initialise the asset                      or run a backtest with auto-collect first (Runs never collect)",
                    timeframe.key(),
                    data_from.format("%Y-%m-%d"),
                    end.format("%Y-%m-%d"),
                ),
            ));
        }

        // A null-world Run sees generated bars, not the stored ones (spec §2.1).
        // The transformation happens here, at the single point where a Run's data
        // is assembled, so no execution path can accidentally run a null-world
        // config against real data.
        let bars = match &cfg.null_world {
            None => bars,
            Some(nw) => apply_null_world(&bars, nw),
        };

        let inputs = SimulationInputs {
            definition,
            instrument_id: instrument_id.clone(),
            venue_id,
            asset_class,
            timeframe,
            quote_currency: quote_currency(&instrument_id),
            initial_balance: self.initial_balance,
            precisions,
            sim_start_ns: start.timestamp_nanos_opt().unwrap_or(0),
            bars,
            features: requirements.features,
        };
        let control: Arc<SimulationControl> = SimulationControl::new();
        run_simulation_detailed(inputs, &control).because(TerminalReason::DependencyFailure)
    }
}

impl RunExecutor for SimRunExecutor {
    fn execute(&self, cfg: &RunConfig, ticket: &ledger::TrialTicket) -> RunResult {
        let started = Instant::now();
        tracing::debug!(
            trial_id = %ticket.trial_id(),
            run_id = cfg.run_id.as_str(),
            "dispatching registered trial"
        );
        let outcome = tokio::task::block_in_place(|| self.handle.block_on(self.execute_async(cfg)));
        let cost = ComputeCost {
            wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            cpu_ms: 0,
        };
        match outcome {
            Ok(o) if o.cancelled => {
                RunResult::failed(cfg, TerminalReason::Cancelled, "simulation cancelled", EXECUTOR_TAG)
            }
            Ok(o) => map_detailed_result(cfg, o, cost, EXECUTOR_TAG),
            Err(f) => {
                let detail = format!("{:#}", f.error);
                tracing::warn!(run_id = cfg.run_id.as_str(), error = %detail, reason = f.reason.as_str(), "run failed");
                RunResult::failed(cfg, f.reason, detail, EXECUTOR_TAG)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_resolution_maps_to_stored_timeframes() {
        assert_eq!(
            timeframe_for(EvalResolution::Min1).unwrap(),
            Timeframe::Minutes1
        );
        assert_eq!(
            timeframe_for(EvalResolution::Hour1).unwrap(),
            Timeframe::Hours1
        );
        assert_eq!(
            timeframe_for(EvalResolution::Day1).unwrap(),
            Timeframe::Daily
        );
        assert!(timeframe_for(EvalResolution::Min10).is_err());
        assert!(timeframe_for(EvalResolution::Min30).is_err());
    }

    #[test]
    fn quote_and_venue_defaults() {
        assert_eq!(quote_currency("BTC-USD"), "USD");
        assert_eq!(quote_currency("eth-usdt"), "USDT");
        assert_eq!(quote_currency("AAPL"), "USD");
        assert_eq!(default_venue("equity"), "alpaca");
        assert_eq!(default_venue("crypto_spot_cex"), "coinbase");
    }
}
