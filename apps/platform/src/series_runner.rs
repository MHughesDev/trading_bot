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

#[cfg(test)]
mod tests {
    //! U-5.5 and U-5.8 against servers: the job fills the ClickHouse series for
    //! a version with folds, each row by the model allowed to answer it, and a
    //! second fill over the window asks the sidecar nothing.
    //!
    //! Runs against a scratch PostgreSQL (0017, 0018, 0146, 0189 and the
    //! instruments table) and a scratch ClickHouse database holding a copy of
    //! some stored bars and `09_model_prediction_series.sql` — never the
    //! platform's own:
    //!
    //! ```text
    //! DATABASE_URL=postgres://…/scratch SERIES_CLICKHOUSE_URL=http://…@localhost:8123/u55_scratch \
    //!   SERIES_INSTRUMENT=BTC-USD SERIES_FROM=2026-09-01T00:00:00Z SERIES_TO=2026-09-11T00:00:00Z \
    //!   cargo test -j 2 -p platform series_runner -- --ignored --nocapture
    //! ```
    use std::sync::{Arc, Mutex};

    use chrono::{DateTime, Duration, Utc};
    use domain::model_def::contract::{
        ContractInput, ContractOutput, CostClass, FeatureSource, InputShape, IoContract, PitSpec,
    };
    use domain::strategy_graph::ValueType;
    use jobs::kinds::predict_series::{PredictSeriesManifest, SeriesRunner};
    use model_registry::provider::V2Client;
    use model_registry::sidecar::{PredictV2Request, PredictV2Response, V2Answer};

    use super::PlatformSeriesRunner;

    /// The inference service's stand-in: answers `f = 1` and records which
    /// artifact each call named.
    #[derive(Default)]
    struct Recording(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl V2Client for Recording {
        async fn predict_v2(&self, req: PredictV2Request) -> anyhow::Result<PredictV2Response> {
            self.0.lock().unwrap().push(req.artifact_uri.clone());
            Ok(PredictV2Response {
                model_id: req.model_id,
                version: req.version,
                answers: req
                    .rows
                    .iter()
                    .map(|r| V2Answer {
                        key: Some(r.key.clone()),
                        status: "answered".into(),
                        outputs: [("f".to_string(), serde_json::json!(1.0))]
                            .into_iter()
                            .collect(),
                        detail: None,
                    })
                    .collect(),
                latency_ms: 0,
            })
        }
    }

    /// A query over ClickHouse's HTTP interface, as tab-separated text.
    async fn clickhouse_tsv(url: &str, sql: &str) -> String {
        let mut u = reqwest::Url::parse(url).unwrap();
        let (user, pass) = (u.username().to_string(), u.password().map(str::to_string));
        let db = u.path().trim_start_matches('/').to_string();
        u.set_username("").unwrap();
        u.set_password(None).unwrap();
        u.set_path("/");
        reqwest::Client::new()
            .post(u)
            .query(&[("database", db)])
            .basic_auth(user, pass)
            .body(sql.to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    fn env(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("{name}"))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs a scratch DATABASE_URL and SERIES_CLICKHOUSE_URL with stored bars"]
    async fn the_series_job_fills_clickhouse_fold_by_fold_and_a_refill_asks_nothing() {
        let pg = sqlx::PgPool::connect(&env("DATABASE_URL")).await.unwrap();
        let ch_url = env("SERIES_CLICKHOUSE_URL");
        let instrument = env("SERIES_INSTRUMENT");
        let from: DateTime<Utc> = env("SERIES_FROM").parse().unwrap();
        let to: DateTime<Utc> = env("SERIES_TO").parse().unwrap();

        sqlx::query(
            "INSERT INTO instruments (instrument_id, asset_class, venue_id, base_precision, \
               quote_precision, tick_size, lot_size, trading_hours_json, halt_policy, trust_tier) \
             VALUES ($1, 'crypto_spot_cex', 'coinbase', 8, 2, 0.01, 0.0001, '{}', 'non_haltable', \
               'centralized_exchange') ON CONFLICT (instrument_id) DO NOTHING",
        )
        .bind(&instrument)
        .execute(&pg)
        .await
        .unwrap();

        // A version trained through 60% of the window, with two earlier folds.
        let model = format!("series_{}", uuid::Uuid::new_v4().simple());
        let span = to - from;
        let ends = [
            from + span * 3 / 10,
            from + span * 45 / 100,
            from + span * 6 / 10,
        ];
        let contract = IoContract {
            inputs: vec![ContractInput {
                name: "features".into(),
                shape: InputShape::Row {
                    features: FeatureSource::Named {
                        features: vec!["rsi_14".into()],
                    },
                },
                optional: false,
            }],
            outputs: vec![ContractOutput {
                name: "f".into(),
                value_type: ValueType::Num,
            }],
            horizon: Some("1h".into()),
            pit: PitSpec {
                walk_forward: true,
                embargo: Some("1h".into()),
            },
            cost_class: CostClass::Cheap,
            timeout_ms: 1000,
        };
        let who = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, \
               definition_json, created_by) \
             VALUES ($1, $1, $1, 'regressor', 'crypto_spot_cex', '{}'::jsonb, $2)",
        )
        .bind(&model)
        .bind(who)
        .execute(&pg)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO model_versions (model_id, version, config_json, created_by, \
               trained_through, io_contract, contract_source) \
             VALUES ($1, 1, '{}'::jsonb, $2, $3, $4, 'declared')",
        )
        .bind(&model)
        .bind(who)
        .bind(ends[2])
        .bind(serde_json::to_value(&contract).unwrap())
        .execute(&pg)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO model_artifacts (model_id, version, storage_uri, artifact_type, size_bytes, sha256) \
             VALUES ($1, 1, 'file:///version', 'model', 1, $2)",
        )
        .bind(&model)
        .bind("f".repeat(64))
        .execute(&pg)
        .await
        .unwrap();
        for (i, end) in ends[..2].iter().enumerate() {
            storage::postgres::models::record_fold_artifact(
                &pg,
                &model,
                1,
                i32::try_from(i).unwrap(),
                *end,
                &format!("file:///fold/{i}"),
                1,
                &"0".repeat(64),
            )
            .await
            .unwrap();
        }

