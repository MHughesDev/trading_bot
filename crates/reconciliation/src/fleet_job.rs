//! Fleet-level reconciliation, run (Set T T-2.2).
//!
//! [`crate::fleet`] is the detector and is pure. This is the part that makes it
//! *run*: read a tenant's forward observations over a window, judge the fleet,
//! file a common-mode finding as a `monitor.platform_defect` (0165) without
//! anyone asking, and shape the whole reading for the divergence dashboard.
//!
//! # The declared policy (ADR-T2-02)
//!
//! [`FleetPolicy`] has no `Default` on purpose, so the yardstick is declared here
//! once, named, and asserted by a test rather than chosen at each call site:
//!
//! - **tolerance 0.0005 per bar** (5 bps): an absolute mean per-bar miss larger
//!   than the round-trip cost model's own resolution on the platform's liquid
//!   instruments. Rejected alternative: each deployment's own
//!   `max_abs_return_divergence` — *k* deployments crossing *k* different lines
//!   is not one fact, which is why `fleet` takes one yardstick.
//! - **quorum 3**: two agreeing misses in a fleet of five are the case
//!   `fleet`'s own tests call not-a-finding; three is the smallest quorum that
//!   is not a coincidence of two.
//! - **30 observed bars**: below that a deployment's mean is noise.
//! - **window 30 days**: long enough to reach 30 bars on a daily-bar strategy.
//!
//! # Filing
//!
//! One open defect per tenant and direction (`dedupe_key`), so the hourly job
//! that sees the same common-mode miss every hour files it once. The finding is
//! filed as what it is — an observation awaiting a replay — and **not** as a
//! `ledger` `EvaluatorDefect`, which needs a `Reproduction` nobody can mint for
//! forward trading (see `fleet::CommonModeFinding::defect`).

use chrono::{DateTime, Duration, Utc};
use domain::forward::{ForwardObservation, ObservationSource};
use domain::numeraire::Numeraire;
use monitor::defects::{DefectSource, Filed, NewDefect};
use monitor::store::MonitorStore;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use crate::fleet::{FleetError, FleetPolicy, FleetReport, FleetVerdict};

/// The fleet tolerance: absolute mean per-bar `realised − expected`.
pub const FLEET_TOLERANCE: f64 = 0.0005;
/// Deployments that must miss the same way before it is the simulator.
pub const FLEET_QUORUM: usize = 3;
/// Observed bars below which a deployment's mean is not a measurement.
pub const FLEET_MIN_BARS: usize = 30;
/// The window the fleet is judged over.
pub const FLEET_WINDOW_DAYS: i64 = 30;

/// The actor a fleet filing is recorded under.
pub const FLEET_ACTOR: &str = "reconciliation";

/// The declared policy.
///
/// # Panics
/// Never: the constants are asserted valid by a test.
#[must_use]
pub fn declared_policy() -> FleetPolicy {
    FleetPolicy::declared(FLEET_TOLERANCE, FLEET_QUORUM, FLEET_MIN_BARS)
        .expect("the declared fleet policy is valid")
}

