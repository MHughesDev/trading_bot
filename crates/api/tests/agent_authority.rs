//! What an agent session may and may not do (AGENT-001 §6, ADR-0025, D-12).
//!
//! The agent runs with bash and code in a sandbox. It can compute anything on any
//! data it can reach and can ignore any instruction. The only thing standing between
//! it and the rest of the platform is the scope set on its token — so these tests
//! assert the boundary from both directions: that a minted token carries exactly the
//! research scopes, and that the database refuses the dangerous ones outright.
//!
//! ```bash
//! API_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p api --test agent_authority -- --test-threads=1
//! ```

use sqlx::{PgPool, Row};
use uuid::Uuid;

use api::auth::scopes;
use api::orchestrator::SessionOrchestrator;
use api::projects::ProjectStore;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("API_TEST_DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

async fn make_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("auth-test-{id}@local"))
        .execute(pool)
        .await
        .expect("create user");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(user)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE user_id=$1")
        .bind(user)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn a_minted_session_token_carries_exactly_the_research_scopes() {
    let Some(pool) = pool().await else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let user = make_user(&pool).await;
    let project = ProjectStore::new(pool.clone())
        .create_research(user, "Authority", None, &[], None, None)
        .await
        .expect("project");

    let orchestrator = SessionOrchestrator::new(pool.clone());
    let minted = orchestrator
        .mint_token(user, project.project_id)
        .await
        .expect("mint");

    let mut granted = minted.scopes.clone();
    granted.sort();
    let mut expected: Vec<String> = scopes::RESEARCH_SCOPES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.sort();
    assert_eq!(granted, expected);

    let stored: Vec<String> = sqlx::query("SELECT scopes FROM sessions WHERE token=$1")
        .bind(&minted.token)
        .fetch_one(&pool)
        .await
        .expect("stored session")
        .get("scopes");
    assert_eq!(stored.len(), scopes::RESEARCH_SCOPES.len());

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_minted_token_can_reach_nothing_dangerous() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let project = ProjectStore::new(pool.clone())
        .create_research(user, "Authority 2", None, &[], None, None)
        .await
        .expect("project");
    let minted = SessionOrchestrator::new(pool.clone())
        .mint_token(user, project.project_id)
        .await
        .expect("mint");

    // The list that matters (D-12): research authority only.
    for capability in [
        "data.holdout",
        "orders.place",
        "orders.cancel",
        "automations.arm",
        "models.promote",
        "skills.admit",
        "web:full",
    ] {
        assert!(
            !scopes::permits(&minted.scopes, capability),
            "a research session must not reach {capability}"
        );
    }

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn the_database_refuses_a_dangerous_scope_on_a_project_bound_session() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let project = ProjectStore::new(pool.clone())
        .create_research(user, "Authority 3", None, &[], None, None)
        .await
        .expect("project");

    // Straight past the orchestrator, as a future bug or a stray script would.
    for scope in ["data.holdout", "orders.place", "skills.admit"] {
        let attempt = sqlx::query(
            "INSERT INTO sessions (token, user_id, kind, scopes, project_id) \
             VALUES ($1,$2,'service',$3,$4)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user)
        .bind(vec![scope.to_string()])
        .bind(project.project_id)
        .execute(&pool)
        .await;

        assert!(
            attempt.is_err(),
            "the database must refuse {scope} on a project-bound session, because the \
             orchestrator is not the only thing that can write this table"
        );
    }

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn revoking_a_token_makes_it_unusable() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let project = ProjectStore::new(pool.clone())
        .create_research(user, "Authority 4", None, &[], None, None)
        .await
        .expect("project");
    let orchestrator = SessionOrchestrator::new(pool.clone());
    let minted = orchestrator
        .mint_token(user, project.project_id)
        .await
        .expect("mint");

    orchestrator
        .revoke_token(&minted.token)
        .await
        .expect("revoke");

    let remaining: i64 = sqlx::query("SELECT count(*) AS n FROM sessions WHERE token=$1")
        .bind(&minted.token)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(
        remaining, 0,
        "a container that outlives its session must not keep working"
    );

    cleanup(&pool, user).await;
}

#[tokio::test]
async fn a_session_token_is_bound_to_one_project() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let store = ProjectStore::new(pool.clone());
    let first = store
        .create_research(user, "Project A", None, &[], None, None)
        .await
        .expect("a");
    let second = store
        .create_research(user, "Project B", None, &[], None, None)
        .await
        .expect("b");

    let minted = SessionOrchestrator::new(pool.clone())
        .mint_token(user, first.project_id)
        .await
        .expect("mint");

    let bound: Uuid = sqlx::query("SELECT project_id FROM sessions WHERE token=$1")
        .bind(&minted.token)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("project_id");

    assert_eq!(bound, first.project_id);
    assert_ne!(
        bound, second.project_id,
        "one token, one project — otherwise 'may not read past the cutoff' has no \
         single answer"
    );

    cleanup(&pool, user).await;
}
