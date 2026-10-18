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
//!
//! `DATABASE_URL` names the database the platform serves from — there is no
//! second one — so every test here runs inside a transaction that is rolled
//! back, and nothing below ever commits. The version of this file that did
//! commit left 18 `strategy_definitions` rows with `definition_json = '{}'` and
//! a NULL head, all of them visible to `list_strategies` and none of them a
//! definition of anything; migration 0140 removes those and makes the shape
//! unwritable. See [`Scratch`] for why a `DELETE` teardown is not available.

use sqlx::postgres::PgArguments;
use sqlx::query::Query;
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// A transaction against the platform's own database that is never committed.
///
/// A teardown that deletes what each test minted is not available here:
/// `strategy_revision` is append-only by trigger, so a row written to it can be
/// removed only by never having committed it. That forces the transaction, and
/// the transaction then forces [`Scratch::refused`] — half the assertions in
/// this file are that a write is *rejected*, and Postgres aborts the enclosing
/// transaction at the first error.
struct Scratch {
    _pool: PgPool,
    tx: Transaction<'static, Postgres>,
}

impl Scratch {
    async fn open() -> Self {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = PgPool::connect(&url).await.expect("connect");
        let tx = pool.begin().await.expect("begin");
        Self { _pool: pool, tx }
    }

    fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Run a statement that must be refused, between a savepoint and a rollback
    /// to it, and hand back the error.
    ///
    /// Without the savepoint the first refusal would poison the transaction and
    /// every statement after it would fail with `current transaction is
    /// aborted` — a test that still goes red, for a reason that has nothing to
    /// do with what it was checking.
    async fn refused(&mut self, q: Query<'_, Postgres, PgArguments>) -> sqlx::Error {
        sqlx::query("SAVEPOINT probe")
            .execute(&mut *self.tx)
            .await
            .expect("savepoint");
        let outcome = q.execute(&mut *self.tx).await;
        sqlx::query("ROLLBACK TO SAVEPOINT probe")
            .execute(&mut *self.tx)
            .await
            .expect("rollback to savepoint");
        sqlx::query("RELEASE SAVEPOINT probe")
            .execute(&mut *self.tx)
            .await
            .expect("release savepoint");
        match outcome {
            Err(e) => e,
            Ok(_) => panic!("the statement was accepted and had to be refused"),
        }
    }

    /// Discard everything the test wrote.
    ///
    /// `Transaction`'s own `Drop` rolls back too, which is what covers a
    /// panicking test; this is the path a passing one takes, and it reports a
    /// failed rollback instead of swallowing it.
    async fn discard(self) {
        self.tx.rollback().await.expect("rollback");
    }
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

/// Fresh per test. The rollback is what makes these suites re-runnable now, but
/// a fresh id is still what keeps one test's rows out of another's `WHERE`
/// clause, and out of the way of whatever the database already holds.
fn strategy_id() -> String {
    format!("s_{}", Uuid::new_v4().simple())
}

fn insert_revision<'q>(
    revision: &'q str,
    strategy: &'q str,
    parent: Option<&'q str>,
) -> Query<'q, Postgres, PgArguments> {
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
}

fn insert_definition<'q>(
    strategy: &'q str,
    head: Option<&'q str>,
) -> Query<'q, Postgres, PgArguments> {
    sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
         VALUES ($1, 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(strategy)
    .bind(head)
}

// ── shape ────────────────────────────────────────────────────────────────────

/// The identity is the content address, and a malformed one is refused by the
/// database as well as by `RevisionHash::parse`.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_hash_must_be_sixty_four_lowercase_hex_characters() {
    let mut db = Scratch::open().await;
    let s = strategy_id();

    for bad in ["not-a-hash", &"A".repeat(64), &"a".repeat(63), &"g".repeat(64)] {
        db.refused(insert_revision(bad, &s, None)).await;
    }
    insert_revision(&hash(&s), &s, None)
        .execute(db.conn())
        .await
        .expect("a well-formed hash is accepted");

    db.discard().await;
}

/// A revision that is its own parent is a cycle of length one, and the only way
/// to write one is a bug in the hasher.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_its_own_parent() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let h = hash(&s);
    db.refused(insert_revision(&h, &s, Some(&h))).await;
    db.discard().await;
}

// ── append-only ──────────────────────────────────────────────────────────────

/// The property the whole migration exists for — and the reason the teardown
/// here is a rollback rather than a `DELETE`.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_revision_cannot_be_edited_or_deleted() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&h, &s, None)
        .execute(db.conn())
        .await
        .expect("inserted");

    let err = db
        .refused(
            sqlx::query(
                "UPDATE strategy_revision SET definition_json = '{\"x\":1}'::jsonb
                  WHERE revision_hash = $1",
            )
            .bind(&h),
        )
        .await;
    assert!(err.to_string().to_lowercase().contains("append"), "{err}");

    db.refused(sqlx::query("DELETE FROM strategy_revision WHERE revision_hash = $1").bind(&h))
        .await;

    db.discard().await;
}

