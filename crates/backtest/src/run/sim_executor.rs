//! Real-simulator executor for the Set J suite (J-0.6 / J-0.7).
//!
//! The suite has always computed its honesty statistics on a *synthetic* executor
//! (an FNV-hash equity curve) so the funnel/vault/counter machinery could be
//! tested hermetically. This is the production executor that runs an actual
//! `market_simulator` backtest per study member, via the trade/equity extraction
//! in [`crate::sim::run_simulation_detailed`].
//!
//! Resolution (strategy definition from Postgres, instrument metadata, ClickHouse
//! bars) is async, but [`RunExecutor::execute`] is sync — so each config's data
//! window is resolved once (cached, shared across a study's members) via
//! `block_in_place` + a captured runtime `Handle`. Members in a study differ by
//! seed/window/cost, not by the loaded bars, so one resolve serves them all.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Duration;
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use tokio::runtime::Handle;

use domain::instrument::AssetClass;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use domain::Instrument;

use crate::requirements::{derive_requirements, FeatureSpec};
use crate::run::config::{DataSlice, EvalResolution};
use crate::run::executor::map_detailed_result;
use crate::run::{ComputeCost, RunConfig, RunExecutor, RunResult, ENGINE_VERSION};
use crate::sim::{run_simulation_detailed, InstrumentPrecisions, SimulationInputs};
use crate::store::{BarStore, LoadedBar};
use nautilus_backtest::sdk::SimulationControl;

/// Default starting balance — not modeled per-experiment in v1.
fn default_balance() -> Decimal {
    Decimal::from(100_000u32)
}

/// Pre-resolved inputs for one experiment's data window, shared across a study's
/// members (which vary only by seed/window/cost, not the loaded bars).
#[derive(Clone)]
struct SimContext {
    definition: StrategyDefinition,
    instrument_id: String,
    venue_id: String,
    asset_class: String,
    quote_currency: String,
    timeframe: Timeframe,
    precisions: Option<InstrumentPrecisions>,
    initial_balance: Decimal,
    sim_start_ns: i64,
    bars: Arc<Vec<LoadedBar>>,
    features: Vec<FeatureSpec>,
}

/// Resolves a suite config (opaque string refs) into the concrete inputs a real
/// simulation needs, reusing the simple-backtest recipe (`manager.rs::drive_inner`).
pub struct SimResolver {
    pg: PgPool,
    ch_url: String,
}

impl SimResolver {
    pub fn new(pg: PgPool, ch_url: impl Into<String>) -> Self {
        Self {
            pg,
            ch_url: ch_url.into(),
        }
    }

