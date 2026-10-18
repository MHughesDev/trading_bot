//! Live-Postgres tests for immutable strategy revisions (migration 0058).
//!
//! The defect these close is quiet: a re-save used to replace the document every
//! result pointed at, so `RunConfig.strategy_version` identified a slot rather
//! than a program and two runs sharing one were only comparable if nobody had
//! saved in between. Nothing checked that, and nothing recorded it.
//!
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p ledger --test pg_strategy_revision -- --ignored --test-threads=1
//! ```
//!
//! ## Why every test here runs inside a transaction it never commits
//!
//! `DATABASE_URL` is the platform's *own* development database. An earlier
//! version of this file wrote through the pool and cleaned up nothing, and the
//! rows it left were not inert: 40 of the 54 rows in `strategy_revision` are its
//! fixtures, and two separate measurements of "how much of the agent's authoring
//! history has lineage" were mostly counting them before anyone noticed.
//!
//! A `DELETE` teardown cannot fix that, and not for want of discipline:
//! `strategy_revision` carries `trg_strategy_revision_immutable`, so a `DELETE`
//! is *refused by the database* — which is the property
//! [`a_revision_cannot_be_edited_or_deleted`] exists to assert. There is no
//! teardown to write. The only way to write to an append-only table and leave
//! nothing behind is to not commit.
//!
//! So [`begin`] hands each test a [`Transaction`] and no test commits one.
//! Dropping it rolls back, and `Transaction`'s own `Drop` runs on the unwind
//! path, on the early-return path, and on the path where the process is killed
//! outright (the server rolls back when the connection closes). That is stronger
//! than a hand-written `Drop` guard, which can only delete what the database
//! will let it delete — here, nothing.
//! [`the_suite_gives_the_database_back_unchanged`] proves it against a real
//! panic rather than asserting it in a comment.

use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

// ── markers ──────────────────────────────────────────────────────────────────

/// Stamped on every revision this file writes.
///
/// Rollback means nothing *should* survive to carry it, so this is the second
/// line and not the first. It matters because the first line can be removed by
/// one careless edit — a test that goes back to writing through the pool leaves
/// rows again, and the question then is whether the next person counting the
/// registry can tell.
///
/// The production writer (`crates/api/src/routes/strategies.rs`) binds
/// `token.user_id().to_string()` here, so a real `actor_id` is always a
/// hyphenated UUID and this value is one the platform has no code path to
/// write. `actor_kind` would be the more natural home for it, but its CHECK is
/// closed over `human|agent|scheduler|import` and widening it needs a migration
/// — see the note at the bottom of this file.
const TEST_ACTOR_ID: &str = "test-fixture:ledger/pg_strategy_revision";

/// Prefix for every `strategy_id` minted here. The residue this file used to
/// leave was named `s_<uuid>`, which is exactly what an agent-authored slug
/// looks like; that is why it was counted as one.
const TEST_STRATEGY_PREFIX: &str = "test-fixture_s_";

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    PgPool::connect(&url).await.expect("connect")
}

/// A pool and a transaction that must never be committed.
///
/// The pool is returned rather than dropped so the caller holds it for the life
/// of the test: the transaction owns its connection independently, but keeping
/// the handle makes the ownership obvious to the next reader.
async fn begin() -> (PgPool, Transaction<'static, Postgres>) {
    let pool = pool().await;
    let tx = pool.begin().await.expect("begin");
    (pool, tx)
}

/// Postgres aborts the *whole* transaction on a failed statement. Every
/// assertion in this file is about a statement the database refuses, so without
/// a savepoint the second refusal in a test reports `current transaction is
/// aborted` instead of the constraint it meant to check — a test that would
/// pass on `is_err()` while checking nothing.
async fn probe(conn: &mut PgConnection) {
    sqlx::query("SAVEPOINT probe")
        .execute(conn)
        .await
        .expect("savepoint");
}

async fn undo(conn: &mut PgConnection) {
    sqlx::query("ROLLBACK TO SAVEPOINT probe")
        .execute(conn)
        .await
        .expect("rollback to savepoint");
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
    format!("{TEST_STRATEGY_PREFIX}{}", Uuid::new_v4().simple())
}

