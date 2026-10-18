//! The skill registry's constraints against PostgreSQL (Set R-a RA-1.1, RA-1.2, RA-1.3).
//!
//! Applies `migrations/0210_skill_registry.sql` itself, so it needs only a scratch
//! database — not the platform's migration history:
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p skills --test pg_registry -- --ignored --test-threads 1
//! ```
//!
//! Tests connect as the owner, which cannot see a missing GRANT, so the platform's
//! role is exercised with `SET LOCAL ROLE app_role` — the pattern of
//! `crates/backtest/tests/pg_universe.rs`.

use sqlx::{Executor, PgPool, Row};
use uuid::Uuid;

const MIGRATION: &str = include_str!("../../../migrations/0210_skill_registry.sql");

async fn pool() -> PgPool {
    let pg = PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects");
    // The platform's role comes from migration 0044; a scratch database has none.
    sqlx::raw_sql(
        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') \
         THEN CREATE ROLE app_role NOLOGIN; END IF; END $$;",
    )
    .execute(&pg)
    .await
    .unwrap();
    sqlx::raw_sql(MIGRATION)
        .execute(&pg)
        .await
        .expect("the migration applies, and applies twice");
    pg
}

fn hash(tag: &str) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for (i, b) in tag.bytes().cycle().take(32).enumerate() {
        write!(s, "{:02x}", b.wrapping_add(i as u8)).unwrap();
    }
    s
}

const ADMIT: &str = "SELECT skill_registry.admit($1, $2, $3, $4, 'research', $5, FALSE, $6, NULL, \
                     'use when testing', 5, 10, $7, 'test')";

async fn admit(
    pg: &PgPool,
    name: &str,
    scope: &str,
    user: Option<Uuid>,
    project: Option<Uuid>,
    status: &str,
) -> Result<Uuid, sqlx::Error> {
    let h = hash(&format!("{name}{scope}{}", Uuid::new_v4()));
    let row = sqlx::query(ADMIT)
        .bind(name)
        .bind(scope)
        .bind(user)
        .bind(project)
        .bind(status)
        .bind(&h)
        .bind(&h)
        .fetch_one(pg)
        .await?;
    Ok(row.get(0))
}

fn tag() -> String {
    Uuid::new_v4().simple().to_string()[..10].to_string()
}