    async fn resolve(
        &self,
        strategy_family: &str,
        slice: &DataSlice,
    ) -> anyhow::Result<SimContext> {
        let definition = load_definition(&self.pg, strategy_family).await?;
        // The universe ref doubles as the instrument id (e.g. "BTC-USDT").
        let instrument_id = slice.universe_ref.clone();
        let inst = storage::postgres::instruments::fetch_by_id(&self.pg, &instrument_id)
            .await
            .map_err(|e| anyhow::anyhow!("instrument lookup failed: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("instrument '{instrument_id}' not found"))?;

        let timeframe = timeframe_of(slice.eval_resolution)?;
        let requirements = derive_requirements(&definition, timeframe)
            .map_err(|e| anyhow::anyhow!("requirements: {e}"))?;
        let warmup_secs =
            i64::try_from(requirements.warmup_bars).unwrap_or(0) * timeframe_secs(timeframe);
        let from = slice.start - Duration::seconds(warmup_secs);
        let bars = BarStore::connect(&self.ch_url)
            .load_bars(&instrument_id, timeframe, from, slice.end)
            .await?;
        anyhow::ensure!(!bars.is_empty(), "no bars for '{instrument_id}' in window");

        Ok(SimContext {
            definition,
            quote_currency: quote_of(&instrument_id),
            asset_class: asset_class_str(inst.asset_class),
            precisions: precisions_of(&inst),
            venue_id: inst.venue_id,
            instrument_id,
            timeframe,
            initial_balance: default_balance(),
            sim_start_ns: slice.start.timestamp_nanos_opt().unwrap_or(0),
            bars: Arc::new(bars),
            features: requirements.features,
        })
    }
}

/// A [`RunExecutor`] backed by the real `market_simulator`. Each member runs a
/// real backtest; the data window is resolved once and cached.
pub struct SimRunExecutor {
    resolver: Arc<SimResolver>,
    handle: Handle,
    cache: Mutex<HashMap<String, Arc<SimContext>>>,
}

impl SimRunExecutor {
    /// Construct from the API state's Postgres pool + ClickHouse URL. Must be
    /// called from within the Tokio runtime (captures `Handle::current()`).
    #[must_use]
    pub fn new(pg: PgPool, ch_url: impl Into<String>) -> Self {
        Self {
            resolver: Arc::new(SimResolver::new(pg, ch_url)),
            handle: Handle::current(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn context(&self, cfg: &RunConfig) -> anyhow::Result<Arc<SimContext>> {
        let s = &cfg.data_slice;
        let key = format!(
            "{}|{}|{}|{}|{:?}",
            cfg.strategy_ref, s.universe_ref, s.start, s.end, s.eval_resolution
        );
        if let Some(ctx) = self
            .cache
            .lock()
            .expect("sim cache poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(ctx);
        }
        // execute() is sync but resolve is async + I/O-bound; block_in_place keeps
        // the multi-threaded runtime healthy while we wait.
        let ctx = Arc::new(tokio::task::block_in_place(|| {
            self.handle
                .block_on(self.resolver.resolve(&cfg.strategy_ref, s))
        })?);
        self.cache
            .lock()
            .expect("sim cache poisoned")
            .insert(key, Arc::clone(&ctx));
        Ok(ctx)
    }
}

impl RunExecutor for SimRunExecutor {
    fn execute(&self, cfg: &RunConfig) -> RunResult {
        let started = std::time::Instant::now();
        let ctx = match self.context(cfg) {
            Ok(c) => c,
            Err(e) => {
                return RunResult::failed(cfg, format!("resolve failed: {e}"), ENGINE_VERSION)
            }
        };

        // v1 LIMITATION: cfg.params (parameter_sweep / neighborhood) are not yet
        // applied to the definition — there is no param-override helper for the
        // node graph. Members that vary params currently run the base definition;
        // seed/window/cost/regime studies (which vary data, not params) are exact.
        let inputs = SimulationInputs {
            definition: ctx.definition.clone(),
            instrument_id: ctx.instrument_id.clone(),
            venue_id: ctx.venue_id.clone(),
            asset_class: ctx.asset_class.clone(),
            timeframe: ctx.timeframe,
            quote_currency: ctx.quote_currency.clone(),
            initial_balance: ctx.initial_balance,
            precisions: ctx.precisions,
            sim_start_ns: ctx.sim_start_ns,
            bars: (*ctx.bars).clone(),
            features: ctx.features.clone(),
        };

        let control = SimulationControl::new();
        match run_simulation_detailed(inputs, &control) {
            Ok(outcome) => {
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                map_detailed_result(
                    cfg,
                    outcome,
                    ComputeCost {
                        wall_ms: ms,
                        cpu_ms: ms,
                    },
                    "suite@sim",
                )
            }
            Err(e) => RunResult::failed(cfg, format!("sim failed: {e}"), ENGINE_VERSION),
        }
    }
}

async fn load_definition(pg: &PgPool, strategy_id: &str) -> anyhow::Result<StrategyDefinition> {
    let row =
        sqlx::query("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(strategy_id)
            .fetch_optional(pg)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("strategy '{strategy_id}' not found in strategy_definitions")
            })?;
    let json: serde_json::Value = row.try_get("definition_json")?;
    Ok(serde_json::from_value(json)?)
}

/// `EvalResolution` → bar `Timeframe`. 10m/30m have no bar timeframe and are
/// rejected (real sims load bars at one of the supported cadences).
fn timeframe_of(r: EvalResolution) -> anyhow::Result<Timeframe> {
    Ok(match r {
        EvalResolution::Min1 => Timeframe::Minutes1,
        EvalResolution::Min5 => Timeframe::Minutes5,
        EvalResolution::Min15 => Timeframe::Minutes15,
        EvalResolution::Hour1 => Timeframe::Hours1,
        EvalResolution::Day1 => Timeframe::Daily,
        other => anyhow::bail!("eval resolution {other:?} has no bar timeframe for real sim"),
    })
}

fn timeframe_secs(t: Timeframe) -> i64 {
    match t {
        Timeframe::Seconds1 => 1,
        Timeframe::Minutes1 => 60,
        Timeframe::Minutes5 => 300,
        Timeframe::Minutes15 => 900,
        Timeframe::Hours1 => 3_600,
        Timeframe::Hours4 => 14_400,
        Timeframe::Daily => 86_400,
    }
}

/// Quote currency from an instrument id like "BTC-USDT" → "USDT" (fallback "USD").
fn quote_of(instrument_id: &str) -> String {
    for sep in ['-', '/'] {
        if let Some((_, quote)) = instrument_id.split_once(sep) {
            if !quote.is_empty() {
                return quote.to_uppercase();
            }
        }
    }
    "USD".to_string()
}

fn asset_class_str(ac: AssetClass) -> String {
    serde_json::to_value(ac)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "crypto_spot_cex".to_string())
}

fn precisions_of(inst: &Instrument) -> Option<InstrumentPrecisions> {
    if inst.tick_size.is_zero() || inst.lot_size.is_zero() {
        return None;
    }
    let scale = |d: Decimal| u8::try_from(d.normalize().scale()).unwrap_or(9).min(9);
    Some(InstrumentPrecisions {
        price: scale(inst.tick_size),
        size: scale(inst.lot_size),
    })
}
