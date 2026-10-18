//! Filling realised outcomes into the forecasts the platform served (Set N N-4.1).
//!
//! `model_forecasts` gets one row per served distribution; the quality monitor, the coverage
//! CUSUM and `POST .../calibrate` all read the rows that have a `realized` value, and nothing
//! ever wrote one, so every one of them ran on an empty table. This is the writer.
//!
//! A forecast made at `t0` over `horizon_bars` bars of `timeframe` is realised once the bar
//! that closes at `t0 + horizon` exists: `realized = exit_close / entry_close - 1`, where the
//! entry is the last bar that had **closed by `t0`** (what the model could have seen) and the
//! exit is the first bar closing at or after the horizon, within one bar of it. A forecast
//! whose horizon has not passed, or whose exit bar is missing, stays unrealised; one logged
//! without a horizon or timeframe (every row written before migration 0227) is counted and
//! left alone rather than guessed at.

use backtest::store::BarStore;
use chrono::{DateTime, Duration, TimeZone, Utc};
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;
use sqlx::PgPool;

/// One closed bar: the instant it closed (Unix ns) and its close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Close {
    pub ts_ns: i64,
    pub close: f64,
}

/// The realised simple return of a forecast issued at `t0_ns`, or `None` while it is not
/// knowable. `bars` must be ascending by `ts_ns`.
#[must_use]
pub fn realised_return(bars: &[Close], t0_ns: i64, horizon_ns: i64, period_ns: i64) -> Option<(f64, i64)> {
    if horizon_ns <= 0 || period_ns <= 0 {
        return None;
    }
    // Entry: the last bar closed at or before the forecast was made.
    let after = bars.partition_point(|b| b.ts_ns <= t0_ns);
    let entry = bars.get(after.checked_sub(1)?)?;
    // Exit: the first bar closing at or after the horizon, and not a gap away from it.
    let target = t0_ns.checked_add(horizon_ns)?;
    let exit = bars.get(bars.partition_point(|b| b.ts_ns < target))?;
    if exit.ts_ns <= entry.ts_ns || exit.ts_ns - target > period_ns {
        return None;
    }
    if !(entry.close.is_finite() && entry.close > 0.0 && exit.close.is_finite()) {
        return None;
    }
    Some((exit.close / entry.close - 1.0, exit.ts_ns))
}

