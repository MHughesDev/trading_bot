//! The two loops (COMP-007 §1 P1, §4, §6).
//!
//! [`MonitorService`] is the one that runs in `tbot-monitor`: renew the lease,
//! measure, evaluate, write events, trip. [`LeaseWatcher`] is the one that runs
//! *inside* the process being watched: read the lease, read the halted scopes,
//! push both into the in-memory mirror the order gate reads.
//!
//! They are deliberately different shapes. The service writes and the watcher only
//! reads, and the watcher's failure mode — cannot reach the database — closes the
//! gate rather than leaving it where it was.
//!
//! # The cadence, in order
//!
//! [`Cadence::assess`] is the decision half, and it is a pure function on purpose:
//! everything about which event gets written and which state the strategy moves to
//! is decided without touching a database, so it can be tested against a synthetic
//! series rather than against a fixture nobody re-runs. The IO half
//! ([`MonitorService::evaluate_strategy`]) does what the decision says, and writes
//! the record **before** the state change, because §8.5 requires the trip record
//! to exist before the first refused order.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use risk::KillSwitch;

use crate::evaluate::{DependencyMap, Evaluation, Evaluator, MetricReading, Outcome};
use crate::events::{EventKind, MonitorEvent};
use crate::family::{Fact, Family};
use crate::lease::{LeaseStatus, LeaseTerms};
use crate::state::{Action, HaltPolicy, Severity, StrategyState, Transition};
use crate::store::MonitorStore;

/// The actor every automatic monitor event carries.
///
/// A constant rather than a literal at each call site: `MonitorEvent::new` refuses
/// an `agent:` actor, and the way that refusal gets bypassed is not by someone
/// writing `"agent:run-9"` here — it is by a caller threading a string through from
/// somewhere else. There is one string, and it is this one.
pub const MONITOR_ACTOR: &str = "monitor";

/// The actor on a record the *watched* process authored.
///
/// A lease gap is the one thing the monitor cannot report, because the monitor
/// is the thing that is missing. [`LeaseWatcher`] writes that row instead, and it
/// signs it as itself: a row signed `monitor` would put an entry in the timeline
/// attributed to a process that was not running, which is the one question an
/// operator reading that timeline is trying to answer.
pub const RUNTIME_ACTOR: &str = "runtime";

/// How far back each cadence measures.
///
/// Deliberately much longer than the cadence itself. The monitor *decides* every
/// sixty seconds, but a fill rate computed over sixty seconds of a strategy that
/// trades a few times an hour is either unobserved or a statistic over one order,
/// and a 3σ band applied to a statistic over one order breaches by construction —
/// the alarm-fatigue failure §4.1 exists to prevent. Overlapping windows are the
/// price: the same order is measured in several consecutive cadences, which is
/// what dwell already assumes, since dwell counts *consecutive evaluations of a
/// condition* rather than independent samples.
pub const MEASUREMENT_WINDOW_MINUTES: i64 = 60;

/// How often the monitor rehearses its own kill switch (COMP-007 §4.2:
/// "monthly regardless").
///
/// Thirty days rather than a calendar month, because a rehearsal schedule that
/// depends on which month it is has an edge case nobody tests and the whole
/// mechanism exists to remove the path nobody exercises.
pub const DRILL_INTERVAL_DAYS: i64 = 30;

/// What one cadence decided about one strategy.
///
/// The events are in the order they must be written, which always begins with the
/// record. Nothing here has happened yet; this is the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct CadenceDecision {
    pub strategy_id: String,
    /// The transition, which may be [`Transition::none`].
    pub transition: Transition,
    /// The events to write, record first.
    pub events: Vec<MonitorEvent>,
    /// The size the strategy should be running at afterwards, in per cent.
    pub scale_pct: u8,
    /// How many readings were judged. Zero means the collector measured nothing
    /// this cadence — which is **not** the same as everything being fine, and is
    /// why it is reported rather than implied by an empty event list.
    pub judged: usize,
    /// Readings for which no limit is declared. A strategy whose every metric is
    /// undeclared produces no events and no transition, and would otherwise look
    /// identical to a strategy that is behaving.
    pub undeclared: Vec<String>,
}

impl CadenceDecision {
    /// Whether this cadence changed anything.
    pub fn acted(&self) -> bool {
        self.transition.changed()
    }
}

/// The decision half of a cadence: readings in, events and a transition out.
///
/// It owns the [`Evaluator`], because dwell state is what makes three consecutive
/// cadences different from three separate ones, and an evaluator rebuilt every
/// cadence would never reach the third.
#[derive(Debug, Default)]
pub struct Cadence {
    evaluator: Evaluator,
}

impl Cadence {
    pub fn new(evaluator: Evaluator) -> Self {
        Self { evaluator }
    }

    pub fn evaluator(&self) -> &Evaluator {
        &self.evaluator
    }

    pub fn evaluator_mut(&mut self) -> &mut Evaluator {
        &mut self.evaluator
    }

    /// Judge one strategy's readings and decide the move.
    ///
    /// When several metrics act in the same cadence the **worst** one drives the
    /// transition and names the event: a P&L breach beats an execution breach
    /// beats a warning. Writing one event per acting metric would put three trip
    /// records in the timeline for one incident, and an operator reading it under
    /// pressure would have to work out that they were the same event.
    pub fn assess(
        &mut self,
        strategy_id: &str,
        readings: Vec<MetricReading>,
        from: StrategyState,
        policy: HaltPolicy,
    ) -> CadenceDecision {
        let judged = readings.len();
        let mut undeclared = Vec::new();
        let mut acting: Vec<Evaluation> = Vec::new();
        let mut blips: Vec<Evaluation> = Vec::new();
        let mut released: Option<Evaluation> = None;

        for reading in readings {
            let ev = self.evaluator.evaluate(reading);
            match ev.outcome {
                Outcome::Undeclared => undeclared.push(ev.reading.metric.clone()),
                Outcome::Blip => blips.push(ev),
                Outcome::Act(_) => acting.push(ev),
                Outcome::Release => released = Some(ev),
                Outcome::Inside | Outcome::Recovering => {}
            }
        }

        let mut events = Vec::new();

        // A blip is recorded and acted on by nothing (§4.1). It is written even
        // when the same cadence also acts, because the point of the blip row is
        // that the monitor's own precision can be scored against what it saw, and
        // a blip dropped because something louder happened is a reading missing
        // from that record.
        for b in &blips {
            events.push(self.event(strategy_id, b, EventKind::Blip, from, from));
        }

        let Some(driver) = worst(&acting) else {
            // Nothing sustained. A completed recovery releases an automatic
            // scale-down; nothing else moves.
            if let Some(rel) = released {
                let t = crate::state::on_recovered(from);
                if t.changed() {
                    let kind = if t.actions.contains(&Action::ReleaseScale) {
                        EventKind::ScaleReleased
                    } else {
                        EventKind::Resumed
                    };
                    events.push(self.event(strategy_id, &rel, kind, t.from, t.to));
                    return CadenceDecision {
                        strategy_id: strategy_id.to_owned(),
                        scale_pct: scale_after(&t),
                        transition: t,
                        events,
                        judged,
                        undeclared,
                    };
                }
            }
            return CadenceDecision {
                strategy_id: strategy_id.to_owned(),
                transition: Transition::none(from),
                events,
                scale_pct: scale_for(from),
                judged,
                undeclared,
            };
        };

        let transition = self.evaluator.transition(&driver, from, policy);
        if transition.changed() {
            let kind = kind_of(&transition);
            events.push(self.event(strategy_id, &driver, kind, transition.from, transition.to));
        }
        let scale_pct = scale_after(&transition);
        CadenceDecision {
            strategy_id: strategy_id.to_owned(),
            transition,
            events,
            scale_pct,
            judged,
            undeclared,
        }
    }

