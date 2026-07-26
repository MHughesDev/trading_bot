//! Owns the set of live in-process pipelines, one per initialized instrument.
//!
//! The platform guarantees that any **initialized** asset (a row in
//! `asset_lifecycle`) has a running pipeline that aggregates the live trade
//! stream into 1-minute bars and persists them — regardless of whether a
//! strategy or automation is subscribed.  Pipelines are started here on boot
//! (resume) and on demand when a new asset is initialized.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use domain::instrument::AssetClass;
use domain::payloads::bar::Timeframe;
use execution::paper::PaperTradingEngine;
use tracing::{info, warn};

use crate::bar_persist::PersistBar;
use crate::hot_path::{self, BarSink, PipelineHandle, RawTick};

/// Shared dependencies needed to spawn a pipeline, plus the registry of those
/// currently running (keyed by `instrument_id`).
pub struct PipelineManager {
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
    bar_persist_tx: tokio::sync::mpsc::UnboundedSender<PersistBar>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
    paper_engine: Arc<PaperTradingEngine>,
    active: Mutex<HashMap<String, PipelineHandle>>,
    ch_url: String,
}

impl PipelineManager {
    pub fn new(
        tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
        bar_persist_tx: tokio::sync::mpsc::UnboundedSender<PersistBar>,
        execution_engine: Arc<execution::ExecutionEngine>,
        risk_gate: Arc<risk::RiskGate>,
        paper_engine: Arc<PaperTradingEngine>,
        ch_url: String,
    ) -> Self {
        Self {
            tee_tx,
            bar_persist_tx,
            execution_engine,
            risk_gate,
            paper_engine,
            active: Mutex::new(HashMap::new()),
            ch_url,
        }
    }

    /// Ensure a continuous 1-minute aggregation pipeline is running for
    /// `instrument_id`.  Idempotent — a no-op if one is already active.
    ///
    /// Only `crypto_spot_cex` has an in-process collector (Kraken) today; other
    /// asset classes are logged and skipped until their live collector is wired.
    pub fn ensure(&self, instrument_id: &str, asset_class: &str) {
        let mut active = self.active.lock().expect("pipeline map lock");
        if active.contains_key(instrument_id) {
            return;
        }

        let Some(plan) = CollectorBinding::for_asset_class(asset_class, instrument_id) else {
            warn!(
                instrument_id,
                asset_class,
                "no in-process collector for this asset class — live 1m aggregation not started"
            );
            return;
        };

        let sink = BarSink {
            tx: self.bar_persist_tx.clone(),
            venue_id: plan.venue_id.clone(),
            source: plan.source.clone(),
            trust_tier: plan.trust_tier.clone(),
        };

        let handle = hot_path::spawn_pipeline(
            plan.symbol,
            instrument_id.to_string(),
            plan.asset_class,
            self.tee_tx.clone(),
            Arc::clone(&self.execution_engine),
            Arc::clone(&self.risk_gate),
            Arc::clone(&self.paper_engine),
            sink,
        );

        info!(
            instrument_id,
            asset_class, "live 1m aggregation pipeline started"
        );
        active.insert(instrument_id.to_string(), handle);

        // Spawn a background gap-fill: if there's stored 1m data but a gap
        // between the last bar and now (e.g. after a platform restart), backfill
        // from Coinbase REST so the chart has a continuous history.
        let ch_url = self.ch_url.clone();
        let instrument_id_owned = instrument_id.to_string();
        let asset_class_owned = asset_class.to_string();
        tokio::spawn(async move {
            if let Err(e) = gap_fill(&ch_url, &instrument_id_owned, &asset_class_owned).await {
                warn!(instrument_id = %instrument_id_owned, error = %e, "gap fill failed (non-fatal)");
            }
        });
    }

    /// Number of pipelines currently running (for diagnostics).
    pub fn active_count(&self) -> usize {
        self.active.lock().expect("pipeline map lock").len()
    }
}

/// How far back to scan for gaps on startup.  Kraken's 1m OHLC endpoint
/// provides up to ~10 hours of rolling history; for gaps older than that the
/// collection will return 0 bars (Kraken has no data), which is logged but not
/// treated as an error.  30 days catches every internal gap that accumulated
/// across multiple restarts and maximises what the exchange API can actually
/// deliver.
const BACKFILL_LOOKBACK_DAYS: i64 = 30;

