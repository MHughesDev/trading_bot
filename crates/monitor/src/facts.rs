//! What a raised fact does, and the table its decision has to travel through
//! (COMP-007 §4, §4.1).
//!
//! # The rule this module exists to make structural
//!
//! The order gate never reads the monitor's tables. It reads an in-memory
//! [`risk::KillSwitch`] which [`LeaseWatcher::refresh`] fills, and `refresh`
//! fills it from exactly three reads: `monitor.lease`, `monitor.kill_switch` and
//! `monitor.strategy_state`. **A decision written anywhere else does not reach
//! the gate.** A `monitor.events` row is a line on a page; a `tracing::warn!` is
//! a line in a log; and an in-process `KillSwitch::trip_scope` with no durable
//! row behind it is *worse* than either, because
//! [`KillSwitch::replace_halted`][rh] replaces rather than merges — the next
//! refresh erases it and whatever was halted resumes trading.
//!
//! That is not hypothetical. It is how a collector-driven halt came to refuse no
//! orders and how automatic scale-down came never to cross the process boundary,
//! and it is still how `reconciliation::divergence::check_divergence` halts a
//! venue today. So every [`FactAction`] names the table it writes to, in a type,
//! and `facts_are_wired` asserts `refresh` reads every one of them.
//!
//! [`LeaseWatcher::refresh`]: crate::service::LeaseWatcher::refresh
//! [rh]: risk::KillSwitch::replace_halted

use risk::Scope;

use crate::evaluate::{DependencyMap, Evaluator};
use crate::family::Fact;
use crate::service::{MONITOR_ACTOR, RUNTIME_ACTOR};
use crate::state::{Action, HaltPolicy, StrategyState};

/// A table the order gate's mirror is refreshed from.
///
/// A closed enum rather than a string for the same reason [`crate::Family`] is
/// one: the whole value of naming the table is that a table nothing reads cannot
/// be named here by accident, and a typo would be a table nothing reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GateTable {
    /// `monitor.lease`. Its *absence* is the halt: no valid row, no orders.
    Lease,
    /// `monitor.kill_switch`. A halted scope refuses that scope's orders.
    KillSwitch,
    /// `monitor.strategy_state`. Carries the automatic scale-down.
    StrategyState,
}

impl GateTable {
    /// Every table, so the guard cannot scan a shorter list than the enum.
    #[must_use]
    pub fn all() -> [Self; 3] {
        [Self::Lease, Self::KillSwitch, Self::StrategyState]
    }

    #[must_use]
    pub fn table(self) -> &'static str {
        match self {
            Self::Lease => "monitor.lease",
            Self::KillSwitch => "monitor.kill_switch",
            Self::StrategyState => "monitor.strategy_state",
        }
    }

    /// The [`MonitorStore`][s] method [`LeaseWatcher::refresh`][r] reads it with.
    ///
    /// This string is what makes the rule checkable: the guard reads `refresh`'s
    /// source and asserts each of these appears in it, so removing a read — which
    /// would silently strand every decision that travels through that table —
    /// fails a test instead of passing every test.
    ///
    /// [s]: crate::store::MonitorStore
    /// [r]: crate::service::LeaseWatcher::refresh
    #[must_use]
    pub fn reader(self) -> &'static str {
        match self {
            Self::Lease => "lease_status",
            Self::KillSwitch => "halted_scopes",
            Self::StrategyState => "strategy_runtime_states",
        }
    }
}

