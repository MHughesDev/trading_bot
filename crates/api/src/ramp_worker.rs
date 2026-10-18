//! The scheduled review (Set T T-3.2, T-3.3; ADR-P2-27).
//!
//! One pass, run from the health jobs' hourly tick, does four things in order:
//!
//! 1. **opens** the first review period of any running deployment that has none;
//! 2. **reviews** every period whose recorded end has passed, by judging the
//!    period's evidence ([`ledger::ramp_gate::judge`]) and folding the outcome
//!    through the state machine ([`ledger::ramp::Ramp`]);
//! 3. **opens** the next period at the **recorded** end of the last, so a pass
//!    that ran late does not move the schedule;
//! 4. **applies** every running deployment's current size, idempotently.
//!
//! Step 4 runs for every deployment on every pass, not just the ones reviewed in
//! this one. A size is a durable fact in `ramp_review` and an effect on the order
//! gate; if the process died between writing the first and applying the second,
//! the next pass repairs it rather than leaving a deployment at a size its record
//! says it no longer has.
//!
//! # What a failing period does
//!
//! It halves the size. There is no configuration key that skips this: the
//! policy ([`RampPolicy`]) holds a period length for deployments that declared
//! none and the observation floor, and nothing else. The database refuses the
//! alternatives (migration 0181: `chk_ramp_review_failure_steps_down`).
//!
//! # The effects
//!
//! A size below full is the monitor's `scale` control; zero is the monitor's
//! `halt` plus withdrawal and closing the record ([`MonitorEffects`]). Both are
//! mechanisms the monitor already owns, so the ramp does not become a second
//! chokepoint (SC-2).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ledger::capital::{self, AllowedFraction};
use ledger::ramp::{self, next_period, Outcome, ReviewEvent, Size};
use ledger::ramp_gate::{self, PeriodEvidence, StepInputs, StepThresholds};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// What the pass needs that is a decision rather than a fact. No `Default`: a
/// period length or an observation floor nobody chose is how a review judges an
/// empty period as clean.
#[derive(Clone, Copy, Debug)]
pub struct RampPolicy {
    /// The review period, in days, for a deployment that did not declare one (a
    /// `legacy_ungated` one).
    pub legacy_review_period_days: i64,
    pub thresholds: StepThresholds,
}

/// What a pass did.
#[derive(Clone, Debug, Default)]
pub struct PassReport {
    pub periods_opened: u32,
    pub reviewed: Vec<(Uuid, Outcome)>,
    pub applied: u32,
    pub errors: Vec<String>,
    /// Running deployments with no forward recorder (ADR-T3-04): nothing can be
    /// measured for them, so they are **flagged and not stepped down**. Reported
    /// on every pass until a recorder exists.
    pub unmeasured: Vec<Uuid>,
}

/// Where a size becomes an effect.
#[async_trait]
pub trait RampEffects: Send + Sync {
    async fn apply_size(&self, deployment_id: Uuid, size: Size, reason: &str) -> Result<(), String>;
}

/// The production effects: the monitor's own controls.
pub struct MonitorEffects {
    pub pg: PgPool,
    pub kill: std::sync::Arc<risk::KillSwitch>,
}

const ACTOR: &str = "scheduler:ramp";

#[async_trait]
impl RampEffects for MonitorEffects {
    async fn apply_size(&self, id: Uuid, size: Size, reason: &str) -> Result<(), String> {
        if size.is_zero() {
            crate::deployment_gate::revoke_one(&self.pg, &self.kill, id, ACTOR, reason)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            let pct = size.scale_pct();
            let key = id.to_string();
            let store = monitor::store::MonitorStore::new(self.pg.clone());
            let event = monitor::events::MonitorEvent::new(
                risk::Scope::strategy(&key),
                monitor::family::Family::Behaviour,
                monitor::events::EventKind::ScaledDown,
                ACTOR,
            )
            .map_err(|e| e.to_string())?
            .with_reason(reason.to_owned());
            store.record_event(&event).await.map_err(|e| e.to_string())?;
            let state = if pct >= 100 { "nominal" } else { "scaled_down" };
            store
                .set_strategy_state(&key, state, i16::from(pct))
                .await
                .map_err(|e| e.to_string())?;
            self.kill.scale_strategy(&key, pct);
            Ok(())
        }
    }
}

