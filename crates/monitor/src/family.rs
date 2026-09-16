//! Metric families, and the three observations that are facts rather than estimates.

use serde::{Deserialize, Serialize};

/// The families of COMP-007 §3.
///
/// A closed enum rather than a string, because the precision report (§4.1) is
/// computed *per family* and a misspelled family is a family whose precision is
/// silently never computed — which reads, on the report, exactly like a family that
/// never fired a false positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Fill rate, slippage vs. the fill model, latency, partial-fill share.
    Execution,
    /// Signals per hour, holding period, order rate, cancel ratio, position count.
    Behaviour,
    /// Notional, leverage, concentration — the risk gate's limits, re-checked here
    /// independently, because a gate that grades its own homework is not a check.
    Exposure,
    /// Rolling Sharpe, CUSUM, drawdown vs. the backtest's Monte Carlo p95.
    Pnl,
    /// Last bar age, gap count, impossible bars.
    Data,
    /// Connection state, error rate, rate-limit headroom, reconciliation drift.
    Venue,
    /// API and job-service heartbeats, DB latency, the monitor's own lag.
    Platform,
    /// The lease itself.
    Lease,
}

impl Family {
    /// The string the database CHECK constraints and the API speak.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Behaviour => "behaviour",
            Self::Exposure => "exposure",
            Self::Pnl => "pnl",
            Self::Data => "data",
            Self::Venue => "venue",
            Self::Platform => "platform",
            Self::Lease => "lease",
        }
    }

    /// Every family, so the precision report cannot quietly omit one.
    #[must_use]
    pub fn all() -> [Self; 8] {
        [
            Self::Execution,
            Self::Behaviour,
            Self::Exposure,
            Self::Pnl,
            Self::Data,
            Self::Venue,
            Self::Platform,
            Self::Lease,
        ]
    }

    /// Whether an open maintenance window may suppress this family (COMP-007 §4.3).
    ///
    /// Exactly one family, and the answer is not configurable. A rolling deploy
    /// looks like a heartbeat loss; it does not look like a reconciliation
    /// mismatch, a stale feed or a strategy losing money, and a window that
    /// suppressed those would be a deploy asking the operator to stop believing the
    /// market.
    #[must_use]
    pub fn suppressible_by_maintenance_window(self) -> bool {
        matches!(self, Self::Platform)
    }
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Family {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .into_iter()
            .find(|f| f.as_str() == s)
            .ok_or_else(|| format!("unknown metric family {s:?}"))
    }
}

/// An observation that acts immediately, skipping dwell (COMP-007 §4.1).
///
/// These are facts, not estimates. A reconciliation mismatch is not a noisy sample
/// of a distribution — the book and the venue disagree about what is owned, and
/// three cadences of that is three cadences of trading on numbers known to be
/// wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    /// `crates/reconciliation` found a mismatch at this venue.
    ReconciliationMismatch { venue: String },
    /// The monitor lost, or never had, its lease.
    LeaseLost,
    /// A feed is stale past the hard multiple (default 10× the timeframe).
    HardDataStaleness {
        instrument: String,
        age_secs: i64,
        timeframe_secs: i64,
    },
}

impl Fact {
    #[must_use]
    pub fn family(&self) -> Family {
        match self {
            Self::ReconciliationMismatch { .. } => Family::Venue,
            Self::LeaseLost => Family::Lease,
            Self::HardDataStaleness { .. } => Family::Data,
        }
    }

    /// The human-readable reason recorded on the trip.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::ReconciliationMismatch { venue } => {
                format!("reconciliation mismatch at venue {venue}")
            }
            Self::LeaseLost => "monitor lease lost".to_owned(),
            Self::HardDataStaleness {
                instrument,
                age_secs,
                timeframe_secs,
            } => format!(
                "{instrument} last bar is {age_secs}s old, \
                 past the hard {}x multiple of its {timeframe_secs}s timeframe",
                crate::evaluate::HARD_STALENESS_MULTIPLE
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn every_family_round_trips_through_its_string() {
        for f in Family::all() {
            assert_eq!(Family::from_str(f.as_str()), Ok(f));
        }
    }

    #[test]
    fn only_platform_health_is_suppressible_by_a_window() {
        // COMP-007 §4.3. Asserted over the whole enum rather than for Platform
        // alone, so adding a family and forgetting this question fails here
        // instead of quietly becoming suppressible or quietly not being.
        let suppressible: Vec<Family> = Family::all()
            .into_iter()
            .filter(|f| f.suppressible_by_maintenance_window())
            .collect();
        assert_eq!(suppressible, vec![Family::Platform]);
    }

    #[test]
    fn a_fact_names_the_family_that_owns_it() {
        assert_eq!(Fact::LeaseLost.family(), Family::Lease);
        assert_eq!(
            Fact::ReconciliationMismatch { venue: "kraken".into() }.family(),
            Family::Venue
        );
        assert_eq!(
            Fact::HardDataStaleness {
                instrument: "BTC-USD".into(),
                age_secs: 700,
                timeframe_secs: 60,
            }
            .family(),
            Family::Data
        );
    }

    #[test]
    fn a_reason_names_the_thing_that_went_wrong() {
        // The operator's first question is always "why", and a reason that does
        // not name the instrument or the venue does not answer it.
        let r = Fact::HardDataStaleness {
            instrument: "BTC-USD".into(),
            age_secs: 700,
            timeframe_secs: 60,
        }
        .reason();
        assert!(r.contains("BTC-USD"), "{r}");
        assert!(r.contains("700"), "{r}");
    }
}