async fn insert_revision(
    conn: &mut PgConnection,
    revision: &str,
    strategy: &str,
    parent: Option<&str>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO strategy_revision (
             revision_hash, strategy_id, parent_hash, definition_version, asset_class,
             definition_json, definition_bytes, actor_kind, actor_id, seq, prev_hash, row_hash
         ) VALUES ($1,$2,$3,'1.3','crypto_spot_cex','{}'::jsonb,'\\x00'::bytea,'agent',$4,
                   0,'\\x'::bytea,'\\x'::bytea)",
    )
    .bind(revision)
    .bind(strategy)
    .bind(parent)
    .bind(TEST_ACTOR_ID)
    .execute(conn)
    .await
}

// ── shape ────────────────────────────────────────────────────────────────────

/// The identity is the content address, and a malformed one is refused by the
/// database as well as by `RevisionHash::parse`.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_hash_must_be_sixty_four_lowercase_hex_characters() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();

    for bad in [
        "not-a-hash",
        &"A".repeat(64),
        &"a".repeat(63),
        &"g".repeat(64),
    ] {
        probe(&mut tx).await;
        assert!(
            insert_revision(&mut tx, bad, &s, None).await.is_err(),
            "{bad:?} is not a revision hash and must be refused"
        );
        undo(&mut tx).await;
    }
    // Derived from `s`, which is a fresh UUID per run. A constant here would
    // collide with itself the moment two invocations overlapped; the rollback
    // means it would no longer collide *across* runs, but a live suite that
    // depends on its own rollback for uniqueness is one bad edit from a
    // permanent primary-key failure.
    assert!(insert_revision(&mut tx, &hash(&s), &s, None).await.is_ok());
}

/// A revision that is its own parent is a cycle of length one, and the only way
/// to write one is a bug in the hasher.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_its_own_parent() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    let h = hash(&s);
    assert!(insert_revision(&mut tx, &h, &s, Some(&h)).await.is_err());
}

// ── append-only ──────────────────────────────────────────────────────────────

/// The property the whole migration exists for — and the reason this file can
/// never clean up after itself with a `DELETE`.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_edited_or_deleted() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&mut tx, &h, &s, None)
        .await
        .expect("inserted");

    probe(&mut tx).await;
    let err = sqlx::query("UPDATE strategy_revision SET definition_json = '{\"x\":1}'::jsonb WHERE revision_hash = $1")
        .bind(&h)
        .execute(&mut *tx)
        .await
        .expect_err("editing a revision must be refused");
    assert!(err.to_string().to_lowercase().contains("append"), "{err}");
    undo(&mut tx).await;

    probe(&mut tx).await;
    assert!(
        sqlx::query("DELETE FROM strategy_revision WHERE revision_hash = $1")
            .bind(&h)
            .execute(&mut *tx)
            .await
            .is_err(),
        "deleting a revision must be refused — which is why this suite rolls back \
         rather than tearing down"
    );
    undo(&mut tx).await;
}

/// The chain is assigned by the trigger, not by the caller — every insert above
/// passes `seq = 0` and none of them ends up at 0 twice.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_chain_is_assigned_by_the_database_and_links_each_row_to_the_last() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    let (a, b) = (hash(&format!("{s}a")), hash(&format!("{s}b")));
    insert_revision(&mut tx, &a, &s, None).await.expect("a");
    insert_revision(&mut tx, &b, &s, Some(&a)).await.expect("b");

    let rows = sqlx::query(
        "SELECT revision_hash, seq, prev_hash, row_hash FROM strategy_revision
          WHERE strategy_id = $1 ORDER BY seq",
    )
    .bind(&s)
    .fetch_all(&mut *tx)
    .await
    .expect("rows");

    assert_eq!(rows.len(), 2);
    let seq_a: i64 = rows[0].get("seq");
    let seq_b: i64 = rows[1].get("seq");
    assert_eq!(
        seq_b,
        seq_a + 1,
        "the trigger assigns consecutive sequence numbers"
    );

    let row_a: Vec<u8> = rows[0].get("row_hash");
    let prev_b: Vec<u8> = rows[1].get("prev_hash");
    assert_eq!(
        prev_b, row_a,
        "each row's prev_hash is the previous row's row_hash"
    );
    assert_eq!(row_a.len(), 32, "sha256");
}