    fn event(
        &self,
        strategy_id: &str,
        ev: &Evaluation,
        kind: EventKind,
        from: StrategyState,
        to: StrategyState,
    ) -> MonitorEvent {
        // `expect` rather than `?`: `MONITOR_ACTOR` is a constant that does not
        // start with `agent:`, so this cannot fail, and threading a `Result` out
        // for an impossible case would invite a caller to handle it by dropping
        // the event.
        MonitorEvent::new(
            risk::Scope::strategy(strategy_id),
            ev.reading.family,
            kind,
            MONITOR_ACTOR,
        )
        .expect("the monitor is not an agent")
        .with_metric(&ev.reading.metric, ev.reading.value, ev.limit_value)
        .with_states(from.as_str(), to.as_str())
        .with_reason(reason_for(ev))
    }
}

/// The sentence the operator reads first.
///
/// Built from the reading rather than from the kind, so it names the number and
/// the bound. "Execution breach" costs the operator the first five minutes of
/// every incident; `fill_rate 0.4 outside its declared limit of 0.8` does not.
fn reason_for(ev: &Evaluation) -> String {
    match ev.limit_value {
        Some(l) => format!(
            "{} {} outside its declared limit of {}",
            ev.reading.metric, ev.reading.value, l
        ),
        None => format!("{} {}", ev.reading.metric, ev.reading.value),
    }
}

/// The acting evaluation that should drive the transition.
///
/// P&L first, because a P&L breach halts and everything else scales, and a cadence
/// in which both happened must halt. Within a rank, the reading furthest outside
/// its limit wins, so the event names the worst number rather than the first one
/// the collector happened to emit.
fn worst(acting: &[Evaluation]) -> Option<Evaluation> {
    acting
        .iter()
        .max_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then(distance(a).total_cmp(&distance(b)))
        })
        .cloned()
}

fn rank(ev: &Evaluation) -> u8 {
    match ev.outcome {
        Outcome::Act(Severity::Breach(Family::Pnl)) => 2,
        Outcome::Act(Severity::Breach(_)) => 1,
        _ => 0,
    }
}

/// How far outside its limit a reading sits, in the metric's own units.
///
/// Absolute rather than signed: a limit may be an upper or a lower bound, and
/// "furthest outside" is the same question either way.
fn distance(ev: &Evaluation) -> f64 {
    ev.limit_value
        .map_or(0.0, |l| (ev.reading.value - l).abs())
}

fn kind_of(t: &Transition) -> EventKind {
    if t.actions.iter().any(|a| matches!(a, Action::Halt(_))) {
        return EventKind::Halted;
    }
    if t.actions.contains(&Action::ScaleTo(crate::SCALE_DOWN_PCT)) {
        return EventKind::ScaledDown;
    }
    match t.to {
        StrategyState::Warning => EventKind::Warning,
        StrategyState::Breach => EventKind::Breach,
        StrategyState::Nominal => EventKind::Resumed,
        StrategyState::ScaledDown => EventKind::ScaledDown,
        StrategyState::Halted => EventKind::Halted,
    }
}

/// The size a strategy runs at in a given state.
fn scale_for(state: StrategyState) -> u8 {
    match state {
        StrategyState::ScaledDown => crate::SCALE_DOWN_PCT,
        // A halted strategy is not "running at zero per cent" — it is refusing
        // orders, which `StrategyState::Halted` already says. Leaving the size at
        // 100 keeps the two facts separate, so a resume restores the size the
        // operator expects rather than silently keeping it at nothing.
        _ => 100,
    }
}

fn scale_after(t: &Transition) -> u8 {
    for a in &t.actions {
        match a {
            Action::ScaleTo(pct) => return *pct,
            Action::ReleaseScale => return 100,
            _ => {}
        }
    }
    scale_for(t.to)
}

/// The independent monitor process's loop.
pub struct MonitorService {
    store: MonitorStore,
    terms: LeaseTerms,
}

impl MonitorService {
    #[must_use]
    pub fn new(store: MonitorStore, terms: LeaseTerms) -> Self {
        Self { store, terms }
    }

    #[must_use]
    pub fn store(&self) -> &MonitorStore {
        &self.store
    }

    #[must_use]
    pub fn terms(&self) -> &LeaseTerms {
        &self.terms
    }

    /// One cadence: renew the lease and beat.
    ///
    /// Renewal comes first and its failure is loud. A monitor that keeps evaluating
    /// while it cannot renew is a monitor that believes it is protecting a platform
    /// which has already stopped listening to it.
    pub async fn tick(&self) -> Result<(), crate::store::StoreError> {
        self.store.renew_lease(&self.terms, Utc::now()).await?;
        self.store
            .heartbeat("tbot-monitor", &self.terms.holder, self.terms.cadence_ms)
            .await?;
        Ok(())
    }

    /// Carry out a decided cadence: record first, then move the state.
    ///
    /// The ordering is the whole of COMP-007 §8.5. A record written after the
    /// state change is a record that is missing in exactly the case that matters —
    /// the process dying between the two — and the first thing anyone would then
    /// see is a halted strategy with nothing in the timeline saying why.
    ///
    /// A failure to write the record therefore **abandons the state change**. The
    /// alternative, halting anyway and logging the failure, produces precisely the
    /// unexplained halt the ordering exists to prevent; the lease is still being
    /// renewed, the next cadence will reach the same verdict, and a monitor that
    /// cannot write at all loses its lease and closes the gate on its own.
    /// A halt additionally **trips the kill switch** for that strategy's scope.
    ///
    /// Without this a collector-driven halt would set `monitor.strategy_state` to
    /// `halted` and refuse nothing: the order gate's mirror is refreshed from
    /// `MonitorStore::halted_scopes`, which reads `monitor.kill_switch` and
    /// nothing else. A strategy could sit at `halted` on the monitor page, with an
    /// event row explaining why, and go on trading. [`MonitorStore::trip`] writes
    /// the event and the switch row in one transaction, record first, so §8.5's
    /// ordering survives here too.
    pub async fn apply(
        &self,
        decision: &CadenceDecision,
    ) -> Result<(), crate::store::StoreError> {
        let halting = decision.transition.to == StrategyState::Halted
            && decision.transition.changed();

        for event in &decision.events {
            if halting && event.kind == EventKind::Halted {
                let reason = event
                    .reason
                    .clone()
                    .unwrap_or_else(|| "monitor halt".to_owned());
                self.store.trip(event, &reason).await?;
            } else {
                self.store.record_event(event).await?;
            }
        }
        if decision.transition.changed() {
            self.store
                .set_strategy_state(
                    &decision.strategy_id,
                    decision.transition.to.as_str(),
                    i16::from(decision.scale_pct),
                )
                .await?;
        }
        Ok(())
    }

