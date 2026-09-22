//! Postgres persistence for the monitor (COMP-007 §5, §6).
//!
//! Its own pool, its own connection, no dependency on the API process. That is the
//! whole point: the monitor's ability to say "halt" must not travel through the
//! thing it may be halting.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use risk::Scope;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::collect::{OrderObservation, OrderOutcome, Window as CollectWindow};
use crate::control::{ResetRefusal, ResetRequest};
use crate::drill::DrillOutcome;
use crate::events::{EventKind, MonitorEvent};
use crate::facts::FactAction;
use crate::family::{Fact, Family};
use crate::lease::{Lease, LeaseStatus, LeaseTerms};
use crate::limits::{ControlLimit, LimitBook, LimitMethod};
use crate::platform::ComponentBeat;
use crate::precision::{FamilyPrecision, PrecisionLabel};
use crate::window::MaintenanceWindow;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("reset refused: {0}")]
    Reset(#[from] ResetRefusal),
    #[error("{0}")]
    Invalid(String),
}

/// Convert a statistic to the numeric the schema stores.
///
/// `None` for a non-finite reading rather than a panic or a zero: a metric whose
/// computation produced NaN has a breach recorded against it (see
/// `ControlLimit::breached_by`) and no value, which is the truth — writing 0 there
/// would put a number in the audit trail that nothing measured.
fn numeric(v: Option<f64>) -> Option<Decimal> {
    v.and_then(Decimal::from_f64_retain)
}

/// The monitor's database.
#[derive(Clone, Debug)]
pub struct MonitorStore {
    pg: PgPool,
}

