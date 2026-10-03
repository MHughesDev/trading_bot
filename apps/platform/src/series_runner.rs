//! The platform's `predict_series` runner (Set U U-5.5).
//!
//! Loads the registry and the window's bars, then runs
//! `backtest::runner::predict_series` — the strategy engine's own loop over a
//! graph that only calls the model — through the point-in-time provider whose
//! cache is the ClickHouse prediction series. Every answer it computes lands in
//! `model_prediction_series`, keyed exactly as a later backtest will ask.

use std::sync::Arc;

use jobs::kinds::predict_series::{PredictSeriesManifest, SeriesOutcome, SeriesRunner};

pub struct PlatformSeriesRunner {
    pub pg: sqlx::PgPool,
    pub ch_url: String,
    pub sidecar: Arc<dyn model_registry::provider::V2Client>,
}

#[async_trait::async_trait]
impl SeriesRunner for PlatformSeriesRunner {
    async fn fill(&self, m: &PredictSeriesManifest) -> anyhow::Result<SeriesOutcome> {
        let registry = Arc::new(backtest::models::RegistrySnapshot::load(&self.pg).await?);
        let tf = <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(
            &m.timeframe,
        )
        .ok_or_else(|| anyhow::anyhow!("unknown timeframe '{}'", m.timeframe))?;
        let meta = storage::postgres::instruments::fetch_by_id(&self.pg, &m.instrument)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no instrument metadata for '{}'", m.instrument))?;
        let asset_class = meta.asset_class;
        let bars = backtest::store::BarStore::connect(&self.ch_url)
            .load_bars(&m.instrument, tf, m.start, m.end)
            .await?;
        let handle = tokio::runtime::Handle::current();
        let cache: Arc<dyn backtest::models::PredictionCache> = Arc::new(
            backtest::models::ClickHouseCache::new(&self.ch_url, handle.clone()),
        );
        let source = model_registry::provider::backtest::SidecarSource::new(
            Arc::clone(&self.sidecar),
            handle,
        );
        let manifest = m.clone();
        let report = tokio::task::spawn_blocking(move || {
            let mut models =
                backtest::models::PitModels::new(Arc::clone(&registry), source, cache, false);
            backtest::runner::predict_series(
                &manifest.model,
                &registry,
                &manifest.instrument,
                &manifest.timeframe,
                asset_class,
                bars,
                manifest.start.timestamp_nanos_opt().unwrap_or(0),
                &mut models,
            )
        })
        .await?
        .map_err(|e| anyhow::anyhow!(e))?;
        Ok(SeriesOutcome {
            answered: report.answered,
            before_training_end: report.before_training_end,
            other: report.other,
        })
    }
}