    /// Judge the platform's own health, and let an open window speak (§3, §4,
    /// §4.3).
    ///
    /// The half of §4's platform row that decides; [`crate::platform::assess`] is
    /// the half that judges, and it is pure. Three outcomes and each writes
    /// something:
    ///
    /// - **Halt** — `monitor.kill_switch` at `global`, through [`MonitorStore::trip`]
    ///   so the record is in the same transaction and comes first (§8.5). This is
    ///   the one automatic action in the crate that stops the whole book, which is
    ///   why the rules in `platform` about *which* components count are enforced
    ///   by tests rather than by care.
    /// - **Suppressed** — an event carrying `suppressed_by_window`, and no halt.
    ///   The row is the difference between a maintenance window and a hole: a
    ///   deploy that swallowed a platform fault silently would leave nobody able
    ///   to tell afterwards that it had.
    /// - **Recovered** — an event, and **no reset**. §5 keeps the reset human:
    ///   the components beating again is a fact about the components, not a
    ///   decision to trade.
    ///
    /// Only on the edges. A verdict repeated every cadence through an outage
    /// writes nothing after the first, for the reason [`lease_note`] exists.
    pub async fn assess_platform(
        &self,
        watch: &mut crate::platform::PlatformWatch,
        now: chrono::DateTime<Utc>,
    ) -> Result<crate::platform::PlatformVerdict, crate::store::StoreError> {
        let beats = self
            .store
            .component_beats(crate::platform::MISSED_BEATS_BEFORE_HALT)
            .await?;
        let windows = self.store.open_windows().await?;
        let verdict = crate::platform::assess(&beats, &windows, now);

        let Some(note) = watch.observe(&verdict) else {
            return Ok(verdict);
        };
        let reason = verdict.reason();
        match (note, &verdict) {
            (crate::platform::PlatformNote::Lost, crate::platform::PlatformVerdict::Halt { .. }) => {
                let event = MonitorEvent::new(
                    risk::Scope::Global,
                    Family::Platform,
                    EventKind::Tripped,
                    MONITOR_ACTOR,
                )
                .map_err(|e| crate::store::StoreError::Invalid(e.to_string()))?
                .with_reason(reason.clone());
                self.store.trip(&event, &reason).await?;
            }
            (
                crate::platform::PlatformNote::Lost,
                crate::platform::PlatformVerdict::Suppressed { window_id, .. },
            ) => {
                // A warning rather than a trip, because nothing was tripped. The
                // window id is on the row, so "which deploy decided this" is a
                // question the timeline answers without anyone reconstructing it.
                let event = MonitorEvent::new(
                    risk::Scope::Global,
                    Family::Platform,
                    EventKind::Warning,
                    MONITOR_ACTOR,
                )
                .map_err(|e| crate::store::StoreError::Invalid(e.to_string()))?
                .with_reason(reason.clone())
                .suppressed_by(*window_id);
                self.store.record_event(&event).await?;
            }
            (crate::platform::PlatformNote::Recovered, _) => {
                let event = MonitorEvent::new(
                    risk::Scope::Global,
                    Family::Platform,
                    EventKind::Resumed,
                    MONITOR_ACTOR,
                )
                .map_err(|e| crate::store::StoreError::Invalid(e.to_string()))?
                .with_reason(
                    "every platform component is beating again; any halt taken for this \
                     fault stands until a human resets it",
                );
                self.store.record_event(&event).await?;
            }
            // `Lost` with a healthy verdict cannot be produced by `observe`.
            (crate::platform::PlatformNote::Lost, crate::platform::PlatformVerdict::Healthy) => {}
        }
        Ok(verdict)
    }

