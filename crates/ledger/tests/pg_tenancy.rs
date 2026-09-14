//! Grants and row-level security (migration 0044). Live Postgres; ignored by default:
//! `cargo test -p ledger --test pg_tenancy -- --ignored --test-threads=1`.

use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, Registration, TrialSubject};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

fn app_pw() -> String {
    std::env::var("PLATFORM_DB_APP_PASSWORD").unwrap_or_else(|_| "platform_app_dev".into())
}

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0044_BODY: &str = include_str!("../../../migrations/0044_roles_grants_rls.sql");

async fn scratch(name: &str) -> (PgPool, String) {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str()).await.unwrap();
    admin.execute(format!("CREATE DATABASE {name}").as_str()).await.unwrap();
    let url = with_db(&base_url(), name);
    let owner = PgPool::connect(&url).await.unwrap();
    // 0044 references public._sqlx_migrations; create it as sqlx would.
    owner.execute("CREATE TABLE IF NOT EXISTS public._sqlx_migrations (version BIGINT PRIMARY KEY)").await.unwrap();
    owner.execute(M0043).await.expect("0043");
    owner.execute(M0044_BODY).await.expect("0044");
    // platform_app is cluster-global: reuse the configured password so a running dev
    // platform is not locked out by the test.
    let app = storage::postgres::connect_app(&owner, &url, &app_pw()).await.expect("app login");
    owner.close().await;
    (app, url)
}

fn subject(n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({}),
        dataset_id: "d".into(),
        split_spec_id: None,
        code_hash: "c".into(),
        image_digest: "i".into(),
        seed_set: vec![1],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    }
}

/// AT-37: isolation holds for the runtime role, and transaction-local context does
/// not survive onto the next checkout of the same pooled connection.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn tenant_isolation_holds_across_pooled_connections() {
    let (app, url) = scratch("tenancy_pool").await;
    let single = PgPoolOptions::new().max_connections(1).connect(&storage::postgres::app_url(&url, "platform_app", &app_pw()).unwrap()).await.unwrap();
    let l = PgTrialLedger::new(single.clone());
    let a = DispatchContext::human("tenant-a", "x", 0.1).with_experiment("e");
    let b = DispatchContext::human("tenant-b", "y", 0.1).with_experiment("e");
    for n in 0..3 {
        let s = subject(n);
        l.register_async(&Registration::new(&a, &s, 0.5)).await.unwrap();
    }
    let s = subject(9);
    l.register_async(&Registration::new(&b, &s, 0.5)).await.unwrap();

    assert_eq!(l.trial_count_async("tenant-a").await.unwrap(), 3);
    assert_eq!(l.trial_count_async("tenant-b").await.unwrap(), 1);

    // Tenant B's context cannot see tenant A's rows even when asking for them.
    let mut tx = ledger::pg::tenant_tx(&single, "tenant-b").await.unwrap();
    let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE tenant_id = 'tenant-a'").fetch_one(&mut *tx).await.unwrap();
    assert_eq!(leaked, 0);
    tx.commit().await.unwrap();

    // Same physical connection, no context: nothing is visible and nothing writes.
    let ctx: Option<String> = sqlx::query_scalar("SELECT NULLIF(current_setting('app.tenant_id', true), '')").fetch_one(&single).await.unwrap();
    assert_eq!(ctx, None, "set_config(..., true) must not outlive its transaction");
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial").fetch_one(&single).await.unwrap();
    assert_eq!(visible, 0, "no tenant context ⇒ no rows");

    // Demonstrate the landmine the rule prevents: a bare session SET does leak.
    sqlx::query("SET app.tenant_id = 'tenant-a'").execute(&single).await.unwrap();
    let after_bare_set: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial").fetch_one(&single).await.unwrap();
    assert_eq!(after_bare_set, 3, "bare SET persists on the pooled connection — which is why the code never uses it");
    sqlx::query("RESET app.tenant_id").execute(&single).await.unwrap();

    // Cross-tenant write is refused by WITH CHECK.
    let mut tx = ledger::pg::tenant_tx(&single, "tenant-b").await.unwrap();
    let err = sqlx::query("INSERT INTO mlops.campaign_event (campaign_id, tenant_id, state) VALUES (gen_random_uuid(), 'tenant-a', 'define')")
        .execute(&mut *tx).await.unwrap_err();
    assert!(err.to_string().contains("row-level security") || err.to_string().contains("foreign key"), "{err}");
    drop(tx);
    single.close().await;
    app.close().await;
}

