//! The comments an author wrote are kept beside the revision, not inside it
//! (S-5.15; migration 0239).
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test strategy_notes_store -- --ignored --test-threads=1
//! ```

use api::routes::strategy_notes::{load, replace, Note};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS strategy_notes_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE strategy_notes_test")
        .await
        .unwrap();
    let _ = admin
        .execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='app_role') \
             THEN CREATE ROLE app_role NOLOGIN; END IF; END $$",
        )
        .await;
    let pool = PgPool::connect(&with_db(&base_url(), "strategy_notes_test"))
        .await
        .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("two below the root");
    let sql =
        std::fs::read_to_string(root.join("migrations/0239_strategy_revision_notes.sql")).unwrap();
    pool.execute(sql.as_str()).await.expect("0239 applies");
    pool
}

const H: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn note(clause: &str, line: &str) -> Note {
    Note {
        clause: clause.into(),
        lines: vec![line.into()],
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_comment_survives_the_store_in_source_order_and_the_last_text_wins() {
    let pool = db().await;
    let me = Uuid::new_v4();
    let n = replace(
        &pool,
        me,
        H,
        &[
            note("header", "why this exists"),
            note(
                "entry.long[0]",
                "60 rather than 70: refuses an extended move",
            ),
        ],
    )
    .await
    .unwrap();
    assert_eq!(n, 2);
    let got = load(&pool, me, H).await.unwrap();
    assert_eq!(got[0].clause, "header");
    assert_eq!(
        got[1].lines[0],
        "60 rather than 70: refuses an extended move"
    );

    // The same logic saved again with reworded comments replaces the notes.
    replace(&pool, me, H, &[note("entry.long[0]", "reworded")])
        .await
        .unwrap();
    let got = load(&pool, me, H).await.unwrap();
    assert_eq!(got, vec![note("entry.long[0]", "reworded")]);

    // An empty list clears them.
    replace(&pool, me, H, &[]).await.unwrap();
    assert!(load(&pool, me, H).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_tenants_notes_are_invisible_to_another_with_the_same_content_hash() {
    let pool = db().await;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    replace(&pool, a, H, &[note("header", "mine")])
        .await
        .unwrap();
    assert!(load(&pool, b, H).await.unwrap().is_empty());
    // And b writing does not touch a's.
    replace(&pool, b, H, &[note("header", "theirs")])
        .await
        .unwrap();
    assert_eq!(load(&pool, a, H).await.unwrap()[0].lines[0], "mine");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn malformed_input_is_refused_before_it_reaches_the_table() {
    let pool = db().await;
    let me = Uuid::new_v4();
    assert!(replace(&pool, me, "not-a-hash", &[note("header", "x")])
        .await
        .is_err());
    assert!(replace(
        &pool,
        me,
        H,
        &[Note {
            clause: "header".into(),
            lines: vec![]
        }]
    )
    .await
    .is_err());
    assert!(replace(&pool, me, H, &[note(" ", "x")]).await.is_err());
}

/// Owner-connected tests cannot see a missing GRANT; this one switches role.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_platform_role_can_replace_and_read_notes() {
    let pool = db().await;
    let me = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    for sql in [
        "INSERT INTO strategy_revision_notes (user_id, revision_hash, clause, lines, position) VALUES ($1, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'header', ARRAY['x'], 0)",
        "UPDATE strategy_revision_notes SET lines = ARRAY['y'] WHERE user_id = $1",
        "SELECT 1 FROM strategy_revision_notes WHERE user_id = $1",
        "DELETE FROM strategy_revision_notes WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(me).execute(&mut *tx).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    tx.rollback().await.unwrap();
}