    /// One whole cadence over the book: measure, judge, record, move.
    ///
    /// The limits are reloaded first, every cadence, so a limit declared or
    /// amended while the monitor is running takes effect without a restart — and,
    /// more to the point, so a strategy promoted to `live` an hour ago is judged
    /// against the limits it was promoted with rather than against whatever the
    /// process happened to load at boot.
    ///
    /// One strategy's failure does not stop the others being judged. A monitor
    /// that abandoned the whole book because one row would not parse would be a
    /// monitor whose coverage depends on its least well-formed strategy.
    pub async fn collect_cadence(
        &self,
        cadence: &mut Cadence,
        now: chrono::DateTime<Utc>,
    ) -> Result<Vec<CadenceDecision>, crate::store::StoreError> {
        cadence.evaluator_mut().limits = self.store.load_limits().await?;

        let window = crate::collect::Window::ending_at(
            now,
            chrono::Duration::minutes(MEASUREMENT_WINDOW_MINUTES),
        );
        let observations =
            crate::collect::by_strategy(self.store.order_observations(&window).await?);
        let known = self.store.strategy_runtime_states().await?;

        // The union of "has a tracked state" and "placed an order this window". A
        // strategy that is tracked but placed nothing is still evaluated, because
        // a strategy that has silently stopped trading is one of the things the
        // behaviour family is declared to catch; a strategy that traded but has no
        // row yet starts from nominal.
        let mut ids: Vec<String> = known.iter().map(|(id, ..)| id.clone()).collect();
        for id in observations.keys() {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids.sort();

        let mut out = Vec::new();
        for id in ids {
            let (state, _, policy) = known
                .iter()
                .find(|(k, ..)| *k == id)
                .map_or((StrategyState::Nominal, 100, HaltPolicy::Hold), |(_, s, p, h)| {
                    (*s, *p, *h)
                });
            let empty: Vec<crate::collect::OrderObservation> = Vec::new();
            let orders = observations.get(&id).unwrap_or(&empty);

            let mut readings =
                crate::collect::ExecutionSample::measure(window, orders).readings(&id);
            readings.extend(
                crate::collect::BehaviourSample::measure(
                    window,
                    orders,
                    // Nothing writes a per-strategy signal count to Postgres yet,
                    // so the monitor has not observed one. `Some(0)` here would
                    // manufacture a measured rate of zero and raise an alarm about
                    // a strategy nobody had looked at.
                    None,
                    // Likewise round trips: `crates/monitor` has no source for
                    // them, and an empty slice is the truthful input — it produces
                    // an unobserved median rather than a fabricated one.
                    &[],
                    // And likewise the position count. `positions` is keyed on
                    // `(account_id, instrument_id)` with no strategy column, so
                    // any per-strategy count derived from it attributes a shared
                    // instrument's position to every strategy that has traded it.
                    // An over-counted position against an upper limit is a
                    // false scale-down, which on this surface costs the operator's
                    // trust in every other alarm. Unobserved is the honest reading
                    // until the schema can answer the question.
                    None,
                )
                .readings(&id),
            );

            match self
                .evaluate_strategy(cadence, &id, readings, state, policy)
                .await
            {
                Ok(d) => out.push(d),
                Err(e) => {
                    tracing::error!(strategy_id = %id, error = %e, "strategy went unjudged this cadence");
                }
            }
        }
        Ok(out)
    }

    /// One strategy, end to end: measure, judge, record, move.
    ///
    /// Separate from [`Self::tick`] because they fail differently and must. A
    /// failed lease renewal means the platform is about to stop trading and is the
    /// loudest thing the monitor can say; a failed evaluation of one strategy
    /// means one strategy went unjudged for a cadence, and must not stop the other
    /// strategies being judged.
    pub async fn evaluate_strategy(
        &self,
        cadence: &mut Cadence,
        strategy_id: &str,
        readings: Vec<MetricReading>,
        from: StrategyState,
        policy: HaltPolicy,
    ) -> Result<CadenceDecision, crate::store::StoreError> {
        let decision = cadence.assess(strategy_id, readings, from, policy);
        self.apply(&decision).await?;
        Ok(decision)
    }

    /// Whether a drill is due (COMP-007 §4.2: "monthly regardless").
    ///
    /// `None` — nothing has ever been rehearsed — is **due**, not recent. A
    /// platform that has never exercised its kill switch is exactly the case §4.2
    /// is about: the most important path is the one path never taken, and it is
    /// first taken on the day it matters, by someone under pressure.
    #[must_use]
    pub fn drill_is_due(last: Option<chrono::DateTime<Utc>>, now: chrono::DateTime<Utc>) -> bool {
        match last {
            None => true,
            Some(t) => now - t >= chrono::Duration::days(DRILL_INTERVAL_DAYS),
        }
    }

    /// How long a scheduled rehearsal may block the monitor's loop.
    ///
    /// Half the renewal interval, because the rehearsal runs **on the loop that
    /// renews the lease**. A rehearsal allowed to wait a cadence or two would
    /// lapse the lease of the process running it, and the platform would halt —
    /// for real, on every order — while the monitor rehearsed how it halts the
    /// platform. Half an interval leaves a whole renewal of slack for the
    /// cleanup, the record and the next `tick`.
    ///
    /// This bounds *waiting*, not the verdict: `DrillOutcome::passed` still
    /// compares propagation against the full cadence, and a rehearsal that runs
    /// out of time reports no verdict rather than a failure.
    #[must_use]
    pub fn drill_deadline_ms(terms: &LeaseTerms) -> i64 {
        (terms.renew_interval_ms() / 2).max(1)
    }

    /// Run the monthly drill if one is due, and say whether it passed.
    ///
    /// Failure to *run* one and failure to *pass* one are different answers and
    /// the caller gets both: `Ok(None)` means none was due, `Ok(Some(outcome))`
    /// means one ran and `outcome.passed()` says what it found.
    ///
    /// # Errors
    /// Anything that stopped the rehearsal completing. A rehearsal that could not
    /// measure its propagation returns an error rather than a failed drill; see
    /// [`crate::rehearsal::RehearsalError::PropagationNotObserved`].
    pub async fn run_scheduled_drill(
        &self,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<crate::drill::DrillOutcome>, crate::rehearsal::RehearsalError> {
        let last = self.store.last_drill_at().await?;
        if !Self::drill_is_due(last, now) {
            return Ok(None);
        }
        // `MONITOR_ACTOR`, because no human asked: the schedule did. It is not an
        // `agent:` actor and the drill table's CHECK agrees.
        let rehearsal = crate::rehearsal::Rehearsal::new(
            self.store.clone(),
            self.terms.cadence_ms,
            MONITOR_ACTOR,
        )?
        .within(Self::drill_deadline_ms(&self.terms));
        let outcome = rehearsal
            .run(crate::drill::DrillTrigger::Scheduled, "scheduled")
            .await?;
        Ok(Some(outcome))
    }

    /// Run until cancelled.
    ///
    /// The lease comes first and the measurement second, in that order and never
    /// the other way. A cadence that spent its time evaluating and then failed to
    /// renew would be a monitor doing the interesting half of its job while the
    /// platform it is protecting stopped listening to it.
    pub async fn run(self) {
        let interval = Duration::from_millis(
            u64::try_from(self.terms.renew_interval_ms()).unwrap_or(crate::DEFAULT_CADENCE_MS),
        );
        tracing::info!(
            holder = %self.terms.holder,
            cadence_ms = self.terms.cadence_ms,
            measurement_window_minutes = MEASUREMENT_WINDOW_MINUTES,
            "tbot-monitor started"
        );
        // One cadence for the life of the process: dwell is what makes three
        // consecutive evaluations different from three separate ones, and a
        // cadence rebuilt each loop would never reach the third.
        let mut cadence = Cadence::default();
        // Likewise one platform watch: its whole job is to tell one cadence from
        // the next, and one rebuilt each loop would report every cadence of an
        // outage as the first.
        let mut platform = crate::platform::PlatformWatch::new();
        loop {
            if let Err(e) = self.tick().await {
                // Not fatal, and not silent. The lease will lapse on its own and
                // the platform will stop; crashing here would only make the same
                // thing happen with less in the log.
                tracing::error!(error = %e, "monitor cadence failed");
            }
            match self.collect_cadence(&mut cadence, Utc::now()).await {
                Ok(decisions) => {
                    for d in decisions.iter().filter(|d| d.acted()) {
                        tracing::warn!(
                            strategy_id = %d.strategy_id,
                            from = d.transition.from.as_str(),
                            to = d.transition.to.as_str(),
                            scale_pct = d.scale_pct,
                            "monitor acted"
                        );
                    }
                    for d in &decisions {
                        if d.judged == 0 {
                            // Not an error and not silence. A strategy the
                            // collector measured nothing about is unwatched this
                            // cadence, and the log is the only place that says so
                            // — there are no events either way.
                            tracing::info!(
                                strategy_id = %d.strategy_id,
                                "nothing measured this cadence"
                            );
                        }
                        if !d.undeclared.is_empty() {
                            tracing::warn!(
                                strategy_id = %d.strategy_id,
                                metrics = ?d.undeclared,
                                "measured with no declared limit"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "the book went unjudged this cadence");
                }
            }
            match self.assess_platform(&mut platform, Utc::now()).await {
                Ok(verdict) if verdict.is_unhealthy() => {
                    tracing::warn!(
                        lost = ?verdict.lost(),
                        reason = %verdict.reason(),
                        "platform health"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    // The platform went unjudged. Not fatal and not silent: the
                    // database this could not read is the one the lease lives in,
                    // so the lease gate is closing on its own if this persists.
                    tracing::error!(error = %e, "platform health went unassessed this cadence");
                }
            }
            match self.run_scheduled_drill(Utc::now()).await {
                Ok(Some(outcome)) => {
                    if outcome.passed() {
                        tracing::info!(
                            drill_id = %outcome.drill_id,
                            propagation_ms = outcome.propagation_ms,
                            "scheduled drill passed"
                        );
                    } else {
                        // §4.2: a drill whose propagation exceeds one cadence is
                        // the fail-closed design failing to hold. It blocks no
                        // deploy — nothing is deploying — but it is the loudest
                        // thing the monitor has to say short of losing its lease.
                        tracing::error!(
                            drill_id = %outcome.drill_id,
                            propagation_ms = outcome.propagation_ms,
                            cadence_ms = outcome.cadence_ms,
                            "scheduled drill FAILED — the kill switch did not reach the gate \
                             within one cadence"
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::error!(error = %e, "the scheduled drill could not be run"),
            }
            tokio::time::sleep(interval).await;
        }
    }
}

/// Refreshes the order gate's in-memory mirror from the monitor's tables.
///
/// This runs in the API / strategy-runtime process. It is the only thing that
/// writes to [`KillSwitch`] in production, and it is why `is_active` can stay an
/// atomic read on the hot path while the authority for what it says lives in
/// another process.
pub struct LeaseWatcher {
    store: MonitorStore,
    kill_switch: Arc<KillSwitch>,
    cadence: Duration,
    /// What the previous refresh saw of the lease, so a gap is recorded on its
    /// **edges** rather than once per cadence.
    ///
    /// A watcher refreshing every five seconds through a twenty-minute monitor
    /// outage would otherwise write two hundred and forty identical rows into
    /// the timeline an operator reads under pressure. Three states, not two: the
    /// first refresh has not seen anything yet, and a process that starts while
    /// the monitor is healthy has no gap to report.
    seen: AtomicU8,
}

/// The three things the watcher can have seen of the lease.
const SEEN_NOTHING_YET: u8 = 0;
const SEEN_LOST: u8 = 1;
const SEEN_VALID: u8 = 2;

/// What a change in the observed lease state is worth recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseNote {
    /// The lease went away. `Fact::LeaseLost`, §4.1.
    Lost,
    /// It came back after a recorded gap.
    Regained,
}

/// Decide what to record about a change in the lease, or nothing.
///
/// Pure, and separate from the refresh, so the edge logic can be tested without
/// a database — the case that matters most is the boring one, a healthy start,
/// which must record nothing at all.
#[must_use]
fn lease_note(previous: u8, now: u8) -> Option<LeaseNote> {
    match (previous, now) {
        (p, SEEN_LOST) if p != SEEN_LOST => Some(LeaseNote::Lost),
        (SEEN_LOST, SEEN_VALID) => Some(LeaseNote::Regained),
        // A first sight of a healthy lease is not a recovery: there was no gap.
        _ => None,
    }
}

impl LeaseWatcher {
    /// Build a watcher and **arm** the lease gate.
    ///
    /// Arming happens in the constructor, before the first refresh, so there is no
    /// window in which the process is running with the gate open and no monitor
    /// seen. Construction is the moment the process commits to being watched.
    #[must_use]
    pub fn new(store: MonitorStore, kill_switch: Arc<KillSwitch>, cadence: Duration) -> Self {
        kill_switch.require_monitor_lease();
        Self {
            store,
            kill_switch,
            cadence,
            seen: AtomicU8::new(SEEN_NOTHING_YET),
        }
    }

    /// Write the record a lease gap leaves behind.
    ///
    /// Deliberately the **last** thing a refresh does, and deliberately unable to
    /// fail the refresh. The mirror updates above are what keeps the gate honest;
    /// this is what lets somebody find out afterwards why every order was refused
    /// with `monitor_unavailable`. A database that cannot be written is exactly
    /// the case in which the lease is missing, so a failure here is expected and
    /// is logged rather than propagated.
    ///
    /// The gate itself is not touched from here. `Fact::LeaseLost`'s action is
    /// [`RecordLeaseGap`][r] precisely because the halt has already
    /// happened, through `monitor.lease` — the first table this function's caller
    /// reads.
    ///
    /// [r]: crate::facts::FactAction::RecordLeaseGap
    async fn note_lease(&self, note: LeaseNote, status: LeaseStatus) {
        let outcome = match note {
            LeaseNote::Lost => {
                tracing::warn!(?status, "monitor lease lost — recording the gap");
                // Through `facts::decide` rather than naming the action here, so
                // what a lost lease does is decided in one place for all three
                // facts rather than in one place per raise site. A raise site
                // that chose its own action would be free to choose a different
                // one, and the drill would then rehearse a path production does
                // not take.
                let fact = Fact::LeaseLost;
                let mut outcome = Ok(());
                for action in crate::facts::decide(&fact, &DependencyMap::new(), &[]) {
                    outcome = self.store.carry_out_fact(&fact, &action).await;
                    if outcome.is_err() {
                        break;
                    }
                }
                outcome
            }
            LeaseNote::Regained => {
                match MonitorEvent::new(
                    risk::Scope::Global,
                    Family::Lease,
                    EventKind::Resumed,
                    RUNTIME_ACTOR,
                ) {
                    Ok(event) => {
                        let event = event
                            .with_reason("a monitor renewed the lease; the lease gate reopened");
                        self.store.record_event(&event).await
                    }
                    Err(e) => Err(crate::store::StoreError::Invalid(e.to_string())),
                }
            }
        };
        if let Err(e) = outcome {
            tracing::error!(error = %e, ?note, "the lease gap could not be recorded");
        }
    }

    /// One refresh. Returns what the lease said.
    pub async fn refresh(&self) -> LeaseStatus {
        let status = self.store.lease_status(Utc::now()).await;
        self.kill_switch.observe_lease(status.permits_orders());

        match self.store.halted_scopes().await {
            Ok(scopes) => self.kill_switch.replace_halted(&scopes),
            Err(e) => {
                // The scoped view could not be read. The lease observation above
                // has already closed the gate if the lease is gone; if the lease
                // is valid but this query failed, leaving the previous scoped view
                // in place is the conservative choice — it can only be too
                // restrictive, never too permissive, because a scope that was
                // halted stays halted until a successful read says otherwise.
                tracing::error!(error = %e, "halted scopes unreadable — keeping the previous mirror");
            }
        }

        // The scale-downs, which travel the same way and for the same reason.
        //
        // Without this the automatic scale-down of COMP-007 §4 P4 does not cross
        // the process boundary at all: the monitor writes `scale_pct` to
        // `monitor.strategy_state` in its own process, the runtime's mirror is
        // never told, and the strategy carries on at full size with a monitor page
        // that says otherwise. The one existing caller of `scale_strategy` is the
        // *manual* API route, which is in the same process as the mirror it
        // updates — so the manual path worked and the automatic one silently did
        // not.
        match self.store.strategy_runtime_states().await {
            Ok(rows) => {
                for (strategy_id, _, scale_pct, _) in rows {
                    self.kill_switch.scale_strategy(&strategy_id, scale_pct);
                }
            }
            Err(e) => {
                // Same conservatism as above: keeping the previous scales can only
                // leave a strategy smaller than the database says, never larger.
                tracing::error!(error = %e, "strategy scales unreadable — keeping the previous mirror");
            }
        }

        // COMP-007 §4.1: "a lost lease is a fact, not an estimate". This is the
        // production raise site, and it is here rather than in the monitor
        // because the monitor is the thing that is missing.
        //
        // The decision this fact carries has already reached the order gate —
        // `observe_lease` above, from `lease_status`, which is `monitor.lease`.
        // What did not exist before was the record.
        let now = if status.permits_orders() {
            SEEN_VALID
        } else {
            SEEN_LOST
        };
        if let Some(note) = lease_note(self.seen.swap(now, Ordering::AcqRel), now) {
            self.note_lease(note, status).await;
        }
        status
    }

    /// Run until cancelled.
    pub async fn run(self) {
        loop {
            let status = self.refresh().await;
            if !status.permits_orders() {
                tracing::warn!(?status, "monitor lease not valid — orders refused");
            }
            tokio::time::sleep(self.cadence).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{
        BehaviourSample, ExecutionSample, Metric, OrderObservation, OrderOutcome,
        Window as CollectWindow,
    };
    use crate::dwell::DwellConfig;
    use crate::limits::{ControlLimit, LimitBook, LimitMethod, LimitSeverity};
    use chrono::{DateTime, Duration as ChronoDuration};

    const STRATEGY: &str = "s-1";

    #[test]
    fn a_process_that_starts_while_the_monitor_is_healthy_records_nothing() {
        // The boring case, and the one that decides whether the timeline is
        // readable. Every API restart would otherwise leave a "lease regained"
        // row for a gap that never happened.
        assert_eq!(lease_note(SEEN_NOTHING_YET, SEEN_VALID), None);
        assert_eq!(lease_note(SEEN_VALID, SEEN_VALID), None);
    }

    #[test]
    fn a_lease_gap_is_recorded_on_its_edges_and_not_once_per_cadence() {
        // COMP-007 §4.1 raises the fact; §6 says the record is what the operator
        // reads. A watcher refreshing every five seconds through a twenty-minute
        // outage writing a row each time makes the second true and useless.
        assert_eq!(lease_note(SEEN_VALID, SEEN_LOST), Some(LeaseNote::Lost));
        assert_eq!(lease_note(SEEN_LOST, SEEN_LOST), None);
        assert_eq!(lease_note(SEEN_LOST, SEEN_VALID), Some(LeaseNote::Regained));
    }

    #[test]
    fn a_process_that_starts_with_no_monitor_at_all_records_the_gap() {
        // The one that matters: a runtime booted while the monitor is down is
        // refusing every order, and with no row saying so the operator's first
        // question has no answer anywhere.
        assert_eq!(lease_note(SEEN_NOTHING_YET, SEEN_LOST), Some(LeaseNote::Lost));
    }

    #[test]
    fn the_lease_fact_is_recorded_and_does_not_trip_the_durable_switch() {
        // The decision, asserted rather than described. Tripping `global` here
        // would need a typed human reset — with reconciliation clean across
        // every venue — after each monitor restart, and `Family::Lease` is not
        // suppressible by a maintenance window, so a rolling deploy would stop
        // the book until somebody performed the whole §5 ritual. The gate is
        // already closed by the missing lease row itself.
        let actions = crate::facts::decide(&Fact::LeaseLost, &DependencyMap::new(), &[]);
        assert_eq!(actions, vec![crate::facts::FactAction::RecordLeaseGap]);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, crate::facts::FactAction::Halt(_))),
            "a lease gap must not need a human to clear it"
        );
        assert_eq!(
            actions[0].gate_table(),
            crate::facts::GateTable::Lease,
            "the fact's decision must travel through a table refresh reads"
        );
    }

    #[test]
    fn the_watched_process_signs_its_own_records() {
        // A row signed `monitor` for a gap in which no monitor was running would
        // answer the operator's first question with a lie.
        assert_ne!(RUNTIME_ACTOR, MONITOR_ACTOR);
        assert!(!RUNTIME_ACTOR.starts_with("agent:"));
    }

    fn at(mins: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + mins * 60, 0).expect("valid timestamp")
    }

    fn limit(
        metric: Metric,
        lower: Option<f64>,
        upper: Option<f64>,
        sev: LimitSeverity,
    ) -> ControlLimit {
        ControlLimit {
            strategy_id: STRATEGY.to_owned(),
            metric: metric.as_str().to_owned(),
            family: metric.family(),
            severity: sev,
            version: 1,
            lower,
            upper,
            method: LimitMethod::PaperEwma3Sigma,
            declared_from: "paper-run-7".to_owned(),
            declared_at: Utc::now(),
            declared_by: "human:mason".to_owned(),
        }
    }

    /// A cadence whose limits were declared before any of the readings below
    /// existed — which is the only order COMP-007 §P2 allows.
    fn cadence() -> Cadence {
        let mut book = LimitBook::new();
        book.declare(limit(Metric::FillRate, Some(0.80), None, LimitSeverity::Breach));
        book.declare(limit(Metric::CancelRatio, None, Some(0.50), LimitSeverity::Breach));
        book.declare(limit(Metric::CusumLow, Some(-5.0), None, LimitSeverity::Breach));
        book.declare(limit(Metric::Sharpe60d, Some(0.50), None, LimitSeverity::Warning));
        Cadence::new(Evaluator::new(book, DwellConfig::default()))
    }

    fn reading(metric: Metric, value: f64) -> MetricReading {
        MetricReading::new(STRATEGY, metric.as_str(), metric.family(), value)
    }

    fn drive(
        c: &mut Cadence,
        readings: &[MetricReading],
        times: usize,
        mut state: StrategyState,
    ) -> CadenceDecision {
        let mut last = None;
        for _ in 0..times {
            let d = c.assess(STRATEGY, readings.to_vec(), state, HaltPolicy::Hold);
            state = d.transition.to;
            last = Some(d);
        }
        last.expect("at least one cadence")
    }

    #[test]
    fn a_sustained_fill_rate_breach_scales_to_a_quarter_and_the_event_names_the_numbers() {
        // COMP-007 §8.4, end to end from a measured window rather than from a
        // hand-written reading: orders in, scale-down out.
        let w = CollectWindow::ending_at(at(60), ChronoDuration::minutes(60));
        let mut orders = Vec::new();
        for i in 0..10 {
            orders.push(OrderObservation {
                strategy_id: STRATEGY.to_owned(),
                instrument_id: "BTC-USD".to_owned(),
                placed_at: at(i),
                // Four in ten fill: a fill rate of 0.4 against a declared floor
                // of 0.8.
                outcome: if i < 4 {
                    OrderOutcome::Filled
                } else {
                    OrderOutcome::Cancelled
                },
                first_fill_at: None,
                slippage_vs_model_bps: None,
            });
        }
        let readings = ExecutionSample::measure(w, &orders).readings(STRATEGY);
        assert!(!readings.is_empty(), "ten terminal orders must measure something");

        let mut c = cadence();
        let d = drive(&mut c, &readings, 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::ScaledDown);
        assert_eq!(d.scale_pct, crate::SCALE_DOWN_PCT);
        assert_eq!(d.events.len(), 1, "one state change, one event");

        let e = &d.events[0];
        assert_eq!(e.kind, EventKind::ScaledDown);
        assert_eq!(e.metric.as_deref(), Some(Metric::FillRate.as_str()));
        assert!(e.names_metric_value_limit_and_scope(), "§8.4");
        assert_eq!(e.limit_value, Some(0.80));
        assert_eq!(e.observed_value, Some(0.4));
        assert_eq!(e.actor, MONITOR_ACTOR);
        assert_eq!(e.scope.to_string(), "strategy:s-1");
        assert_eq!(e.to_state.as_deref(), Some("scaled_down"));
    }

    #[test]
    fn one_bad_cadence_records_a_blip_and_moves_nothing() {
        // §4.1. The blip is written — the monitor's own precision is scored
        // against what it saw — but nothing acts on it.
        let mut c = cadence();
        let d = c.assess(
            STRATEGY,
            vec![reading(Metric::FillRate, 0.1)],
            StrategyState::Nominal,
            HaltPolicy::Hold,
        );
        assert!(!d.acted());
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.events[0].kind, EventKind::Blip);
        assert!(!d.events[0].kind.is_scoreable_alarm());
        assert_eq!(d.scale_pct, 100);
    }

    #[test]
    fn a_cadence_that_measured_nothing_says_so_rather_than_reporting_calm() {
        // The dangerous direction on this surface. A collector that read no
        // orders must not be indistinguishable from one that read a healthy
        // book — and in the event log it is, because there are no events either
        // way. Only `judged` separates them.
        let mut c = cadence();
        let nothing = c.assess(STRATEGY, Vec::new(), StrategyState::Nominal, HaltPolicy::Hold);
        assert_eq!(nothing.judged, 0);
        assert!(!nothing.acted());

        let healthy = c.assess(
            STRATEGY,
            vec![reading(Metric::FillRate, 0.95)],
            StrategyState::Nominal,
            HaltPolicy::Hold,
        );
        assert_eq!(healthy.judged, 1);
        assert!(!healthy.acted());
        assert_eq!(
            nothing.events, healthy.events,
            "the event log cannot tell these apart, which is why `judged` must"
        );
        assert_ne!(nothing.judged, healthy.judged);
    }

    #[test]
    fn a_metric_with_no_declared_limit_is_named_rather_than_passed() {
        // A strategy whose every metric is undeclared produces no events and no
        // transition, and would otherwise be the best-behaved row on the page.
        let mut c = cadence();
        let d = c.assess(
            STRATEGY,
            vec![reading(Metric::LatencyP95Ms, 9_999.0)],
            StrategyState::Nominal,
            HaltPolicy::Hold,
        );
        assert_eq!(d.undeclared, vec![Metric::LatencyP95Ms.as_str().to_owned()]);
        assert!(!d.acted());
        assert_eq!(d.judged, 1);
    }

    #[test]
    fn a_pnl_breach_in_the_same_cadence_as_an_execution_breach_halts() {
        // Both sustained at once. Scaling to 25% and calling it handled would
        // leave a strategy whose returns have broken still trading, at a quarter
        // of the size, which is not what §4 says happens to it.
        let mut c = cadence();
        let readings = vec![reading(Metric::FillRate, 0.1), reading(Metric::CusumLow, -9.0)];
        let d = drive(&mut c, &readings, 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::Halted);
        assert_eq!(d.events.len(), 1, "one incident, one record");
        assert_eq!(d.events[0].kind, EventKind::Halted);
        assert_eq!(d.events[0].metric.as_deref(), Some(Metric::CusumLow.as_str()));
        assert_eq!(d.events[0].family, Family::Pnl);
    }

    #[test]
    fn when_two_metrics_of_one_family_breach_the_event_names_the_worse_one() {
        // The operator's first question is "how bad", and an event naming
        // whichever metric the collector happened to emit first does not answer
        // it.
        let mut c = cadence();
        let readings = vec![
            reading(Metric::FillRate, 0.79),    // a hair under the 0.80 floor
            reading(Metric::CancelRatio, 0.99), // far over the 0.50 ceiling
        ];
        let d = drive(&mut c, &readings, 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::ScaledDown);
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.events[0].metric.as_deref(), Some(Metric::CancelRatio.as_str()));
    }

    #[test]
    fn a_warning_records_a_warning_and_does_not_scale() {
        let mut c = cadence();
        let d = drive(&mut c, &[reading(Metric::Sharpe60d, 0.1)], 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::Warning);
        assert_eq!(d.scale_pct, 100);
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.events[0].kind, EventKind::Warning);
    }

    #[test]
    fn the_record_is_always_the_first_thing_the_plan_does() {
        // §8.5, asserted on the plan rather than on a comment: the transition
        // carries `Action::Record` first, and `apply` writes the events before it
        // touches the state.
        let mut c = cadence();
        let d = drive(&mut c, &[reading(Metric::CusumLow, -9.0)], 3, StrategyState::Nominal);
        assert_eq!(d.transition.actions.first(), Some(&Action::Record));
        assert!(!d.events.is_empty(), "and there is a record to write");
    }

    #[test]
    fn recovery_needs_twice_the_dwell_and_then_releases_the_scale() {
        // §4.1's hysteresis, through the cadence rather than through the tracker.
        let mut c = cadence();
        let breach = drive(&mut c, &[reading(Metric::FillRate, 0.1)], 3, StrategyState::Nominal);
        assert_eq!(breach.transition.to, StrategyState::ScaledDown);

        let recovery = DwellConfig::default().recovery_cadences();
        let good = vec![reading(Metric::FillRate, 0.99)];
        let mut state = StrategyState::ScaledDown;
        for i in 1..recovery {
            let d = c.assess(STRATEGY, good.clone(), state, HaltPolicy::Hold);
            assert!(!d.acted(), "released after {i} good cadences, needs {recovery}");
            state = d.transition.to;
        }
        let d = c.assess(STRATEGY, good, state, HaltPolicy::Hold);
        assert_eq!(d.transition.to, StrategyState::Nominal);
        assert_eq!(d.scale_pct, 100);
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.events[0].kind, EventKind::ScaleReleased);
    }

    #[test]
    fn an_oscillating_metric_never_flaps_the_book() {
        // §8.9, driven through the cadence: a fill rate bouncing either side of
        // its declared floor every cadence, for a hundred cadences.
        let mut c = cadence();
        let mut state = StrategyState::Nominal;
        let mut changes = 0;
        let mut blips = 0;
        for i in 0..100 {
            let v = if i % 2 == 0 { 0.79 } else { 0.81 };
            let d = c.assess(
                STRATEGY,
                vec![reading(Metric::FillRate, v)],
                state,
                HaltPolicy::Hold,
            );
            if d.transition.changed() {
                changes += 1;
            }
            blips += d
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Blip)
                .count();
            state = d.transition.to;
        }
        assert_eq!(changes, 0, "the book flapped {changes} times");
        assert_eq!(state, StrategyState::Nominal);
        // And this did not pass by judging nothing: every odd cadence was outside
        // the declared floor and was recorded as a blip.
        assert_eq!(blips, 50, "the excursions must have been seen, and recorded");
    }

