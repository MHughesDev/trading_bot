//! The semantic-memory store refuses what it claims to (Set S task S-4.3).
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p invariants --test semantic_memory_refuses -- --ignored --test-threads=1
//! ```
//!
//! # Why this is separate from the schema-mirror test
//!
//! `schema_mirrors_memory_types.rs` reads the migration as text and asserts the
//! CHECK lists match the Rust enums. It needs no database, which is what makes
//! it run — but it can only prove the constraint is *written*, not that it
//! *fires*.
//!
//! S-4.3's acceptance is a claim about behaviour: an entry without provenance
//! **cannot be inserted**. That is a sentence about what the database does when
//! you try, and the only way to check it is to try. A `NOT NULL` that reads
//! correctly and sits on a nullable column would pass the text scan and fail
//! here, which is the whole point of having both.
//!
//! # Each case names the constraint it expects
//!
//! Not just "the insert failed". A row can fail for the wrong reason — a typo in
//! the fixture, a primary-key collision, a foreign key nobody meant — and an
//! assertion that only checks for failure passes on all of them while proving
//! nothing about the rule it is named for. So each case asserts the error
//! mentions the constraint by name.

use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A database holding the table under test.
///
/// **Not the whole migration chain**, and the reason is worth stating. The chain
/// stops at `0055_knowledge_plane.sql`, which does `CREATE EXTENSION vector` —
/// the deployment image is `pgvector/pgvector:pg16` for exactly that migration,
/// and a stock Postgres does not have it. So on a developer machine without
/// pgvector, `run_migrations` cannot reach 0122 at all, and a test that insisted
/// on it would simply never run.
///
/// "Every migration applies to an empty database" is `db_invariants`' claim, not
/// this file's. This file's claim is narrower and is the one S-4.3's acceptance
/// actually makes: that the store **refuses** what `Entry::validate` refuses.
/// Every constraint under test is over the shared columns; none touches
/// `embedding`. So the migration is applied on its own, and when pgvector is
/// missing the embedding column's type is substituted — loudly, in a printed
/// line, so nobody reads a pass here as a full-fidelity run.
async fn migrated() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS semantic_memory_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE semantic_memory_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "semantic_memory_test"))
        .await
        .unwrap();

    let has_vector = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0)
        > 0;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0122_semantic_memory.sql");
    let mut sql = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    if has_vector {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&pool)
            .await
            .expect("pgvector reported available and would not install");
    } else {
        println!(
            "NOTE: pgvector is not available here, so `embedding vector(768)` is applied as              `bytea` for this run. Every constraint under test is over the shared columns and              none touches `embedding` — but this is not a full-fidelity apply, and the              whole-chain check belongs to `db_invariants`."
        );
        sql = sql.replace("embedding       vector(768),", "embedding       bytea,");
        assert!(
            !sql.contains("vector(768)"),
            "the embedding substitution did not match, so this run would fail for a reason              that has nothing to do with the constraints"
        );
    }

    // Statement at a time: sqlx's simple protocol will not take the whole file
    // with its dollar-free CHECK bodies in one round trip on every server.
    pool.execute(sql.as_str())
        .await
        .expect("0122_semantic_memory.sql applies to an empty database");
    pool
}

/// The columns every case fills, so each fixture differs in exactly one way and
/// a refusal can only be about that way.
const COLUMNS: &str = "entry_id,user_id,kind,subject,claim,confidence,evidence_class,\
                       window_start,window_end,prov_run_id,prov_tool,prov_tool_version,\
                       prov_template_hash,created_by";

const USER: &str = "'00000000-0000-0000-0000-000000000001'";

fn values(id: &str, overrides: &[(&str, &str)]) -> String {
    let mut v: Vec<(&str, String)> = vec![
        ("entry_id", format!("'{id}'")),
        ("user_id", USER.into()),
        ("kind", "'finding'".into()),
        ("subject", "'BTC-USD'".into()),
        ("claim", "'a claim'".into()),
        ("confidence", "'established'".into()),
        ("evidence_class", "'direct_stat'".into()),
        ("window_start", "'2026-04-01'".into()),
        ("window_end", "'2026-06-30'".into()),
        ("prov_run_id", "'run_1'".into()),
        ("prov_tool", "'get_bars'".into()),
        ("prov_tool_version", "'1'".into()),
        ("prov_template_hash", "'abc'".into()),
        ("created_by", "'agent'".into()),
    ];
    for (col, val) in overrides {
        if let Some(slot) = v.iter_mut().find(|(c, _)| c == col) {
            slot.1 = (*val).to_string();
        }
    }
    v.into_iter()
        .map(|(_, val)| val)
        .collect::<Vec<_>>()
        .join(",")
}

