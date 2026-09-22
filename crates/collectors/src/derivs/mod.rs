//! Derivatives reference series: funding rates and open interest (DATA-005 §11 S6).
//!
//! These are **forward-collect only**. Under a free-only data policy (ADR-0027)
//! there is no vendor selling last year's open interest, and there never will be.
//! Every hour this collector is not running is an hour that is permanently missing
//! from every study the platform ever runs on the series — `docs/artifact.md`
//! FM-11.
//!
//! Funding is the partial exception and the exception proves the rule: Kraken
//! publishes about a rolling year of hourly settlements, so funding can be
//! backfilled *once, to a year*, and the window rolls. Open interest has no history
//! endpoint at all.

pub mod funding;
pub mod service;
pub mod sink;

pub use service::{ForwardCollector, PollReport, RunReport, ServiceError, TickerSource};
pub use sink::{Beat, ClickHouseForwardSink, ForwardSink, SinkError, SourceIds};

pub use funding::{
    FundingKind, FundingObservation, KrakenFuturesClient, OpenInterestObservation, TickerSnapshot,
};
