//! N-4.1: served forecasts are joined to the bars that realise them (migration 0227), against
//! PostgreSQL, including what the platform role can do (`SET LOCAL ROLE app_role`).
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -p model-registry --test pg_forecast_realisation -- --ignored --test-threads=1
//! ```
//! The scratch database needs migrations 0017-0020, 0028, 0044 and 0222, then 0227.

use chrono::{DateTime, Duration, Utc};
use model_registry::forecast_realisation::{realise_pending, Close, ClosedBars, NoBars};
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

async fn model(pg: &sqlx::PgPool) -> String {
    let id = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by)
         VALUES ($1, $1, 'm', 'forecaster', 'crypto', '{}'::jsonb, $2)",
    )
    .bind(&id)
    .bind(Uuid::new_v4())
    .execute(pg)
    .await
    .unwrap();
    id
}

async fn forecast(
    pg: &sqlx::PgPool,
    m: &str,
    at: DateTime<Utc>,
    horizon: Option<i32>,
    tf: Option<&str>,
) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO model_forecasts (model_id, version, instrument_id, predicted_q50, horizon_bars, timeframe, created_at)
         VALUES ($1, 1, 'BTC-USD', 0.01, $2, $3, $4) RETURNING forecast_id",
    )
    .bind(m)
    .bind(horizon)
    .bind(tf)
    .bind(at)
    .fetch_one(pg)
    .await
    .unwrap();
    id
}

/// One-minute bars closing at every whole minute from `start`, close = 100 + minute index.
struct Minutes(DateTime<Utc>, i64);

#[async_trait::async_trait]
impl ClosedBars for Minutes {
    async fn closes(
        &self,
        _: &str,
        _: domain::payloads::bar::Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Close>> {
        Ok((0..self.1)
            .map(|m| (self.0 + Duration::minutes(m), m))
            .filter(|(t, _)| *t >= from && *t < to)
            .map(|(t, m)| Close {
                ts_ns: t.timestamp_nanos_opt().unwrap(),
                close: 100.0 + m as f64,
            })
            .collect())
    }
}

async fn realized(pg: &sqlx::PgPool, id: Uuid) -> Option<f64> {
    sqlx::query_scalar("SELECT realized FROM model_forecasts WHERE forecast_id = $1")
        .bind(id)
        .fetch_one(pg)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0227"]
async fn a_due_forecast_is_realised_from_the_bars_and_a_pending_or_legacy_one_is_not() {
    let pg = pool().await;
    let m = model(&pg).await;
    let now = Utc::now();
    let start = now - Duration::hours(3);
    let t0 = start + Duration::minutes(10) + Duration::seconds(30);
    let due = forecast(&pg, &m, t0, Some(5), Some("1m")).await;
    // horizon has not passed: 5 bars after "now - 1 minute"
    let pending = forecast(&pg, &m, now - Duration::minutes(1), Some(5), Some("1m")).await;
    // logged before 0227: no horizon or timeframe
    let legacy = forecast(&pg, &m, t0, None, None).await;

    let r = realise_pending(&pg, &Minutes(start, 180), now, 100).await.unwrap();
    assert!(r.filled >= 1);

    // entry = close of minute 10 (110), exit = first close at/after 15.5 min = minute 16 (116)
    let got = realized(&pg, due).await.expect("realised");
    assert!((got - (116.0 / 110.0 - 1.0)).abs() < 1e-12, "{got}");
    assert!(realized(&pg, pending).await.is_none(), "the horizon has not passed");
    assert!(realized(&pg, legacy).await.is_none(), "a legacy row is never guessed at");
    assert!(r.unresolvable >= 1);

    // idempotent: a second pass does not rewrite or double-count it
    let again = realise_pending(&pg, &Minutes(start, 180), now, 100).await.unwrap();
    assert_eq!(realized(&pg, due).await, Some(got));
    assert!(again.filled <= r.filled);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0227"]
async fn with_no_bar_store_every_due_forecast_waits_and_nothing_is_invented() {
    let pg = pool().await;
    let m = model(&pg).await;
    let id = forecast(&pg, &m, Utc::now() - Duration::hours(2), Some(5), Some("1m")).await;
    let r = realise_pending(&pg, &NoBars, Utc::now(), 100).await.unwrap();
    assert!(r.waiting_for_bars >= 1);
    assert!(realized(&pg, id).await.is_none());
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0227"]
async fn the_platform_role_can_write_the_realised_value_and_the_new_column() {
    let pg = pool().await;
    let m = model(&pg).await;
    let id = forecast(&pg, &m, Utc::now(), Some(5), Some("1m")).await;
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE model_forecasts SET realized = 0.02, realized_at = now(), timeframe = '5m' WHERE forecast_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("app_role can fill realized and write timeframe");
    sqlx::query(
        "INSERT INTO model_forecasts (model_id, version, instrument_id, horizon_bars, timeframe) VALUES ($1, 1, 'X', 3, '1h')",
    )
    .bind(&m)
    .execute(&mut *tx)
    .await
    .expect("app_role can log a forecast with its horizon and timeframe");
    tx.rollback().await.unwrap();
}
