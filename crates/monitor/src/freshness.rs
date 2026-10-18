//! The data family's input: how old each live feed's last bar is (COMP-007 §3;
//! S-7.5).
//!
//! # Who measures, and why it is not this crate
//!
//! The bars live in ClickHouse and this crate has no client for it and must not
//! grow one: its Postgres-only dependency set is what makes it a separate failure
//! domain. So the platform process — which *observes* each completed bar as it
//! publishes it to the strategies — writes one gauge row per feed into
//! `monitor.feed_freshness` (migration 0165), through [`MonitorStore::observe_bar`]
//! and [`MonitorStore::affirm_feeds`]. Those two functions are the only writers;
//! a writer that did not see the bar would be guessing its age.
//!
//! # Three answers that are not "stale"
//!
//! [`judge`] refuses to call a feed stale in two cases where the arithmetic would:
//!
//! - **The session is shut.** A US equity feed is silent every night and every
//!   weekend. Halting every equity strategy at 16:10 New York time and requiring
//!   a typed human reset each morning is the "safety mechanism becomes a habit of
//!   ignoring it" failure §4.3 exists to prevent. The producer knows the asset
//!   class and says whether bars are expected; this crate believes it.
//! - **The producer went quiet.** If the gauge itself has not been affirmed
//!   recently, the age it holds is not a measurement of the feed: it is the age
//!   of the last time anyone looked. The platform being down is the lease's
//!   business, and judging every feed hard-stale on top would give one fault two
//!   verdicts.
//!
//! Both are reported as what they are — [`FeedReading::SessionClosed`] and
//! [`FeedReading::ProducerSilent`] — and neither is `Fresh`.
//!
//! # The dependency edge
//!
//! A v3 deployment trades one instrument and decides on the bars of *the* live
//! pipeline for it (`apps/platform/src/live_bars.rs`), which runs at exactly one
//! venue. The producer writes the row under that venue, so the edge from a
//! deployment to its feed is `strategy_deployment.instrument_id == symbol`, and
//! the [`FeedKey`] carries the venue the bars actually came from — not the
//! deployment's `platform_paper` execution venue, which prices nothing.

use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::evaluate::{staleness, DependencyMap, Staleness};
use crate::feed::{FeedKey, Venue};
use crate::store::{MonitorStore, StoreError};

/// The timeframe of every live feed today: the 1-minute bars the pipelines build.
pub const LIVE_FEED_TIMEFRAME_SECS: i32 = 60;

/// How recently the producer must have affirmed a gauge row for its age to count
/// as a measurement.
///
/// Five minutes: the producer affirms every thirty seconds, so this is ten missed
/// affirmations — long enough that a slow tick is not a verdict, short enough
/// that a dead producer is noticed long before a 1-minute feed reaches its
/// ten-minute hard multiple on stale information.
pub const PRODUCER_SILENCE_SECS: i64 = 300;

/// One row of `monitor.feed_freshness`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedRow {
    pub feed: FeedKey,
    pub timeframe_secs: i64,
    pub last_bar_at: DateTime<Utc>,
    pub session_open: bool,
    pub observed_at: DateTime<Utc>,
}

/// A running deployment, as the monitor needs it: what it is called on its
/// orders, and what it trades.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningDeployment {
    /// `strategy_deployment.deployment_id`, which is the `strategy_id` a v3
    /// deployment's orders carry (`OrderIntent::new(.., Some(deployment_id))`),
    /// and so the id the order gate's `strategy:` scope is keyed on.
    pub strategy_id: String,
    pub instrument_id: String,
    pub environment: String,
    pub started_at: DateTime<Utc>,
}

/// What one gauge row means now.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedReading {
    /// The producer is live and the session is open, so the age is a
    /// measurement. [`Staleness`] says what it means.
    Judged(Staleness),
    /// Bars are not expected now. No verdict, and not a good one either.
    SessionClosed(FeedKey),
    /// The gauge has not been affirmed for `silent_secs`: its age is the age of
    /// the last look, not of the feed.
    ProducerSilent { feed: FeedKey, silent_secs: i64 },
}