    #[test]
    fn a_halted_strategy_is_not_resumed_by_a_quiet_cadence() {
        // The adversarial one. Release of a halt is a typed human action (§5); if
        // a run of good readings released it, the confirmation would be
        // decorative.
        let mut c = cadence();
        let mut state = StrategyState::Halted;
        for _ in 0..50 {
            let d = c.assess(
                STRATEGY,
                vec![reading(Metric::FillRate, 0.99), reading(Metric::CusumLow, 0.0)],
                state,
                HaltPolicy::Hold,
            );
            state = d.transition.to;
        }
        assert_eq!(state, StrategyState::Halted);
    }

    #[test]
    fn a_maintenance_window_cannot_suppress_anything_the_collector_measures() {
        // §4.3: a window suppresses platform health and nothing else. The
        // collector's families are facts about the market and the book, and a
        // deploy is no reason to stop believing them.
        let w = crate::window::MaintenanceWindow::open(
            "human:mason",
            "rolling deploy",
            crate::window::DEFAULT_WINDOW_MINUTES,
            Utc::now(),
        )
        .expect("a bounded, attributed window");
        for m in Metric::all() {
            assert!(
                !w.suppresses(m.family(), Utc::now()),
                "{m} is in the {} family, which a window must never suppress",
                m.family()
            );
        }
    }