/// Where closed bars come from. The platform's is the point-in-time store; a test supplies its own.
#[async_trait::async_trait]
pub trait ClosedBars: Send + Sync {
    /// Bars of `instrument` at `tf` whose close falls in `[from, to)`, ascending.
    async fn closes(
        &self,
        instrument: &str,
        tf: Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Close>>;
}

/// The point-in-time bar store.
pub struct StoreBars(pub BarStore);

#[async_trait::async_trait]
impl ClosedBars for StoreBars {
    async fn closes(
        &self,
        instrument: &str,
        tf: Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Close>> {
        let bars = self.0.load_bars(instrument, tf, from, to).await?;
        Ok(bars
            .iter()
            .filter_map(|b| b.close.to_f64().map(|c| Close { ts_ns: b.ts_ns, close: c }))
            .collect())
    }
}

/// No bar store is configured: every read fails, so every due forecast waits.
pub struct NoBars;

#[async_trait::async_trait]
impl ClosedBars for NoBars {
    async fn closes(&self, _: &str, _: Timeframe, _: DateTime<Utc>, _: DateTime<Utc>) -> anyhow::Result<Vec<Close>> {
        anyhow::bail!("no bar store is configured")
    }
}

/// What one pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Realised {
    /// Rows given a `realized` value.
    pub filled: u64,
    /// Due rows whose exit (or entry) bar was not available yet; retried next pass.
    pub waiting_for_bars: u64,
    /// Unrealised rows with no recorded horizon or timeframe; never guessed.
    pub unresolvable: u64,
}

/// Realise every due forecast, at most `limit` rows per pass.
///
/// # Errors
/// A Postgres failure. A bar read that fails is not: those rows are `waiting_for_bars`, and the
/// report says so.
pub async fn realise_pending(
    pg: &PgPool,
    bars: &dyn ClosedBars,
    now: DateTime<Utc>,
    limit: i64,
) -> anyhow::Result<Realised> {
    use backtest::types::TimeframeExt;

    let (unresolvable,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM model_forecasts \
         WHERE realized IS NULL AND (horizon_bars IS NULL OR timeframe IS NULL)",
    )
    .fetch_one(pg)
    .await?;
    let mut out = Realised {
        unresolvable: u64::try_from(unresolvable).unwrap_or(0),
        ..Realised::default()
    };

    let due: Vec<(uuid::Uuid, String, i32, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT forecast_id, instrument_id, horizon_bars, timeframe, created_at \
           FROM model_forecasts \
          WHERE realized IS NULL AND horizon_bars > 0 AND timeframe IS NOT NULL \
            AND instrument_id IS NOT NULL AND created_at > $1 \
          ORDER BY created_at ASC LIMIT $2",
    )
    .bind(now - Duration::days(30))
    .bind(limit)
    .fetch_all(pg)
    .await?;

    let mut groups: std::collections::BTreeMap<(String, String), Vec<(uuid::Uuid, i32, DateTime<Utc>)>> =
        std::collections::BTreeMap::new();
    for (id, inst, h, tf, at) in due {
        groups.entry((inst, tf)).or_default().push((id, h, at));
    }

    for ((instrument, tf_key), rows) in groups {
        let Some(tf) = <Timeframe as TimeframeExt>::from_key(&tf_key) else {
            out.waiting_for_bars += rows.len() as u64;
            continue;
        };
        let period = i64::try_from(<Timeframe as TimeframeExt>::seconds(&tf)).unwrap_or(60);
        let period_ns = period * 1_000_000_000;
        let first = rows.iter().map(|r| r.2).min().unwrap_or(now);
        let last_target = rows
            .iter()
            .map(|r| r.2 + Duration::seconds(period * i64::from(r.1)))
            .max()
            .unwrap_or(now);
        let from = first - Duration::seconds(period * 3);
        let to = (last_target + Duration::seconds(period * 3)).min(now);
        let closes = match bars.closes(&instrument, tf, from, to).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("realising forecasts for {instrument}: bar read failed: {e}");
                out.waiting_for_bars += rows.len() as u64;
                continue;
            }
        };
        for (id, h, at) in rows {
            let t0 = at.timestamp_nanos_opt().unwrap_or(i64::MAX);
            match realised_return(&closes, t0, period_ns * i64::from(h), period_ns) {
                Some((r, exit_ns)) => {
                    sqlx::query("UPDATE model_forecasts SET realized = $2, realized_at = $3 WHERE forecast_id = $1 AND realized IS NULL")
                        .bind(id)
                        .bind(r)
                        .bind(Utc.timestamp_nanos(exit_ns))
                        .execute(pg)
                        .await?;
                    out.filled += 1;
                }
                None => out.waiting_for_bars += 1,
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000_000;

    fn series() -> Vec<Close> {
        // a bar closes every minute; the close at minute m is 100 + m
        (0..120)
            .map(|m| Close {
                ts_ns: m * MIN,
                close: 100.0 + m as f64,
            })
            .collect()
    }

    #[test]
    fn the_return_runs_from_the_last_closed_bar_to_the_horizon() {
        // issued at minute 10.5, horizon 5 bars: entry = close(10), exit = first close at/after 15.5 = minute 16
        let (r, exit) = realised_return(&series(), 10 * MIN + MIN / 2, 5 * MIN, MIN).unwrap();
        assert_eq!(exit, 16 * MIN);
        assert!((r - (116.0 / 110.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn the_entry_never_uses_a_bar_that_closed_after_the_forecast() {
        // issued exactly at a close: that bar is the entry (it had closed by then)
        let (r, _) = realised_return(&series(), 20 * MIN, 5 * MIN, MIN).unwrap();
        assert!((r - (125.0 / 120.0 - 1.0)).abs() < 1e-12);
        // not realised while the horizon has not passed
        assert!(realised_return(&series()[..24], 20 * MIN, 5 * MIN, MIN).is_none());
    }

    #[test]
    fn a_gap_at_the_horizon_leaves_the_forecast_unrealised() {
        let mut s = series();
        s.retain(|b| !(14 * MIN..40 * MIN).contains(&b.ts_ns));
        // target minute 15; next bar closes at minute 40: 25 bars away, more than one period
        assert!(realised_return(&s, 10 * MIN, 5 * MIN, MIN).is_none());
    }

    #[test]
    fn nothing_before_the_first_bar_and_no_zero_horizon() {
        assert!(realised_return(&series(), -5 * MIN, 5 * MIN, MIN).is_none());
        assert!(realised_return(&series(), 10 * MIN, 0, MIN).is_none());
        assert!(realised_return(&[], 10 * MIN, 5 * MIN, MIN).is_none());
    }
}