/// AT-20: the runtime role holds no UPDATE/DELETE on the ledger (grant layer, not
/// just the trigger).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn runtime_role_has_no_update_or_delete_on_the_ledger() {
    let (app, _url) = scratch("tenancy_grants").await;
    for table in ["trial", "trial_event", "decision", "audit_event", "campaign", "gate_profile", "sealed_holdout_call"] {
        for privilege in ["UPDATE", "DELETE", "TRUNCATE"] {
            let has: bool = sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, $2)")
                .bind(format!("mlops.{table}"))
                .bind(privilege)
                .fetch_one(&app)
                .await
                .unwrap();
            assert!(!has, "platform_app must not hold {privilege} on mlops.{table}");
        }
    }
    let insert_profile: bool = sqlx::query_scalar("SELECT has_table_privilege(current_user, 'mlops.gate_profile', 'INSERT')").fetch_one(&app).await.unwrap();
    assert!(!insert_profile, "gate thresholds are Tier C: no runtime writes (INV-23)");
    let err = sqlx::query("UPDATE mlops.gate_profile SET thresholds = '{}'").execute(&app).await.unwrap_err();
    assert!(err.to_string().contains("permission denied"), "{err}");
    app.close().await;
}

/// AT-38 and the FORCE landmine: every tenant table has FORCE RLS and exactly one
/// permissive policy (a second would widen access, since policies OR together),
/// and a non-superuser table owner is still isolated.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn force_rls_and_single_policy_per_table() {
    let (app, url) = scratch("tenancy_force").await;
    let audit: Vec<(String, bool, bool, i64)> = sqlx::query_as("SELECT table_name, rls_enabled, rls_forced, permissive_policies FROM mlops.rls_policy_audit")
        .fetch_all(&app).await.unwrap();
    assert!(audit.len() >= 11);
    for (t, enabled, forced, policies) in &audit {
        assert!(*enabled && *forced, "{t}: RLS must be enabled and FORCED");
        assert_eq!(*policies, 1, "{t}: exactly one permissive policy");
    }

    // A non-superuser OWNER: without FORCE it would bypass RLS.
    let owner = PgPool::connect(&url).await.unwrap();
    owner.execute("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='rls_owner_probe') THEN CREATE ROLE rls_owner_probe LOGIN PASSWORD 'probe' NOSUPERUSER NOBYPASSRLS; END IF; END $$").await.unwrap();
    owner.execute("GRANT USAGE ON SCHEMA mlops TO rls_owner_probe").await.unwrap();
    owner.execute("ALTER TABLE mlops.campaign_event OWNER TO rls_owner_probe").await.unwrap();
    owner.execute("ALTER TABLE mlops.campaign_event DROP CONSTRAINT campaign_event_campaign_id_fkey").await.unwrap();
    owner.execute("SELECT set_config('app.tenant_id','tenant-a',false)").await.unwrap();
    owner.close().await;
    let mut admin = PgConnection::connect(&url).await.unwrap();
    admin.execute("INSERT INTO mlops.campaign_event (campaign_id, tenant_id, state) VALUES (gen_random_uuid(), 'tenant-a', 'define')").await.unwrap();
    admin.close().await.ok();

    let probe = PgPool::connect(&storage::postgres::app_url(&url, "rls_owner_probe", "probe").unwrap()).await.unwrap();
    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.campaign_event").fetch_one(&probe).await.unwrap();
    assert_eq!(seen, 0, "FORCE ROW LEVEL SECURITY applies to the table owner");
    probe.close().await;
    app.close().await;
}