/// One thing a raised fact does.
///
/// Every variant names its [`GateTable`], because a fact that decides something
/// and writes it nowhere the gate reads is the defect this type exists to make
/// unspellable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactAction {
    /// Halt a scope durably. [`MonitorStore::carry_out_fact`][c] routes this
    /// through [`MonitorStore::trip`][t], which writes the event and the
    /// `monitor.kill_switch` row in one transaction, record first (§8.5).
    ///
    /// [c]: crate::store::MonitorStore::carry_out_fact
    /// [t]: crate::store::MonitorStore::trip
    Halt(Scope),
    /// Scale one strategy down: the event first, then `monitor.strategy_state`.
    ///
    /// Carries `from` as well as `state` so the event names both ends of the
    /// move. An event that says only where a strategy ended up leaves the
    /// operator to work out where it came from, which is the first thing they
    /// need and the thing the row was supposed to save them.
    Scale {
        strategy_id: String,
        from: StrategyState,
        state: StrategyState,
        scale_pct: u8,
    },
    /// Record the gap and nothing else.
    ///
    /// The one case where recording is the whole action, and it is not a
    /// shortcut: the gate is *already* closed, by the absence of a valid
    /// `monitor.lease` row, which `refresh` turns into
    /// [`KillSwitch::observe_lease`][o] on every cadence. Tripping the durable
    /// global switch on top of that would convert a self-healing mechanism into
    /// one needing a typed human reset after every monitor restart — and
    /// `Family::Lease` is not suppressible by a maintenance window, so a rolling
    /// deploy of the monitor would stop the book until someone performed the
    /// full §5 ritual. That is exactly the "safety mechanism becomes a nuisance
    /// and then a habit of ignoring it" failure §4.3 exists to prevent.
    ///
    /// What was missing was never the halt. It was the record: the operator saw
    /// every order refused with `monitor_unavailable` and had nothing in the
    /// timeline saying why.
    ///
    /// [o]: risk::KillSwitch::observe_lease
    RecordLeaseGap,
}

impl FactAction {
    /// The table this action's decision reaches the order gate through.
    #[must_use]
    pub fn gate_table(&self) -> GateTable {
        match self {
            Self::Halt(_) => GateTable::KillSwitch,
            Self::Scale { .. } => GateTable::StrategyState,
            Self::RecordLeaseGap => GateTable::Lease,
        }
    }

    /// The scope the resulting event is written against.
    #[must_use]
    pub fn scope(&self) -> Scope {
        match self {
            Self::Halt(scope) => scope.clone(),
            Self::Scale { strategy_id, .. } => Scope::strategy(strategy_id.as_str()),
            Self::RecordLeaseGap => Scope::Global,
        }
    }

    /// Who the event is attributed to.
    ///
    /// Derived from the action rather than passed in. An actor parameter is a
    /// free-form string threaded from a caller, and a free-form string is how an
    /// `agent:` actor eventually reaches a table whose CHECK constraint then
    /// rejects the write — at which point the halt is lost rather than
    /// misattributed. There are two actors and they are these two.
    #[must_use]
    pub fn actor(&self) -> &'static str {
        match self {
            // The monitor process decided it.
            Self::Halt(_) | Self::Scale { .. } => MONITOR_ACTOR,
            // The watched process noticed it, because the monitor is the thing
            // that is missing. Claiming `monitor` here would put a row in the
            // timeline signed by a process that was not running.
            Self::RecordLeaseGap => RUNTIME_ACTOR,
        }
    }
}

