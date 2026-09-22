//! Metric families, and the three observations that are facts rather than estimates.

use serde::{Deserialize, Serialize};

use crate::feed::{FeedKey, Venue};

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

    /// Whether a sustained breach in this family **halts** the strategy rather
    /// than scaling it to [`SCALE_DOWN_PCT`](crate::SCALE_DOWN_PCT) (COMP-007
    /// §3, §4; ADR-D-11).
    ///
    /// The single authority for the question. [`state::on_sustained`][s]
    /// branches on it, [`service`][sv]'s cadence ranks a mixed cadence by it,
    /// and the tests read it instead of restating a list — because a second
    /// list is exactly how COMP-007 came to say *halt* in §3's Data row and
    /// *scale down* in §4's response table at the same time, and how a test
    /// named `a_hard_stale_feed_halts_...` came to assert `ScaledDown`.
    ///
    /// Two families answer yes, and it is one reason said twice.
    ///
    /// **P&L**: a CUSUM trip, or a drawdown past the backtest's Monte Carlo
    /// p95, is not "this is noisy today" — it is the distribution having
    /// changed, which is the case size reduction does not fix.
    ///
    /// **Data**: scaling to a quarter *still trades on a stale price*. Every
    /// other response in §4's table reduces exposure to a risk that is being
    /// **measured**; a feed more than
    /// [`HARD_STALENESS_MULTIPLE`](crate::evaluate::HARD_STALENESS_MULTIPLE)
    /// times its timeframe stale is an **absent measurement**, and a
    /// quarter-size position priced off a dead feed is the same error at 25% of
    /// the size.
    ///
    /// There is no configuration here, and that is the decision, not an
    /// omission: a flag reading "data staleness does not halt" is the failure
    /// ADR-D-11 closes.
    ///
    /// [s]: crate::state::on_sustained
    /// [sv]: crate::service
    #[must_use]
    pub fn breach_halts(self) -> bool {
        matches!(self, Self::Pnl | Self::Data)
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
    ReconciliationMismatch { venue: Venue },
    /// The monitor lost, or never had, its lease.
    LeaseLost,
    /// A feed is stale past the hard multiple — ten times its own timeframe,
    /// from [`HARD_STALENESS_MULTIPLE`](crate::evaluate::HARD_STALENESS_MULTIPLE).
    /// Not a default and not configurable: a switch that let this stop halting
    /// is the failure ADR-D-11 closes.
    ///
    /// The subject is a [`FeedKey`] and not a symbol. `BTC-USD` is three feeds
    /// in the live database — coinbase, kraken and binance — and on 2026-09-21
    /// their last 60-second bars were 90 minutes, 92 seconds and 91 days old. A
    /// fact naming only the symbol would have to pick one of those ages, and
    /// since ADR-D-11 made this a halt, picking the stale one stops strategies
    /// whose feed is fine while picking the fresh one leaves a strategy pricing
    /// off a feed that died in June.
    HardDataStaleness {
        feed: FeedKey,
        age_secs: i64,
        timeframe_secs: i64,
    },
}

/// The `Fact` variants as a closed list, so the wiring guard can iterate them
/// without inventing field values for each.
///
/// [`Fact::kind`] is an exhaustive match, so a new `Fact` variant does not
/// compile until it gains a kind here — and therefore until somebody answers
/// [`FactKind::wiring`] for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactKind {
    ReconciliationMismatch,
    LeaseLost,
    HardDataStaleness,
}

/// Whether anything in a *running platform* can raise a fact.
///
/// Not a doc line and not a comment. COMP-007 §4.1 says a reconciliation
/// mismatch, a lost lease and hard staleness "act immediately"; on 2026-09-21
/// that sentence was true in the type system, true in nine tests, and did
/// nothing at all in production, because no non-test code raised any of the
/// three. A sentence cannot notice that about itself. An exhaustive match can,
/// and `facts_are_wired` asserts the [`Wiring::Unwired`] answers only ever
/// shrink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wiring {
    /// Raised in production by this function, named as a `::` path.
    RaisedBy(&'static str),
    /// Nothing in a running platform raises it.
    Unwired {
        /// Why not — the obstacle, not an apology.
        because: &'static str,
        /// The exact function that has to raise it, named as a `::` path. A
        /// blocker with no named caller is a blocker nobody can clear.
        needs: &'static str,
    },
}

impl Wiring {
    /// Whether a running platform can raise the fact this describes.
    #[must_use]
    pub fn is_raised_in_production(self) -> bool {
        matches!(self, Self::RaisedBy(_))
    }
}