struct Running {
    deployment_id: Uuid,
    user_id: String,
    forward_deployment_id: Option<Uuid>,
    anchor: DateTime<Utc>,
    period_days: i64,
    profile_id: String,
    authorises_capital: bool,
    verdict: Vec<(i32, bool)>,
}

async fn running(pg: &PgPool, policy: &RampPolicy) -> Result<Vec<Running>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT d.deployment_id, d.user_id, d.forward_deployment_id,
                COALESCE(m.marked_at, d.started_at) AS anchor,
                p.review_period_days, p.profile_id, p.verdict
           FROM strategy_deployment d
           LEFT JOIN deployment_authorisation a ON a.deployment_id = d.deployment_id
           LEFT JOIN deployment_proposal p ON p.proposal_id = a.proposal_id
           LEFT JOIN deployment_legacy_marker m ON m.deployment_id = d.deployment_id
          WHERE d.stopped_at IS NULL AND d.parent_deployment_id IS NULL
          ORDER BY d.started_at",
    )
    .fetch_all(pg)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        // A deployment with no proposal has no profile pass on record. The
        // profile it must pass to keep its size is the paper one (ADR-P2-14).
        let profile_id: String = r
            .get::<Option<String>, _>("profile_id")
            .unwrap_or_else(|| "paper_v1".to_owned());
        let authorises: Option<bool> =
            sqlx::query_scalar("SELECT authorises_capital FROM mlops.gate_profile WHERE profile_id = $1")
                .bind(&profile_id)
                .fetch_optional(pg)
                .await?;
        let verdict: Vec<(i32, bool)> = r
            .get::<Option<serde_json::Value>, _>("verdict")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|g| {
                Some((
                    i32::try_from(g.get("gate_no")?.as_i64()?).ok()?,
                    g.get("passed")?.as_bool()?,
                ))
            })
            .collect();
        out.push(Running {
            deployment_id: r.get("deployment_id"),
            user_id: r.get("user_id"),
            forward_deployment_id: r.get("forward_deployment_id"),
            anchor: r.get("anchor"),
            period_days: r
                .get::<Option<i32>, _>("review_period_days")
                .map_or(policy.legacy_review_period_days, i64::from),
            profile_id,
            authorises_capital: authorises.unwrap_or(false),
            verdict,
        });
    }
    Ok(out)
}

