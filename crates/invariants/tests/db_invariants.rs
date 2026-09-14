//! Schema invariants decided against the real catalog after every migration has run.
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p invariants --test db_invariants -- --ignored --test-threads=1
//! ```

use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

async fn migrated() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute("DROP DATABASE IF EXISTS invariants_test WITH (FORCE)").await.unwrap();
    admin.execute("CREATE DATABASE invariants_test").await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "invariants_test")).await.unwrap();
    storage::postgres::run_migrations(&pool).await.expect("every migration applies to an empty database");
    pool
}

async fn rows(pool: &PgPool, sql: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(sql).fetch_all(pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"))
}

const PLANE: &str = "('mlops','dataplane')";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn catalog_invariants() {
    let pool = migrated().await;

    // AT-05 ⛔ · INV-04: `symbol` is never part of a primary or foreign key.
    let keyed = rows(
        &pool,
        &format!(
            "SELECT DISTINCT tc.table_schema || '.' || tc.table_name || '.' || kcu.column_name
               FROM information_schema.table_constraints tc
               JOIN information_schema.key_column_usage kcu
                 ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
              WHERE tc.constraint_type IN ('PRIMARY KEY','FOREIGN KEY')
                AND tc.table_schema IN {PLANE}
                AND kcu.column_name ILIKE '%symbol%'"
        ),
    )
    .await;
    assert!(keyed.is_empty(), "symbol used as a key: {keyed:?}");

    // AT-03 ⛔ · INV-03: no adjusted-price column anywhere.
    let adjusted = rows(
        &pool,
        "SELECT table_schema || '.' || table_name || '.' || column_name FROM information_schema.columns
          WHERE table_schema NOT IN ('pg_catalog','information_schema')
            AND (column_name LIKE 'adj\\_%' OR column_name LIKE '%\\_adjusted' OR column_name = 'adjusted_close')",
    )
    .await;
    assert!(adjusted.is_empty(), "adjusted price columns: {adjusted:?}");

    // AT-04 ⛔ · INV-05: no binary float in the data plane's price-bearing columns.
    let floats = rows(
        &pool,
        "SELECT table_schema || '.' || table_name || '.' || column_name FROM information_schema.columns
          WHERE table_schema = 'dataplane' AND data_type IN ('real','double precision')
            AND column_name ~ '^(open|high|low|close|vwap|strike|rate|tick_size|lot_size|contract_size|tick_value|price_factor|volume_factor|cash_amount|.*price.*|.*_close)$'",
    )
    .await;
    assert!(floats.is_empty(), "float price columns: {floats:?}");
    let wrong_scale = rows(
        &pool,
        "SELECT table_schema || '.' || table_name || '.' || column_name FROM information_schema.columns
          WHERE table_schema = 'dataplane' AND data_type = 'numeric'
            AND (numeric_precision IS DISTINCT FROM 38 OR numeric_scale IS DISTINCT FROM 18)",
    )
    .await;
    assert!(wrong_scale.is_empty(), "numeric columns not (38,18): {wrong_scale:?}");

    // AT-08 · INV-07: no greek column in any table.
    let greeks = rows(
        &pool,
        "SELECT table_schema || '.' || table_name || '.' || column_name FROM information_schema.columns
          WHERE table_schema IN ('mlops','dataplane')
            AND column_name IN ('delta','gamma','vega','theta','rho','vanna','volga','charm')",
    )
    .await;
    assert!(greeks.is_empty(), "greek columns: {greeks:?}");

    // S-1 · the runtime role can bypass nothing.
    let (superuser, bypass): (bool, bool) =
        sqlx::query_as("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = 'platform_app'")
            .fetch_one(&pool)
            .await
            .expect("platform_app exists");
    assert!(!superuser && !bypass, "platform_app must not be superuser or BYPASSRLS");

    // S-2 / AT-37 · every tenant-scoped table forces row-level security.
    let unforced = rows(
        &pool,
        &format!(
            "SELECT n.nspname || '.' || c.relname FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               JOIN information_schema.columns col
                 ON col.table_schema = n.nspname AND col.table_name = c.relname AND col.column_name = 'tenant_id'
              WHERE c.relkind IN ('r','p') AND n.nspname IN {PLANE}
                AND NOT (c.relrowsecurity AND c.relforcerowsecurity)"
        ),
    )
    .await;
    assert!(unforced.is_empty(), "tenant tables without FORCE RLS: {unforced:?}");

    // INV-19 ⛔ · the runtime role cannot rewrite history.
    let mutable = rows(
        &pool,
        "SELECT table_schema || '.' || table_name || ':' || privilege_type FROM information_schema.role_table_grants
          WHERE grantee IN ('platform_app','app_role')
            AND table_schema = 'mlops'
            AND table_name IN ('trial','trial_event','decision','ledger_anchor','audit_event','campaign_event',
                               'sealed_holdout_call','sealed_holdout_attempt','model_promotion','internal_model_freeze',
                               'trial_return_series','ledger_verification')
            AND privilege_type IN ('UPDATE','DELETE','TRUNCATE')",
    )
    .await;
    assert!(mutable.is_empty(), "append-only tables grant mutation: {mutable:?}");

    // INV-23 · no DDL: the runtime role owns nothing and cannot create in the plane.
    let owned = rows(
        &pool,
        &format!(
            "SELECT n.nspname || '.' || c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname IN {PLANE} AND pg_get_userbyid(c.relowner) IN ('platform_app','app_role')"
        ),
    )
    .await;
    assert!(owned.is_empty(), "runtime role owns objects: {owned:?}");
    for schema in ["mlops", "dataplane"] {
        let can_create: bool = sqlx::query_scalar("SELECT has_schema_privilege('platform_app', $1, 'CREATE')")
            .bind(schema)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!can_create, "platform_app can CREATE in {schema}");
    }

    pool.close().await;
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.unwrap();
    admin.execute("DROP DATABASE IF EXISTS invariants_test WITH (FORCE)").await.unwrap();
}

