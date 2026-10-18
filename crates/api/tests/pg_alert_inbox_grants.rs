//! `alert_inbox_state` (migration 0158) against PostgreSQL, as the role the
//! running platform uses.
//!
//! Every other test connects as the database owner, who can do anything, so a
//! missing or excessive GRANT passes them all and fails live (ADR-P0-13; 0145).
//! This one applies the migration and then acts as `app_role` — the pattern in
//! `crates/backtest/tests/pg_universe.rs`.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p api --test pg_alert_inbox_grants -- --ignored
//! ```
//!
//! The scratch database needs the schema through `users` (migration 0001-ish);
//! the test applies 0158 and 0167 itself, idempotently.

use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with the users table; applies 0158 and 0167 itself"]
async fn app_role_can_read_write_and_update_the_inbox_but_never_delete_from_it() {
    let pg = pool().await;
    // As the owner: the migration under test, exactly as shipped.
    sqlx::raw_sql(include_str!(
        "../../../migrations/0158_alert_inbox_state.sql"
    ))
    .execute(&pg)
    .await
    .expect("0158 applies (and applies twice)");
    // 0158 alone leaves DELETE granted, by 0044's default privileges; 0167 is
    // what takes it away, and this test is what found that it had to.
    sqlx::raw_sql(include_str!(
        "../../../migrations/0167_alert_inbox_no_delete.sql"
    ))
    .execute(&pg)
    .await
    .expect("0167 applies");

    let user = Uuid::new_v4();
    let mut tx = pg.begin().await.expect("tx");
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("inbox-grants-{user}@local"))
        .execute(&mut *tx)
        .await
        .expect("a user for the foreign key");

    // From here on, exactly what the platform can do.
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .expect("assume app_role");

    sqlx::query(
        "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
         VALUES ($1, 'monitor', 'event-1', 'acknowledged')",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .expect("app_role can INSERT");

    let state: String = sqlx::query_scalar(
        "SELECT state FROM alert_inbox_state WHERE user_id = $1 AND item_id = 'event-1'",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .expect("app_role can SELECT");
    assert_eq!(state, "acknowledged");

    let updated = sqlx::query(
        "UPDATE alert_inbox_state SET state = 'dismissed', acted_at = now()
          WHERE user_id = $1 AND source = 'monitor' AND item_id = 'event-1'",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .expect("app_role can UPDATE")
    .rows_affected();
    assert_eq!(updated, 1);

    // The CHECKs are the schema's, not the route's: a state outside the two
    // admitted is refused whoever asks.
    sqlx::query("SAVEPOINT bad_state")
        .execute(&mut *tx)
        .await
        .expect("sp");
    assert!(
        sqlx::query(
            "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
             VALUES ($1, 'monitor', 'event-2', 'deleted')",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .is_err(),
        "a state the CHECK does not admit was stored"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT bad_state")
        .execute(&mut *tx)
        .await
        .expect("rb");

    // And what it must not do. A dismissal is a record of what a user did; a
    // platform that could delete it could un-dismiss or erase one.
    sqlx::query("SAVEPOINT no_delete")
        .execute(&mut *tx)
        .await
        .expect("sp");
    let denied = sqlx::query("DELETE FROM alert_inbox_state WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await;
    assert!(
        denied.is_err(),
        "app_role could DELETE from alert_inbox_state; 0158 grants SELECT, INSERT, UPDATE only"
    );
    let msg = denied.unwrap_err().to_string();
    assert!(
        msg.contains("permission denied"),
        "refused for the wrong reason: {msg}"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT no_delete")
        .execute(&mut *tx)
        .await
        .expect("rb");

    tx.rollback().await.expect("rollback leaves nothing behind");
}

/// 0245 widens the source CHECK to admit `calibration` and changes nothing else: the
/// platform role still cannot DELETE, and a source outside the three is still refused.
#[tokio::test]
#[ignore = "needs DATABASE_URL with the users table; applies 0158, 0167 and 0245 itself"]
async fn calibration_is_an_admitted_source_and_delete_is_still_revoked() {
    let pg = pool().await;
    for sql in [
        include_str!("../../../migrations/0158_alert_inbox_state.sql"),
        include_str!("../../../migrations/0167_alert_inbox_no_delete.sql"),
        include_str!("../../../migrations/0245_alert_inbox_calibration_source.sql"),
        // Twice: the migration is idempotent.
        include_str!("../../../migrations/0245_alert_inbox_calibration_source.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pg).await.expect("applies");
    }
    let user = Uuid::new_v4();
    let mut tx = pg.begin().await.expect("tx");
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("inbox-cal-{user}@local"))
        .execute(&mut *tx)
        .await
        .expect("a user");
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .expect("assume app_role");

    sqlx::query(
        "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
         VALUES ($1, 'calibration', 'a-derived-id', 'acknowledged')",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .expect("app_role can record a calibration item's state");

    sqlx::query("SAVEPOINT bad_source")
        .execute(&mut *tx)
        .await
        .expect("sp");
    assert!(
        sqlx::query(
            "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
             VALUES ($1, 'bogus', 'x', 'acknowledged')",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .is_err(),
        "a source outside monitor, price and calibration was stored"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT bad_source")
        .execute(&mut *tx)
        .await
        .expect("rb");

    sqlx::query("SAVEPOINT no_delete")
        .execute(&mut *tx)
        .await
        .expect("sp");
    let denied = sqlx::query("DELETE FROM alert_inbox_state WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await;
    assert!(
        denied.is_err(),
        "0245 must not give DELETE back to app_role"
    );
    assert!(denied
        .unwrap_err()
        .to_string()
        .contains("permission denied"));
    sqlx::query("ROLLBACK TO SAVEPOINT no_delete")
        .execute(&mut *tx)
        .await
        .expect("rb");
    tx.rollback().await.expect("rollback");
}