/// Judge one gauge row at `now`.
#[must_use]
pub fn judge(row: &FeedRow, now: DateTime<Utc>) -> FeedReading {
    let silent_secs = (now - row.observed_at).num_seconds();
    if silent_secs > PRODUCER_SILENCE_SECS {
        return FeedReading::ProducerSilent {
            feed: row.feed.clone(),
            silent_secs,
        };
    }
    if !row.session_open {
        return FeedReading::SessionClosed(row.feed.clone());
    }
    // A bar stamped in the future (clock skew between the producer and here) is
    // an age of zero, not a negative age: negative would pass every threshold
    // and so would read as the freshest feed on the platform.
    let age = (now - row.last_bar_at).num_seconds().max(0);
    FeedReading::Judged(staleness(&row.feed, Some(age), row.timeframe_secs))
}

/// The edges from running deployments to the feeds they decide on.
///
/// Built from rows rather than from configuration, so a deployment started a
/// minute ago is covered on the next cadence and a stopped one is not.
#[must_use]
pub fn dependencies(deployments: &[RunningDeployment], feeds: &[FeedRow]) -> DependencyMap {
    let mut deps = DependencyMap::new();
    for d in deployments {
        for f in feeds.iter().filter(|f| f.feed.symbol() == d.instrument_id) {
            deps.add(&f.feed, &d.strategy_id);
        }
    }
    deps
}

/// Deployments with no gauge row for their instrument at all.
///
/// Not an error and not fine: nothing measures the data those strategies decide
/// on, so the data family is blind to them. Reported so it is said out loud.
#[must_use]
pub fn unwatched<'a>(
    deployments: &'a [RunningDeployment],
    feeds: &[FeedRow],
) -> Vec<&'a RunningDeployment> {
    deployments
        .iter()
        .filter(|d| !feeds.iter().any(|f| f.feed.symbol() == d.instrument_id))
        .collect()
}

