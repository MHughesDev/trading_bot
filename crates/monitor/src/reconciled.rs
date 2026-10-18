//! Per-venue reconciliation verdicts (COMP-007 §4, §5, §8.3; S-7.6).
//!
//! # What this replaces
//!
//! "Every venue reconciles clean" used to be derived as "no venue is halted".
//! That proxy was wrong in both directions: a venue halted for a connection loss
//! read as *not reconciled*, and a venue that failed reconciliation without being
//! halted read as *clean*. And a venue halted *because* it disagreed with our book
//! could be resumed with a typed string, a reason and five minutes' patience —
//! nothing asked whether the disagreement had cleared.
//!
//! Now the verdict is a row (`monitor.venue_reconciliation`, append-only), and
//! the rule is a SQL function both this crate and a human at `psql` can ask,
//! `monitor.venue_reconciled(venue)`: a venue reconciles when it has never been
//! checked, or when its most recent check agreed.
//!
//! [`MonitorStore::reset`](crate::store::MonitorStore::reset) reads it **itself**
//! rather than trusting the caller's request. A precondition the caller asserts
//! is not a precondition.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::store::{MonitorStore, StoreError};

/// Who a verdict written by `crates/reconciliation` is attributed to: the
/// platform process that compared the books, not the monitor, which compared
/// nothing.
pub const RECONCILIATION_ACTOR: &str = "reconciliation";

/// What one reconciliation pass found at one venue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VenueVerdict {
    /// Our book and the venue's agree on every instrument checked.
    Agreed,
    /// They disagree on at least one.
    Disagreed,
    /// The venue could not be read. Not clean: an unread venue has not agreed.
    Unreadable,
}

impl VenueVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agreed => "agreed",
            Self::Disagreed => "disagreed",
            Self::Unreadable => "unreadable",
        }
    }
}

impl std::str::FromStr for VenueVerdict {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agreed" => Ok(Self::Agreed),
            "disagreed" => Ok(Self::Disagreed),
            "unreadable" => Ok(Self::Unreadable),
            other => Err(format!("unknown reconciliation verdict {other:?}")),
        }
    }
}

/// One recorded check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueCheck {
    pub check_id: Uuid,
    pub venue: String,
    pub verdict: VenueVerdict,
    pub instruments_checked: i32,
    pub detail: Option<String>,
    pub checked_by: String,
    pub checked_at: DateTime<Utc>,
}

impl MonitorStore {
    /// Record one venue's verdict.
    ///
    /// # Errors
    /// The database, a blank venue, an `agent:` author, or a verdict that is not
    /// clean and says nothing about why (the schema refuses that too).
    pub async fn record_venue_check(
        &self,
        venue: &str,
        verdict: VenueVerdict,
        instruments_checked: usize,
        detail: Option<&str>,
        checked_by: &str,
    ) -> Result<Uuid, StoreError> {
        if venue.trim().is_empty() {
            return Err(StoreError::Invalid(
                "a reconciliation verdict must name its venue".to_owned(),
            ));
        }
        if checked_by.starts_with("agent:") {
            return Err(StoreError::Invalid(format!(
                "{checked_by} may not record a reconciliation verdict"
            )));
        }
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO monitor.venue_reconciliation
               (check_id, venue, verdict, instruments_checked, detail, checked_by)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(venue)
        .bind(verdict.as_str())
        .bind(i32::try_from(instruments_checked).unwrap_or(i32::MAX))
        .bind(detail)
        .bind(checked_by)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// Whether `venue` reconciles now: never checked, or last check agreed.
    ///
    /// # Errors
    /// The database. A caller must treat an error as *not* reconciled.
    pub async fn venue_reconciled(&self, venue: &str) -> Result<bool, StoreError> {
        let ok: bool = sqlx::query_scalar("SELECT monitor.venue_reconciled($1)")
            .bind(venue)
            .fetch_one(self.pool())
            .await?;
        Ok(ok)
    }

    /// Whether every venue that has ever been checked reconciles now — the
    /// precondition of a global reset (§5).
    ///
    /// # Errors
    /// The database. A caller must treat an error as *not* reconciled.
    pub async fn all_venues_reconciled(&self) -> Result<bool, StoreError> {
        let failing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT DISTINCT venue FROM monitor.venue_reconciliation) v
              WHERE NOT monitor.venue_reconciled(v.venue)",
        )
        .fetch_one(self.pool())
        .await?;
        Ok(failing == 0)
    }

    /// The latest check per venue, for the operator.
    ///
    /// # Errors
    /// The database.
    pub async fn latest_venue_checks(&self) -> Result<Vec<VenueCheck>, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (venue)
                    check_id, venue, verdict, instruments_checked, detail, checked_by, checked_at
               FROM monitor.venue_reconciliation
              ORDER BY venue, checked_at DESC, check_id DESC",
        )
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let verdict: String = r.get("verdict");
            out.push(VenueCheck {
                check_id: r.get("check_id"),
                venue: r.get("venue"),
                verdict: verdict.parse().map_err(StoreError::Invalid)?,
                instruments_checked: r.get("instruments_checked"),
                detail: r.get("detail"),
                checked_by: r.get("checked_by"),
                checked_at: r.get("checked_at"),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_round_trip_through_the_strings_the_schema_admits() {
        for v in [
            VenueVerdict::Agreed,
            VenueVerdict::Disagreed,
            VenueVerdict::Unreadable,
        ] {
            assert_eq!(v.as_str().parse::<VenueVerdict>(), Ok(v));
        }
        assert!("clean".parse::<VenueVerdict>().is_err());
    }
}