/// What a fact does, per scope it reaches.
///
/// Pure, and given the strategies' current states rather than reading them, so
/// the whole policy can be tested against a synthetic book rather than a fixture
/// nobody re-runs.
///
/// Dwell is not consulted, and that is the point of §4.1's exemption: these are
/// facts, not samples of a distribution.
///
/// The per-strategy policy is taken from [`Evaluator::on_fact`] rather than
/// restated here, so there is one authority for what a data fact does and not
/// two that can drift apart. A strategy the transition does not move — one
/// already halted — produces no action, so a fact raised every cadence while the
/// condition persists does not write a second trip record for one incident. A
/// strategy already *scaled down* is not in that set: since ADR-D-11 a data
/// breach halts, and a quarter-size position priced off a dead feed is the same
/// error at 25% of the size, so the fact escalates it.
///
/// An empty result is a real answer and is not the same as "nothing was wrong":
/// a stale feed that reaches no strategy reaches none because nothing depends on
/// **that feed** — that symbol *at that venue* (COMP-007 §3). Callers say so out
/// loud rather than treating it as success. The distinction is not pedantic: a
/// dependency map keyed on the symbol alone would answer this question for three
/// different BTC-USD feeds at once, two of them wrongly.
#[must_use]
pub fn decide(
    fact: &Fact,
    deps: &DependencyMap,
    strategy_states: &[(String, StrategyState, HaltPolicy)],
) -> Vec<FactAction> {
    match fact {
        // §4: "venue reconciliation mismatch or connection loss → halt venue
        // order flow". That venue and no other: the fact established is that
        // this venue and our book disagree about what is owned, which says
        // nothing at all about the others.
        Fact::ReconciliationMismatch { venue } => {
            vec![FactAction::Halt(Scope::venue(venue.as_str()))]
        }
        Fact::LeaseLost => vec![FactAction::RecordLeaseGap],
        Fact::HardDataStaleness { feed, .. } => deps
            .strategies_for(feed)
            .iter()
            .filter_map(|strategy_id| {
                let (from, policy) = strategy_states
                    .iter()
                    .find(|(id, ..)| id == strategy_id)
                    .map_or((StrategyState::Nominal, HaltPolicy::Hold), |(_, s, p)| {
                        (*s, *p)
                    });
                let transition = Evaluator::on_fact(fact, from, policy);
                if !transition.changed() {
                    return None;
                }
                if transition
                    .actions
                    .iter()
                    .any(|a| matches!(a, Action::Halt(_)))
                {
                    return Some(FactAction::Halt(Scope::strategy(strategy_id.as_str())));
                }
                let scale_pct = transition
                    .actions
                    .iter()
                    .find_map(|a| match a {
                        Action::ScaleTo(pct) => Some(*pct),
                        Action::ReleaseScale => Some(100),
                        _ => None,
                    })
                    .unwrap_or(100);
                Some(FactAction::Scale {
                    strategy_id: strategy_id.clone(),
                    from: transition.from,
                    state: transition.to,
                    scale_pct,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::{FactKind, Wiring};
    use crate::feed::{FeedKey, Venue};

    /// A feed, through the only constructor there is.
    fn feed(symbol: &str, venue: &str) -> FeedKey {
        FeedKey::new(symbol, Venue::named(venue).expect("a venue")).expect("a feed")
    }

    fn venue(name: &str) -> Venue {
        Venue::named(name).expect("a venue")
    }

    fn stale(symbol: &str) -> Fact {
        Fact::HardDataStaleness {
            feed: feed(symbol, "kraken"),
            age_secs: 700,
            timeframe_secs: 60,
        }
    }

    fn deps() -> DependencyMap {
        let mut d = DependencyMap::new();
        d.add(&feed("BTC-USD", "kraken"), "s-btc");
        d.add(&feed("BTC-USD", "kraken"), "s-both");
        d.add(&feed("ETH-USD", "kraken"), "s-eth");
        d
    }

    #[test]
    fn a_reconciliation_mismatch_halts_that_venue_and_only_that_venue() {
        // COMP-007 §4, §8.3.
        let actions = decide(
            &Fact::ReconciliationMismatch {
                venue: venue("kraken"),
            },
            &deps(),
            &[],
        );
        assert_eq!(actions, vec![FactAction::Halt(Scope::venue("kraken"))]);
    }

    #[test]
    fn a_hard_stale_feed_reaches_only_the_strategies_that_depend_on_it() {
        // COMP-007 §3 / S-7.5. The one that matters is `s-eth`: a strategy that
        // trades a different instrument must not be touched, and a
        // dependency-blind halt would look identical on every test that only
        // checks "something happened".
        let actions = decide(&stale("BTC-USD"), &deps(), &[]);
        let touched: Vec<String> = actions.iter().map(|a| a.scope().to_string()).collect();
        assert_eq!(touched, ["strategy:s-btc", "strategy:s-both"]);
    }

    #[test]
    fn a_dead_feed_at_one_venue_does_not_halt_the_same_symbol_at_another() {
        // COMP-007 §3's "and only those", read at the level it is actually
        // violated. `a_hard_stale_feed_reaches_only_the_strategies_that_depend_on_it`
        // above distinguishes BTC-USD from ETH-USD, which any keying gets right.
        // This one distinguishes BTC-USD from BTC-USD, which only the (symbol,
        // venue) key gets right — and it is the case the live database is in:
        // binance's BTC-USD 60s bars stopped on 2026-06-22 while kraken's were
        // 92 seconds old.
        //
        // Since ADR-D-11 the failure is not cosmetic. Under the symbol-keyed
        // map this crate had until 2026-09-21, `s-kraken` appears in the
        // dependants of the dead binance feed and is HALTED — a strategy pricing
        // off a 92-second-old feed stops trading because of a venue it never
        // touches.
        let mut deps = DependencyMap::new();
        deps.add(&feed("BTC-USD", "kraken"), "s-kraken");
        deps.add(&feed("BTC-USD", "binance"), "s-binance");

        let dead_binance = Fact::HardDataStaleness {
            feed: feed("BTC-USD", "binance"),
            age_secs: 7_893_272,
            timeframe_secs: 60,
        };
        let actions = decide(&dead_binance, &deps, &[]);
        assert_eq!(
            actions,
            vec![FactAction::Halt(Scope::strategy("s-binance"))],
            "a feed dead since June halted a strategy whose prices are 92 seconds old"
        );
        // Anti-vacuity: the map really does hold a kraken dependant, so the
        // assertion above is a discrimination and not an empty map.
        assert_eq!(
            decide(
                &Fact::HardDataStaleness {
                    feed: feed("BTC-USD", "kraken"),
                    age_secs: 700,
                    timeframe_secs: 60,
                },
                &deps,
                &[],
            ),
            vec![FactAction::Halt(Scope::strategy("s-kraken"))]
        );
    }

    #[test]
    fn a_stale_instrument_nothing_depends_on_acts_on_nothing() {
        assert!(decide(&stale("SOL-USD"), &deps(), &[]).is_empty());
    }

    #[test]
    fn a_strategy_already_scaled_down_is_escalated_to_a_halt_by_a_stale_feed() {
        // Scaling down is not a lesser halt (ADR-D-11). A strategy an earlier
        // execution breach had already cut to a quarter is still trading, and a
        // quarter-size position priced off a dead feed is the same error at 25%
        // of the size — so the fact escalates it rather than finding nothing
        // left to do.
        let states = vec![(
            "s-btc".to_owned(),
            StrategyState::ScaledDown,
            HaltPolicy::Hold,
        )];
        let actions = decide(&stale("BTC-USD"), &deps(), &states);
        assert_eq!(
            actions,
            vec![
                FactAction::Halt(Scope::strategy("s-btc")),
                FactAction::Halt(Scope::strategy("s-both")),
            ]
        );
    }

    #[test]
    fn a_halt_from_a_stale_feed_reaches_the_gate_through_the_kill_switch() {
        // The rule of this module applied to the behaviour ADR-D-11 changed. The
        // scale-down this used to produce travelled through
        // `monitor.strategy_state`; a halt travels through
        // `monitor.kill_switch`, and a decision written anywhere else does not
        // reach the order gate at all.
        for action in decide(&stale("BTC-USD"), &deps(), &[]) {
            assert_eq!(action.gate_table(), GateTable::KillSwitch, "{action:?}");
            assert_eq!(action.actor(), MONITOR_ACTOR, "{action:?}");
        }
    }

    #[test]
    fn a_halted_strategy_is_not_moved_by_a_later_fact() {
        let states = vec![("s-btc".to_owned(), StrategyState::Halted, HaltPolicy::Hold)];
        let actions = decide(&stale("BTC-USD"), &deps(), &states);
        assert!(
            actions
                .iter()
                .all(|a| a.scope() != Scope::strategy("s-btc")),
            "a halted strategy is already refusing orders"
        );
    }

    #[test]
    fn a_data_fact_halts_the_strategies_that_depend_on_the_feed() {
        // Q-S7-01 is closed by ADR-D-11: §3 wins and a hard-stale feed halts.
        // This asserts both halves — that a data fact halts at all, and that
        // `decide` says the same thing `Evaluator::on_fact` does. The second is
        // the one that would hurt if it drifted: the drill would rehearse one
        // reading and production would run the other.
        //
        // The expectation is taken from `Family::breach_halts` rather than
        // written out, so this test cannot be the place that still claims a
        // scale-down after the policy moves.
        let fact = stale("BTC-USD");
        assert!(fact.family().breach_halts());
        let from_evaluator = Evaluator::on_fact(&fact, StrategyState::Nominal, HaltPolicy::Hold);
        assert_eq!(from_evaluator.to, StrategyState::Halted);

        let actions = decide(&fact, &deps(), &[]);
        assert!(!actions.is_empty(), "two strategies depend on BTC-USD");
        for action in &actions {
            match action {
                FactAction::Halt(scope) => assert!(
                    matches!(scope, Scope::Strategy(_)),
                    "a stale instrument halts the strategies that use it, not the book: {scope:?}"
                ),
                other => panic!("expected a halt, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_action_writes_to_a_table_the_gate_is_refreshed_from() {
        // The rule of this module, over the whole `FactAction` enum rather than
        // one variant, so a variant added later has to answer the question.
        let all = [
            FactAction::Halt(Scope::venue("kraken")),
            FactAction::Scale {
                strategy_id: "s-1".to_owned(),
                from: StrategyState::Nominal,
                state: StrategyState::ScaledDown,
                scale_pct: 25,
            },
            FactAction::RecordLeaseGap,
        ];
        for action in &all {
            assert!(
                GateTable::all().contains(&action.gate_table()),
                "{action:?} writes somewhere the order gate never reads"
            );
        }
        let tables: Vec<GateTable> = all.iter().map(FactAction::gate_table).collect();
        assert_eq!(
            tables,
            vec![
                GateTable::KillSwitch,
                GateTable::StrategyState,
                GateTable::Lease
            ]
        );
    }

    #[test]
    fn no_action_is_ever_attributed_to_an_agent() {
        // COMP-007 §8.6. The actor is derived from the action and there are two
        // of them, so there is no string for a caller to thread through.
        for action in [
            FactAction::Halt(Scope::Global),
            FactAction::Scale {
                strategy_id: "s-1".to_owned(),
                from: StrategyState::Nominal,
                state: StrategyState::ScaledDown,
                scale_pct: 25,
            },
            FactAction::RecordLeaseGap,
        ] {
            assert!(!action.actor().starts_with("agent:"), "{action:?}");
        }
    }

    #[test]
    fn every_fact_kind_decides_something() {
        // The anti-vacuity half: a `decide` that fell through to an empty vector
        // for a whole variant would pass every test above, because every test
        // above asserts about the variants it names.
        let all_deps = {
            let mut d = DependencyMap::new();
            d.add(&feed("BTC-USD", "kraken"), "s-btc");
            d
        };
        let samples = [
            Fact::ReconciliationMismatch {
                venue: venue("kraken"),
            },
            Fact::LeaseLost,
            stale("BTC-USD"),
        ];
        let kinds: Vec<FactKind> = samples.iter().map(Fact::kind).collect();
        assert_eq!(
            kinds.len(),
            FactKind::all().len(),
            "a sample is missing for a new Fact variant"
        );
        for fact in &samples {
            assert!(
                !decide(fact, &all_deps, &[]).is_empty(),
                "{fact:?} decides nothing at all"
            );
        }
    }

    #[test]
    fn a_wiring_answer_that_says_unwired_names_the_caller_that_would_fix_it() {
        // A blocker with no named caller is a blocker nobody can clear, and this
        // repository's dominant defect shape is code that is complete and has no
        // caller. The reason must be specific enough to act on.
        for kind in FactKind::all() {
            match kind.wiring() {
                Wiring::RaisedBy(site) => assert!(
                    site.contains("::"),
                    "{kind:?} names its raiser as {site:?}, which is not a path"
                ),
                Wiring::Unwired { because, needs } => {
                    assert!(because.len() > 20, "{kind:?}: {because:?}");
                    assert!(
                        needs.contains("::"),
                        "{kind:?} must name the exact function that has to raise it, got {needs:?}"
                    );
                }
            }
        }
    }
}