    #[test]
    fn a_behaviour_breach_scales_rather_than_halts() {
        // §4's table: behaviour, execution and data breaches scale down, because
        // scale-down is reversible and therefore does not need a human in the
        // moment. Driven from a measured window.
        let w = CollectWindow::ending_at(at(60), ChronoDuration::minutes(60));
        let orders: Vec<OrderObservation> = (0..10)
            .map(|i| OrderObservation {
                strategy_id: STRATEGY.to_owned(),
                instrument_id: "BTC-USD".to_owned(),
                placed_at: at(i),
                outcome: if i < 9 {
                    OrderOutcome::Cancelled
                } else {
                    OrderOutcome::Filled
                },
                first_fill_at: None,
                slippage_vs_model_bps: None,
            })
            .collect();
        let readings: Vec<MetricReading> = BehaviourSample::measure(w, &orders, Some(4), &[], Some(2))
            .readings(STRATEGY)
            .into_iter()
            .filter(|r| r.metric == Metric::CancelRatio.as_str())
            .collect();
        assert_eq!(readings.len(), 1, "the cancel ratio must have been measured");

        let mut c = cadence();
        let d = drive(&mut c, &readings, 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::ScaledDown);
        assert_eq!(d.events[0].family, Family::Behaviour);
        assert_eq!(d.events[0].observed_value, Some(0.9));
    }

