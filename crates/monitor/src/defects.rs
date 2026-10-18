//! Platform defects and fail-closed drill runs (Set T T-2.2, T-2.4; migration
//! 0165).
//!
//! # A defect is filed, not logged
//!
//! Two things in the fleet's life mean the *platform* is wrong rather than a
//! strategy: a divergence common to many deployments (the simulator is wrong in
//! a systematic way) and a fail-closed drill that failed (the kill switch does
//! not do what it claims). Each files one `monitor.platform_defect` row. A log
//! line is read by whoever happens to be looking; a row is a queue somebody
//! resolves, by name, with a sentence (the schema refuses a resolution with
//! either missing).
//!
//! `dedupe_key` keeps one *open* defect per condition, so an hourly job that sees
//! the same common-mode divergence every hour files it once and then finds it
//! already filed.
//!
//! # Why this is not `ledger::falsification::EvaluatorDefect`
//!
//! That type demands a sealed `Reproduction` — a replay somebody ran. A forward
//! divergence is the one thing nobody can replay, and `reconciliation::fleet`
//! refuses to mint one. The filing is this row; promoting it into the ledger is
//! the human who re-ran the backtests.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::store::{MonitorStore, StoreError};

/// What filed a defect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefectSource {
    /// `reconciliation::fleet` found a divergence common to the fleet.
    FleetCommonMode,
    /// A fail-closed drill failed.
    DrillFailed,
}

impl DefectSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FleetCommonMode => "fleet_common_mode",
            Self::DrillFailed => "drill_failed",
        }
    }
}

/// A defect to file.
#[derive(Clone, Debug, PartialEq)]
pub struct NewDefect {
    pub source: DefectSource,
    /// One open defect per key. Same condition, same key.
    pub dedupe_key: String,
    pub summary: String,
    pub evidence: serde_json::Value,
    pub filed_by: String,
}

/// What filing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filed {
    /// A new open defect.
    New(Uuid),
    /// The same condition was already open; nothing new was written.
    AlreadyOpen(Uuid),
}

impl Filed {
    #[must_use]
    pub fn defect_id(self) -> Uuid {
        match self {
            Self::New(id) | Self::AlreadyOpen(id) => id,
        }
    }
}

/// One defect as read back.
#[derive(Clone, Debug, PartialEq)]
pub struct DefectRow {
    pub defect_id: Uuid,
    pub source: String,
    pub dedupe_key: String,
    pub summary: String,
    pub evidence: serde_json::Value,
    pub filed_by: String,
    pub filed_at: DateTime<Utc>,
    pub status: String,
}

/// Which of the four T-2.4 drills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrillKind {
    /// The monitor dies: order flow stops within one cadence.
    MonitorLost,
    /// A divergence is planted: the strategy halts before its next order.
    DivergencePlanted,
    /// The kill switch trips: every deployment stops, and no agent can reset it.
    KillSwitchTripped,
    /// Service is restored: orders flow again once the cause is gone.
    ServiceRestored,
}

impl DrillKind {
    #[must_use]
    pub fn all() -> [Self; 4] {
        [
            Self::MonitorLost,
            Self::DivergencePlanted,
            Self::KillSwitchTripped,
            Self::ServiceRestored,
        ]
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MonitorLost => "monitor_lost",
            Self::DivergencePlanted => "divergence_planted",
            Self::KillSwitchTripped => "kill_switch_tripped",
            Self::ServiceRestored => "service_restored",
        }
    }
}

/// One drill's result, to record.
#[derive(Clone, Debug, PartialEq)]
pub struct DrillRun {
    pub drill: DrillKind,
    /// `true` for the schedule, `false` for a human asking.
    pub scheduled: bool,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub passed: bool,
    pub measurements: serde_json::Value,
    pub failure: Option<String>,
    pub run_by: String,
}