impl MonitorStore {
    /// Every gauge row.
    ///
    /// # Errors
    /// The database, or a row whose key the schema should have made impossible.
    pub async fn feed_rows(&self) -> Result<Vec<FeedRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT symbol, venue, timeframe_secs, last_bar_at, session_open, observed_at
               FROM monitor.feed_freshness
              ORDER BY symbol, venue, timeframe_secs",
        )
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let symbol: String = r.get("symbol");
            let venue: String = r.get("venue");
            let venue = Venue::named(&venue).map_err(|e| StoreError::Invalid(e.to_string()))?;
            let feed = FeedKey::new(&symbol, venue).map_err(|e| StoreError::Invalid(e.to_string()))?;
            let tf: i32 = r.get("timeframe_secs");
            out.push(FeedRow {
                feed,
                timeframe_secs: i64::from(tf),
                last_bar_at: r.get("last_bar_at"),
                session_open: r.get("session_open"),
                observed_at: r.get("observed_at"),
            });
        }
        Ok(out)
    }

    /// Record that a bar closing at `bar_close` arrived on `feed`. The producer's
    /// write, once per bar it observed.
    ///
    /// `GREATEST` so a late, out-of-order bar can never make a feed look older
    /// than the newest bar already seen.
    ///
    /// # Errors
    /// The database, or a non-positive timeframe.
    pub async fn observe_bar(
        &self,
        feed: &FeedKey,
        timeframe_secs: i32,
        bar_close: DateTime<Utc>,
        session_open: bool,
    ) -> Result<(), StoreError> {
        if timeframe_secs <= 0 {
            return Err(StoreError::Invalid(format!(
                "a feed's timeframe must be positive, got {timeframe_secs}"
            )));
        }
        sqlx::query(
            "INSERT INTO monitor.feed_freshness
               (symbol, venue, timeframe_secs, last_bar_at, session_open, observed_at)
             VALUES ($1, $2, $3, $4, $5, now())
             ON CONFLICT (symbol, venue, timeframe_secs) DO UPDATE
               SET last_bar_at  = GREATEST(monitor.feed_freshness.last_bar_at, EXCLUDED.last_bar_at),
                   session_open = EXCLUDED.session_open,
                   observed_at  = now()",
        )
        .bind(feed.symbol())
        .bind(feed.venue().as_str())
        .bind(timeframe_secs)
        .bind(bar_close)
        .bind(session_open)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The producer's heartbeat on its feeds: "I am still looking, and this is
    /// whether bars are expected now". Touches only rows that exist — a feed
    /// nobody has seen a bar from has no age to affirm.
    ///
    /// # Errors
    /// The database.
    pub async fn affirm_feeds(&self, sessions: &[(FeedKey, bool)]) -> Result<u64, StoreError> {
        let mut touched = 0;
        for (feed, open) in sessions {
            touched += sqlx::query(
                "UPDATE monitor.feed_freshness
                    SET session_open = $3, observed_at = now()
                  WHERE symbol = $1 AND venue = $2",
            )
            .bind(feed.symbol())
            .bind(feed.venue().as_str())
            .bind(*open)
            .execute(self.pool())
            .await?
            .rows_affected();
        }
        Ok(touched)
    }

    /// Every deployment that is running now (`strategy_deployment`, 0142).
    ///
    /// # Errors
    /// The database.
    pub async fn running_deployments(&self) -> Result<Vec<RunningDeployment>, StoreError> {
        let rows = sqlx::query(
            "SELECT deployment_id::text AS strategy_id, instrument_id, environment, started_at
               FROM strategy_deployment
              WHERE stopped_at IS NULL
              ORDER BY started_at, deployment_id",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| RunningDeployment {
                strategy_id: r.get("strategy_id"),
                instrument_id: r.get("instrument_id"),
                environment: r.get("environment"),
                started_at: r.get("started_at"),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::Fact;
    use chrono::Duration;

    fn feed(symbol: &str, venue: &str) -> FeedKey {
        FeedKey::new(symbol, Venue::named(venue).expect("venue")).expect("feed")
    }

    fn row(age_secs: i64, open: bool, affirmed_secs_ago: i64, now: DateTime<Utc>) -> FeedRow {
        FeedRow {
            feed: feed("BTC-USD", "kraken"),
            timeframe_secs: 60,
            last_bar_at: now - Duration::seconds(age_secs),
            session_open: open,
            observed_at: now - Duration::seconds(affirmed_secs_ago),
        }
    }

    fn deployment(id: &str, instrument: &str) -> RunningDeployment {
        RunningDeployment {
            strategy_id: id.to_owned(),
            instrument_id: instrument.to_owned(),
            environment: "paper".to_owned(),
            started_at: Utc::now(),
        }
    }

    #[test]
    fn a_feed_past_ten_timeframes_is_a_hard_fact_naming_the_feed_and_the_age() {
        let now = Utc::now();
        match judge(&row(700, true, 10, now), now) {
            FeedReading::Judged(Staleness::Hard(Fact::HardDataStaleness {
                feed: f,
                age_secs,
                timeframe_secs,
            })) => {
                assert_eq!(f, feed("BTC-USD", "kraken"));
                assert_eq!(age_secs, 700);
                assert_eq!(timeframe_secs, 60);
            }
            other => panic!("expected a hard staleness fact, got {other:?}"),
        }
    }

    #[test]
    fn a_shut_session_is_not_stale() {
        let now = Utc::now();
        assert_eq!(
            judge(&row(100_000, false, 10, now), now),
            FeedReading::SessionClosed(feed("BTC-USD", "kraken"))
        );
    }

    #[test]
    fn a_silent_producer_is_unmeasured_not_stale() {
        let now = Utc::now();
        match judge(&row(100_000, true, 3_600, now), now) {
            FeedReading::ProducerSilent { silent_secs, .. } => assert_eq!(silent_secs, 3_600),
            other => panic!("a gauge nobody affirmed must not be judged: {other:?}"),
        }
    }

    #[test]
    fn a_future_bar_is_fresh_not_negative() {
        let now = Utc::now();
        assert_eq!(
            judge(&row(-30, true, 0, now), now),
            FeedReading::Judged(Staleness::Fresh)
        );
    }

    #[test]
    fn a_stale_feed_reaches_only_the_deployments_on_its_instrument() {
        let now = Utc::now();
        let feeds = vec![
            row(700, true, 10, now),
            FeedRow {
                feed: feed("SPY", "alpaca"),
                ..row(10, true, 10, now)
            },
        ];
        let deps = dependencies(
            &[deployment("d-btc", "BTC-USD"), deployment("d-spy", "SPY")],
            &feeds,
        );
        assert_eq!(deps.strategies_for(&feed("BTC-USD", "kraken")), ["d-btc"]);
        assert_eq!(deps.strategies_for(&feed("SPY", "alpaca")), ["d-spy"]);
        assert!(deps.strategies_for(&feed("BTC-USD", "coinbase")).is_empty());
    }

    #[test]
    fn a_deployment_with_no_gauge_is_reported_unwatched() {
        let now = Utc::now();
        let deployments = [deployment("d-eth", "ETH-USD"), deployment("d-btc", "BTC-USD")];
        let blind = unwatched(&deployments, &[row(10, true, 10, now)]);
        assert_eq!(blind.len(), 1);
        assert_eq!(blind[0].strategy_id, "d-eth");
    }
}