/// Scans the last [`BACKFILL_LOOKBACK_DAYS`] days for coverage gaps in the 1m
/// history of `instrument_id` and fills each one via the appropriate REST
/// collector.
///
/// Unlike the old trailing-gap approach (fill from last bar to now) this uses
/// the same `daily_counts` + `gaps::analyze` logic as the backtest system, so
/// it catches **all** internal gaps from previous outages, not just the most
/// recent one.
///
/// Re-inserting bars that already exist is safe — ClickHouse's
/// `ReplacingMergeTree` + UUIDv5 dedup key makes the operation idempotent.
async fn gap_fill(ch_url: &str, instrument_id: &str, asset_class: &str) -> anyhow::Result<()> {
    let store = backtest::store::BarStore::connect(ch_url);

    let now = Utc::now();
    // Leave a 90-second buffer so we don't try to backfill the current
    // incomplete bar that the live aggregator is still building.
    let fill_to = now - chrono::Duration::seconds(90);
    let fill_from = now - chrono::Duration::days(BACKFILL_LOOKBACK_DAYS);

    // Per-day bar counts for the window.
    let counts = store
        .daily_counts(instrument_id, Timeframe::Minutes1, fill_from, fill_to)
        .await?;

    // If we have no data at all, seed from the beginning of the window.
    // Otherwise use gap analysis to find all under-covered days.
    let schedule = backtest::gaps::ScheduleKind::for_asset_class(asset_class);
    let coverage = backtest::gaps::analyze(fill_from, fill_to, &counts, Timeframe::Minutes1, schedule);

    if coverage.missing_ranges.is_empty() {
        return Ok(()); // nothing to fill
    }

    info!(
        instrument_id,
        gaps = coverage.missing_ranges.len(),
        missing_bars = coverage.expected_bars.saturating_sub(coverage.present_bars),
        "gap fill: found missing ranges, backfilling"
    );

    let plan = backtest::collect::CollectorPlan::for_asset_class(asset_class, instrument_id)?;
    let venue_id = match &plan {
        backtest::collect::CollectorPlan::KrakenOhlc { .. } => "kraken",
        backtest::collect::CollectorPlan::BinanceKlines { .. } => "binance",
        backtest::collect::CollectorPlan::AlpacaBars { .. } => "alpaca",
    };
    let http = reqwest::Client::new();
    let collected = AtomicU64::new(0);
    let cancel = AtomicBool::new(false);

    let bars = backtest::collect::collect_ranges(
        &http,
        &store,
        &plan,
        instrument_id,
        venue_id,
        Timeframe::Minutes1,
        &coverage.missing_ranges,
        &collected,
        &cancel,
    )
    .await?;

    info!(instrument_id, bars, "gap fill complete");
    Ok(())
}

/// Resolves the live collector + venue metadata for an asset class.
struct CollectorBinding {
    asset_class: AssetClass,
    /// Exchange-format symbol the collector subscribes to (e.g. `BTC/USD`).
    symbol: String,
    venue_id: String,
    source: String,
    trust_tier: String,
}

impl CollectorBinding {
    fn for_asset_class(asset_class: &str, instrument_id: &str) -> Option<Self> {
        match asset_class {
            // Kraken is the crypto in-process collector.  `BTC-USD` → `BTC/USD`.
            "crypto_spot_cex" => Some(Self {
                asset_class: AssetClass::CryptoSpotCex,
                symbol: instrument_id.replace('-', "/"),
                venue_id: "kraken".to_string(),
                source: "kraken_ws".to_string(),
                trust_tier: "centralized_exchange".to_string(),
            }),
            // Alpaca is the equity in-process collector. Symbol is the ticker as-is.
            "equity" => Some(Self {
                asset_class: AssetClass::Equity,
                symbol: instrument_id.to_string(),
                venue_id: "alpaca".to_string(),
                source: "alpaca_ws".to_string(),
                trust_tier: "regulated".to_string(),
            }),
            "etf" => Some(Self {
                asset_class: AssetClass::Etf,
                symbol: instrument_id.to_string(),
                venue_id: "alpaca".to_string(),
                source: "alpaca_ws".to_string(),
                trust_tier: "regulated".to_string(),
            }),
            _ => None,
        }
    }
}