/// Re-saving an identical document is a no-op, because the content hash is the
/// identity. Anything else would fill the table with duplicates of one program.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_same_document_twice_is_one_revision() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&mut tx, &h, &s, None).await.expect("first");

    probe(&mut tx).await;
    assert!(
        insert_revision(&mut tx, &h, &s, None).await.is_err(),
        "a bare re-insert conflicts; the write path uses ON CONFLICT DO NOTHING"
    );
    undo(&mut tx).await;

    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM strategy_revision WHERE strategy_id = $1")
            .bind(&s)
            .fetch_one(&mut *tx)
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
    let (_pool, mut tx) = begin().await;
    let (mine, theirs) = (strategy_id(), strategy_id());
    let h = hash(&theirs);
    insert_revision(&mut tx, &h, &theirs, None)
        .await
        .expect("theirs");

    probe(&mut tx).await;
    let err = sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(&mine)
    .bind(&h)
    .execute(&mut *tx)
    .await
    .expect_err("a head belonging to another strategy must be refused");
    assert!(err.to_string().contains("belongs to strategy"), "{err}");
    undo(&mut tx).await;
}

/// The same rule for instances: an instance that names a revision it has no
/// relationship to is how a live position ends up unexplainable.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn an_instance_cannot_bind_to_another_strategys_revision() {
    let (_pool, mut tx) = begin().await;
    let (a, b) = (strategy_id(), strategy_id());
    let h = hash(&b);
    insert_revision(&mut tx, &h, &b, None)
        .await
        .expect("b's revision");

    // Each definition points at its own head. Since 0140 there is no other way
    // to write one — see `a_definition_with_no_head_is_refused`.
    let own_a = hash(&a);
    insert_revision(&mut tx, &own_a, &a, None)
        .await
        .expect("a's revision");
    for (s, head) in [(&a, &own_a), (&b, &h)] {
        sqlx::query(
            "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
             VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2) ON CONFLICT DO NOTHING",
        )
        .bind(s)
        .bind(head)
        .execute(&mut *tx)
        .await
        .expect("definition");
    }

    // The instance table references accounts and instruments, so this asserts
    // against the trigger by targeting it directly rather than by inserting a
    // full instance row: the point is which check fires first, and the trigger
    // is BEFORE INSERT.
    probe(&mut tx).await;
    let err = sqlx::query(
        "INSERT INTO strategy_instances (strategy_id, account_id, instrument_id, revision_hash)
         VALUES ($1, gen_random_uuid(), 'nonexistent', $2)",
    )
    .bind(&a)
    .bind(&h)
    .execute(&mut *tx)
    .await
    .expect_err("mismatched revision must be refused");
    assert!(
        err.to_string().contains("but revision"),
        "the revision/strategy trigger must fire before the foreign keys: {err}"
    );
    undo(&mut tx).await;
}

/// A head naming a revision that does not exist at all.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_head_revision_that_does_not_exist_is_refused() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    assert!(sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(&s)
    .bind(hash("nothing-points-here"))
    .execute(&mut *tx)
    .await
    .is_err());
}

/// A definition with no head revision is refused (migration 0140).
///
/// This assertion used to say the opposite, and the inversion is the point.
/// 0058 left `head_revision` nullable for two states — rows written before it,
/// and the window inside its own backfill — and an earlier version of *this
/// file* then used the hatch as an ordinary write path, which is how 18
/// definitions with no lineage, no document and no author came to sit in the
/// live registry alongside 14 real ones. 0140 removed them and closed the
/// column.
///
/// Kept as a test rather than deleted with the hatch: a `SET NOT NULL` is one
/// `ALTER` away from being reverted by somebody who reads it as ceremony, and
/// what it actually stops is a definition whose document cannot be
/// reconstructed from any immutable record.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_definition_with_no_head_is_refused() {
    let (_pool, mut tx) = begin().await;
    let s = strategy_id();
    let err = sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb)",
    )
    .bind(&s)
    .execute(&mut *tx)
    .await
    .expect_err("a definition that names no revision must be refused");
    assert!(
        err.to_string().contains("head_revision"),
        "the refusal must name the column, not just fail: {err}"
    );
}