impl FactKind {
    /// Every kind, so the guard cannot quietly scan a shorter list than the enum.
    #[must_use]
    pub fn all() -> [Self; 3] {
        [
            Self::ReconciliationMismatch,
            Self::LeaseLost,
            Self::HardDataStaleness,
        ]
    }

    /// The variant's Rust name, which is also what the guard greps for.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReconciliationMismatch => "ReconciliationMismatch",
            Self::LeaseLost => "LeaseLost",
            Self::HardDataStaleness => "HardDataStaleness",
        }
    }

    /// Who raises this fact in a running platform, or what would have to.
    #[must_use]
    pub fn wiring(self) -> Wiring {
        match self {
            // Found by `crates/reconciliation`, which runs inside the platform
            // process. Today `check_divergence` calls `KillSwitch::trip_scope`
            // and writes no row — and `LeaseWatcher::refresh` calls
            // `replace_halted`, which *replaces* the scoped view from
            // `monitor.kill_switch`. So that halt is not merely unrecorded: it is
            // erased within one refresh cadence and the venue resumes trading.
            Self::ReconciliationMismatch => Wiring::Unwired {
                because: concat!(
                    "the mismatch is detected in crates/reconciliation, which this ",
                    "crate may not edit; it trips the in-memory KillSwitch only, and ",
                    "KillSwitch::replace_halted erases that trip on the next refresh ",
                    "because no monitor.kill_switch row backs it",
                ),
                needs: concat!(
                    "reconciliation::divergence::check_divergence must replace its ",
                    "`kill_switch.trip_scope(&Scope::venue(v))` with an await on ",
                    "monitor::store::MonitorStore::carry_out_mismatch(v, kill_switch), ",
                    "which writes the durable row first and trips the mirror second; ",
                    "crates/reconciliation/Cargo.toml must gain `monitor = { path = ",
                    "\"../monitor\" }` as crates/api does (ADR-S7-01, no root manifest ",
                    "edit), check_divergence must become async, and its callers must ",
                    "hand it the MonitorStore they already hold",
                ),
            },
            Self::LeaseLost => Wiring::RaisedBy("monitor::service::LeaseWatcher::refresh"),
            // `evaluate::staleness` builds this fact correctly and nothing calls
            // it, because nothing can: the age of the last bar lives in
            // ClickHouse and this crate deliberately has no client for it. Its
            // Postgres-only dependency set is what keeps it a separate failure
            // domain, so the answer is a producer, not a second database handle.
            //
            // Checked against the running platform on 2026-09-21 rather than
            // inferred, because the three candidate sources a reader would reach
            // for all look plausible from the source and are all wrong from the
            // database. The findings are in `because`; none of them is a table
            // this crate could start reading tomorrow.
            Self::HardDataStaleness => Wiring::Unwired {
                because: concat!(
                    "no table in Postgres records that a bar arrived. The age of an ",
                    "instrument's last bar lives only in ClickHouse `trading.market_bar`, ",
                    "written by the platform's live 1m persister, and crates/monitor has ",
                    "no client for it and must not grow one — its Postgres-only ",
                    "dependency set is what makes it a separate failure domain, and a ",
                    "monitor that could not reach ClickHouse would be a monitor unable to ",
                    "measure staleness at the moment staleness is most likely. Three ",
                    "Postgres columns look like the answer and are not: (1) ",
                    "dataplane.data_coverage_census.last_bar holds 0 rows, its only writer ",
                    "backtest::census::take_census/persist has no caller anywhere in the ",
                    "workspace, and migration 0097 makes it an append-only series keyed on ",
                    "`as_of` with an immutability trigger — so `now() - last_bar` ",
                    "over-states age by the whole interval since the snapshot, which is ",
                    "the false-halt direction; (2) dataplane.instrument.last_seen is NULL ",
                    "in all 10 rows and nothing in Rust writes it, though migration 0046 ",
                    "grants UPDATE (last_seen) to app_role; (3) monitor.heartbeat already ",
                    "carries (instance, beat_at, cadence_ms) and needs no migration, but ",
                    "it is read by component_beats/stale_components into Family::Platform, ",
                    "which IS suppressible by a maintenance window and scales rather than ",
                    "halts — beating feeds into it would give one condition two verdicts ",
                    "and let a deploy window suppress the weaker one",
                ),
                needs: concat!(
                    "three things, in this order. FIRST a migration (this crate must not ",
                    "write one) creating monitor.feed_freshness as a GAUGE, not a census: ",
                    "PRIMARY KEY (instrument_id, venue_id), last_bar_at TIMESTAMPTZ NOT ",
                    "NULL, timeframe_secs INT NOT NULL CHECK (timeframe_secs > 0) — the ",
                    "CHECK is what keeps the undeclared-timeframe case out of the gauge ",
                    "rather than a comment saying it should be — observed_at TIMESTAMPTZ ",
                    "NOT NULL DEFAULT now(), and GRANT SELECT TO app_role plus GRANT ",
                    "INSERT, UPDATE (last_bar_at, observed_at) TO app_role. SECOND the ",
                    "producer: apps/platform/src/bar_persist.rs::run_bar_persist is the ",
                    "single writer of live bars and already holds every column ",
                    "(instrument_id, venue_id, bar.available_time, Timeframe::Minutes1) ",
                    "per bar, so it upserts one row per bar it persists and measures the ",
                    "age rather than inferring it — no other caller may write this table, ",
                    "because a writer that did not observe the bar is guessing. THIRD, ",
                    "here: monitor::service::MonitorService::collect_cadence reads the ",
                    "gauge, passes Some(age) or None to monitor::evaluate::staleness, and ",
                    "routes a Staleness::Hard through monitor::facts::decide and ",
                    "monitor::store::MonitorStore::carry_out_fact. The keying half of ",
                    "the third step is DONE and is no longer the blocker it was: ",
                    "Fact::HardDataStaleness and DependencyMap are keyed on the (symbol, ",
                    "venue) pair monitor::feed::FeedKey::new builds, which has no ",
                    "constructor from a single string and whose venue half is its own ",
                    "type so the two cannot be transposed; the re-keying was needed ",
                    "because in the live database BTC-USD is three ",
                    "feeds — dataplane instrument_ids 1005, 1007 and 1008 at venues ",
                    "coinbase, kraken and binance — whose last 60s bars were 90 minutes, ",
                    "92 seconds and 91 days old at one instant on 2026-09-21. TWO joins ",
                    "remain, and only the second needs anybody's permission. FIRST, the ",
                    "gauge's key is numeric and a FeedKey is not: the reader translates ",
                    "with `SELECT s.symbol, v.name FROM dataplane.instrument_symbol s ",
                    "JOIN dataplane.venue v USING (venue_id) WHERE s.valid_to IS NULL`, ",
                    "which is in Postgres, is already populated for all ten instruments, ",
                    "and is keyed (instrument_id, venue_id, valid_from, knowledge_time) ",
                    "so the as-of read stays point-in-time; NOT through ",
                    "instruments.instrument_key, which is NULL for BTC-USD. SECOND, the ",
                    "dependency edge cannot yet express what the fact now says: it is ",
                    "strategy_instances (strategy_id, instrument_id) -> instruments, ",
                    "strategy_instances holds 0 rows, and instruments is PRIMARY KEY ",
                    "(instrument_id) over a bare symbol with venue_id as an ordinary ",
                    "column — so exactly one BTC-USD row can exist and it is pinned to ",
                    "coinbase, while kraken is the venue whose feed is actually fresh. ",
                    "Until a migration re-keys instruments on (instrument_id, venue_id), ",
                    "or strategy_instances gains its own venue column, every edge ",
                    "monitor::evaluate::DependencyMap::add could be given names a venue ",
                    "the strategy was never declared to trade. Building the map from ",
                    "instruments.venue_id today is a guess with a type around it",
                ),
            },
        }
    }
}

