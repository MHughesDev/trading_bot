//! The day budget in trials (AGENT-014 §2.1, §7 row 2; Set S tasks S-11.5,
//! S-11.6; migration 0161; ADR-P11-05).
//!
//! The budget is in dollars and a run is metered in trials, so for as long as the
//! day's only figure was `usd_per_day` the day gate could not fire. A session now
//! has a `trials_per_day` of its own, in the unit a run is metered in, filled at
//! creation with [`research_session::charter::DEFAULT_TRIALS_PER_DAY`] and
//! editable like the dollar figure.
//!
//! A session with **no row** — one created before 0161 — reads as `None`, which
//! `execute::day_bound` turns into `NotDenominated`: the gate names what is
//! missing and every lease says so, exactly as before. It is never read as "no
//! cap" and never silently defaulted at read time: a default nobody wrote down is
//! a budget nobody chose.

use sqlx::PgPool;
use uuid::Uuid;

/// The session's day budget in trials, or `None` when nobody set one.
///
/// # Errors
/// Database errors. A failed read is an `Err`, not a `None`: the caller must not
/// mistake an unread budget for an unset one.
pub async fn trials_per_day(pg: &PgPool, session: Uuid) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT trials_per_day FROM session_day_budget WHERE session_id = $1")
            .bind(session)
            .fetch_optional(pg)
            .await?;
    Ok(row.map(|r| r.0))
}

/// Why a day budget was refused.
#[derive(Debug, thiserror::Error)]
pub enum DayBudgetError {
    #[error(
        "a day budget of {0} trials is not a budget: it is at least one trial, because a session \
         that may spend none cannot research"
    )]
    NotPositive(i64),
    #[error("{0}")]
    Db(#[from] sqlx::Error),
}

/// Set (or amend) the day budget.
///
/// # Errors
/// [`DayBudgetError::NotPositive`] for zero or less; database errors.
pub async fn set_trials_per_day(
    pg: &PgPool,
    session: Uuid,
    trials: i64,
) -> Result<(), DayBudgetError> {
    if trials <= 0 {
        return Err(DayBudgetError::NotPositive(trials));
    }
    sqlx::query(
        "INSERT INTO session_day_budget (session_id, trials_per_day) VALUES ($1, $2)
         ON CONFLICT (session_id) DO UPDATE
             SET trials_per_day = EXCLUDED.trials_per_day, set_at = now()",
    )
    .bind(session)
    .bind(trials)
    .execute(pg)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_that_buys_no_trial_is_refused_with_the_reason() {
        for bad in [0, -1, i64::MIN] {
            let e = DayBudgetError::NotPositive(bad).to_string();
            assert!(e.contains("at least one trial"), "{e}");
        }
    }
}
