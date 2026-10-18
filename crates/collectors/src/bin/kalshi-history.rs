//! Fetch Kalshi candlestick history for named markets into
//! `prediction_probabilities` (Set P P-3.4).
//!
//! ```text
//! DATABASE_URL=postgres://…  CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//!   cargo run -p collectors --bin kalshi-history -- KXHIGHNY-26SEP23-B65.5 [more tickers…]
//! ```
//!
//! `KALSHI_HISTORY_DAYS` (default 30) bounds the window back from now;
//! `KALSHI_PERIOD_MINUTES` is Kalshi's `period_interval` (1, 60 or 1440; default 60).
//! Public, unauthenticated GETs only; every row is derived (knowledge time = period
//! close + 60 s, flagged), never presented as observed.

use chrono::{Duration, Utc};
use collectors::backfill::ReqwestGet;
use collectors::prediction::history;
use collectors::ratelimit::RateLimiter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pg_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let ch_url = std::env::var("CLICKHOUSE_URL").map_err(|_| "CLICKHOUSE_URL is required")?;
    let days: i64 = std::env::var("KALSHI_HISTORY_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let period: u32 = std::env::var("KALSHI_PERIOD_MINUTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let tickers: Vec<String> = std::env::args().skip(1).collect();
    if tickers.is_empty() {
        return Err("name at least one market ticker".into());
    }
    let pg = storage::postgres::connect(&pg_url).await?;
    let ids = storage::clickhouse::forward::source_ids(&pg).await?;
    pg.close().await;
    let source_id = *ids
        .get(history::SOURCE_NAME)
        .ok_or("dataplane.source has no kalshi_history row (migration 0191)")?;

    let http = ReqwestGet::new("tbot-research-collector");
    let limiter = RateLimiter::new(history::limit());
    let to = Utc::now();
    let from = to - Duration::days(days);
    for ticker in tickers {
        let bars = history::fetch_candlesticks(
            &http,
            &limiter,
            history::KALSHI_REST_BASE,
            &ticker,
            from,
            to,
            period,
        )
        .await?;
        let rows = history::to_rows(&bars, source_id);
        let written = storage::clickhouse::alt::write_probabilities(&ch_url, &rows).await?;
        println!(
            "{ticker} candles_with_trades={} written={written}",
            bars.len()
        );
    }
    Ok(())
}