/// AT-63 ⛔ · The platform seed is unreadable by the agent (SPEC §12.7,
/// ADR-P2-18).
///
/// §12.7's countermeasure against gate-hacking is that some of the randomness a
/// result depends on is held by the platform and never shown to the thing being
/// evaluated. A seed the agent can read is a seed the agent can search over,
/// and then the "platform-held" seed is just another parameter.
///
/// The mechanism is a **column grant**, checked by the database on every read,
/// rather than a field the API layer remembers to omit.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn at63_the_platform_seed_is_unreadable_by_the_agent() {
    let pool = migrated().await;

    // The agent may read the campaign; it may not read this column.
    let granted = rows(
        &pool,
        "SELECT column_name FROM information_schema.column_privileges
          WHERE grantee = 'agent_role' AND table_schema = 'mlops' AND table_name = 'campaign'
            AND privilege_type = 'SELECT'
          ORDER BY column_name",
    )
    .await;
    assert!(
        !granted.is_empty(),
        "the agent must still be able to read a campaign's declared facts"
    );
    assert!(
        !granted.iter().any(|c| c == "platform_seed"),
        "agent_role has SELECT on campaign.platform_seed: {granted:?}"
    );

    // And the refusal is the database's, not a convention: ask it directly.
    let can_read: bool = sqlx::query_scalar(
        "SELECT has_column_privilege('agent_role', 'mlops.campaign', 'platform_seed', 'SELECT')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!can_read, "the database would let agent_role read the platform seed");

    // No write, either — a seed the agent can set is a seed it has chosen.
    for privilege in ["INSERT", "UPDATE"] {
        let can_write: bool = sqlx::query_scalar(
            "SELECT has_column_privilege('agent_role', 'mlops.campaign', 'platform_seed', $1)",
        )
        .bind(privilege)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!can_write, "agent_role has {privilege} on campaign.platform_seed");
    }

    // AT-65's database half: the capital ladder refuses a raise under a profile
    // that does not authorise capital, whoever writes the row.
    let paper_authorises: bool =
        sqlx::query_scalar("SELECT authorises_capital FROM mlops.gate_profile WHERE profile_id = 'paper_v1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!paper_authorises, "paper_v1 must not authorise capital");

    let refused = sqlx::query(
        "INSERT INTO mlops.capital_authorisation
             (tenant_id, subject, profile_id, previous_fraction, allowed_fraction,
              direction, gates_passed, reason, decided_by)
         VALUES ('t','lineage-a','paper_v1', 0, 0.100, 'raise', 16, 'sixteen passes', 't')",
    )
    .execute(&pool)
    .await;
    assert!(refused.is_err(), "a paper_v1 pass must not be able to raise the capital ramp");

    pool.close().await;
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.unwrap();
    admin.execute("DROP DATABASE IF EXISTS invariants_test WITH (FORCE)").await.unwrap();
}

