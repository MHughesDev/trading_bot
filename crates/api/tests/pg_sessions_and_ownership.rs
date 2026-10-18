//! Set E 1.1 / 1.2 / 1.4 against a real Postgres: hashed sessions with sliding
//! expiry and a sweep, owner-scoped automations, and venue credentials that are
//! encrypted at rest.
//!
//! Skips (loudly) when `API_TEST_DATABASE_URL` is unset. Needs migrations
//! through 0154:
//!
//! ```bash
//! API_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/scratch \
//!   cargo test -j 2 -p api --test pg_sessions_and_ownership -- --test-threads=1
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use api::auth::session;
use api::auth::venue_credentials::VenueCredentialStore;
use api::credentials::CredentialCrypto;
use chrono::Utc;
use sqlx::PgPool;
use storage::automation::{self, AutomationRow};
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("API_TEST_DATABASE_URL") else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping the live half");
        return None;
    };
    Some(PgPool::connect(&url).await.expect("connect"))
}

async fn make_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("pg-own-{id}@local"))
        .execute(pool)
        .await
        .expect("create user");
    id
}

async fn cleanup(pool: &PgPool, users: &[Uuid]) {
    for u in users {
        for sql in [
            "DELETE FROM sessions WHERE user_id=$1",
            "DELETE FROM automations WHERE user_id=$1",
            "DELETE FROM venue_credentials WHERE user_id=$1",
            "DELETE FROM users WHERE user_id=$1",
        ] {
            sqlx::query(sql).bind(u).execute(pool).await.ok();
        }
    }
}

async fn insert_session(pool: &PgPool, token: &str, user: Uuid, kind: &str, created_days_ago: i32, expires_in: &str) {
    let scopes = if kind == "web" { vec!["web:full".to_owned()] } else { vec![] };
    sqlx::query(&format!(
        "INSERT INTO sessions (token, user_id, kind, scopes, created_at, last_seen_at, expires_at) \
         VALUES ($1, $2, $3, $4, now() - make_interval(days => $5), \
                 now() - interval '2 hours', now() + interval '{expires_in}')"
    ))
    .bind(token)
    .bind(user)
    .bind(kind)
    .bind(scopes)
    .bind(created_days_ago)
    .execute(pool)
    .await
    .expect("insert session");
}

#[tokio::test]
async fn a_session_is_stored_as_a_digest_and_only_the_credential_resolves() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let raw = format!("raw-{}", Uuid::new_v4().simple());
    insert_session(&pool, &raw, user, "web", 0, "1 hour").await;

    let stored: String = sqlx::query_scalar("SELECT token FROM sessions WHERE user_id = $1")
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, raw, "the credential must not be what is stored");
    assert_eq!(stored.len(), 64);
    assert!(stored.chars().all(|c| c.is_ascii_hexdigit()));

    let ok = session::resolve(&pool, &raw).await.unwrap().expect("the credential resolves");
    assert_eq!(ok.user_id, user);
    assert_eq!(ok.key, stored, "the extractor carries the digest, never the credential");
    assert_eq!(ok.scopes, vec!["web:full".to_owned()]);

    // Presenting the stored digest as if it were the credential must not work:
    // a leaked database row is not a credential.
    assert!(session::resolve(&pool, &stored).await.unwrap().is_none());
    assert!(session::resolve(&pool, "").await.unwrap().is_none());
    assert!(session::resolve(&pool, "nonsense").await.unwrap().is_none());

    // And the key cannot be rewritten to put cleartext back.
    let rewrite = sqlx::query("UPDATE sessions SET token = $1 WHERE user_id = $2")
        .bind(&raw)
        .bind(user)
        .execute(&pool)
        .await;
    assert!(rewrite.is_err(), "sessions.token must be immutable");
    cleanup(&pool, &[user]).await;
}