/// The period evidence for `[start, end]`, from the forward record.
async fn evidence(
    pg: &PgPool,
    d: &Running,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<PeriodEvidence, sqlx::Error> {
    let profile_passed = (1..=15).all(|n| d.verdict.iter().any(|(g, ok)| *g == n && *ok));
    let mut ev = PeriodEvidence {
        period_start: start,
        period_end: end,
        evaluated_at: now,
        observed_bars: 0,
        divergence_halts: 0,
        signal_match: None,
        slippage_ratio: None,
        turnover_deviation: None,
        model_rejects: 0,
        profile_passed,
        gate_verdicts: d.verdict.clone(),
    };
    let Some(forward) = d.forward_deployment_id else {
        return Ok(ev);
    };
    let mut tx = pg.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(&d.user_id)
        .execute(&mut *tx)
        .await?;
    // Observed rows only (D-T1): a backfilled row is never forward evidence.
    let r = sqlx::query(
        "SELECT count(*)::bigint AS n,
                avg(CASE WHEN signal_matched THEN 1.0 ELSE 0.0 END)::float8 AS sig,
                COALESCE(sum(realised_slippage_bps), 0)::float8 AS slip_r,
                COALESCE(sum(modelled_slippage_bps), 0)::float8 AS slip_m,
                COALESCE(sum(realised_turnover), 0)::float8 AS turn_r,
                COALESCE(sum(expected_turnover), 0)::float8 AS turn_e,
                COALESCE(sum(model_attributable_rejects), 0)::bigint AS rejects
           FROM mlops.forward_observation
          WHERE deployment_id = $1 AND source = 'observed' AND bar > $2 AND bar <= $3",
    )
    .bind(forward)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;
    let halts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mlops.divergence_halt
          WHERE deployment_id = $1 AND raised_at > $2 AND raised_at <= $3",
    )
    .bind(forward)
    .bind(start)
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let n: i64 = r.get("n");
    ev.observed_bars = u32::try_from(n).unwrap_or(u32::MAX);
    ev.divergence_halts = u32::try_from(halts).unwrap_or(u32::MAX);
    ev.model_rejects = u32::try_from(r.get::<i64, _>("rejects")).unwrap_or(u32::MAX);
    if n > 0 {
        ev.signal_match = r.get("sig");
        let (sr, sm): (f64, f64) = (r.get("slip_r"), r.get("slip_m"));
        ev.slippage_ratio = Some(if sm.abs() < 1e-12 {
            if sr.abs() < 1e-12 { 1.0 } else { 1e9 }
        } else {
            sr / sm
        });
        let (tr, te): (f64, f64) = (r.get("turn_r"), r.get("turn_e"));
        ev.turnover_deviation = Some((tr - te).abs() / te.abs().max(1e-12));
    }
    Ok(ev)
}

async fn open_period(
    pg: &PgPool,
    id: Uuid,
    no: i32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(no)
    .bind(start)
    .bind(end)
    .execute(pg)
    .await?;
    Ok(())
}

async fn recorded_events(pg: &PgPool, id: Uuid) -> Result<Vec<ReviewEvent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT period_no, outcome, halvings_before, halvings_after FROM ramp_review
          WHERE deployment_id = $1 ORDER BY period_no",
    )
    .bind(id)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(ReviewEvent {
                period_no: u32::try_from(r.get::<i32, _>("period_no")).ok()?,
                outcome: Outcome::parse(&r.get::<String, _>("outcome"))?,
                halvings_before: u8::try_from(r.get::<i16, _>("halvings_before")).ok()?,
                halvings_after: u8::try_from(r.get::<i16, _>("halvings_after")).ok()?,
            })
        })
        .collect())
}

/// One pass. See the module docs.
///
/// # Errors
/// Only a failure to list the running deployments; a failure on one deployment is
/// recorded in the report and does not stop the others.
pub async fn run_pass(
    pg: &PgPool,
    now: DateTime<Utc>,
    policy: &RampPolicy,
    effects: &dyn RampEffects,
) -> Result<PassReport, sqlx::Error> {
    let mut report = PassReport::default();
    for d in running(pg, policy).await? {
        // ADR-T3-04: a deployment with no forward recorder has no evidence to be
        // judged on, whatever it does, so a period for it would only ever be
        // `NotComputable`. Halving it would punish a missing measurement and walk
        // every hand-run deployment to zero in 56 days before anything had been
        // measured. It is flagged instead, every pass, and no period is opened.
        if d.forward_deployment_id.is_none() {
            report.unmeasured.push(d.deployment_id);
            continue;
        }
        if let Err(e) = one(pg, now, policy, effects, &d, &mut report).await {
            report.errors.push(format!("{}: {e}", d.deployment_id));
        }
    }
    Ok(report)
}

