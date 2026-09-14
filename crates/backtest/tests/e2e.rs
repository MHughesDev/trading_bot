//! End-to-end backtest pipeline test against a real `ClickHouse` (#23).
//!
//! This exercises the data path the in-process unit tests can't: a genuine
//! `ClickHouse` round-trip — typed `RowBinary` insert → deduplicated `argMax`
//! read → simulation — proving the bars written by the collection phase are the
//! bars the simulator replays, end to end.
//!
//! It is **hermetic with respect to the network**: it seeds `ClickHouse`
//! directly (no collector / REST backfill) and drives the simulation in
//! process, so nothing reaches a third-party venue.  It still needs a live
//! `ClickHouse`, so it is gated on `BACKTEST_E2E_CLICKHOUSE_URL`; when that is
//! unset the test logs and returns, keeping `cargo test` green in environments
//! without the service (the `sim.rs` hermetic bridge test covers the sim path
//! there).
//!
//! To run it:
//!
//! ```bash
//! # bring up the bundled ClickHouse (see docker-compose), then:
//! BACKTEST_E2E_CLICKHOUSE_URL=http://localhost:8123 cargo test -p backtest --test e2e
//! ```

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use backtest::requirements::{FeatureKind, FeatureSpec};
use backtest::sim::{run_simulation, SimulationInputs};
use backtest::store::{BarStore, CollectedBar, LoadedBar};
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use nautilus_backtest::sdk::SimulationControl;

/// EMA-cross-long definition (fast over slow ⇒ one buy on the rising edge).
fn ema_cross_long_def() -> StrategyDefinition {
    serde_json::from_str(
        r#"{
            "strategy_id": "ema_cross_e2e",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [
                { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                { "lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"] }
            ],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
            ],
            "actions": [
                { "on_signal": "long", "type": "place_order",
                  "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
            ]
        }"#,
    )
    .expect("valid fixture definition")
}

fn feature_specs() -> Vec<FeatureSpec> {
    vec![
        FeatureSpec {
            name: "ema_7".into(),
            kind: FeatureKind::Ema,
            period: 7,
        },
        FeatureSpec {
            name: "ema_21".into(),
            kind: FeatureKind::Ema,
            period: 21,
        },
    ]
}

#[tokio::test]
async fn seeded_clickhouse_insert_load_and_simulate() {
    let Ok(base) = std::env::var("BACKTEST_E2E_CLICKHOUSE_URL") else {
        eprintln!("BACKTEST_E2E_CLICKHOUSE_URL unset — skipping live ClickHouse e2e");
        return;
    };

    // A throwaway database: test identities must never collide with real ones.
    let admin = storage::clickhouse::connect(&base);
    admin.query("DROP DATABASE IF EXISTS backtest_e2e").execute().await.unwrap();
    admin.query("CREATE DATABASE backtest_e2e").execute().await.unwrap();
    let (scheme, rest) = base.split_once("://").unwrap();
    let url = format!("{scheme}://{}/backtest_e2e", rest.split('/').next().unwrap());
    storage::clickhouse::migrate::run_migrations(&url)
        .await
        .expect("apply clickhouse schema");
    let identity = std::sync::Arc::new(storage::identity::MemoryIdentity::new());
    identity.add_source("e2e_test", storage::identity::SourceInfo { source_id: 16, declared_vendor_lag: std::time::Duration::ZERO, live: false });
    storage::identity::install(identity.clone());

    let store = BarStore::connect(&url);
    // Unique instrument id per run so the test never collides with other data.
    let instrument_id = format!("E2E-{}", Uuid::new_v4().simple());
    let timeframe = Timeframe::Minutes1;
    let base = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

    // 60 one-minute bars with a strictly rising close (100 → 159): the fast EMA
    // crosses above the slow EMA exactly once and stays above.
    let collected: Vec<CollectedBar> = (0..60i64)
        .map(|i| {
            let close = 100 + i;
            CollectedBar {
                available_time: base + chrono::Duration::minutes(i),
                sequence: u64::try_from(i).unwrap(),
                open: close.to_string(),
                high: close.to_string(),
                low: close.to_string(),
                close: close.to_string(),
                volume: "1".to_string(),
                trade_count: 1,
            }
        })
        .collect();

    store
        .insert_collected(
            &instrument_id,
            "binance",
            "e2e_test",
            "test",
            timeframe,
            &collected,
        )
        .await
        .expect("seed market_bar");
    identity.sync_dims(&url).await.expect("sync identity dims");

    let from = base - chrono::Duration::minutes(1);
    let to = base + chrono::Duration::minutes(61);

    // Read the bars back through the deduplicated argMax path.
    let bars: Vec<LoadedBar> = store
        .load_bars(&instrument_id, timeframe, from, to)
        .await
        .expect("load_bars");
    assert_eq!(bars.len(), 60, "all seeded bars load back");
    assert!(
        bars.windows(2).all(|w| w[0].ts_ns < w[1].ts_ns),
        "bars come back strictly ordered by available_time"
    );
    assert_eq!(bars[0].close, Decimal::from(100));
    assert_eq!(bars[59].close, Decimal::from(159));

    // Drive the full simulation over the freshly loaded bars.
    let inputs = SimulationInputs {
        definition: ema_cross_long_def(),
        instrument_id: instrument_id.clone(),
        venue_id: "binance".into(),
        asset_class: "crypto_spot_cex".into(),
        timeframe,
        quote_currency: "USDT".into(),
        initial_balance: Decimal::from(100_000),
        precisions: None,
        sim_start_ns: 0,
        bars,
        features: feature_specs(),
    };

    let control = SimulationControl::new();
    let report =
        tokio::task::spawn_blocking(move || run_simulation(inputs, &control).expect("simulate"))
            .await
            .expect("blocking task");

    assert!(!report.cancelled);
    let total_orders = report.result["total_orders"].as_u64().unwrap_or(0);
    assert_eq!(
        total_orders, 1,
        "one rising-edge crossover over the round-tripped bars ⇒ one order"
    );
}
