//! The daily regime labelling job (SPEC §5.4, checklist 3.5, ADR-P3-02).
//!
//! The rule tier, run on a timer: daily returns for each market scope go in,
//! volatility terciles come out, and `regime_causal.regime_state` holds the
//! result. That is the whole model for now, and it is deliberately the whole
//! model — a three-state filtered HMM replaces the internals later without
//! changing the table, and until it exists Gate 11 has real regimes to measure
//! coverage against rather than a column of nulls.
//!
//! The job writes **filtered** states only. There is no smoothed pass here and
//! no function in `api::knowledge` that would store one; the smoothed table
//! exists for research and lives behind a schema grant this process's role does
//! not hold (AT-42).

use std::time::Duration;

use api::knowledge::RegimeStore;
use sqlx::PgPool;
use tracing::{error, info, warn};

/// How often the labels are recomputed.
///
/// Daily, because the input is a daily return series and the tercile boundaries
/// move only when a day is added. More often would relabel the same history with
/// the same answer; less often would leave the newest days unlabelled, and an
/// unlabelled day is one Gate 11 cannot count toward coverage.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The trailing window the realized volatility is measured over.
///
/// Twenty-one trading days — a month. Short enough to move when the market does,
/// long enough that a single day cannot flip the label, which matters because
/// the label is what Gate 11 counts regimes with.
const VOL_WINDOW: usize = 21;

/// How much history to label on each pass.
const LOOKBACK_DAYS: i64 = 400;

/// Start the daily loop.
pub fn spawn(pg: PgPool, clickhouse_url: String, scopes: Vec<String>) {
    if scopes.is_empty() {
        info!("no market scopes configured; regime labelling is not running");
        return;
    }
    tokio::spawn(async move {
        loop {
            run_once(&pg, &clickhouse_url, &scopes).await;
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

async fn run_once(pg: &PgPool, clickhouse_url: &str, scopes: &[String]) {
    let store = RegimeStore::new(pg.clone());
    for scope in scopes {
        match daily_returns(clickhouse_url, scope).await {
            Ok(returns) if returns.len() > VOL_WINDOW => {
                match store.write_rule_tier(scope, &returns, VOL_WINDOW).await {
                    Ok(n) => info!(scope, days = n, "regime labels written"),
                    Err(e) => error!(scope, error = %e, "regime labels not written"),
                }
            }
            Ok(returns) => warn!(
                scope,
                days = returns.len(),
                window = VOL_WINDOW,
                "not enough history to label a regime; nothing written rather than a label from \
                 fewer days than the window"
            ),
            Err(e) => error!(scope, error = %e, "could not read daily returns"),
        }
    }
}

/// Daily log returns for a market scope, through the single PIT reader.
///
/// The scope is an instrument id: the market a strategy's regime is read against
/// is the market it trades, and a platform-wide "the market" would be a number
/// that means something different for every asset class here.
async fn daily_returns(
    clickhouse_url: &str,
    scope: &str,
) -> anyhow::Result<Vec<(chrono::DateTime<chrono::Utc>, f64)>> {
    use backtest::BarStore;
    use domain::payloads::bar::Timeframe;

    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(LOOKBACK_DAYS);
    let bars = BarStore::connect(clickhouse_url)
        .load_bars(scope, Timeframe::Daily, start, end)
        .await?;

    let mut out = Vec::with_capacity(bars.len().saturating_sub(1));
    for pair in bars.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        let (p0, p1) = (to_f64(prev.close), to_f64(next.close));
        if p0 > 0.0 && p1 > 0.0 {
            // The bar's *close*: the earliest instant a strategy acting on the
            // whole bar could have known its return.
            let ts = chrono::DateTime::from_timestamp_nanos(next.ts_ns);
            out.push((ts, (p1 / p0).ln()));
        }
    }
    Ok(out)
}

fn to_f64(d: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}