impl MonitorStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pg
    }

    // ── Lease ───────────────────────────────────────────────────────────────

    /// Renew (or take) the lease.
    pub async fn renew_lease(
        &self,
        terms: &LeaseTerms,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO monitor.lease (lease_id, holder, cadence_ms, renewed_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (lease_id) DO UPDATE
               SET holder = EXCLUDED.holder,
                   cadence_ms = EXCLUDED.cadence_ms,
                   renewed_at = EXCLUDED.renewed_at,
                   expires_at = EXCLUDED.expires_at",
        )
        .bind(&terms.lease_id)
        .bind(&terms.holder)
        .bind(terms.cadence_ms)
        .bind(now)
        .bind(terms.expires_at(now))
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Read the lease row, if any.
    pub async fn read_lease(&self) -> Result<Option<Lease>, StoreError> {
        let row = sqlx::query(
            "SELECT lease_id, holder, cadence_ms, renewed_at, expires_at
               FROM monitor.lease WHERE lease_id = $1",
        )
        .bind(crate::LEASE_ID)
        .fetch_optional(&self.pg)
        .await?;
        Ok(row.map(|r| Lease {
            lease_id: r.get("lease_id"),
            holder: r.get("holder"),
            cadence_ms: r.get("cadence_ms"),
            renewed_at: r.get("renewed_at"),
            expires_at: r.get("expires_at"),
        }))
    }

    /// The order path's question, answered fail-closed.
    ///
    /// An unreadable database is [`LeaseStatus::Unreadable`], which does not permit
    /// orders. The error is *not* propagated with `?` on purpose: a caller that
    /// wrote `store.lease_status().await?` would turn a database blip into an
    /// error that some outer layer logs and ignores, and the gate would be open.
    pub async fn lease_status(&self, now: DateTime<Utc>) -> LeaseStatus {
        match self.read_lease().await {
            Ok(lease) => LeaseStatus::of(lease.as_ref(), now),
            Err(e) => {
                tracing::error!(error = %e, "monitor lease unreadable — failing closed");
                LeaseStatus::Unreadable
            }
        }
    }

    /// Record a heartbeat for a platform component.
    pub async fn heartbeat(
        &self,
        component: &str,
        instance: &str,
        cadence_ms: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO monitor.heartbeat (component, instance, beat_at, cadence_ms)
             VALUES ($1, $2, now(), $3)
             ON CONFLICT (component, instance) DO UPDATE
               SET beat_at = now(), cadence_ms = EXCLUDED.cadence_ms",
        )
        .bind(component)
        .bind(instance)
        .bind(cadence_ms)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Every component's heartbeat, with staleness decided per row against that
    /// row's own declared cadence (COMP-007 §3 "Platform").
    ///
    /// Every row rather than only the stale ones, because the question
    /// [`platform::assess`][a] answers is about *components* and not about rows:
    /// nothing deletes heartbeats, so a restarted process leaves a permanently
    /// stale row beside its live one, and a caller given only the stale rows
    /// cannot tell the difference between a component that is down and one that
    /// has ever been redeployed. That distinction is the difference between an
    /// ordinary deploy and a global halt.
    ///
    /// [a]: crate::platform::assess
    pub async fn component_beats(&self, missed: i64) -> Result<Vec<ComponentBeat>, StoreError> {
        let rows = sqlx::query(
            "SELECT component, instance,
                    beat_at < now() - (cadence_ms * $1 || ' milliseconds')::interval AS stale
               FROM monitor.heartbeat
              ORDER BY component, instance",
        )
        .bind(missed)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ComponentBeat {
                component: r.get("component"),
                instance: r.get("instance"),
                // A row whose staleness the database could not decide — a null
                // `beat_at` cannot happen under the schema, but the expression is
                // nullable to Postgres — counts as stale. The safe direction for
                // a liveness question is "not seen".
                stale: r.get::<Option<bool>, _>("stale").unwrap_or(true),
            })
            .collect())
    }

    /// Components that have missed `missed` consecutive beats as of `now`.
    pub async fn stale_components(&self, missed: i64) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query(
            "SELECT component, instance FROM monitor.heartbeat
              WHERE beat_at < now() - (cadence_ms * $1 || ' milliseconds')::interval",
        )
        .bind(missed)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let c: String = r.get("component");
                let i: String = r.get("instance");
                format!("{c}/{i}")
            })
            .collect())
    }

    // ── Kill switch ─────────────────────────────────────────────────────────

    /// Every scope currently halted.
    pub async fn halted_scopes(&self) -> Result<Vec<Scope>, StoreError> {
        let rows = sqlx::query("SELECT scope FROM monitor.kill_switch WHERE state = 'halted'")
            .fetch_all(&self.pg)
            .await?;
        let mut out = Vec::new();
        for r in rows {
            let s: String = r.get("scope");
            match s.parse::<Scope>() {
                Ok(scope) => out.push(scope),
                // Fail closed on a row we cannot read: an unparseable scope is a
                // halt somebody meant, and dropping it silently would resume
                // whatever it was protecting.
                Err(e) => {
                    tracing::error!(error = %e, "unparseable kill-switch scope — treating as global");
                    out.push(Scope::Global);
                }
            }
        }
        Ok(out)
    }

    /// Trip a scope and write its event, in one transaction.
    ///
    /// The event goes in **first**. COMP-007 §8.5 requires the trip record to be
    /// present before the first refused order, and a record written after the state
    /// change is a record that is missing in exactly the case that matters — the
    /// process dying between the two.
    pub async fn trip(&self, event: &MonitorEvent, reason: &str) -> Result<(), StoreError> {
        if event.actor.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{} may not trip the kill switch",
                event.actor
            )));
        }
        let mut tx = self.pg.begin().await?;
        insert_event(&mut tx, event).await?;
        sqlx::query(
            "INSERT INTO monitor.kill_switch
               (scope, state, tripped_by, reason, metric, observed_value, limit_value,
                tripped_at, updated_at)
             VALUES ($1, 'halted', $2, $3, $4, $5, $6, now(), now())
             ON CONFLICT (scope) DO UPDATE
               SET state = 'halted',
                   tripped_by = EXCLUDED.tripped_by,
                   reason = EXCLUDED.reason,
                   metric = EXCLUDED.metric,
                   observed_value = EXCLUDED.observed_value,
                   limit_value = EXCLUDED.limit_value,
                   tripped_at = now(),
                   updated_at = now()",
        )
        .bind(event.scope.to_string())
        .bind(&event.actor)
        .bind(reason)
        .bind(event.metric.as_deref())
        .bind(numeric(event.observed_value))
        .bind(numeric(event.limit_value))
        .execute(&mut *tx)
        .await?;
        sync_legacy_boot_column(&mut tx, &event.scope, false).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Carry out one [`FactAction`], record first (COMP-007 §4.1, §8.5).
    ///
    /// The funnel every raised fact goes through, and the reason it exists is
    /// the rule in [`crate::facts`]: each arm writes to the table that action's
    /// [`GateTable`][gt] names, and those are the three tables
    /// [`LeaseWatcher::refresh`][r] reads. Nothing here writes only an event and
    /// calls it a halt.
    ///
    /// The actor comes from the action rather than from a parameter, so there is
    /// no string a caller can thread an `agent:` prefix through.
    ///
    /// [gt]: crate::facts::GateTable
    /// [r]: crate::service::LeaseWatcher::refresh
    pub async fn carry_out_fact(
        &self,
        fact: &Fact,
        action: &FactAction,
    ) -> Result<(), StoreError> {
        let reason = fact.reason();
        let event = MonitorEvent::new(
            action.scope(),
            fact.family(),
            match action {
                // The same kind for two actions, and deliberately: a lease gap
                // *is* a halt from the order path's side — every order is refused
                // with `monitor_unavailable` — and the timeline is what the
                // operator reads to find out why. What differs is which table
                // carried the decision, which `FactAction::gate_table` answers.
                FactAction::Halt(_) | FactAction::RecordLeaseGap => EventKind::Halted,
                FactAction::Scale { .. } => EventKind::ScaledDown,
            },
            action.actor(),
        )
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .with_reason(reason.clone());

        match action {
            // One transaction, event then switch row, so §8.5's ordering holds
            // even if the process dies between the two — there is no "between".
            FactAction::Halt(_) => self.trip(&event, &reason).await,
            FactAction::Scale {
                strategy_id,
                from,
                state,
                scale_pct,
            } => {
                let event = event.with_states(from.as_str(), state.as_str());
                self.record_event(&event).await?;
                self.set_strategy_state(strategy_id, state.as_str(), i16::from(*scale_pct))
                    .await
            }
            // The gate is already closed by the missing lease row itself; see
            // `FactAction::RecordLeaseGap`. What was missing is this row.
            FactAction::RecordLeaseGap => self.record_event(&event).await,
        }
    }

    /// The receiving half of a venue reconciliation mismatch (COMP-007 §4, §8.3;
    /// S-7.6).
    ///
    /// One call, because the alternative is four: `facts::decide`, a
    /// `DependencyMap`, a state vector and a loop, in a crate whose author is
    /// reconciling positions and has no reason to know what any of them are. A
    /// caller assembling its own [`FactAction`] could pair a mismatch with a
    /// scale-down, or with the wrong venue, and every test on this side would
    /// still pass.
    ///
    /// **The order is the whole point.** `reconciliation::divergence::
    /// check_divergence` today calls `KillSwitch::trip_scope` and writes no row.
    /// The mirror is refreshed from `monitor.kill_switch` by
    /// [`KillSwitch::replace_halted`][rh], which **replaces** — so that halt is
    /// not merely unrecorded, it is erased within one refresh cadence and the
    /// venue that disagrees with our book about what is owned resumes trading
    /// about five seconds later. Here the durable row is written first and the
    /// mirror second, so the venue is halted before this returns and stays halted
    /// after the next refresh.
    ///
    /// If the durable write fails, the mirror is left alone and the error is
    /// returned. That is not a gap: the database this could not write is the same
    /// database [`LeaseWatcher::refresh`][r] reads the lease from, so the lease
    /// gate has already refused every order platform-wide (§4 "fail closed"). An
    /// in-memory trip with nothing behind it would add five seconds of protection
    /// and then silently resume, which is the failure this method exists to
    /// remove.
    ///
    /// Returns the scope that is now halted, so the caller logs the venue it
    /// halted rather than the venue it meant to.
    ///
    /// [rh]: risk::KillSwitch::replace_halted
    /// [r]: crate::service::LeaseWatcher::refresh
    pub async fn carry_out_mismatch(
        &self,
        venue: &str,
        gate: &risk::KillSwitch,
    ) -> Result<Scope, StoreError> {
        // The one place in this crate where a `&str` becomes a
        // [`Venue`](crate::feed::Venue), and deliberately at the boundary rather
        // than deeper: `crates/reconciliation` hands over the venue it
        // reconciled as a string, it is checked here, and nothing inside this
        // crate then handles a venue nobody validated.
        let venue = crate::feed::Venue::named(venue).map_err(|_| {
            StoreError::Invalid("a reconciliation mismatch must name the venue it is at".to_owned())
        })?;
        let fact = Fact::ReconciliationMismatch {
            venue: venue.clone(),
        };
        // Through `facts::decide` rather than naming the halt here, so the venue
        // scoping and the "that venue and no other" rule stay in one place for
        // every raise site.
        let actions = crate::facts::decide(&fact, &crate::evaluate::DependencyMap::new(), &[]);
        let mut halted = None;
        for action in &actions {
            self.carry_out_fact(&fact, action).await?;
            halted = Some(action.scope());
        }
        let halted = halted.ok_or_else(|| {
            StoreError::Invalid(format!(
                "a reconciliation mismatch at {venue} decided nothing at all"
            ))
        })?;
        // Durable first, mirror second. Now, rather than at the next refresh, so
        // there is no window in which the row says halted and the gate does not.
        gate.trip_scope(&halted);
        Ok(halted)
    }

    /// When this scope was last reset, for the rate limit.
    pub async fn last_reset_at(&self, scope: &Scope) -> Result<Option<DateTime<Utc>>, StoreError> {
        let row = sqlx::query("SELECT reset_at FROM monitor.kill_switch WHERE scope = $1")
            .bind(scope.to_string())
            .fetch_optional(&self.pg)
            .await?;
        Ok(row.and_then(|r| r.get::<Option<DateTime<Utc>>, _>("reset_at")))
    }

    /// Release a halt, having validated the request.
    ///
    /// Validation happens here rather than only in the route, because the rate
    /// limit needs the stored `reset_at` and a check that reads its own input from
    /// the caller is not a check.
    pub async fn reset(&self, request: &ResetRequest) -> Result<(), StoreError> {
        let last = self.last_reset_at(&request.scope).await?;
        request.validate(last, Utc::now())?;

        let event = MonitorEvent::new(
            request.scope.clone(),
            Family::Lease,
            EventKind::Reset,
            &request.actor,
        )
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .with_reason(request.reason.clone());

        let mut tx = self.pg.begin().await?;
        insert_event(&mut tx, &event).await?;
        sqlx::query(
            "UPDATE monitor.kill_switch
                SET state = 'armed', reset_by = $2, reset_reason = $3,
                    reset_at = now(), updated_at = now()
              WHERE scope = $1",
        )
        .bind(request.scope.to_string())
        .bind(&request.actor)
        .bind(&request.reason)
        .execute(&mut *tx)
        .await?;
        sync_legacy_boot_column(&mut tx, &request.scope, true).await?;
        tx.commit().await?;
        Ok(())
    }

    /// What the durable tables say the global gate is, at boot.
    ///
    /// Fail-closed: an unreadable database, an unreadable column, or a disagreement
    /// between the two records all answer "halted". A process that cannot determine
    /// whether it was stopped must not decide it was not.
    pub async fn global_halted_at_boot(&self) -> bool {
        let scoped: Option<bool> =
            sqlx::query("SELECT state = 'halted' AS halted FROM monitor.kill_switch WHERE scope = 'global'")
                .fetch_optional(&self.pg)
                .await
                .ok()
                .flatten()
                .map(|r| r.get("halted"));

        let legacy: Option<bool> =
            sqlx::query("SELECT NOT trading_enabled AS halted FROM global_risk_config LIMIT 1")
                .fetch_optional(&self.pg)
                .await
                .ok()
                .flatten()
                .map(|r| r.get("halted"));

        match (scoped, legacy) {
            // Both present and agreeing, or only one present: take what is there.
            (Some(a), Some(b)) if a == b => a,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            // Disagreement, or nothing readable at all. Halted.
            //
            // A disagreement between two records of the same fact is the exact
            // situation in which the optimistic reading is worth the least: one of
            // them says somebody stopped this platform, and the cost of believing
            // that wrongly is a person typing a reset.
            (Some(a), Some(b)) => {
                tracing::error!(
                    scoped_halted = a,
                    legacy_halted = b,
                    "kill-switch records disagree at boot — failing closed"
                );
                true
            }
            (None, None) => {
                tracing::warn!("no kill-switch state readable at boot — failing closed");
                true
            }
        }
    }

    /// Write a state-change record that is not itself a trip.
    pub async fn record_event(&self, event: &MonitorEvent) -> Result<(), StoreError> {
        let mut tx = self.pg.begin().await?;
        insert_event(&mut tx, event).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The most recent events, newest first.
    pub async fn recent_events(&self, limit: i64) -> Result<Vec<(Uuid, String, String, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT event_id, scope, family, kind FROM monitor.events
              ORDER BY occurred_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get("event_id"),
                    r.get("scope"),
                    r.get("family"),
                    r.get("kind"),
                )
            })
            .collect())
    }

    // ── Observations (COMP-007 §3; S-7.7) ───────────────────────────────────

    /// Every strategy order that was placed inside `window`, with its outcome and
    /// its time to first fill.
    ///
    /// Manual orders (`strategy_id IS NULL`) are excluded: the execution and
    /// behaviour families are declared *per strategy*, and a human clicking buy
    /// has no baseline to be three sigma away from.
    ///
    /// `slippage_vs_model_bps` comes back `None` for every row, because nothing in
    /// `orders` or `fills` records what the fill model predicted. That is the
    /// truth and it is deliberately not smoothed over: the slippage statistics
    /// then report [`Unobserved::NoQualifyingObservations`][u] rather than a
    /// comfortable zero, which is what "the fill model is not wired up" should
    /// look like on the monitor page.
    ///
    /// [u]: crate::collect::Unobserved::NoQualifyingObservations
    pub async fn order_observations(
        &self,
        window: &CollectWindow,
    ) -> Result<Vec<OrderObservation>, StoreError> {
        let rows = sqlx::query(
            "SELECT o.strategy_id      AS strategy_id,
                    o.instrument_id    AS instrument_id,
                    o.created_at       AS placed_at,
                    o.state            AS state,
                    COUNT(f.fill_id)   AS fill_count,
                    MIN(f.filled_at)   AS first_fill_at
               FROM orders o
               LEFT JOIN fills f ON f.idempotency_key = o.idempotency_key
              WHERE o.strategy_id IS NOT NULL
                AND o.created_at >= $1
                AND o.created_at <= $2
              GROUP BY o.idempotency_key, o.strategy_id, o.instrument_id,
                       o.created_at, o.state
              ORDER BY o.created_at",
        )
        .bind(window.from)
        .bind(window.to)
        .fetch_all(&self.pg)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let state: String = r.get("state");
            let fill_count: i64 = r.get("fill_count");
            out.push(OrderObservation {
                strategy_id: r.get("strategy_id"),
                instrument_id: r.get("instrument_id"),
                placed_at: r.get("placed_at"),
                outcome: OrderOutcome::observed(
                    parse_order_state(&state)?,
                    usize::try_from(fill_count).unwrap_or(0),
                ),
                first_fill_at: r.get("first_fill_at"),
                slippage_vs_model_bps: None,
            });
        }
        Ok(out)
    }

    // ── Strategy state ──────────────────────────────────────────────────────

    pub async fn set_strategy_state(
        &self,
        strategy_id: &str,
        state: &str,
        scale_pct: i16,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO monitor.strategy_state (strategy_id, state, scale_pct, last_evaluated_at, updated_at)
             VALUES ($1, $2, $3, now(), now())
             ON CONFLICT (strategy_id) DO UPDATE
               SET state = EXCLUDED.state,
                   scale_pct = EXCLUDED.scale_pct,
                   last_evaluated_at = now(),
                   updated_at = now()",
        )
        .bind(strategy_id)
        .bind(state)
        .bind(scale_pct)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Every strategy the monitor is tracking, typed, with the halt policy that
    /// applies if it breaches.
    ///
    /// Separate from [`Self::strategy_states`], which serves the API as strings.
    /// The cadence needs the parsed [`StrategyState`] and the
    /// [`HaltPolicy`][hp] — and it needs an unparseable row to be an error rather
    /// than a default, because defaulting a state the monitor cannot read to
    /// `nominal` would resume a halted strategy on the next cadence.
    ///
    /// [hp]: crate::state::HaltPolicy
    pub async fn strategy_runtime_states(
        &self,
    ) -> Result<Vec<(String, crate::state::StrategyState, u8, crate::state::HaltPolicy)>, StoreError>
    {
        let rows = sqlx::query(
            "SELECT strategy_id, state, scale_pct, halt_policy FROM monitor.strategy_state",
        )
        .fetch_all(&self.pg)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let state: String = r.get("state");
            let policy: String = r.get("halt_policy");
            let scale: i16 = r.get("scale_pct");
            out.push((
                r.get("strategy_id"),
                state.parse().map_err(StoreError::Invalid)?,
                u8::try_from(scale).unwrap_or(100),
                policy.parse().map_err(StoreError::Invalid)?,
            ));
        }
        Ok(out)
    }

    pub async fn strategy_states(&self) -> Result<Vec<(String, String, i16)>, StoreError> {
        let rows = sqlx::query("SELECT strategy_id, state, scale_pct FROM monitor.strategy_state")
            .fetch_all(&self.pg)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("strategy_id"), r.get("state"), r.get("scale_pct")))
            .collect())
    }

    // ── Limits ──────────────────────────────────────────────────────────────

    pub async fn declare_limit(&self, limit: &ControlLimit) -> Result<(), StoreError> {
        if limit.declared_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{} may not declare a control limit",
                limit.declared_by
            )));
        }
        sqlx::query(
            "INSERT INTO monitor.limits
               (strategy_id, metric, version, family, severity, lower_limit, upper_limit,
                method, declared_from, declared_at, declared_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&limit.strategy_id)
        .bind(&limit.metric)
        .bind(limit.version)
        .bind(limit.family.as_str())
        .bind(limit.severity.as_str())
        .bind(numeric(limit.lower))
        .bind(numeric(limit.upper))
        .bind(limit.method.as_str())
        .bind(&limit.declared_from)
        .bind(limit.declared_at)
        .bind(&limit.declared_by)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Load every strategy's current limits.
    pub async fn load_limits(&self) -> Result<LimitBook, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (strategy_id, metric)
                    strategy_id, metric, version, family, severity,
                    lower_limit, upper_limit, method, declared_from, declared_at, declared_by
               FROM monitor.limits
              ORDER BY strategy_id, metric, version DESC",
        )
        .fetch_all(&self.pg)
        .await?;

        let mut book = LimitBook::new();
        for r in rows {
            let family: String = r.get("family");
            let severity: String = r.get("severity");
            let method: String = r.get("method");
            let lower: Option<Decimal> = r.get("lower_limit");
            let upper: Option<Decimal> = r.get("upper_limit");
            book.declare(ControlLimit {
                strategy_id: r.get("strategy_id"),
                metric: r.get("metric"),
                family: family.parse().map_err(StoreError::Invalid)?,
                severity: severity.parse().map_err(StoreError::Invalid)?,
                version: r.get("version"),
                lower: lower.and_then(|d| d.to_string().parse().ok()),
                upper: upper.and_then(|d| d.to_string().parse().ok()),
                method: parse_method(&method)?,
                declared_from: r.get("declared_from"),
                declared_at: r.get("declared_at"),
                declared_by: r.get("declared_by"),
            });
        }
        Ok(book)
    }

    // ── Drills and windows ──────────────────────────────────────────────────

    pub async fn record_drill(&self, drill: &DrillOutcome) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO monitor.drill
               (drill_id, scope, trigger_kind, started_at, cadence_ms, detection_ms,
                propagation_ms, notification_ms, passed, refused_real, operator, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(drill.drill_id)
        .bind(drill.scope.to_string())
        .bind(drill.trigger.as_str())
        .bind(drill.started_at)
        .bind(drill.cadence_ms)
        .bind(drill.detection_ms)
        .bind(drill.propagation_ms)
        .bind(drill.notification_ms)
        .bind(drill.passed())
        .bind(drill.refused_real)
        .bind(&drill.operator)
        .bind(drill.notes.as_deref())
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// When a drill last ran, for the monthly schedule (COMP-007 §4.2).
    ///
    /// `None` means no drill has ever been recorded, which the scheduler must
    /// treat as **overdue** rather than as recent: a platform that has never
    /// rehearsed the kill switch is the case §4.2 is about.
    pub async fn last_drill_at(&self) -> Result<Option<DateTime<Utc>>, StoreError> {
        let row = sqlx::query("SELECT max(started_at) AS last FROM monitor.drill")
            .fetch_optional(&self.pg)
            .await?;
        Ok(row.and_then(|r| r.get::<Option<DateTime<Utc>>, _>("last")))
    }

    /// Whether an event is readable on the surface the operator reads.
    ///
    /// Used by [`Rehearsal`][r] to measure notification latency over the channel
    /// that exists. A plain existence check rather than a scan of
    /// [`Self::recent_events`], so the answer does not depend on how busy the
    /// timeline was when the drill ran.
    ///
    /// [r]: crate::rehearsal::Rehearsal
    pub async fn event_is_visible(&self, event_id: Uuid) -> Result<bool, StoreError> {
        let row = sqlx::query("SELECT 1 AS present FROM monitor.events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(&self.pg)
            .await?;
        Ok(row.is_some())
    }

    /// Put a rehearsed scope back, and record that the rehearsal ended.
    ///
    /// **Refuses any scope that can refuse a real order.** This is the one method
    /// here that arms a halt without the §5 reset ritual — no typed confirmation,
    /// no reason, no rate limit — and the only thing that makes that safe is that
    /// it cannot be pointed at anything real. So that is a type check on
    /// [`Scope::affects_real_flow`], not a convention: a `global`, `venue:` or
    /// `strategy:` scope is rejected before a statement is prepared.
    ///
    /// A drill that left a halted row behind would be indistinguishable from a
    /// real trip on the kill-switch page, and the first thing anyone would do is
    /// reset it — which is the habit §4.3 exists to avoid teaching.
    pub async fn end_drill(&self, scope: &Scope) -> Result<(), StoreError> {
        if scope.affects_real_flow() {
            return Err(StoreError::Invalid(format!(
                "{scope} is not a drill scope; ending a rehearsal may never arm a real halt"
            )));
        }
        let event = MonitorEvent::new(
            scope.clone(),
            Family::Platform,
            EventKind::Drill,
            crate::service::MONITOR_ACTOR,
        )
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .with_reason("drill ended; the rehearsed scope is armed again");

        let mut tx = self.pg.begin().await?;
        insert_event(&mut tx, &event).await?;
        sqlx::query(
            "UPDATE monitor.kill_switch SET state = 'armed', updated_at = now()
              WHERE scope = $1",
        )
        .bind(scope.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn open_window(&self, window: &MaintenanceWindow) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO monitor.maintenance_window
               (window_id, opened_by, reason, opened_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(window.window_id)
        .bind(&window.opened_by)
        .bind(&window.reason)
        .bind(window.opened_at)
        .bind(window.expires_at)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Windows that are open right now. Visible in the desk chrome exactly as a
    /// trip is (§4.3).
    pub async fn open_windows(&self) -> Result<Vec<MaintenanceWindow>, StoreError> {
        let rows = sqlx::query(
            "SELECT window_id, opened_by, reason, opened_at, expires_at, closed_at
               FROM monitor.maintenance_window
              WHERE closed_at IS NULL AND expires_at > now()
              ORDER BY opened_at DESC",
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| MaintenanceWindow {
                window_id: r.get("window_id"),
                opened_by: r.get("opened_by"),
                reason: r.get("reason"),
                opened_at: r.get("opened_at"),
                expires_at: r.get("expires_at"),
                closed_at: r.get("closed_at"),
            })
            .collect())
    }

    pub async fn close_window(&self, window_id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE monitor.maintenance_window SET closed_at = now()
              WHERE window_id = $1 AND closed_at IS NULL",
        )
        .bind(window_id)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    // ── Precision ───────────────────────────────────────────────────────────

    pub async fn label_breach(
        &self,
        event_id: Uuid,
        family: Family,
        label: PrecisionLabel,
        labelled_by: &str,
        note: Option<&str>,
    ) -> Result<(), StoreError> {
        if labelled_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{labelled_by} may not label a breach"
            )));
        }
        sqlx::query(
            "INSERT INTO monitor.breach_label (event_id, family, label, labelled_by, note)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (event_id) DO UPDATE
               SET label = EXCLUDED.label,
                   labelled_by = EXCLUDED.labelled_by,
                   labelled_at = now(),
                   note = EXCLUDED.note",
        )
        .bind(event_id)
        .bind(family.as_str())
        .bind(label.as_str())
        .bind(labelled_by)
        .bind(note)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Compute and store the month's precision for every family.
    pub async fn compute_precision(
        &self,
        period_month: NaiveDate,
    ) -> Result<Vec<FamilyPrecision>, StoreError> {
        let rows = sqlx::query(
            "SELECT family,
                    COUNT(*) FILTER (WHERE label = 'true_positive')  AS tp,
                    COUNT(*) FILTER (WHERE label = 'false_positive') AS fp
               FROM monitor.breach_label
              WHERE date_trunc('month', labelled_at) = $1::timestamptz
              GROUP BY family",
        )
        .bind(period_month)
        .fetch_all(&self.pg)
        .await?;

        let mut counts: Vec<(Family, u32, u32)> = Vec::new();
        for r in rows {
            let family: String = r.get("family");
            let tp: i64 = r.get("tp");
            let fp: i64 = r.get("fp");
            counts.push((
                family.parse().map_err(StoreError::Invalid)?,
                u32::try_from(tp).unwrap_or(u32::MAX),
                u32::try_from(fp).unwrap_or(u32::MAX),
            ));
        }

        let report = crate::precision::report(period_month, &counts);
        for row in &report {
            sqlx::query(
                "INSERT INTO monitor.family_precision
                   (period_month, family, true_positives, false_positives,
                    precision_value, flagged, computed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, now())
                 ON CONFLICT (period_month, family) DO UPDATE
                   SET true_positives = EXCLUDED.true_positives,
                       false_positives = EXCLUDED.false_positives,
                       precision_value = EXCLUDED.precision_value,
                       flagged = EXCLUDED.flagged,
                       computed_at = now()",
            )
            .bind(row.period_month)
            .bind(row.family.as_str())
            .bind(i32::try_from(row.true_positives).unwrap_or(i32::MAX))
            .bind(i32::try_from(row.false_positives).unwrap_or(i32::MAX))
            .bind(numeric(row.precision))
            .bind(row.flagged)
            .execute(&self.pg)
            .await?;
        }
        Ok(report)
    }
}

