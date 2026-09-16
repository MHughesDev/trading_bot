//! The durable backfill cursor, in Postgres (`dataplane.backfill_state`,
//! migration 0099; DATA-005 §12; Set P P-1.1).
//!
//! The whole point of this store is that a worker can die at any instruction and
//! the next one continues from exactly where the last page landed. Two rules
//! make that true, and both are enforced here rather than asked of callers:
//!
//! * **[`ensure`](PgBackfillState::ensure) never resets an existing row.** It is
//!   `INSERT … ON CONFLICT DO NOTHING` followed by a read, so resubmitting a
//!   window that already completed returns a completed row and the engine
//!   fetches nothing.
//! * **[`advance`](PgBackfillState::advance) only ever moves the cursor
//!   forward.** A page that arrives out of order — a retry of an older page
//!   after a newer one landed — cannot rewind coverage and cause a refetch.
//!
//! `attempts` counts *pauses*, not failures. A rate-limited window with 40
//! attempts is a healthy window against a slow venue.
//!
//! The SQL is spelled out per statement rather than assembled from shared
//! fragments: a predicate built by string concatenation is a predicate nobody
//! can read in place, and this one decides whether history is refetched.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use dataplane::backfill::{BackfillKey, BackfillState, BackfillStateStore, BackfillStatus, StateError};

/// Postgres-backed [`BackfillStateStore`].
#[derive(Clone, Debug)]
pub struct PgBackfillState {
    pool: PgPool,
}

type Row = (
    String,                // source
    String,                // venue
    String,                // instrument
    String,                // timeframe
    DateTime<Utc>,         // window_start
    DateTime<Utc>,         // window_end
    Option<DateTime<Utc>>, // cursor
    String,                // status
    i32,                   // attempts
    Option<DateTime<Utc>>, // backoff_until
    Option<String>,        // last_error
    i64,                   // pages_fetched
    i64,                   // rows_written
    DateTime<Utc>,         // updated_at
);

fn err(e: sqlx::Error) -> StateError {
    StateError::Backend(e.to_string())
}

fn row_to_state(r: Row) -> Result<BackfillState, StateError> {
    let status = BackfillStatus::parse(&r.7)
        .ok_or_else(|| StateError::Backend(format!("unknown backfill status {:?} in the database", r.7)))?;
    Ok(BackfillState {
        key: BackfillKey {
            source: r.0,
            venue: r.1,
            instrument: r.2,
            timeframe: r.3,
            window_start: r.4,
            window_end: r.5,
        },
        cursor: r.6,
        status,
        attempts: r.8,
        backoff_until: r.9,
        last_error: r.10,
        pages_fetched: r.11,
        rows_written: r.12,
        updated_at: r.13,
    })
}

const SELECT_ROW: &str = "SELECT source, venue, instrument, timeframe, window_start, window_end,
                                 cursor, status, attempts, backoff_until, last_error,
                                 pages_fetched, rows_written, updated_at
                            FROM dataplane.backfill_state";

impl PgBackfillState {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Windows a worker may claim right now: pending, or paused with their
    /// backoff elapsed. Ordered oldest-window-first so deep history lands before
    /// recent history — the deep end is what `strict_v1` Gate 9 is short of.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn claimable(&self, limit: i64) -> Result<Vec<BackfillState>, StateError> {
        let sql = format!(
            "{SELECT_ROW}
              WHERE status = 'pending' OR (status = 'paused' AND backoff_until <= now())
              ORDER BY window_start ASC
              LIMIT $1"
        );
        let rows: Vec<Row> = sqlx::query_as(&sql).bind(limit).fetch_all(&self.pool).await.map_err(err)?;
        rows.into_iter().map(row_to_state).collect()
    }

    /// Every window recorded for an instrument, for the coverage census (P-1.7).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn for_instrument(&self, instrument: &str) -> Result<Vec<BackfillState>, StateError> {
        let sql = format!("{SELECT_ROW} WHERE instrument = $1 ORDER BY timeframe, window_start");
        let rows: Vec<Row> = sqlx::query_as(&sql).bind(instrument).fetch_all(&self.pool).await.map_err(err)?;
        rows.into_iter().map(row_to_state).collect()
    }

    /// Attach the job-service manifest hash to a window.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn record_manifest(&self, key: &BackfillKey, manifest_hash: &str) -> Result<(), StateError> {
        sqlx::query(
            "UPDATE dataplane.backfill_state SET manifest_hash = $7, updated_at = now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .bind(manifest_hash)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }
}