#[tokio::test]
async fn an_expired_session_does_not_resolve_and_the_sweep_removes_it() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let raw = format!("old-{}", Uuid::new_v4().simple());
    insert_session(&pool, &raw, user, "web", 5, "-1 hour").await;
    assert!(session::resolve(&pool, &raw).await.unwrap().is_none());
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 1);
    let swept = session::sweep_expired(&pool).await.unwrap();
    assert!(swept >= 1);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, 0, "the sweep deletes expired rows");
    cleanup(&pool, &[user]).await;
}

#[tokio::test]
async fn a_web_session_slides_up_to_its_absolute_cap_and_a_service_session_never_does() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;

    // Used today, expiring in an hour: resolving it slides expiry to ~7 days.
    let web = format!("web-{}", Uuid::new_v4().simple());
    insert_session(&pool, &web, user, "web", 1, "1 hour").await;
    let s = session::resolve(&pool, &web).await.unwrap().unwrap();
    assert!(
        s.expires_at > Utc::now() + chrono::Duration::days(6),
        "sliding expiry did not move: {}",
        s.expires_at
    );

    // 29 days old: the slide is capped at created_at + 30 days (about 1 day out).
    let old = format!("cap-{}", Uuid::new_v4().simple());
    insert_session(&pool, &old, user, "web", 29, "1 hour").await;
    let s = session::resolve(&pool, &old).await.unwrap().unwrap();
    assert!(s.expires_at < Utc::now() + chrono::Duration::days(2), "{}", s.expires_at);
    assert!(s.expires_at > Utc::now() + chrono::Duration::hours(12), "{}", s.expires_at);

    // A service session keeps the lifetime it was minted with.
    let svc = format!("svc-{}", Uuid::new_v4().simple());
    insert_session(&pool, &svc, user, "service", 0, "1 hour").await;
    let s = session::resolve(&pool, &svc).await.unwrap().unwrap();
    assert!(s.expires_at < Utc::now() + chrono::Duration::hours(2), "{}", s.expires_at);
    assert!(session::touch_key(&pool, &s.key).await.unwrap().is_none(), "service sessions never slide");

    // The explicit touch (POST /auth/touch) slides a web session.
    let web_key: String = sqlx::query_scalar("SELECT token FROM sessions WHERE user_id=$1 AND kind='web' AND created_at > now() - interval '2 days'")
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(session::touch_key(&pool, &web_key).await.unwrap().is_some());
    cleanup(&pool, &[user]).await;
}