async fn one(
    pg: &PgPool,
    now: DateTime<Utc>,
    policy: &RampPolicy,
    effects: &dyn RampEffects,
    d: &Running,
    report: &mut PassReport,
) -> Result<(), String> {
    let db = |e: sqlx::Error| e.to_string();
    // 1. the first period
    let last: Option<(i32, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT period_no, starts_at, ends_at FROM ramp_period
          WHERE deployment_id = $1 ORDER BY period_no DESC LIMIT 1",
    )
    .bind(d.deployment_id)
    .fetch_optional(pg)
    .await
    .map_err(db)?;
    let mut current = match last {
        Some(p) => p,
        None => {
            let (s, e) = next_period(d.anchor, d.period_days);
            open_period(pg, d.deployment_id, 1, s, e).await.map_err(db)?;
            report.periods_opened += 1;
            (1, s, e)
        }
    };
    // 2 + 3. review what has ended, open what follows; bounded so a deployment
    // that was neglected for years is caught up over a few passes, not one.
    for _ in 0..64 {
        let (no, start, end) = current;
        if end > now {
            break;
        }
        let reviewed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM ramp_review WHERE deployment_id = $1 AND period_no = $2)",
        )
        .bind(d.deployment_id)
        .bind(no)
        .fetch_one(pg)
        .await
        .map_err(db)?;
        let size_zero = if reviewed {
            let events = recorded_events(pg, d.deployment_id).await.map_err(db)?;
            ramp::replay(d.authorises_capital, &events)
                .map_err(|e| e.to_string())?
                .size()
                .is_zero()
        } else {
            let outcome = review_one(pg, now, policy, d, no, start, end).await?;
            report.reviewed.push((d.deployment_id, outcome));
            outcome == Outcome::Zero
        };
        if size_zero {
            break;
        }
        let (s, e) = next_period(end, d.period_days);
        open_period(pg, d.deployment_id, no + 1, s, e).await.map_err(db)?;
        report.periods_opened += 1;
        current = (no + 1, s, e);
    }
    // 4. the size, as an effect. Idempotent: reapplied every pass.
    let events = recorded_events(pg, d.deployment_id).await.map_err(db)?;
    let state = ramp::replay(d.authorises_capital, &events).map_err(|e| e.to_string())?;
    if state.size() != Size::FULL {
        effects
            .apply_size(
                d.deployment_id,
                state.size(),
                &format!(
                    "ramp: {} halving(s) after {} review period(s) (ADR-P2-27)",
                    state.size().halvings(),
                    state.last_period()
                ),
            )
            .await?;
        report.applied += 1;
    }
    Ok(())
}