#[async_trait]
impl BackfillStateStore for PgBackfillState {
    async fn load(&self, key: &BackfillKey) -> Result<Option<BackfillState>, StateError> {
        let sql = format!(
            "{SELECT_ROW}
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6"
        );
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(&key.source)
            .bind(&key.venue)
            .bind(&key.instrument)
            .bind(&key.timeframe)
            .bind(key.window_start)
            .bind(key.window_end)
            .fetch_optional(&self.pool)
            .await
            .map_err(err)?;
        row.map(row_to_state).transpose()
    }

    async fn ensure(&self, key: &BackfillKey) -> Result<BackfillState, StateError> {
        // DO NOTHING, never DO UPDATE: a second submission of a window that is
        // already half-done must not rewind its cursor. That is the whole
        // idempotency claim (JB-02, DA-11).
        sqlx::query(
            "INSERT INTO dataplane.backfill_state (source, venue, instrument, timeframe, window_start, window_end)
             VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        self.load(key)
            .await?
            .ok_or_else(|| StateError::Backend("backfill_state row vanished immediately after insert".into()))
    }

    async fn start(&self, key: &BackfillKey) -> Result<(), StateError> {
        // A completed window is never re-opened. Without this guard a stray
        // resubmission would set a finished window back to 'running' and the
        // engine would refetch the whole thing.
        sqlx::query(
            "UPDATE dataplane.backfill_state
                SET status='running', backoff_until=NULL, updated_at=now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6 AND status <> 'completed'",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn advance(&self, key: &BackfillKey, cursor: DateTime<Utc>, rows_written: i64) -> Result<(), StateError> {
        // GREATEST, so an out-of-order retry cannot rewind coverage; LEAST, so a
        // source that over-reports cannot push the cursor past the window and
        // trip the 0099 CHECK.
        sqlx::query(
            "UPDATE dataplane.backfill_state
                SET cursor = GREATEST(COALESCE(cursor, window_start), LEAST($7, window_end)),
                    pages_fetched = pages_fetched + 1,
                    rows_written = rows_written + $8,
                    status = 'running', backoff_until = NULL, last_error = NULL,
                    updated_at = now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6 AND status <> 'completed'",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .bind(cursor)
        .bind(rows_written)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn pause(&self, key: &BackfillKey, until: DateTime<Utc>, reason: &str) -> Result<(), StateError> {
        sqlx::query(
            "UPDATE dataplane.backfill_state
                SET status='paused', backoff_until=$7, attempts = attempts + 1,
                    last_error=$8, updated_at=now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6 AND status <> 'completed'",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .bind(until)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn complete(&self, key: &BackfillKey) -> Result<(), StateError> {
        // The cursor is set to window_end in the same statement, because the
        // 0099 CHECK refuses a completed row whose cursor is short of it.
        // Completion is therefore a statement about coverage, not a flag.
        sqlx::query(
            "UPDATE dataplane.backfill_state
                SET status='completed', cursor=window_end, backoff_until=NULL,
                    last_error=NULL, updated_at=now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn fail(&self, key: &BackfillKey, reason: &str) -> Result<(), StateError> {
        sqlx::query(
            "UPDATE dataplane.backfill_state
                SET status='failed', last_error=$7, attempts = attempts + 1,
                    backoff_until=NULL, updated_at=now()
              WHERE source=$1 AND venue=$2 AND instrument=$3 AND timeframe=$4
                AND window_start=$5 AND window_end=$6 AND status <> 'completed'",
        )
        .bind(&key.source)
        .bind(&key.venue)
        .bind(&key.instrument)
        .bind(&key.timeframe)
        .bind(key.window_start)
        .bind(key.window_end)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }
}
