//! Live-Postgres tests for immutable strategy revisions (migration 0058).
//!
//! The defect these close is quiet: a re-save used to replace the document every
//! result pointed at, so `RunConfig.strategy_version` identified a slot rather
//! than a program and two runs sharing one were only comparable if nobody had
//! saved in between. Nothing checked that, and nothing recorded it.
//!
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -p ledger --test pg_strategy_revision -- --ignored --test-threads=1
//! ```

use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    PgPool::connect(&url).await.expect("connect")
}

/// A deterministic 64-hex string from a seed. FNV-1a over the whole seed rather
/// than the seed's own bytes: two seeds sharing a long prefix (which every
/// `strategy_id` here does) must not collide, and an earlier version of this
/// helper truncated at 32 bytes and did exactly that.
fn hash(seed: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for lane in 0..4u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ lane;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let _ = write!(out, "{h:016x}");
    }
    out
}

fn strategy_id() -> String {
    format!("s_{}", Uuid::new_v4().simple())
}

async fn insert_revision(
    pool: &PgPool,
    revision: &str,
    strategy: &str,
    parent: Option<&str>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO strategy_revision (
             revision_hash, strategy_id, parent_hash, definition_version, asset_class,
             definition_json, definition_bytes, actor_kind, actor_id, seq, prev_hash, row_hash
         ) VALUES ($1,$2,$3,'1.3','crypto_spot_cex','{}'::jsonb,'\\x00'::bytea,'agent','a',
                   0,'\\x'::bytea,'\\x'::bytea)",
    )
    .bind(revision)
    .bind(strategy)
    .bind(parent)
    .execute(pool)
    .await
}

// ── shape ────────────────────────────────────────────────────────────────────

/// The identity is the content address, and a malformed one is refused by the
/// database as well as by `RevisionHash::parse`.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_hash_must_be_sixty_four_lowercase_hex_characters() {
    let pool = pool().await;
    let s = strategy_id();

    for bad in ["not-a-hash", &"A".repeat(64), &"a".repeat(63), &"g".repeat(64)] {
        assert!(
            insert_revision(&pool, bad, &s, None).await.is_err(),
            "{bad:?} is not a revision hash and must be refused"
        );
    }
    // Derived from `s`, which is a fresh UUID per run. A constant here passes
    // the first time against a database and fails forever after on the
    // primary key — these suites are run repeatedly against the developer
    // database, so a non-idempotent live test is a test that breaks itself.
    assert!(insert_revision(&pool, &hash(&s), &s, None).await.is_ok());
}

/// A revision that is its own parent is a cycle of length one, and the only way
/// to write one is a bug in the hasher.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_its_own_parent() {
    let pool = pool().await;
    let s = strategy_id();
    let h = hash(&s);
    assert!(insert_revision(&pool, &h, &s, Some(&h)).await.is_err());
}

// ── append-only ──────────────────────────────────────────────────────────────

/// The property the whole migration exists for.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_edited_or_deleted() {
    let pool = pool().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&pool, &h, &s, None).await.expect("inserted");

    let err = sqlx::query("UPDATE strategy_revision SET definition_json = '{\"x\":1}'::jsonb WHERE revision_hash = $1")
        .bind(&h)
        .execute(&pool)
        .await
        .expect_err("editing a revision must be refused");
    assert!(err.to_string().to_lowercase().contains("append"), "{err}");

    assert!(
        sqlx::query("DELETE FROM strategy_revision WHERE revision_hash = $1")
            .bind(&h)
            .execute(&pool)
            .await
            .is_err(),
        "deleting a revision must be refused"
    );
}

