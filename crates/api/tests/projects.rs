//! Research projects against a real Postgres (DATA-005 §4, DA-05, DA-15).
//!
//! The unit tests in `api::projects` cover clipping arithmetic. The guarantees that
//! matter are enforced in the database, and only a real one can show them: that a
//! user has exactly one Desk, that a project's kind cannot change, and above all
//! that a cutoff cannot be moved once the project has an experiment.
//!
//! ```bash
//! API_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p api --test projects -- --test-threads=1
//! ```

use chrono::{Duration, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use api::projects::{ProjectError, ProjectKind, ProjectStore};

async fn pool() -> Option<PgPool> {
    let url = std::env::var("API_TEST_DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

/// A throwaway user, since projects are user-scoped by foreign key.
async fn make_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("proj-test-{id}@local"))
        .execute(pool)
        .await
        .expect("create user");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    sqlx::query("DELETE FROM users WHERE user_id=$1")
        .bind(user)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn a_user_has_exactly_one_desk_however_often_it_is_asked_for() {
    let Some(pool) = pool().await else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());

    let first = store.desk(user).await.expect("desk");
    let second = store.desk(user).await.expect("desk again");
    assert_eq!(first.project_id, second.project_id);
    assert_eq!(first.kind, ProjectKind::Desk);
    assert!(first.research_cutoff.is_none(), "the Desk reads as of now");

    let count: i64 =
        sqlx::query("SELECT count(*) AS n FROM research_projects WHERE user_id=$1 AND kind='desk'")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
    assert_eq!(count, 1, "\"the Desk\" must be unambiguous");

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_second_desk_is_refused_by_the_database() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    ProjectStore::new(pool.clone())
        .desk(user)
        .await
        .expect("desk");

    let second = sqlx::query(
        "INSERT INTO research_projects (project_id, user_id, kind, name, research_cutoff) \
         VALUES ($1,$2,'desk','Sneaky Desk',NULL)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(&pool)
    .await;

    assert!(second.is_err(), "the unique index must hold below the API");

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_research_project_gets_a_cutoff_even_when_none_is_asked_for() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());

    let project = store
        .create_research(user, "Default holdout", None, &[], None, None)
        .await
        .expect("create");

    let cutoff = project
        .research_cutoff
        .expect("a research project has a cutoff");
    let age = Utc::now() - cutoff;
    assert!(
        age >= Duration::days(89) && age <= Duration::days(91),
        "the default holdout should be ~90 days, got {age}"
    );

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_cutoff_in_the_future_is_refused() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());

    // A cutoff at or after now leaves nothing held out, so every result would be
    // in-sample while still passing through the machinery that says otherwise.
    let outcome = store
        .create_research(
            user,
            "No holdout",
            None,
            &[],
            Some(Utc::now() + Duration::days(1)),
            None,
        )
        .await;
    assert!(matches!(outcome, Err(ProjectError::Invalid(_))));

    cleanup(&pool, user).await;
}

/// DA-05, and the reason the whole holdout means anything.
#[tokio::test]
async fn a_cutoff_cannot_move_once_the_project_has_an_experiment() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());
    let project = store
        .create_research(user, "Immutable", None, &[], None, None)
        .await
        .expect("create");

    // Before any experiment, the cutoff may still be adjusted.
    let early = sqlx::query(
        "UPDATE research_projects SET research_cutoff = research_cutoff - interval '1 day' \
         WHERE project_id=$1",
    )
    .bind(project.project_id)
    .execute(&pool)
    .await;
    assert!(
        early.is_ok(),
        "no experiment yet, so the cutoff is still soft"
    );

    // An experiment exists the moment a trial-counted job is recorded against it.
    sqlx::query(
        "INSERT INTO jobs (job_id, kind, project_id, user_id, submitted_by, experiment_id, \
                           queue, worker_class, manifest, manifest_hash, state) \
         VALUES ($1,'backtest',$2,$3,'agent','exp_x','agent','backtest','{}'::jsonb,$4,'queued')",
    )
    .bind(format!("job_test_{}", Uuid::new_v4().simple()))
    .bind(project.project_id)
    .bind(user)
    .bind(Uuid::new_v4().to_string())
    .execute(&pool)
    .await
    .expect("record an experiment job");

    let moved = sqlx::query(
        "UPDATE research_projects SET research_cutoff = now() - interval '1 day' \
         WHERE project_id=$1",
    )
    .bind(project.project_id)
    .execute(&pool)
    .await;

    assert!(
        moved.is_err(),
        "a researcher who disliked a result could otherwise move the cutoff forward, \
         re-run, and present the second answer as the first — with nothing in the data \
         showing it happened"
    );

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_project_cannot_change_kind() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());
    let project = store
        .create_research(user, "Fixed kind", None, &[], None, None)
        .await
        .expect("create");

    // Flipping a research project to a desk would drop its cutoff and hand over the
    // holdout in one statement.
    let flipped = sqlx::query("UPDATE research_projects SET kind='desk' WHERE project_id=$1")
        .bind(project.project_id)
        .execute(&pool)
        .await;
    assert!(flipped.is_err());

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_research_project_without_a_cutoff_cannot_exist() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;

    let bad = sqlx::query(
        "INSERT INTO research_projects (project_id, user_id, kind, name, research_cutoff) \
         VALUES ($1,$2,'research','No cutoff',NULL)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(&pool)
    .await;
    assert!(
        bad.is_err(),
        "a research project with no holdout is meaningless"
    );

    // And the mirror image: a Desk with a cutoff could not answer a live question.
    let also_bad = sqlx::query(
        "INSERT INTO research_projects (project_id, user_id, kind, name, research_cutoff) \
         VALUES ($1,$2,'desk','Desk with cutoff', now() - interval '1 day')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(&pool)
    .await;
    assert!(also_bad.is_err());

    cleanup(&pool, user).await;
}
