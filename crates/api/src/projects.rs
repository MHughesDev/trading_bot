//! Research projects, the Desk, and the research cutoff (DATA-005 §4, ADR-0025).
//!
//! A project carries the one thing that makes research honest here: a **research
//! cutoff**. Data after it is holdout, and the Data API will not return it to a
//! research token. The agent cannot opt out, because the clipping happens in the
//! service rather than in a prompt (D-10).
//!
//! Each user also has exactly one **Desk** project, whose cutoff is "now". That
//! exists to resolve a genuine tension: a researcher legitimately needs to ask "what
//! is BTC doing right now", but answering that inside a project with a fixed cutoff
//! would mean handing over the holdout. So live questions go to the Desk, and the
//! Desk cannot run the gates that a holdout protects (DA-15).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Default holdout length when a project does not specify one (DATA-005 §4).
///
/// 90 days is the floor from the spec's `data.holdout_defaults` for intraday crypto.
/// A shorter holdout on minute bars leaves too few independent regimes to say
/// anything; a longer one on a platform with months of history leaves nothing to
/// research on.
pub const DEFAULT_HOLDOUT_DAYS: i64 = 90;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    /// The request asked for data the project's cutoff forbids (DA-03).
    ///
    /// The Display text carries no code prefix: the HTTP layer puts the code in its
    /// own field, and printing it twice reads like two different problems.
    #[error("{0}")]
    LiveDataDeskOnly(String),
    /// A Desk project cannot run the gates a holdout protects (DA-15).
    #[error("{0}")]
    DeskExploratoryOnly(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Research,
    Desk,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Research => "research",
            ProjectKind::Desk => "desk",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "research" => Some(ProjectKind::Research),
            "desk" => Some(ProjectKind::Desk),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub kind: ProjectKind,
    pub name: String,
    pub goal: Option<String>,
    pub instruments: Vec<String>,
    /// `None` for the Desk, which reads as of now.
    pub research_cutoff: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

impl Project {
    /// The instant this project may read up to.
    ///
    /// For the Desk that is now; for a research project it is the cutoff. Every data
    /// read goes through this, which is why there is no way to hold a project handle
    /// and accidentally read past its horizon.
    pub fn horizon(&self) -> DateTime<Utc> {
        self.research_cutoff.unwrap_or_else(Utc::now)
    }

    pub fn is_desk(&self) -> bool {
        self.kind == ProjectKind::Desk
    }

    /// Clips a requested window to the project's horizon (DA-03).
    ///
    /// Returns the effective end and whether clipping occurred. Clipping is silent in
    /// the data and explicit in the manifest: an agent that asked for more than it may
    /// see gets correct data for the window it may see, plus a `cutoff_applied` field
    /// saying so. Refusing outright would be worse — it teaches nothing and invites
    /// the caller to probe for the boundary.
    /// The second element is `Some` only when a **research cutoff** did the clipping.
    /// The Desk also clips — nobody reads the future — but calling that a
    /// `cutoff_applied` would tell a reader a holdout was enforced when none exists,
    /// and the whole value of that field is that it means exactly one thing.
    pub fn clip(&self, requested_end: DateTime<Utc>) -> (DateTime<Utc>, Option<DateTime<Utc>>) {
        let horizon = self.horizon();
        let effective = requested_end.min(horizon);
        let cutoff_applied = match self.research_cutoff {
            Some(cutoff) if requested_end > cutoff => Some(cutoff),
            _ => None,
        };
        (effective, cutoff_applied)
    }

    /// Whether a live read is permitted (DA-15).
    pub fn allow_live(&self) -> Result<(), ProjectError> {
        if self.is_desk() {
            Ok(())
        } else {
            Err(ProjectError::LiveDataDeskOnly(format!(
                "project {} has a research cutoff, so live data is out of reach; ask the \
                 Desk project instead",
                self.project_id
            )))
        }
    }

    /// Whether this project may run the significance gate and the vault (DA-15).
    pub fn allow_confirmatory(&self) -> Result<(), ProjectError> {
        if self.is_desk() {
            Err(ProjectError::DeskExploratoryOnly(
                "the Desk reads up to now, so it has no holdout to test against; run \
                 confirmatory work in a research project"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }
}

pub struct ProjectStore {
    pool: PgPool,
}

impl ProjectStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates a research project with a cutoff.
    ///
    /// When no cutoff is given it defaults to `now − holdout_len`, so a project
    /// created without thinking about the holdout still has one. Defaulting the other
    /// way — no holdout unless asked for — would make the safe path the one nobody
    /// takes.
    pub async fn create_research(
        &self,
        user_id: Uuid,
        name: &str,
        goal: Option<&str>,
        instruments: &[String],
        cutoff: Option<DateTime<Utc>>,
        holdout_days: Option<i64>,
    ) -> Result<Project, ProjectError> {
        let holdout = holdout_days.unwrap_or(DEFAULT_HOLDOUT_DAYS);
        if holdout <= 0 {
            return Err(ProjectError::Invalid(
                "holdout_days must be positive: a research project without a holdout \
                 cannot make a confirmatory claim"
                    .into(),
            ));
        }
        let cutoff = cutoff.unwrap_or_else(|| Utc::now() - Duration::days(holdout));
        if cutoff >= Utc::now() {
            return Err(ProjectError::Invalid(
                "the research cutoff must be in the past, or there is no holdout".into(),
            ));
        }

        let project_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO research_projects \
               (project_id, user_id, kind, name, goal, instruments, research_cutoff, \
                holdout_len, workspace_volume) \
             VALUES ($1,$2,'research',$3,$4,$5,$6, make_interval(days => $7), $8)",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(name)
        .bind(goal)
        .bind(instruments)
        .bind(cutoff)
        .bind(holdout as i32)
        .bind(format!("tbot-ws-{project_id}"))
        .execute(&self.pool)
        .await?;

        self.get(project_id).await
    }

    /// Returns the user's Desk project, creating it on first use (DA-15).
    pub async fn desk(&self, user_id: Uuid) -> Result<Project, ProjectError> {
        if let Some(existing) =
            sqlx::query("SELECT project_id FROM research_projects WHERE user_id=$1 AND kind='desk'")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?
        {
            return self.get(existing.get("project_id")).await;
        }

        let project_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO research_projects \
               (project_id, user_id, kind, name, goal, research_cutoff, workspace_volume) \
             VALUES ($1,$2,'desk','Desk','Live and recent questions', NULL, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(format!("tbot-ws-{project_id}"))
        .execute(&self.pool)
        .await?;

        // A concurrent caller may have won the unique index; re-read rather than
        // assuming this insert is the one that landed.
        let row = sqlx::query(
            "SELECT project_id FROM research_projects WHERE user_id=$1 AND kind='desk'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        self.get(row.get("project_id")).await
    }

    pub async fn get(&self, project_id: Uuid) -> Result<Project, ProjectError> {
        let row = sqlx::query("SELECT * FROM research_projects WHERE project_id=$1")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| ProjectError::NotFound(project_id.to_string()))?;
        row_to_project(&row)
    }

    pub async fn list(&self, user_id: Uuid) -> Result<Vec<Project>, ProjectError> {
        let rows = sqlx::query(
            "SELECT * FROM research_projects WHERE user_id=$1 AND status='active' \
             ORDER BY kind, created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_project).collect()
    }
}

fn row_to_project(row: &sqlx::postgres::PgRow) -> Result<Project, ProjectError> {
    let kind: String = row.get("kind");
    Ok(Project {
        project_id: row.get("project_id"),
        user_id: row.get("user_id"),
        kind: ProjectKind::parse(&kind)
            .ok_or_else(|| ProjectError::Invalid(format!("unknown project kind {kind}")))?,
        name: row.get("name"),
        goal: row.get("goal"),
        instruments: row.get("instruments"),
        research_cutoff: row.get("research_cutoff"),
        status: row.get("status"),
        created_at: row.get("created_at"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(kind: ProjectKind, cutoff: Option<DateTime<Utc>>) -> Project {
        Project {
            project_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            kind,
            name: "t".into(),
            goal: None,
            instruments: vec![],
            research_cutoff: cutoff,
            status: "active".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn a_research_window_is_clipped_to_the_cutoff() {
        let cutoff = Utc::now() - Duration::days(90);
        let p = project(ProjectKind::Research, Some(cutoff));
        let (end, applied) = p.clip(Utc::now());
        assert_eq!(end, cutoff);
        assert_eq!(applied, Some(cutoff));
    }

    #[test]
    fn a_window_inside_the_cutoff_is_untouched() {
        let cutoff = Utc::now() - Duration::days(90);
        let p = project(ProjectKind::Research, Some(cutoff));
        let asked = cutoff - Duration::days(10);
        let (end, applied) = p.clip(asked);
        assert_eq!(end, asked);
        assert_eq!(applied, None, "no clipping means no cutoff_applied claim");
    }

    #[test]
    fn the_desk_reads_up_to_now_and_allows_live() {
        let desk = project(ProjectKind::Desk, None);
        assert!(desk.allow_live().is_ok());
        let (end, applied) = desk.clip(Utc::now() - Duration::hours(1));
        assert!(applied.is_none());
        assert!(end <= Utc::now());
    }

    #[test]
    fn the_desk_still_cannot_read_the_future_but_reports_no_cutoff() {
        // The Desk clips a future window to now — nobody reads the future — but it
        // must not claim a `cutoff_applied`. That field means "a holdout was
        // enforced here", and on the Desk there is no holdout to enforce; saying
        // otherwise would let a Desk result look like a confirmatory one.
        let desk = project(ProjectKind::Desk, None);
        let (end, applied) = desk.clip(Utc::now() + Duration::days(365));
        assert!(end <= Utc::now() + Duration::seconds(1));
        assert_eq!(applied, None);
    }

    #[test]
    fn a_research_project_refuses_live_and_points_at_the_desk() {
        let p = project(ProjectKind::Research, Some(Utc::now() - Duration::days(30)));
        let err = p.allow_live().expect_err("live must be refused");
        assert!(
            format!("{err}").contains("Desk"),
            "the refusal must say where to go instead: {err}"
        );
    }

    #[test]
    fn the_desk_cannot_run_confirmatory_work() {
        // The Desk reads up to now, so it has nothing held out to test against. If it
        // could run the significance gate, every Desk result would be in-sample and
        // would look exactly like a real one.
        let desk = project(ProjectKind::Desk, None);
        assert!(desk.allow_confirmatory().is_err());
        let research = project(ProjectKind::Research, Some(Utc::now() - Duration::days(1)));
        assert!(research.allow_confirmatory().is_ok());
    }

    #[test]
    fn clipping_never_widens_a_window() {
        // Property: whatever is asked for, the effective end is never past the
        // horizon. This is the whole guarantee in one line.
        let cutoff = Utc::now() - Duration::days(45);
        let p = project(ProjectKind::Research, Some(cutoff));
        for days in [-1000i64, -100, -45, -44, 0, 1, 1000] {
            let asked = Utc::now() + Duration::days(days);
            let (end, _) = p.clip(asked);
            assert!(end <= cutoff, "asked {asked}, got {end}, cutoff {cutoff}");
        }
    }
}