/// Anything that stopped a fleet reading.
#[derive(Debug, thiserror::Error)]
pub enum FleetJobError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Fleet(#[from] FleetError),
    #[error(transparent)]
    Store(#[from] monitor::store::StoreError),
}

/// One tenant's fleet reading and what it filed.
#[derive(Clone, Debug)]
pub struct FleetAssessment {
    pub tenant_id: String,
    pub window_from: DateTime<Utc>,
    pub window_to: DateTime<Utc>,
    pub report: FleetReport,
    /// The defect a common-mode finding filed, or found already open.
    pub filed: Option<Filed>,
}

/// Every observation of every deployment of one tenant in `[from, to]`.
///
/// Both sources: the fleet module counts backfilled rows out itself and reports
/// how many it excluded.
///
/// # Errors
/// The database, or a value the table's CHECKs make unreachable.
pub async fn load_window(
    pool: &PgPool,
    tenant_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<ForwardObservation>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Tenant-scoped: `mlops.forward_observation` is FORCE ROW LEVEL SECURITY.
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query(
        "SELECT deployment_id, strategy_id, bar, realised_return, expected_return, numeraire, \
                source, signal_matched, realised_slippage_bps, modelled_slippage_bps, \
                realised_turnover, expected_turnover, model_attributable_rejects, observed_at \
           FROM mlops.forward_observation \
          WHERE tenant_id = $1 AND bar >= $2 AND bar <= $3 \
          ORDER BY strategy_id, deployment_id, bar",
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let source: String = r.try_get("source")?;
        let numeraire: String = r.try_get("numeraire")?;
        let rejects: i32 = r.try_get("model_attributable_rejects")?;
        out.push(ForwardObservation {
            deployment_id: r.try_get("deployment_id")?,
            strategy_id: r.try_get("strategy_id")?,
            bar: r.try_get("bar")?,
            realised_return: r.try_get("realised_return")?,
            expected_return: r.try_get("expected_return")?,
            numeraire: numeraire
                .parse::<Numeraire>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            source: source
                .parse::<ObservationSource>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            signal_matched: r.try_get("signal_matched")?,
            realised_slippage_bps: r.try_get("realised_slippage_bps")?,
            modelled_slippage_bps: r.try_get("modelled_slippage_bps")?,
            realised_turnover: r.try_get("realised_turnover")?,
            expected_turnover: r.try_get("expected_turnover")?,
            model_attributable_rejects: u32::try_from(rejects).unwrap_or(0),
            observed_at: r.try_get("observed_at")?,
        });
    }
    Ok(out)
}

/// The defect a common-mode finding files, built without a database so its
/// shape is tested.
#[must_use]
pub fn defect_for(tenant_id: &str, report: &FleetReport) -> Option<NewDefect> {
    let f = report.verdict.common_mode()?;
    Some(NewDefect {
        source: DefectSource::FleetCommonMode,
        dedupe_key: format!("fleet:{tenant_id}:{}", f.direction),
        summary: f.observed_statement(),
        evidence: json!({
            "tenant_id": tenant_id,
            "claimed_guarantee": f.claimed_guarantee(),
            "expected": f.expected_statement(),
            "observed": f.observed_statement(),
            "direction": f.direction.as_str(),
            "mean_divergence": f.mean_divergence,
            "tolerance": f.tolerance,
            "fleet_measurable": f.fleet_measurable,
            "deployments": f.deployments.iter().map(|d| json!({
                "deployment_id": d.deployment_id,
                "strategy_id": d.strategy_id,
                "bars": d.bars,
                "mean_divergence": d.mean_divergence,
                "cumulative_divergence": d.cumulative_divergence,
            })).collect::<Vec<_>>(),
            "replay_steps": f.replay_steps(),
            "awaiting": "a human replay; this is an observation, not a Reproduction",
        }),
        filed_by: FLEET_ACTOR.to_owned(),
    })
}

/// Judge one tenant's fleet, and — when `file` — file a common-mode finding.
///
/// The hourly job files; a dashboard read does not, so looking at the page is
/// never the thing that files a defect (and filing is idempotent either way).
///
/// # Errors
/// Reading the window, a window spanning numeraires, or filing.
pub async fn assess_tenant(
    pool: &PgPool,
    store: &MonitorStore,
    tenant_id: &str,
    now: DateTime<Utc>,
    file: bool,
) -> Result<FleetAssessment, FleetJobError> {
    let from = now - Duration::days(FLEET_WINDOW_DAYS);
    let observations = load_window(pool, tenant_id, from, now).await?;
    let report = FleetReport::assess(&observations, &declared_policy())?;
    let filed = match defect_for(tenant_id, &report) {
        Some(d) if file => Some(store.file_defect(&d).await?),
        _ => None,
    };
    if let Some(Filed::New(id)) = filed {
        tracing::error!(%tenant_id, defect_id = %id,
            "COMMON-MODE FORWARD DIVERGENCE — a platform defect against the simulator has been filed");
    }
    Ok(FleetAssessment {
        tenant_id: tenant_id.to_owned(),
        window_from: from,
        window_to: now,
        report,
        filed,
    })
}

/// The dashboard's payload: per deployment, in aggregate, and judged.
///
/// `crate::fleet` carries no `serde` (it is the detector and stays pure), so the
/// wire shape is built here, once, rather than in each route.
#[must_use]
pub fn report_json(a: &FleetAssessment) -> Value {
    let r = &a.report;
    let (verdict, detail) = match &r.verdict {
        FleetVerdict::NotComputable(why) => ("not_computable", json!(why.to_string())),
        FleetVerdict::NoDivergence { measurable, bars } => {
            ("no_divergence", json!({ "measurable": measurable, "bars": bars }))
        }
        FleetVerdict::Idiosyncratic {
            diverging,
            measurable,
        } => (
            "idiosyncratic",
            json!({ "diverging": diverging, "measurable": measurable }),
        ),
        FleetVerdict::CommonMode(f) => (
            "common_mode",
            json!({
                "direction": f.direction.as_str(),
                "mean_divergence": f.mean_divergence,
                "observed": f.observed_statement(),
                "deployments": f.deployments.iter().map(|d| d.deployment_id).collect::<Vec<_>>(),
            }),
        ),
    };
    json!({
        "window": { "from": a.window_from, "to": a.window_to, "days": FLEET_WINDOW_DAYS },
        "policy": {
            "tolerance_per_bar": FLEET_TOLERANCE,
            "quorum": FLEET_QUORUM,
            "min_bars": FLEET_MIN_BARS,
        },
        "deployments": r.deployments.iter().map(|d| json!({
            "deployment_id": d.deployment_id,
            "strategy_id": d.strategy_id,
            "numeraire": d.numeraire.to_string(),
            "bars": d.bars,
            "backfilled_excluded": d.backfilled_excluded,
            "mean_divergence": d.mean_divergence,
            "cumulative_divergence": d.cumulative_divergence,
            "slippage_excess_bps": d.slippage_excess_bps,
        })).collect::<Vec<_>>(),
        "aggregate": r.aggregate.as_ref().map(|s| json!({
            "deployments": s.deployments,
            "measurable": s.measurable,
            "bars": s.bars,
            "numeraire": s.numeraire.to_string(),
            "mean_divergence": s.mean_divergence,
            "diverging": s.diverging,
        })),
        "verdict": verdict,
        "detail": detail,
        "defect_id": a.filed.map(Filed::defect_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn obs(deployment: Uuid, strategy: &str, i: i64, realised: f64, expected: f64) -> ForwardObservation {
        let bar = DateTime::from_timestamp(1_800_000_000 + i * 60, 0).expect("bar");
        ForwardObservation {
            deployment_id: deployment,
            strategy_id: strategy.to_owned(),
            bar,
            realised_return: realised,
            expected_return: expected,
            numeraire: Numeraire::Usd,
            source: ObservationSource::Observed,
            signal_matched: true,
            realised_slippage_bps: 2.0,
            modelled_slippage_bps: 1.0,
            realised_turnover: 1.0,
            expected_turnover: 1.0,
            model_attributable_rejects: 0,
            observed_at: bar,
        }
    }

    #[test]
    fn the_declared_policy_is_valid_and_is_the_one_documented() {
        let p = declared_policy();
        assert_eq!(p.tolerance(), FLEET_TOLERANCE);
        assert_eq!(p.min_common(), FLEET_QUORUM);
        assert_eq!(p.min_bars(), FLEET_MIN_BARS);
    }

    #[test]
    fn a_planted_systematic_cost_error_files_one_defect_naming_every_deployment() {
        // Four deployments, each realising 10 bps per bar less than its backtest:
        // the simulator's cost model is optimistic for all of them.
        let mut rows = Vec::new();
        let ids: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();
        for (k, id) in ids.iter().enumerate() {
            for i in 0..40 {
                rows.push(obs(*id, &format!("s{k}"), i, 0.000, 0.001));
            }
        }
        let report = FleetReport::assess(&rows, &declared_policy()).expect("assess");
        let defect = defect_for("tenant-a", &report).expect("a common-mode finding files a defect");
        assert_eq!(defect.source, DefectSource::FleetCommonMode);
        assert_eq!(defect.dedupe_key, "fleet:tenant-a:simulator_optimistic");
        assert_eq!(defect.evidence["deployments"].as_array().map(Vec::len), Some(4));
        let a = FleetAssessment {
            tenant_id: "tenant-a".into(),
            window_from: Utc::now(),
            window_to: Utc::now(),
            report,
            filed: None,
        };
        let j = report_json(&a);
        assert_eq!(j["verdict"], "common_mode");
        assert_eq!(j["deployments"].as_array().map(Vec::len), Some(4));
        assert_eq!(j["aggregate"]["diverging"], 4);
    }

    #[test]
    fn misses_in_disagreeing_directions_file_nothing() {
        let mut rows = Vec::new();
        for k in 0..4 {
            let id = Uuid::new_v4();
            let (r, e) = if k % 2 == 0 { (0.0, 0.001) } else { (0.001, 0.0) };
            for i in 0..40 {
                rows.push(obs(id, &format!("s{k}"), i, r, e));
            }
        }
        let report = FleetReport::assess(&rows, &declared_policy()).expect("assess");
        assert!(defect_for("t", &report).is_none());
        assert_eq!(
            report_json(&FleetAssessment {
                tenant_id: "t".into(),
                window_from: Utc::now(),
                window_to: Utc::now(),
                report,
                filed: None,
            })["verdict"],
            "idiosyncratic"
        );
    }

    #[test]
    fn an_empty_fleet_is_not_computable_not_clean() {
        let report = FleetReport::assess(&[], &declared_policy()).expect("assess");
        let j = report_json(&FleetAssessment {
            tenant_id: "t".into(),
            window_from: Utc::now(),
            window_to: Utc::now(),
            report,
            filed: None,
        });
        assert_eq!(j["verdict"], "not_computable");
        assert!(j["aggregate"].is_null());
    }
}