/// Parse `orders.state`, refusing an unknown one.
///
/// Not defaulted to `Working`, which would be the comfortable choice: an order
/// state the monitor cannot read is an order it cannot score, and silently
/// dropping it out of the denominator would make the fill rate a statistic over
/// the orders the monitor happened to understand.
fn parse_order_state(s: &str) -> Result<domain::order::OrderState, StoreError> {
    use domain::order::OrderState as S;
    Ok(match s {
        "accepted" => S::Accepted,
        "submitted" => S::Submitted,
        "partially_filled" => S::PartiallyFilled,
        "filled" => S::Filled,
        "cancelled" => S::Cancelled,
        "rejected" => S::Rejected,
        other => return Err(StoreError::Invalid(format!("unknown order state {other:?}"))),
    })
}

fn parse_method(s: &str) -> Result<LimitMethod, StoreError> {
    Ok(match s {
        "paper_ewma_3sigma" => LimitMethod::PaperEwma3Sigma,
        "backtest_3sigma" => LimitMethod::Backtest3Sigma,
        "venue_rate_ceiling" => LimitMethod::VenueRateCeiling,
        "risk_gate_mirror" => LimitMethod::RiskGateMirror,
        "declared_absolute" => LimitMethod::DeclaredAbsolute,
        "mc_p95_drawdown" => LimitMethod::McP95Drawdown,
        "cusum" => LimitMethod::Cusum,
        other => return Err(StoreError::Invalid(format!("unknown limit method {other:?}"))),
    })
}