    #[test]
    fn a_measured_pnl_series_halts_the_strategy_through_the_same_cadence() {
        // COMP-007 §8.5, joined up: a real return series, standardised against a
        // declared baseline, produces readings; the readings go through the same
        // `assess` as every other family; the strategy halts and the record names
        // the numbers. Without this the P&L module would be complete, tested and
        // reachable by nothing.
        let baseline = crate::pnl::ReturnBaseline::declared(
            0.002,
            0.010,
            crate::pnl::CONTINUOUS_PERIODS_PER_YEAR,
            "backtest-run-41",
        )
        .expect("a declared baseline");
        let returns: Vec<crate::pnl::DailyReturn> = (0..40)
            .map(|i| crate::pnl::DailyReturn {
                date: chrono::NaiveDate::from_num_days_from_ce_opt(738_000 + i)
                    .expect("valid date"),
                value: -0.008,
            })
            .collect();
        let sample = crate::pnl::PnlSample::measure(&returns, baseline);
        let mut chart = crate::baseline::Cusum::default();
        let readings = sample.readings(STRATEGY, &mut chart);
        assert!(
            readings.iter().any(|r| r.metric == Metric::CusumLow.as_str()),
            "the series must produce a cusum reading"
        );

        let mut c = cadence();
        let d = drive(&mut c, &readings, 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::Halted);
        assert_eq!(d.events.len(), 1);
        assert_eq!(d.events[0].kind, EventKind::Halted);
        assert!(d.events[0].names_metric_value_limit_and_scope());
        assert_eq!(d.events[0].limit_value, Some(-5.0), "the declared decision interval");
    }