        let sidecar = Arc::new(Recording::default());
        let runner = PlatformSeriesRunner {
            pg: pg.clone(),
            ch_url: ch_url.clone(),
            sidecar: sidecar.clone(),
        };
        let manifest = PredictSeriesManifest {
            model: format!("{model}@1"),
            instrument: instrument.clone(),
            timeframe: "1h".into(),
            start: from,
            end: to,
        };
        let first = runner.fill(&manifest).await.unwrap();
        println!(
            "first fill: answered {}, before training end {}, other {}",
            first.answered, first.before_training_end, first.other
        );
        assert!(first.answered > 0 && first.before_training_end > 0);
        let calls = sidecar.0.lock().unwrap().clone();
        assert_eq!(
            calls.len() as u64,
            first.answered,
            "one call per answered decision"
        );
        for uri in ["file:///fold/0", "file:///fold/1", "file:///version"] {
            assert!(calls.iter().any(|c| c == uri), "{uri} answered its window");
        }

        // What the server holds: one row per answer, each tagged with the
        // training end of the latest model trained before it.
        let rows: Vec<(i64, Option<i64>)> = clickhouse_tsv(
            &ch_url,
            &format!(
                "SELECT at_ns, trained_through_ns FROM model_prediction_series FINAL \
                 WHERE model_id = '{model}' ORDER BY at_ns FORMAT TSV"
            ),
        )
        .await
        .lines()
        .map(|l| {
            let (at, through) = l.split_once('\t').unwrap();
            (at.parse().unwrap(), through.parse().ok())
        })
        .collect();
        assert_eq!(
            rows.len() as u64,
            first.answered,
            "every answer stored, once"
        );
        let hour = Duration::hours(1).num_nanoseconds().unwrap();
        for (at, through) in &rows {
            let want = ends
                .iter()
                .map(|e| e.timestamp_nanos_opt().unwrap())
                .rfind(|t| t + hour < *at)
                .expect("no row before the first fold may answer");
            assert_eq!(*through, Some(want), "at {at}");
        }

        // The same window again reads the series: the sidecar is not asked.
        let second = runner.fill(&manifest).await.unwrap();
        assert_eq!(second.answered, first.answered);
        assert_eq!(
            sidecar.0.lock().unwrap().len() as u64,
            first.answered,
            "the refill made no sidecar calls"
        );
    }
}