async fn insert(pool: &PgPool, id: &str, overrides: &[(&str, &str)]) -> Result<(), String> {
    let sql = format!(
        "INSERT INTO agent_memory.entry ({COLUMNS}) VALUES ({})",
        values(id, overrides)
    );
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_store_refuses_what_entry_validate_refuses() {
    let pool = migrated().await;

    // The control. If this fails, every refusal below is meaningless — they
    // would all be failing on whatever is wrong with the fixture.
    insert(&pool, "ok_1", &[])
        .await
        .expect("a complete entry inserts, or nothing below proves anything");

    // Each case: (id, override, the constraint that must appear in the error).
    let cases: &[(&str, &[(&str, &str)], &str)] = &[
        // S-4.3's acceptance, stated exactly: no provenance, no row.
        ("no_run", &[("prov_run_id", "NULL")], "prov_run_id"),
        ("no_tool", &[("prov_tool", "NULL")], "prov_tool"),
        ("no_ver", &[("prov_tool_version", "NULL")], "prov_tool_version"),
        ("no_hash", &[("prov_template_hash", "NULL")], "prov_template_hash"),
        // All four, because `Provenance::is_complete` requires all four: a run
        // id with no tool is not provenance.
        ("blank_tool", &[("prov_tool", "'   '")], "entry_prov_tool_check"),
        ("blank_run", &[("prov_run_id", "''")], "entry_prov_run_id_check"),
        // `EntryError::ClaimTooLong` / `EmptyClaim`.
        ("long_claim", &[("claim", "repeat('x',301)")], "entry_claim_check"),
        ("blank_claim", &[("claim", "'  '")], "entry_claim_check"),
        // `EntryError::EmptySubject`.
        ("blank_subj", &[("subject", "''")], "entry_subject_check"),
        // The closed vocabularies.
        ("bad_kind", &[("kind", "'hunch'")], "entry_kind_check"),
        ("bad_conf", &[("confidence", "'certain'")], "entry_confidence_check"),
        (
            "bad_class",
            &[("evidence_class", "'vibes'")],
            "entry_evidence_class_check",
        ),
        // §2.3: a model's own summary is never established.
        (
            "ms_est",
            &[("evidence_class", "'model_summary'")],
            "a_model_summary_is_never_established",
        ),
        // A window that runs backwards is not a window.
        (
            "back_window",
            &[("window_start", "'2026-06-30'"), ("window_end", "'2026-04-01'")],
            "the_window_runs_forwards",
        ),
    ];

    for (id, overrides, expected) in cases {
        let err = insert(&pool, id, overrides)
            .await
            .expect_err(&format!("`{id}` must be refused, and it was accepted"));
        assert!(
            err.contains(expected),
            "`{id}` was refused, but not by `{expected}` — it failed with: {err}\n\nA row that \
             fails for the wrong reason passes an assertion that only checks for failure, and \
             proves nothing about the rule this case is named for."
        );
    }
}

/// A measurement is whole or absent, never half.
///
/// Separate because it is the one constraint over a *combination* of columns
/// rather than over one, so a per-column scan would not find it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_measurement_is_whole_or_absent() {
    let pool = migrated().await;

    let half = "INSERT INTO agent_memory.entry (entry_id,user_id,kind,subject,claim,confidence,\
                evidence_class,window_start,window_end,prov_run_id,prov_tool,prov_tool_version,\
                prov_template_hash,created_by,direction) VALUES ('half',"
        .to_string()
        + USER
        + ",'finding','BTC-USD','direction only','suggestive','derived','2026-04-01',\
           '2026-06-30','run_1','get_bars','1','abc','agent','positive')";
    let err = sqlx::query(&half)
        .execute(&pool)
        .await
        .expect_err("a direction with no value is not a measurement");
    assert!(
        err.to_string().contains("measurement_is_whole_or_absent"),
        "refused by the wrong constraint: {err}"
    );

    let whole = "INSERT INTO agent_memory.entry (entry_id,user_id,kind,subject,claim,confidence,\
                 evidence_class,window_start,window_end,prov_run_id,prov_tool,prov_tool_version,\
                 prov_template_hash,created_by,quantity,direction,value,se,n) VALUES ('whole',"
        .to_string()
        + USER
        + ",'finding','BTC-USD','sharpe is positive','suggestive','derived','2026-04-01',\
           '2026-06-30','run_1','get_bars','1','abc','agent','sharpe','positive',1.4,0.3,812)";
    sqlx::query(&whole)
        .execute(&pool)
        .await
        .expect("a whole measurement inserts");
}

/// Corroborations start at 1 — the run that wrote the entry — so a zero is a
/// bug rather than a fresh row (§2.5).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_entry_is_corroborated_at_least_once_by_the_run_that_wrote_it() {
    let pool = migrated().await;
    let sql = format!(
        "INSERT INTO agent_memory.entry ({COLUMNS},corroborations) VALUES ({},0)",
        values("zero", &[])
    );
    let err = sqlx::query(&sql)
        .execute(&pool)
        .await
        .expect_err("zero corroborations means nothing measured it, including the writer");
    assert!(
        err.to_string().contains("entry_corroborations_check"),
        "refused by the wrong constraint: {err}"
    );
}