impl Fact {
    /// Which kind this is. Exhaustive, so a new variant fails to compile until
    /// [`FactKind`] and therefore [`FactKind::wiring`] have an answer for it.
    #[must_use]
    pub fn kind(&self) -> FactKind {
        match self {
            Self::ReconciliationMismatch { .. } => FactKind::ReconciliationMismatch,
            Self::LeaseLost => FactKind::LeaseLost,
            Self::HardDataStaleness { .. } => FactKind::HardDataStaleness,
        }
    }

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
            // The venue is in the sentence, not merely in the key. An operator
            // told "BTC-USD last bar is 700s old" while three BTC-USD feeds are
            // running, one of them 92 seconds old, has been told something that
            // is false of two of them — and the first thing they will do is go
            // and look at the wrong feed.
            Self::HardDataStaleness {
                feed,
                age_secs,
                timeframe_secs,
            } => format!(
                "{feed} last bar is {age_secs}s old, \
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

    /// The feed the live database's three BTC-USD rows disagree about.
    ///
    /// Through the real constructor rather than a literal, so a test cannot
    /// build a key production could not.
    fn feed(symbol: &str, venue: &str) -> FeedKey {
        FeedKey::new(symbol, Venue::named(venue).expect("a venue")).expect("a feed")
    }

    fn stale(symbol: &str, venue: &str) -> Fact {
        Fact::HardDataStaleness {
            feed: feed(symbol, venue),
            age_secs: 700,
            timeframe_secs: 60,
        }
    }

    fn venue(name: &str) -> Venue {
        Venue::named(name).expect("a venue")
    }

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
    fn exactly_the_pnl_and_data_families_halt_on_a_breach() {
        // ADR-D-11, and the one place the answer is written down. Asserted over
        // the whole enum rather than family by family, so a family added later
        // has to answer the question here instead of silently inheriting
        // "scales down" — and so `state::on_sustained` and `service`'s ranking,
        // which both read `breach_halts`, cannot drift apart from it or from
        // each other.
        let halting: Vec<Family> = Family::all()
            .into_iter()
            .filter(|f| f.breach_halts())
            .collect();
        assert_eq!(halting, vec![Family::Pnl, Family::Data]);
    }

    #[test]
    fn the_family_of_a_hard_stale_feed_is_a_family_that_halts() {
        // The join between the two halves of ADR-D-11: §3's Data row says a
        // feed past the hard multiple halts, and it can only do that if the
        // family the fact reports under is one that halts. Stated as a link
        // between two functions rather than as `Family::Data`, so renaming the
        // family does not quietly break the rule.
        let fact = stale("BTC-USD", "kraken");
        assert!(
            fact.family().breach_halts(),
            "a quarter-size position priced off a dead feed is the same error at 25% of the size"
        );
    }

    #[test]
    fn a_fact_names_the_family_that_owns_it() {
        assert_eq!(Fact::LeaseLost.family(), Family::Lease);
        assert_eq!(
            Fact::ReconciliationMismatch { venue: venue("kraken") }.family(),
            Family::Venue
        );
        assert_eq!(stale("BTC-USD", "kraken").family(), Family::Data);
    }

    #[test]
    fn every_fact_carries_the_kind_that_owns_its_wiring_answer() {
        // The coupling that makes the guard's question unavoidable: a new `Fact`
        // variant does not compile until `kind` has an arm, and an arm needs a
        // `FactKind`, and a `FactKind` needs a `wiring` answer.
        let samples = [
            Fact::ReconciliationMismatch { venue: venue("kraken") },
            Fact::LeaseLost,
            stale("BTC-USD", "kraken"),
        ];
        let kinds: Vec<FactKind> = samples.iter().map(Fact::kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), kinds.len(), "two variants share one kind");
        assert_eq!(sorted.len(), FactKind::all().len());
    }