// ── the suite's own footprint ────────────────────────────────────────────────

/// This suite writes to the platform's development database and must give it
/// back exactly as it found it — *including* when a test panics partway through.
///
/// The panic is real and deliberate: the work runs in a spawned task that
/// unwinds after a successful insert, so the assertion below is about the
/// unwind path and not the happy one. Cleanup that only runs on the success
/// path is precisely what produced the 40 stranded rows this file is named for,
/// and a version of this check that let the inner body finish normally would
/// pass against that bug.
///
/// A `DELETE` teardown could not stand in for this. `strategy_revision` refuses
/// `DELETE` outright, so there is no correct teardown to compare against — the
/// rollback is not the convenient option, it is the only one.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_suite_gives_the_database_back_unchanged() {
    async fn counts(pool: &PgPool) -> (i64, i64) {
        let revisions: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_revision")
            .fetch_one(pool)
            .await
            .expect("count revisions");
        let definitions: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_definitions")
            .fetch_one(pool)
            .await
            .expect("count definitions");
        (revisions, definitions)
    }

    let before_pool = pool().await;
    let before = counts(&before_pool).await;

    let spawned = before_pool.clone();
    let panicked = tokio::spawn(async move {
        let mut tx = spawned.begin().await.expect("begin");
        let s = strategy_id();
        let h = hash(&s);
        insert_revision(&mut tx, &h, &s, None).await.expect("a revision is written");
        sqlx::query(
            "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
             VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
        )
        .bind(&s)
        .bind(&h)
        .execute(&mut *tx)
        .await
        .expect("a definition is written");

        // Both rows exist right now, inside this transaction.
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_revision WHERE actor_id = $1")
            .bind(TEST_ACTOR_ID)
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        assert!(n >= 1, "the fixture really was written before the panic");

        panic!("deliberate: the transaction must roll back on the unwind path");
    })
    .await;
    assert!(panicked.is_err(), "the spawned task panicked, as intended");

    // Close the pool so every connection's rollback is flushed and the
    // connections are gone, then ask a fresh one. Counting through the same
    // pool would only prove the rows were invisible, not that they were undone.
    before_pool.close().await;
    let fresh = pool().await;
    let after = counts(&fresh).await;

    assert_eq!(
        before, after,
        "a panicking test left rows behind. `strategy_revision` refuses DELETE, so \
         anything committed here is permanent and every later count of the strategy \
         registry is wrong by that much."
    );

    // And nothing bearing this file's marker survived, on any path.
    let marked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM strategy_revision WHERE actor_id = $1")
            .bind(TEST_ACTOR_ID)
            .fetch_one(&fresh)
            .await
            .expect("count marked");
    assert_eq!(marked, 0, "a marked fixture row survived the rollback");
}

// ── what this file cannot fix, and what would ────────────────────────────────
//
// `actor_kind` is the column that ought to carry the marker: it already exists
// to say what sort of author wrote a revision, and `TEST_ACTOR_ID` is doing that
// job from `actor_id` only because `actor_kind`'s CHECK is closed over
// `human|agent|scheduler|import`. A fixture therefore has to claim `'agent'`,
// which is the single most expensive lie this table can tell — `actor_kind`
// 'agent' vs 'human' is spent downstream in the multiple-testing accounting, so
// a fixture counted as an agent draw inflates the recorded search width.
//
// Closing that needs a migration, which is out of scope here. Exactly what it
// must contain:
//
//   * widen the CHECK on `strategy_revision.actor_kind` to admit `'test'`;
//   * a CHECK that ties the two together in both directions, so neither can be
//     written without the other:
//       CHECK ((actor_kind = 'test') = (actor_id LIKE 'test-fixture:%'))
//     — this is what makes the marker unforgeable rather than conventional, and
//     it is also what stops a fixture ever being written as 'agent' again;
//   * no backfill. The 40 stranded rows predate the marker and cannot be
//     updated (the table refuses UPDATE as well as DELETE), so the constraint
//     has to be added `NOT VALID` or the migration will not apply.
//
// Until then every reader that counts this table should exclude
// `actor_id LIKE 'test-fixture:%'` as well as the 40 rows identified by
// `actor_id = 'a'`.