async fn visible(pg: &PgPool, name: &str) -> i64 {
    sqlx::query("SELECT count(*) FROM skill_registry.skill_glossary WHERE name = $1")
        .bind(name)
        .fetch_one(pg)
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn an_admitted_skill_is_in_the_glossary_in_the_transaction_that_wrote_it() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    let mut tx = pg.begin().await.unwrap();
    let h = hash(&name);
    sqlx::query(ADMIT)
        .bind(&name)
        .bind("global")
        .bind(None::<Uuid>)
        .bind(None::<Uuid>)
        .bind("admitted")
        .bind(&h)
        .bind(&h)
        .execute(&mut *tx)
        .await
        .unwrap();
    // Inside the transaction that wrote it, the glossary already lists it.
    let n: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_glossary WHERE name = $1")
        .bind(&name)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
    tx.rollback().await.unwrap();
    // A rolled-back admission leaves nothing visible.
    assert_eq!(visible(&pg, &name).await, 0);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_visible_status_cannot_exist_without_a_version_and_admit_refuses_a_hidden_one() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    let direct = sqlx::query(
        "INSERT INTO skill_registry.skill (name, scope, category, status) \
         VALUES ($1, 'global', 'research', 'admitted')",
    )
    .bind(&name)
    .execute(&pg)
    .await;
    assert!(direct.is_err(), "no current_version: the CHECK refuses it");
    let hidden = admit(&pg, &name, "global", None, None, "quarantined").await;
    assert!(
        hidden.is_err(),
        "admit() makes a skill visible; quarantined is not visible"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_lower_scope_cannot_shadow_a_core_or_global_name_in_either_direction() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    let user = Uuid::new_v4();
    let project = Uuid::new_v4();
    admit(&pg, &name, "core", None, None, "admitted")
        .await
        .unwrap();
    assert!(admit(&pg, &name, "user", Some(user), None, "provisional")
        .await
        .is_err());
    assert!(
        admit(&pg, &name, "project", None, Some(project), "provisional")
            .await
            .is_err()
    );

    // The other direction: a name taken at a lower scope cannot be promoted onto.
    let low = format!("s-{}", tag());
    admit(&pg, &low, "user", Some(user), None, "provisional")
        .await
        .unwrap();
    assert!(admit(&pg, &low, "global", None, None, "admitted")
        .await
        .is_err());

    // Two users may each have their own skill of one name.
    let other = format!("s-{}", tag());
    admit(
        &pg,
        &other,
        "user",
        Some(Uuid::new_v4()),
        None,
        "provisional",
    )
    .await
    .unwrap();
    admit(
        &pg,
        &other,
        "user",
        Some(Uuid::new_v4()),
        None,
        "provisional",
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn scope_decides_who_may_own_a_skill() {
    let pg = pool().await;
    let n = format!("s-{}", tag());
    let u = Uuid::new_v4();
    assert!(
        admit(&pg, &n, "user", None, None, "provisional")
            .await
            .is_err(),
        "a user skill needs an owner"
    );
    assert!(
        admit(&pg, &n, "project", None, None, "provisional")
            .await
            .is_err(),
        "a project skill needs a project"
    );
    assert!(
        admit(&pg, &n, "global", Some(u), None, "admitted")
            .await
            .is_err(),
        "a global skill has no owner"
    );
    let pinned = sqlx::query(
        "INSERT INTO skill_registry.skill (name, scope, owner_user_id, category, status, pinned, current_version) \
         VALUES ($1, 'user', $2, 'c', 'quarantined', TRUE, NULL)",
    )
    .bind(&n)
    .bind(u)
    .execute(&pg)
    .await;
    assert!(pinned.is_err(), "only a global skill can be pinned");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_version_is_immutable_and_size_limits_hold_at_the_schema() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    admit(&pg, &name, "global", None, None, "admitted")
        .await
        .unwrap();
    let upd = sqlx::query("UPDATE skill_registry.skill_version SET description = 'changed' WHERE description = 'use when testing'")
        .execute(&pg)
        .await;
    assert!(upd.is_err(), "a version is never edited");
    let del = sqlx::query(
        "DELETE FROM skill_registry.skill_version WHERE description = 'use when testing'",
    )
    .execute(&pg)
    .await;
    assert!(del.is_err());

    // 101 tokens and 501 lines are refused whatever wrote them (SK-08).
    let h = hash(&format!("big{}", tag()));
    for (tokens, lines) in [(101, 10), (10, 501)] {
        let name = format!("s-{}", tag());
        let r = sqlx::query(
            "SELECT skill_registry.admit($1, 'global', NULL, NULL, 'c', 'admitted', FALSE, $2, NULL, 'd', $3, $4, $5, 't')",
        )
        .bind(&name)
        .bind(hash(&format!("{h}{tokens}{lines}")))
        .bind(tokens)
        .bind(lines)
        .bind(&h)
        .fetch_one(&pg)
        .await;
        assert!(r.is_err(), "{tokens} tokens / {lines} lines");
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn quarantine_removes_a_skill_from_the_glossary_at_once_and_a_patch_moves_the_version() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    let id = admit(&pg, &name, "global", None, None, "admitted")
        .await
        .unwrap();
    assert_eq!(visible(&pg, &name).await, 1);
    let v1: String =
        sqlx::query("SELECT current_version FROM skill_registry.skill WHERE skill_id = $1")
            .bind(id)
            .fetch_one(&pg)
            .await
            .unwrap()
            .get(0);
    let v2 = hash(&format!("patch{}", tag()));
    sqlx::query("SELECT skill_registry.admit($1, 'global', NULL, NULL, 'research', 'admitted', FALSE, $2, $3, 'patched', 3, 12, $2, 't')")
        .bind(&name)
        .bind(&v2)
        .bind(&v1)
        .execute(&pg)
        .await
        .unwrap();
    let (cur, desc): (String, String) = {
        let r = sqlx::query("SELECT current_version, description FROM skill_registry.skill_glossary WHERE name = $1")
            .bind(&name)
            .fetch_one(&pg)
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!((cur.as_str(), desc.as_str()), (v2.as_str(), "patched"));
    // The old version is still readable: history is not rewritten.
    let old: i64 =
        sqlx::query("SELECT count(*) FROM skill_registry.skill_version WHERE version_hash = $1")
            .bind(&v1)
            .fetch_one(&pg)
            .await
            .unwrap()
            .get(0);
    assert_eq!(old, 1);

    sqlx::query("UPDATE skill_registry.skill SET status = 'quarantined' WHERE skill_id = $1")
        .bind(id)
        .execute(&pg)
        .await
        .unwrap();
    assert_eq!(
        visible(&pg, &name).await,
        0,
        "gone from every glossary in the statement that quarantines it"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_proposal_without_a_source_cannot_be_written() {
    let pg = pool().await;
    let insert = |reason: &'static str, source: &'static str| {
        let pg = pg.clone();
        async move {
            sqlx::query(
                "INSERT INTO skill_registry.skill_proposal (proposed_name, project_id, proposer, reason, source_ref) \
                 VALUES ('p-one', $1, 'agent', $2, $3)",
            )
            .bind(Uuid::new_v4())
            .bind(reason)
            .bind(source)
            .execute(&pg)
            .await
        }
    };
    assert!(
        insert("T2", "").await.is_err(),
        "speculative: no trajectory"
    );
    assert!(insert("T2", "   ").await.is_err());
    assert!(insert("T9", "traj-1").await.is_err(), "an unknown trigger");
    assert!(insert("T2", "traj-1").await.is_ok());
    let decided = sqlx::query(
        "INSERT INTO skill_registry.skill_proposal (proposed_name, project_id, proposer, reason, source_ref, status) \
         VALUES ('p-two', $1, 'agent', 'T1', 'x', 'rejected')",
    )
    .bind(Uuid::new_v4())
    .execute(&pg)
    .await;
    assert!(decided.is_err(), "a decided proposal carries its verdict");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn the_frozen_selection_is_append_only() {
    let pg = pool().await;
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO skill_registry.project_skill_selection VALUES ($1, 1, 'a', 0)")
        .bind(project)
        .execute(&pg)
        .await
        .unwrap();
    assert!(sqlx::query(
        "UPDATE skill_registry.project_skill_selection SET rank = 5 WHERE project_id = $1"
    )
    .bind(project)
    .execute(&pg)
    .await
    .is_err());
    assert!(sqlx::query(
        "DELETE FROM skill_registry.project_skill_selection WHERE project_id = $1"
    )
    .bind(project)
    .execute(&pg)
    .await
    .is_err());
}

/// The grant 0145, 0147 and 0166 each had to add afterwards: the platform's own
/// role must be able to do what the platform does, and must not be able to edit history.
#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn the_platform_role_can_admit_and_read_and_cannot_edit_a_version() {
    let pg = pool().await;
    let name = format!("s-{}", tag());
    let user = Uuid::new_v4();
    let mut tx = pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    let h = hash(&name);
    sqlx::query(ADMIT)
        .bind(&name)
        .bind("user")
        .bind(Some(user))
        .bind(None::<Uuid>)
        .bind("provisional")
        .bind(&h)
        .bind(&h)
        .execute(&mut *tx)
        .await
        .expect("app_role can execute admit()");
    let n: i64 = sqlx::query("SELECT count(*) FROM skill_registry.skill_glossary WHERE name = $1")
        .bind(&name)
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can read the glossary")
        .get(0);
    assert_eq!(n, 1);
    sqlx::query("INSERT INTO skill_registry.skill_run (skill_id, version_hash, state) \
                 SELECT skill_id, current_version, 'exposed' FROM skill_registry.skill WHERE name = $1")
        .bind(&name)
        .execute(&mut *tx)
        .await
        .expect("app_role can record an exposure");
    // No grant to change or remove a version.
    let upd = sqlx::query(
        "UPDATE skill_registry.skill_version SET description = 'x' WHERE version_hash = $1",
    )
    .bind(&h)
    .execute(&mut *tx)
    .await;
    assert!(upd.is_err(), "app_role has no UPDATE on skill_version");
    tx.rollback().await.unwrap();
}
