//! The production entry point for the forward derivatives collector (P-0.3).
//!
//! [`ForwardCollector`] could poll and [`ClickHouseForwardSink`] could write, and
//! nothing in any binary constructed either: on 2026-10-03 `trading.funding_rates`
//! and `trading.open_interest` held **zero rows**, while `dataplane.forward_series`
//! had recorded `collecting_since = 2026-09-21` for both. Twelve days of a series
//! that can never be backfilled (FM-11), lost to a missing caller.
//!
//! This module is the one function a process calls to start the clock. It takes
//! the source catalogue as an argument rather than reading Postgres itself, so the
//! collectors crate stays free of a database client, and so the caller — which
//! already holds the platform's pool — is the one that resolves ids from
//! `dataplane.source` (the only place they are defined).
//!
//! # Cadence
//!
//! [`DEFAULT_POLL_SECS`] is the venue's settlement interval. One `/tickers` request
//! covers every perpetual, so an hourly poll is one request an hour against a limit
//! of hundreds — frugal by a wide margin, which is the posture Kraken's terms call
//! for (see `funding.rs`). The registry declares `cadence_secs = 3600` and alarms
//! after three missed cadences; polling faster than the cadence is allowed and
//! polling slower is not, which [`poll_interval`] enforces.

use std::time::Duration;

use tracing::info;

use super::funding::KrakenFuturesClient;
use super::service::{ForwardCollector, RunReport};
use super::sink::{ClickHouseForwardSink, SourceIds};

/// The registry's declared cadence for both Kraken Futures series.
pub const DEFAULT_POLL_SECS: u64 = 3600;

/// The two sources this collector writes under. Both must resolve, or the
/// collector refuses to start rather than writing rows it cannot attribute.
pub const REQUIRED_SOURCES: &[&str] = &["kraken_futures_rest", "kraken_futures_history"];

/// Why the collector would not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// A declared source is missing from the catalogue handed in.
    #[error(
        "source {0:?} is not in the catalogue; refusing to start a collector whose rows cannot be attributed"
    )]
    MissingSource(String),
    /// The requested poll interval is slower than the registry's cadence, which
    /// would make the silence alarm fire on a healthy collector.
    #[error("poll every {requested}s is slower than the declared cadence of {cadence}s")]
    TooSlow { requested: u64, cadence: u64 },
    /// The HTTP client could not be built.
    #[error("client: {0}")]
    Client(String),
}

/// Validate a requested poll interval against the declared cadence.
///
/// # Errors
/// [`StartError::TooSlow`] when `secs` exceeds [`DEFAULT_POLL_SECS`], or is zero.
pub fn poll_interval(secs: u64) -> Result<Duration, StartError> {
    if secs == 0 || secs > DEFAULT_POLL_SECS {
        return Err(StartError::TooSlow {
            requested: secs,
            cadence: DEFAULT_POLL_SECS,
        });
    }
    Ok(Duration::from_secs(secs))
}

/// Check the catalogue holds every source this collector writes.
///
/// # Errors
/// [`StartError::MissingSource`] naming the first absent one.
pub fn check_sources(sources: &SourceIds) -> Result<(), StartError> {
    for name in REQUIRED_SOURCES {
        sources
            .get(name)
            .map_err(|_| StartError::MissingSource((*name).to_string()))?;
    }
    Ok(())
}

/// Build the production collector: the live venue client over the ClickHouse sink.
///
/// # Errors
/// A missing source, or an HTTP client that would not build.
pub fn build(
    ch_url: &str,
    sources: SourceIds,
) -> Result<ForwardCollector<KrakenFuturesClient, ClickHouseForwardSink>, StartError> {
    check_sources(&sources)?;
    let client = KrakenFuturesClient::new().map_err(|e| StartError::Client(e.to_string()))?;
    Ok(ForwardCollector::new(
        client,
        ClickHouseForwardSink::new(ch_url, sources),
    ))
}

/// Run the forward collector until `shutdown` resolves.
///
/// # Errors
/// As [`build`] and [`poll_interval`]. Once running it does not return an error: a
/// failed tick is logged, counted, and the next one is attempted.
pub async fn run(
    ch_url: &str,
    sources: SourceIds,
    every: Duration,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> Result<RunReport, StartError> {
    let every = poll_interval(every.as_secs())?;
    let collector = build(ch_url, sources)?;
    info!(
        venue = super::funding::VENUE,
        every_secs = every.as_secs(),
        "forward derivatives collector started"
    );
    Ok(collector.run(every, shutdown).await)
}

/// Spawn the collector on the current runtime for the life of the process.
///
/// This is what `apps/platform` calls. It validates before spawning, so a missing
/// source is an error at boot rather than a task that died silently — the exact
/// failure this series cannot afford.
///
/// # Errors
/// As [`build`].
pub fn spawn(
    ch_url: String,
    sources: SourceIds,
) -> Result<tokio::task::JoinHandle<RunReport>, StartError> {
    let collector = build(&ch_url, sources)?;
    let every = Duration::from_secs(DEFAULT_POLL_SECS);
    info!(
        venue = super::funding::VENUE,
        every_secs = DEFAULT_POLL_SECS,
        "forward derivatives collector spawned"
    );
    Ok(tokio::spawn(async move {
        collector.run(every, std::future::pending::<()>()).await
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> SourceIds {
        SourceIds::from_pairs(&[("kraken_futures_rest", 17), ("kraken_futures_history", 18)])
    }

    #[test]
    fn a_catalogue_missing_either_source_refuses_to_start() {
        let only_one = SourceIds::from_pairs(&[("kraken_futures_rest", 17)]);
        let err = check_sources(&only_one).unwrap_err();
        assert!(
            matches!(&err, StartError::MissingSource(s) if s == "kraken_futures_history"),
            "{err}"
        );
        assert!(check_sources(&SourceIds::default()).is_err());
        check_sources(&full()).unwrap();
    }

    #[test]
    fn build_refuses_before_constructing_a_client_when_a_source_is_missing() {
        let r = build("http://127.0.0.1:1/trading", SourceIds::default());
        assert!(matches!(r, Err(StartError::MissingSource(_))));
    }

    /// The registry alarms after three missed cadences. A collector polling slower
    /// than the cadence would trip that alarm while healthy, and an alarm that
    /// fires on health is the one that gets muted.
    #[test]
    fn polling_slower_than_the_declared_cadence_is_refused() {
        assert!(poll_interval(DEFAULT_POLL_SECS).is_ok());
        assert!(poll_interval(900).is_ok());
        assert!(matches!(
            poll_interval(DEFAULT_POLL_SECS + 1),
            Err(StartError::TooSlow { .. })
        ));
        assert!(poll_interval(0).is_err());
    }
}