/// The chain is assigned by the trigger, not by the caller — every insert above
/// passes `seq = 0` and none of them ends up at 0 twice.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_chain_is_assigned_by_the_database_and_links_each_row_to_the_last() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let (a, b) = (hash(&format!("{s}a")), hash(&format!("{s}b")));
    insert_revision(&a, &s, None).execute(db.conn()).await.expect("a");
    insert_revision(&b, &s, Some(&a)).execute(db.conn()).await.expect("b");

    let rows = sqlx::query(
        "SELECT revision_hash, seq, prev_hash, row_hash FROM strategy_revision
          WHERE strategy_id = $1 ORDER BY seq",
    )
    .bind(&s)
    .fetch_all(db.conn())
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

    db.discard().await;
}

/// Re-saving an identical document is a no-op, because the content hash is the
/// identity. Anything else would fill the table with duplicates of one program.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_same_document_twice_is_one_revision() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let h = hash(&s);
    insert_revision(&h, &s, None).execute(db.conn()).await.expect("first");
    db.refused(insert_revision(&h, &s, None)).await;

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_revision WHERE strategy_id = $1")
        .bind(&s)
        .fetch_one(db.conn())
        .await
        .expect("count");
    assert_eq!(
        n, 1,
        "a bare re-insert conflicts; the write path uses ON CONFLICT DO NOTHING"
    );

    db.discard().await;
}

// ── the pointer ──────────────────────────────────────────────────────────────

/// A definition may not point at another strategy's revision. Without this the
/// binding would look perfectly well-formed while naming someone else's program.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_definition_cannot_point_at_another_strategys_revision() {
    let mut db = Scratch::open().await;
    let (mine, theirs) = (strategy_id(), strategy_id());
    let h = hash(&theirs);
    insert_revision(&h, &theirs, None)
        .execute(db.conn())
        .await
        .expect("theirs");

    let err = db.refused(insert_definition(&mine, Some(&h))).await;
    assert!(err.to_string().contains("belongs to strategy"), "{err}");

    db.discard().await;
}

/// The same rule for instances: an instance that names a revision it has no
/// relationship to is how a live position ends up unexplainable.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn an_instance_cannot_bind_to_another_strategys_revision() {
    let mut db = Scratch::open().await;
    let (a, b) = (strategy_id(), strategy_id());
    let (ha, hb) = (hash(&a), hash(&b));

    for (s, h) in [(&a, &ha), (&b, &hb)] {
        insert_revision(h, s, None)
            .execute(db.conn())
            .await
            .expect("revision");
        insert_definition(s, Some(h))
            .execute(db.conn())
            .await
            .expect("definition");
    }

    // The instance table references accounts and instruments, so this asserts
    // against the trigger by targeting it directly rather than by inserting a
    // full instance row: the point is which check fires first, and the trigger
    // is BEFORE INSERT.
    let err = db
        .refused(
            sqlx::query(
                "INSERT INTO strategy_instances (strategy_id, account_id, instrument_id, revision_hash)
                 VALUES ($1, gen_random_uuid(), 'nonexistent', $2)",
            )
            .bind(&a)
            .bind(&hb),
        )
        .await;
    assert!(
        err.to_string().contains("but revision"),
        "the revision/strategy trigger must fire before the foreign keys: {err}"
    );

    db.discard().await;
}

/// A head naming a revision that does not exist at all.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_head_revision_that_does_not_exist_is_refused() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let missing = hash("nothing-points-here");
    db.refused(insert_definition(&s, Some(&missing))).await;
    db.discard().await;
}

/// 0058 left a NULL head legal for two reasons, and both are spent: rows that
/// predated it were backfilled, and the backfill's own intermediate state ended
/// when it committed. What the hatch was still being used for was this file,
/// which minted 18 definitions of nothing before it learned to roll back — so
/// 0140 closed it. A definition names the revision it serves or it is not a
/// definition, and a slug whose document is `{}` is one `list_strategies` hands
/// out and `get_strategy` then answers with a 500.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_definition_must_name_the_revision_it_serves() {
    let mut db = Scratch::open().await;
    let s = strategy_id();
    let h = hash(&s);

    let err = db.refused(insert_definition(&s, None)).await;
    assert!(
        err.to_string().contains("head_revision"),
        "a headless definition must be refused by name: {err}"
    );

    insert_revision(&h, &s, None)
        .execute(db.conn())
        .await
        .expect("revision");
    insert_definition(&s, Some(&h))
        .execute(db.conn())
        .await
        .expect("a definition with a head of its own is accepted");

    db.discard().await;
}