#[tokio::test]
async fn automations_are_visible_and_changeable_only_by_their_owner() {
    let Some(pool) = pool().await else { return };
    let (alice, bob) = (make_user(&pool).await, make_user(&pool).await);
    let id = Uuid::new_v4();
    automation::insert_automation(
        &pool,
        &AutomationRow {
            id,
            user_id: alice,
            kind: "single_instrument".into(),
            account_mode: "paper".into(),
            spec: serde_json::json!({}),
            armed: true,
            created_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    assert_eq!(automation::list_automations_for(&pool, alice).await.unwrap().len(), 1);
    assert!(automation::list_automations_for(&pool, bob).await.unwrap().is_empty());
    assert!(automation::get_automation_owned(&pool, bob, id).await.unwrap().is_none());
    assert!(!automation::disarm_automation_owned(&pool, bob, id).await.unwrap(), "bob cannot disarm alice's");
    assert!(!automation::delete_automation_owned(&pool, bob, id).await.unwrap(), "bob cannot delete alice's");
    let still = automation::get_automation_owned(&pool, alice, id).await.unwrap().unwrap();
    assert!(still.armed, "a stranger's disarm must not have taken effect");

    assert!(automation::disarm_automation_owned(&pool, alice, id).await.unwrap());
    assert!(automation::delete_automation_owned(&pool, alice, id).await.unwrap());

    // The nil owner cannot be written (migration 0151's CHECK).
    let nil = automation::insert_automation(
        &pool,
        &AutomationRow {
            id: Uuid::new_v4(),
            user_id: Uuid::nil(),
            kind: "single_instrument".into(),
            account_mode: "paper".into(),
            spec: serde_json::json!({}),
            armed: false,
            created_at: Utc::now(),
        },
    )
    .await;
    assert!(nil.is_err(), "a nil-owned automation must be refused by the database");
    cleanup(&pool, &[alice, bob]).await;
}

#[tokio::test]
async fn venue_credentials_are_encrypted_at_rest_scoped_to_their_owner_and_deletable() {
    let Some(pool) = pool().await else { return };
    let (alice, bob) = (make_user(&pool).await, make_user(&pool).await);
    let crypto = Arc::new(CredentialCrypto::new(vec![7u8; 32], 1).unwrap());
    let store = VenueCredentialStore::new(pool.clone(), Some(crypto));

    let mut fields = BTreeMap::new();
    fields.insert("api_key".to_owned(), "PK-supersecret-key-9999".to_owned());
    fields.insert("api_secret".to_owned(), "hunter2-hunter2".to_owned());
    store.save(alice, "alpaca", &fields).await.unwrap();

    let (ct, names, last4): (Vec<u8>, Vec<String>, Option<String>) = sqlx::query_as(
        "SELECT ciphertext, field_names, key_last4 FROM venue_credentials WHERE user_id=$1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !String::from_utf8_lossy(&ct).contains("supersecret"),
        "the ciphertext column must not hold the plaintext"
    );
    assert_eq!(names, vec!["api_key".to_owned(), "api_secret".to_owned()]);
    assert_eq!(last4.as_deref(), Some("9999"));

    let status = store.status(alice).await.unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].venue, "alpaca");
    assert!(!serde_json::to_string(&status).unwrap().contains("supersecret"));

    assert_eq!(store.load(alice, "alpaca").await.unwrap().as_ref(), Some(&fields));
    assert!(store.load(bob, "alpaca").await.unwrap().is_none(), "bob cannot read alice's");
    assert!(store.status(bob).await.unwrap().is_empty());
    assert!(!store.remove(bob, "alpaca").await.unwrap());
    assert!(store.remove(alice, "alpaca").await.unwrap());

    // With no KEK nothing is stored.
    let keyless = VenueCredentialStore::new(pool.clone(), None);
    assert!(keyless.save(alice, "alpaca", &fields).await.is_err());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM venue_credentials WHERE user_id=$1")
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    cleanup(&pool, &[alice, bob]).await;
}

/// The platform's own role can do all of the above (an owner-connected test
/// cannot see a missing grant): sessions, automations, venue credentials.
#[tokio::test]
async fn app_role_holds_the_grants_these_tables_need() {
    let Some(pool) = pool().await else { return };
    let user = make_user(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO sessions (token, user_id, kind, scopes) VALUES ('grant-check', $1, 'web', ARRAY['web:full'])")
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("app_role can insert sessions");
    sqlx::query("SELECT 1 FROM sessions WHERE token = session_token_hash('grant-check')")
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can execute session_token_hash and read sessions");
    sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE token = session_token_hash('grant-check')")
        .execute(&mut *tx)
        .await
        .expect("app_role can touch sessions");
    sqlx::query("DELETE FROM sessions WHERE token = session_token_hash('grant-check')")
        .execute(&mut *tx)
        .await
        .expect("app_role can delete sessions");
    sqlx::query("SELECT count(*) FROM venue_credentials").fetch_one(&mut *tx).await.expect("venue_credentials");
    sqlx::query("SELECT count(*) FROM automations").fetch_one(&mut *tx).await.expect("automations");
    sqlx::query("SELECT count(*) FROM pnl_lots").fetch_one(&mut *tx).await.expect("pnl_lots");
    tx.rollback().await.unwrap();
    cleanup(&pool, &[user]).await;
}