async fn review_one(
    pg: &PgPool,
    now: DateTime<Utc>,
    policy: &RampPolicy,
    d: &Running,
    no: i32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Outcome, String> {
    let db = |e: sqlx::Error| e.to_string();
    let events = recorded_events(pg, d.deployment_id).await.map_err(db)?;
    let mut ramp = ramp::replay(d.authorises_capital, &events).map_err(|e| e.to_string())?;
    let inputs = StepInputs {
        profile_id: d.profile_id.clone(),
        authorises_capital: d.authorises_capital,
        thresholds: policy.thresholds,
        current_rung: ramp.rung(),
    };
    let ev = evidence(pg, d, start, end, now).await.map_err(db)?;
    let judgement = ramp_gate::judge(&inputs, &ev);
    let authority = if judgement.step_up_allowed {
        let mut verdicts: Vec<(i32, bool)> = d.verdict.iter().copied().filter(|(n, _)| *n != 16).collect();
        verdicts.push((16, true));
        Some(
            capital::authorise(&d.profile_id, d.authorises_capital, &verdicts)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let rung_before = ramp.rung();
    let event = ramp
        .review(u32::try_from(no).map_err(|e| e.to_string())?, &judgement, authority.as_ref())
        .map_err(|e| e.to_string())?;
    // Gate 16, rendered from the judged period (T-3.2): the gate's own verdict is
    // stored beside the judgement it was rendered from.
    let profile = if d.profile_id == "paper_v1" {
        backtest::gates::profile::GateProfile::paper_v1()
    } else {
        backtest::gates::profile::GateProfile::strict_v1().authorising_capital(d.authorises_capital)
    };
    let mut gate_verdicts: Vec<(i32, bool)> = d.verdict.iter().copied().filter(|(n, _)| *n != 16).collect();
    gate_verdicts.push((16, true));
    let gate16 = backtest::gates::evaluators::gate_16_capital_ramp(
        &profile,
        rung_before.unwrap_or_default(),
        &gate_verdicts,
        Some(&judgement),
    );
    let evidence_json = json!({
        "inputs": &inputs,
        "evidence": &ev,
        "judgement": &judgement,
        "gate_16": {
            "passed": gate16.passed,
            "inconclusive": gate16.inconclusive,
            "detail": gate16.detail,
        },
    });
    let hash = ramp_gate::evidence_hash(&inputs, &ev);

    let mut tx = pg.begin().await.map_err(db)?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(&d.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let review_id: Uuid = sqlx::query_scalar(
        "INSERT INTO ramp_review
           (deployment_id, period_no, profile_id, outcome, passed, not_computable_reason,
            halvings_before, halvings_after, evidence, evidence_hash, reason, decided_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING review_id",
    )
    .bind(d.deployment_id)
    .bind(no)
    .bind(&d.profile_id)
    .bind(event.outcome.as_str())
    .bind(judgement.passed())
    .bind(judgement.not_computable_reason())
    .bind(i16::from(event.halvings_before))
    .bind(i16::from(event.halvings_after))
    .bind(&evidence_json)
    .bind(&hash)
    .bind(judgement.reason())
    .bind(ACTOR)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if event.outcome == Outcome::StepUp {
        let (Some(before), Some(after)) = (rung_before, ramp.rung()) else {
            return Err("a step-up without rungs".into());
        };
        record_raise(&mut tx, d, before, after, review_id).await.map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(event.outcome)
}

async fn record_raise(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    d: &Running,
    before: AllowedFraction,
    after: AllowedFraction,
    review_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mlops.capital_authorisation
           (tenant_id, subject, profile_id, previous_fraction, allowed_fraction, direction,
            gates_passed, reason, decided_by, ramp_review_id)
         VALUES ($1,$2,$3,$4::numeric,$5::numeric,'raise',16,$6,$7,$8)",
    )
    .bind(&d.user_id)
    .bind(d.deployment_id.to_string())
    .bind(&d.profile_id)
    .bind(format!("{:.3}", before.value()))
    .bind(format!("{:.3}", after.value()))
    .bind(format!("Gate 16 passed a completed review period: {} -> {}", before.as_str(), after.as_str()))
    .bind(ACTOR)
    .bind(review_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl RampPolicy {
    /// The two chosen numbers, from the newest `ramp_policy` row (migration 0190,
    /// ADR-T3-04), under the profile's own thresholds. There is no fallback: with
    /// no row there is no policy, and a review judged on a default nobody chose
    /// is how an empty period reads as clean.
    ///
    /// # Errors
    /// The database, or no row.
    pub async fn load(
        pg: &PgPool,
        profile: &backtest::gates::profile::GateProfile,
    ) -> Result<Self, sqlx::Error> {
        let (days, bars): (i32, i32) = sqlx::query_as(
            "SELECT legacy_review_period_days, min_observed_bars FROM current_ramp_policy",
        )
        .fetch_one(pg)
        .await?;
        Ok(Self::from_profile(
            profile,
            i64::from(days),
            u32::try_from(bars).unwrap_or(u32::MAX),
        ))
    }

    /// The policy a deployment's profile implies: the profile's own Gate 15
    /// thresholds (so the review judges by the bar the strategy was admitted
    /// under, and a threshold change is a new profile), plus the two numbers the
    /// profile does not carry and somebody must choose.
    #[must_use]
    pub fn from_profile(
        profile: &backtest::gates::profile::GateProfile,
        legacy_review_period_days: i64,
        min_observed_bars: u32,
    ) -> Self {
        let t = profile.thresholds();
        Self {
            legacy_review_period_days,
            thresholds: StepThresholds {
                min_observed_bars,
                paper_signal_match_gte: t.paper_signal_match_gte,
                paper_slippage_ratio_lte: t.paper_slippage_ratio_lte,
                paper_turnover_tolerance: t.paper_turnover_tolerance,
                paper_model_rejects_max: t.paper_model_rejects_max,
            },
        }
    }
}