/// AT-42 ⛔ · Nothing that evaluates a strategy can read a smoothed regime
/// probability (SPEC §5.4, R-07, ADR-P3-02).
///
/// Smoothed probabilities are `P(state | all data)` — the answer computed with
/// hindsight. A strategy that sees one earns roughly 2.2× the Sharpe it will
/// earn live, and the inflation is invisible in the backtest because nothing
/// about the number looks wrong. The separation is a **schema with its own
/// grants**, not a column somebody has to remember not to select.
///
/// AT-44 ⛔ rides along: the recommender's raw point estimate is not granted to
/// anything outside it, and the view every other caller reads does not have the
/// column at all.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn at42_smoothed_regimes_are_unreachable_from_anything_that_evaluates() {
    let pool = migrated().await;

    for role in ["backtest_role", "agent_role", "internal_ml_role"] {
        let can_use: bool =
            sqlx::query_scalar("SELECT has_schema_privilege($1, 'regime_research', 'USAGE')")
                .bind(role)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!can_use, "{role} can reach into regime_research at all");

        let can_read: bool = sqlx::query_scalar(
            "SELECT has_table_privilege($1, 'regime_research.regime_state_smoothed', 'SELECT')",
        )
        .bind(role)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!can_read, "{role} can read smoothed regime probabilities (R-07)");

        // And the filtered state it *is* allowed to read really is reachable —
        // the separation has to leave the causal half usable or strategies just
        // stop asking about regimes.
        let filtered: bool = sqlx::query_scalar(
            "SELECT has_table_privilege($1, 'regime_causal.regime_state', 'SELECT')",
        )
        .bind(role)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(filtered, "{role} cannot read the filtered regime state either");
    }

    // AT-44 ⛔ — no raw point estimate leaves the recommender.
    for role in ["app_role", "agent_role"] {
        let can_read: bool = sqlx::query_scalar(
            "SELECT has_column_privilege($1, 'knowledge.recommendation', 'dr_estimate', 'SELECT')",
        )
        .bind(role)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!can_read, "{role} can read dr_estimate (R-06)");
    }
    let public_columns = rows(
        &pool,
        "SELECT column_name FROM information_schema.columns
          WHERE table_schema = 'knowledge' AND table_name = 'recommendation_public'
          ORDER BY column_name",
    )
    .await;
    assert!(
        !public_columns.iter().any(|c| c == "dr_estimate"),
        "the public recommendation view exposes the point estimate: {public_columns:?}"
    );
    assert!(
        public_columns.iter().any(|c| c == "returned_score"),
        "the public view must still carry the shrunk score"
    );

    // Memory without evidence is a rumour the platform will act on later.
    let no_evidence = sqlx::query(
        "INSERT INTO knowledge.insight (tenant_id, tier, scope, claim, evidence_trial_ids)
         VALUES ('t', 3, '{}'::jsonb, 'momentum works', ARRAY[]::uuid[])",
    )
    .execute(&pool)
    .await;
    assert!(no_evidence.is_err(), "an insight with no evidence must be refused");

    pool.close().await;
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.unwrap();
    admin.execute("DROP DATABASE IF EXISTS invariants_test WITH (FORCE)").await.unwrap();
}