    #[test]
    fn a_halt_decision_carries_the_event_that_trips_the_switch() {
        // The gap this guards, which cost a real bug: `apply` used to write the
        // event and the strategy state and stop there, and the order gate's mirror
        // is refreshed from `halted_scopes`, which reads `monitor.kill_switch`.
        // A halted strategy with an event row explaining why would have gone on
        // trading. `apply` now routes the halt event through `MonitorStore::trip`,
        // which needs exactly two things from the decision: a `Halted` event, and
        // a reason on it.
        let mut c = cadence();
        let d = drive(&mut c, &[reading(Metric::CusumLow, -9.0)], 3, StrategyState::Nominal);
        assert_eq!(d.transition.to, StrategyState::Halted);
        let halt = d
            .events
            .iter()
            .find(|e| e.kind == EventKind::Halted)
            .expect("a halting decision must carry a Halted event for `trip` to find");
        let reason = halt.reason.as_deref().expect("`trip` requires a reason");
        assert!(reason.contains(Metric::CusumLow.as_str()), "{reason}");
        assert!(reason.contains("-5"), "the reason must name the bound: {reason}");
        assert_eq!(halt.scope.to_string(), "strategy:s-1");
    }

    #[test]
    fn every_acting_decision_names_its_numbers_in_a_reason() {
        // "Execution breach" costs the operator the first five minutes of every
        // incident.
        let mut c = cadence();
        for (m, v) in [
            (Metric::FillRate, 0.1),
            (Metric::CancelRatio, 0.99),
            (Metric::Sharpe60d, 0.1),
        ] {
            let mut c2 = cadence();
            let d = drive(&mut c2, &[reading(m, v)], 3, StrategyState::Nominal);
            assert!(d.acted(), "{m} must act after three cadences");
            for e in &d.events {
                let r = e.reason.as_deref().unwrap_or("");
                assert!(r.contains(m.as_str()), "{m}: {r}");
            }
        }
        // And a blip carries one too, since it is the row the precision report is
        // later labelled against.
        let d = c.assess(
            STRATEGY,
            vec![reading(Metric::FillRate, 0.1)],
            StrategyState::Nominal,
            HaltPolicy::Hold,
        );
        assert_eq!(d.events.len(), 1);
        assert!(d.events[0].reason.is_some());
    }

    #[test]
    fn the_cadence_the_running_process_builds_still_waits_three_evaluations() {
        // `run` builds a `Cadence::default()`. A default that carried a dwell of
        // zero would act on every single excursion in production while every test
        // that constructs its own config passed — the shape of bug that only ever
        // shows up as alarm fatigue on a live book.
        let mut c = Cadence::default();
        assert_eq!(c.evaluator().dwell_tracker().config().min_dwell_cadences, 3);
        // And with no limits loaded yet it judges nothing, rather than treating
        // every reading as inside.
        let d = c.assess(
            STRATEGY,
            vec![reading(Metric::FillRate, 0.0)],
            StrategyState::Nominal,
            HaltPolicy::Hold,
        );
        assert_eq!(d.undeclared, vec![Metric::FillRate.as_str().to_owned()]);
        assert!(!d.acted());
    }

    #[test]
    fn no_cadence_event_can_be_authored_by_an_agent() {
        // §8.6 at this layer too. The actor is a constant, and `MonitorEvent::new`
        // refuses an `agent:` one — so the way this breaks is a caller threading a
        // string in, and `assess` has no parameter to thread one through.
        assert!(!MONITOR_ACTOR.starts_with("agent:"));
        let mut c = cadence();
        let d = drive(&mut c, &[reading(Metric::FillRate, 0.1)], 3, StrategyState::Nominal);
        assert!(!d.events.is_empty());
        for e in &d.events {
            assert_eq!(e.actor, MONITOR_ACTOR);
            assert!(!e.is_drill, "a real cadence is not a rehearsal");
        }
    }
}