/// Keep the pre-COMP-007 boot column in step with the global scope.
///
/// `apps/platform` reads `global_risk_config.trading_enabled` at startup to decide
/// whether to come up halted. Before this, `POST /api/trading/kill` only flipped an
/// in-process `AtomicBool` and never wrote that column — so a switch tripped on
/// Monday was silently cleared by the next restart, with nothing in the log saying
/// the platform had decided to start trading again. The operator's model ("I killed
/// it") and the system's state diverged with no signal at all.
///
/// Two records of one fact is a smell, and the right eventual fix is for the boot
/// path to read `monitor.kill_switch`. Until `apps/platform/src/main.rs` can be
/// changed, they are written in the **same transaction**, so they cannot disagree;
/// and `global_halted_at_boot` treats a disagreement as halted anyway.
async fn sync_legacy_boot_column(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &Scope,
    trading_enabled: bool,
) -> Result<(), sqlx::Error> {
    if *scope != Scope::Global {
        return Ok(());
    }
    sqlx::query("UPDATE global_risk_config SET trading_enabled = $1")
        .bind(trading_enabled)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &MonitorEvent,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO monitor.events
           (event_id, occurred_at, scope, family, kind, metric, observed_value,
            limit_value, from_state, to_state, actor, reason, is_drill,
            suppressed_by_window)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(event.event_id)
    .bind(event.occurred_at)
    .bind(event.scope.to_string())
    .bind(event.family.as_str())
    .bind(event.kind.as_str())
    .bind(event.metric.as_deref())
    .bind(numeric(event.observed_value))
    .bind(numeric(event.limit_value))
    .bind(event.from_state.as_deref())
    .bind(event.to_state.as_deref())
    .bind(&event.actor)
    .bind(event.reason.as_deref())
    .bind(event.is_drill)
    .bind(event.suppressed_by_window)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store whose database cannot be reached, which is the failure the
    /// ordering rule is about. `connect_lazy` never dials, so every statement
    /// fails on use and nothing here touches a real platform.
    fn unreachable_store() -> MonitorStore {
        MonitorStore::new(
            sqlx::postgres::PgPoolOptions::new()
                // Short, so the suite measures the rule rather than sqlx's default
                // half-minute wait for a host that is not there.
                .acquire_timeout(std::time::Duration::from_millis(200))
                .connect_lazy("postgres://nobody@127.0.0.1:1/none")
                .expect("lazy pool"),
        )
    }

    #[tokio::test]
    async fn a_mismatch_that_could_not_be_recorded_does_not_trip_the_mirror() {
        // The rule S-7.6 exists for, in the direction that is easy to get wrong.
        //
        // `KillSwitch::replace_halted` *replaces* the scoped view from
        // `monitor.kill_switch` on every refresh, so an in-memory trip with no
        // durable row behind it is erased within one cadence: the venue reads as
        // halted for about five seconds and then silently resumes trading. That
        // is strictly worse than not halting, because the five seconds are the
        // ones anybody watching would see.
        //
        // And nothing is lost by declining: the database this could not write is
        // the same one the lease lives in, so `LeaseWatcher::refresh` is already
        // failing to read it and the lease gate is refusing every order platform
        // -wide (§4 "fail closed").
        let gate = risk::KillSwitch::new(false);
        let err = unreachable_store()
            .carry_out_mismatch("kraken", &gate)
            .await
            .expect_err("an unreachable database cannot record a halt");
        assert!(matches!(err, StoreError::Db(_)), "{err}");
        assert!(
            !gate.is_active_scope(&Scope::venue("kraken")),
            "a mirror trip with no durable row behind it is erased within one refresh"
        );
    }

    #[tokio::test]
    async fn a_mismatch_must_name_the_venue_it_is_at() {
        // §4 halts the venue that disagrees and leaves the others trading. A blank
        // venue would produce `venue:` — a scope the kill-switch CHECK rejects and
        // `Scope::from_str` refuses, so the halt would be lost at the database
        // rather than misapplied. Refusing here says so where the caller is.
        let gate = risk::KillSwitch::new(false);
        let err = unreachable_store()
            .carry_out_mismatch("   ", &gate)
            .await
            .expect_err("an unnamed venue must be refused");
        assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    }

    #[tokio::test]
    async fn ending_a_rehearsal_can_never_arm_a_real_halt() {
        // `end_drill` is the one method here that arms a scope with none of §5's
        // ritual — no typed confirmation, no reason, no rate limit. The only thing
        // that makes that safe is that it cannot be pointed at anything real, so
        // the refusal is checked before a statement is prepared, over every scope
        // shape that can refuse an order.
        let store = unreachable_store();
        for scope in [Scope::Global, Scope::venue("kraken"), Scope::strategy("s-1")] {
            let err = store
                .end_drill(&scope)
                .await
                .expect_err("a real scope must be refused");
            assert!(
                matches!(err, StoreError::Invalid(_)),
                "{scope} was refused for the wrong reason: {err}"
            );
        }
        // And the anti-vacuity half: a drill scope gets as far as the database,
        // which is where this store fails. A refusal that rejected everything
        // would pass the loop above and protect nothing.
        let err = store
            .end_drill(&Scope::drill("rehearsal"))
            .await
            .expect_err("this store has no database");
        assert!(
            matches!(err, StoreError::Db(_)),
            "a drill scope must be accepted by the check and fail at the database: {err}"
        );
    }

    #[test]
    fn a_non_finite_reading_stores_no_value_rather_than_zero() {
        assert_eq!(numeric(Some(f64::NAN)), None);
        assert_eq!(numeric(Some(f64::INFINITY)), None);
        assert_eq!(numeric(None), None);
        assert!(numeric(Some(0.25)).is_some());
    }

    #[test]
    fn an_unknown_order_state_is_an_error_rather_than_a_default() {
        // Defaulting an unreadable state to "still working" would drop the order
        // out of the fill rate's denominator, making it a statistic over the
        // orders the monitor happened to understand.
        assert!(parse_order_state("in_flight").is_err());
        for s in [
            "accepted",
            "submitted",
            "partially_filled",
            "filled",
            "cancelled",
            "rejected",
        ] {
            assert!(parse_order_state(s).is_ok(), "{s}");
        }
    }

    #[test]
    fn an_unknown_limit_method_is_an_error_rather_than_a_default() {
        // Defaulting would silently reclassify a limit's provenance, which is the
        // one field that distinguishes a control limit from a number somebody typed.
        assert!(parse_method("whatever").is_err());
        assert!(parse_method("cusum").is_ok());
    }
}