/// The chain is assigned by the trigger, not by the caller — every insert above
/// passes `seq = 0` and none of them ends up at 0 twice.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_chain_is_assigned_by_the_database_and_links_each_row_to_the_last() {
    let pool = pool().await;
    let s = strategy_id();
    let (a, b) = (hash(&format!("{s}a")), hash(&format!("{s}b")));
    insert_revision(&pool, &a, &s, None).await.expect("a");
    insert_revision(&pool, &b, &s, Some(&a)).await.expect("b");

    let rows = sqlx::query(
        "SELECT revision_hash, seq, prev_hash, row_hash FROM strategy_revision
          WHERE strategy_id = $1 ORDER BY seq",
    )
    .bind(&s)
    .fetch_all(&pool)
    .await
    .expect("rows");

    assert_eq!(rows.len(), 2);
    let seq_a: i64 = rows[0].get("seq");
    let seq_b: i64 = rows[1].get("seq");
    assert_eq!(seq_b, seq_a + 1, "the trigger assigns consecutive sequence numbers");

    let row_a: Vec<u8> = rows[0].get("row_hash");
    let prev_b: Vec<u8> = rows[1].get("prev_hash");
    assert_eq!(prev_b, row_a, "each row's prev_hash is the previous row's row_hash");
    assert_eq!(row_a.len(), 32, "sha256");
}

/// Re-saving an identical document is a no-op, because the content hash is the
/// identity. Anything else would fill the table with duplicates of one program.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_same_document_twice_is_one_revision() {
    let pool = pool().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&pool, &h, &s, None).await.expect("first");
    assert!(
        insert_revision(&pool, &h, &s, None).await.is_err(),
        "a bare re-insert conflicts; the write path uses ON CONFLICT DO NOTHING"
    );

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_revision WHERE strategy_id = $1")
        .bind(&s)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(n, 1);
}

// ── the pointer ──────────────────────────────────────────────────────────────

/// A definition may not point at another strategy's revision. Without this the
/// binding would look perfectly well-formed while naming someone else's program.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_definition_cannot_point_at_another_strategys_revision() {
    let pool = pool().await;
    let (mine, theirs) = (strategy_id(), strategy_id());
    let h = hash(&theirs);
    insert_revision(&pool, &h, &theirs, None).await.expect("theirs");

    let err = sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(&mine)
    .bind(&h)
    .execute(&pool)
    .await
    .expect_err("a head belonging to another strategy must be refused");
    assert!(err.to_string().contains("belongs to strategy"), "{err}");
}

/// The same rule for instances: an instance that names a revision it has no
/// relationship to is how a live position ends up unexplainable.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn an_instance_cannot_bind_to_another_strategys_revision() {
    let pool = pool().await;
    let (a, b) = (strategy_id(), strategy_id());
    let h = hash(&b);
    insert_revision(&pool, &h, &b, None).await.expect("b's revision");

    for s in [&a, &b] {
        sqlx::query(
            "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json)
             VALUES ($1, 'crypto_spot_cex', '{}'::jsonb) ON CONFLICT DO NOTHING",
        )
        .bind(s)
        .execute(&pool)
        .await
        .expect("definition");
    }

    // The instance table references accounts and instruments, so this asserts
    // against the trigger by targeting it directly rather than by inserting a
    // full instance row: the point is which check fires first, and the trigger
    // is BEFORE INSERT.
    let err = sqlx::query(
        "INSERT INTO strategy_instances (strategy_id, account_id, instrument_id, revision_hash)
         VALUES ($1, gen_random_uuid(), 'nonexistent', $2)",
    )
    .bind(&a)
    .bind(&h)
    .execute(&pool)
    .await
    .expect_err("mismatched revision must be refused");
    assert!(
        err.to_string().contains("but revision"),
        "the revision/strategy trigger must fire before the foreign keys: {err}"
    );
}

/// A head naming a revision that does not exist at all.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_head_revision_that_does_not_exist_is_refused() {
    let pool = pool().await;
    let s = strategy_id();
    assert!(sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(&s)
    .bind(hash("nothing-points-here"))
    .execute(&pool)
    .await
    .is_err());
}

/// Rows predating 0058 keep a NULL head, and must still be writable — a
/// migration that breaks every existing definition is one that gets reverted.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_definition_with_no_head_is_still_accepted() {
    let pool = pool().await;
    let s = strategy_id();
    assert!(sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb)",
    )
    .bind(&s)
    .execute(&pool)
    .await
    .is_ok());
}