    #[test]
    fn exactly_one_fact_is_raised_in_production_today() {
        // Not an aspiration — a measurement, so the number moving is visible in
        // a diff. Two of the three are blocked on producers in crates this crate
        // may not edit, and each names the caller that would clear it.
        let raised: Vec<&str> = FactKind::all()
            .into_iter()
            .filter(|k| k.wiring().is_raised_in_production())
            .map(FactKind::as_str)
            .collect();
        assert_eq!(raised, vec!["LeaseLost"]);
    }

    #[test]
    fn a_reason_names_the_thing_that_went_wrong() {
        // The operator's first question is always "why", and a reason that does
        // not name the instrument or the venue does not answer it.
        let r = stale("BTC-USD", "binance").reason();
        assert!(r.contains("BTC-USD"), "{r}");
        assert!(r.contains("700"), "{r}");
        // And the venue, which is the half that was missing. Three BTC-USD
        // feeds were live on 2026-09-21 and one of them was 92 seconds old; a
        // reason naming only the symbol sends the operator to whichever one
        // they think of first.
        assert!(
            r.contains("binance"),
            "a reason that does not name the venue names two feeds that are fine: {r}"
        );
        // And it names the multiple from the constant, not from a literal typed
        // into the sentence: a message that says "10x" after the constant moved
        // tells the operator a number that is not the one that fired.
        assert!(
            r.contains(&format!("{}x", crate::evaluate::HARD_STALENESS_MULTIPLE)),
            "{r}"
        );
    }
}
