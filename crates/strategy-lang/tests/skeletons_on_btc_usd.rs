//! The skeletons on real BTC-USD hourly bars (S-5.12; AGENT-011 §8.2, AC-2).
//!
//! The acceptance is stated on BTC-USD 1h: the skeletons "compile, validate and
//! preview on BTC-USD 1h with non-zero entries and no warm-up violation".
//! `skeletons_fire_on_hourly_bars.rs` proves it over a synthetic series because a
//! test cannot assume a database. This is the same check over the real bars, and
//! it is **ignored unless a ClickHouse is configured**:
//!
//! ```text
//! CLICKHOUSE_URL=http://user:pass@localhost:8123/trading \
//!   cargo test -p strategy-lang --test skeletons_on_btc_usd -- --ignored --nocapture
//! ```
//!
//! The URL comes from the environment and is never written here.
//!
//! # The window
//!
//! 2024-01-01 to 2025-01-01: a year of hourly bars, long enough that a warm-up of
//! a few hundred bars is a small part of it, and entirely in the past of any
//! research project's sealed holdout, which is the last days of the data. This
//! reads the bar store directly, as `backtest`'s own tests do; it does not go
//! through a project, so it is not a way around the holdout and must not be
//! pointed at a window near the present.

mod support;

use backtest::store::BarStore;
use chrono::{TimeZone, Utc};
use domain::payloads::bar::Timeframe;

#[tokio::test]
#[ignore = "needs a ClickHouse with BTC-USD hourly bars: set CLICKHOUSE_URL"]
async fn every_worked_skeleton_fires_on_btc_usd_hourly_bars() {
    let url = std::env::var("CLICKHOUSE_URL")
        .expect("set CLICKHOUSE_URL to a ClickHouse holding BTC-USD hourly bars");
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let bars = BarStore::connect(&url)
        .load_bars("BTC-USD", Timeframe::Hours1, start, end)
        .await
        .expect("the bars load");
    assert!(
        bars.len() > 8_000,
        "only {} hourly bars in 2024; this is not the data the acceptance names",
        bars.len()
    );

    let mut failures = Vec::new();
    let mut table = String::new();
    for s in strategy_lang::skeleton::worked() {
        let def = s.definition().expect("compiles");
        // Validated the way the save route does: after parameters take defaults.
        let valid = strategy_validator::validate(&support::materialized(&def)).is_ok();
        let result = backtest::preview::preview_signals(
            &support::materialized(&def),
            &bars,
            Timeframe::Hours1,
        );
        match result {
            Ok(p) => {
                table.push_str(&format!(
                    "{:<16} valid={valid} entries={} per_month={:?} warmup={} evaluated={}/{}\n",
                    s.family.to_string(),
                    p.entries,
                    p.entries_per_month.map(|m| (m * 10.0).round() / 10.0),
                    p.warmup_bars,
                    p.bars_evaluated,
                    p.n_bars
                ));
                if !valid || p.entries == 0 || p.bars_evaluated == 0 {
                    failures.push(format!("{}: {:?}", s.family, p.never_fires_because));
                }
            }
            Err(e) => {
                table.push_str(&format!("{:<16} valid={valid} REFUSED: {e}\n", s.family.to_string()));
                failures.push(format!("{}: refused: {e}", s.family));
            }
        }
    }
    println!("BTC-USD 1h, {} bars, 2024:\n{table}", bars.len());
    assert!(failures.is_empty(), "{failures:#?}");
}