impl MonitorStore {
    /// File a defect, once per open condition.
    ///
    /// # Errors
    /// The database, or an `agent:` filer.
    pub async fn file_defect(&self, d: &NewDefect) -> Result<Filed, StoreError> {
        if d.filed_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{} may not file a platform defect",
                d.filed_by
            )));
        }
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO monitor.platform_defect (source, dedupe_key, summary, evidence, filed_by)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (dedupe_key) WHERE status = 'open' DO NOTHING
             RETURNING defect_id",
        )
        .bind(d.source.as_str())
        .bind(&d.dedupe_key)
        .bind(&d.summary)
        .bind(&d.evidence)
        .bind(&d.filed_by)
        .fetch_optional(self.pool())
        .await?;
        if let Some(id) = inserted {
            return Ok(Filed::New(id));
        }
        let open: Uuid = sqlx::query_scalar(
            "SELECT defect_id FROM monitor.platform_defect
              WHERE dedupe_key = $1 AND status = 'open'",
        )
        .bind(&d.dedupe_key)
        .fetch_one(self.pool())
        .await?;
        Ok(Filed::AlreadyOpen(open))
    }

    /// Open defects, newest first.
    ///
    /// # Errors
    /// The database.
    pub async fn open_defects(&self) -> Result<Vec<DefectRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT defect_id, source, dedupe_key, summary, evidence, filed_by, filed_at, status
               FROM monitor.platform_defect
              WHERE status = 'open'
              ORDER BY filed_at DESC",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| DefectRow {
                defect_id: r.get("defect_id"),
                source: r.get("source"),
                dedupe_key: r.get("dedupe_key"),
                summary: r.get("summary"),
                evidence: r.get("evidence"),
                filed_by: r.get("filed_by"),
                filed_at: r.get("filed_at"),
                status: r.get("status"),
            })
            .collect())
    }

    /// Resolve a defect. A named human, with a sentence.
    ///
    /// # Errors
    /// The database (including the trigger refusing a second resolution), a
    /// non-human actor or an empty resolution.
    pub async fn resolve_defect(
        &self,
        defect_id: Uuid,
        actor: &str,
        resolution: &str,
    ) -> Result<bool, StoreError> {
        if !actor.starts_with("human:") || actor.len() <= "human:".len() {
            return Err(StoreError::Invalid(format!("{actor} may not resolve a defect")));
        }
        if resolution.trim().is_empty() {
            return Err(StoreError::Invalid(
                "a defect is resolved with a sentence saying how".to_owned(),
            ));
        }
        let n = sqlx::query(
            "UPDATE monitor.platform_defect
                SET status = 'resolved', resolved_by = $2, resolution = $3, resolved_at = now()
              WHERE defect_id = $1 AND status = 'open'",
        )
        .bind(defect_id)
        .bind(actor)
        .bind(resolution)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(n == 1)
    }

    /// Record one drill run. A failed drill files its defect first, so the run
    /// row can name it — the schema refuses a failed run that does not.
    ///
    /// # Errors
    /// The database, or an `agent:` runner.
    pub async fn record_drill_run(&self, run: &DrillRun) -> Result<(Uuid, Option<Uuid>), StoreError> {
        if run.run_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!("{} may not run a drill", run.run_by)));
        }
        let defect = if run.passed {
            None
        } else {
            let failure = run
                .failure
                .clone()
                .unwrap_or_else(|| "the drill failed without saying why".to_owned());
            Some(
                self.file_defect(&NewDefect {
                    source: DefectSource::DrillFailed,
                    dedupe_key: format!("drill:{}", run.drill.as_str()),
                    summary: format!("fail-closed drill {} failed: {failure}", run.drill.as_str()),
                    evidence: run.measurements.clone(),
                    filed_by: run.run_by.clone(),
                })
                .await?
                .defect_id(),
            )
        };
        let duration_ms = (run.finished_at - run.started_at).num_milliseconds().max(0);
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO monitor.drill_run
               (drill, trigger, venue_class, started_at, finished_at, duration_ms, passed,
                measurements, failure, defect_id, run_by)
             VALUES ($1, $2, 'paper', $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING run_id",
        )
        .bind(run.drill.as_str())
        .bind(if run.scheduled { "scheduled" } else { "on_demand" })
        .bind(run.started_at)
        .bind(run.finished_at)
        .bind(duration_ms)
        .bind(run.passed)
        .bind(&run.measurements)
        .bind(run.failure.as_deref())
        .bind(defect)
        .bind(&run.run_by)
        .fetch_one(self.pool())
        .await?;
        Ok((id, defect))
    }

    /// When each drill last ran, for the schedule. A drill that never ran is
    /// absent, which the scheduler reads as due.
    ///
    /// # Errors
    /// The database.
    pub async fn last_drill_runs(&self) -> Result<Vec<(String, DateTime<Utc>, bool)>, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (drill) drill, started_at, passed
               FROM monitor.drill_run
              ORDER BY drill, started_at DESC",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("drill"), r.get("started_at"), r.get("passed")))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_drills_speak_the_schemas_strings() {
        let names: Vec<&str> = DrillKind::all().iter().map(|d| d.as_str()).collect();
        assert_eq!(
            names,
            [
                "monitor_lost",
                "divergence_planted",
                "kill_switch_tripped",
                "service_restored"
            ]
        );
    }

    #[test]
    fn filed_carries_its_id_either_way() {
        let id = Uuid::new_v4();
        assert_eq!(Filed::New(id).defect_id(), id);
        assert_eq!(Filed::AlreadyOpen(id).defect_id(), id);
    }
}
