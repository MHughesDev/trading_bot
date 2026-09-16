//! `MemoryStore` writes rows the schema accepts and reads back what it wrote.
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test memory_store_round_trip -- --ignored --test-threads=1
//! ```
//!
//! # Why this needs a database and the unit tests do not
//!
//! `memory_store`'s unit tests cover the enum round trips — every `Kind`,
//! `Confidence`, `EvidenceClass` and `Direction` survives its string — with no
//! database, because that is pure mapping.
//!
//! What they cannot cover is the part most likely to be wrong: `put` binds
//! **twenty-nine columns positionally**. A name misspelled, a pair transposed,
//! a type that does not match its column — none of that is visible to the
//! compiler, and all of it fails at the first real write. An integration test is
//! the only thing that reads that INSERT the way Postgres does.
//!
//! The round trip is the assertion rather than the insert, because a write that
//! lands in the wrong column still succeeds. Only reading it back catches
//! `subject` and `claim` swapped.

use harness::memory::{
    Confidence, DataWindow, Direction, Entry, EvidenceClass, Kind, Measurement, Provenance,
};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// See `crates/invariants/tests/semantic_memory_refuses.rs` for why this applies
/// one migration rather than the chain: `0055_knowledge_plane.sql` needs
/// pgvector, which a stock Postgres does not have, so the chain cannot reach
/// 0122 on a developer machine.
async fn store_db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS memory_store_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE memory_store_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "memory_store_test"))
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
    let mut sql = std::fs::read_to_string(&path).expect("the migration is readable");
    if has_vector {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&pool)
            .await
            .expect("pgvector installs");
    } else {
        println!("NOTE: pgvector absent; `embedding` applied as bytea. No column under test.");
        sql = sql.replace("embedding       vector(768),", "embedding       bytea,");
    }
    pool.execute(sql.as_str())
        .await
        .expect("the migration applies");

    // The negative-result constraints ride on the same table (S-4.4).
    let class_sql = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crates/<name> sits two below the root")
            .join("migrations/0125_negative_result_class.sql"),
    )
    .expect("0125 is readable");
    pool.execute(class_sql.as_str())
        .await
        .expect("0125_negative_result_class.sql applies");
    pool
}

fn full_entry(id: &str) -> Entry {
    let mut e = Entry::finding(
        id,
        "BTC-USD 1h",
        "the residual t-stat is 3.1 after Newey-West",
        DataWindow::new("2026-04-01", "2026-06-30"),
        Provenance::new("run_9", "run_backtest", "3", "hash_abc"),
    );
    e.measurement = Some(Measurement {
        quantity: "residual_t".into(),
        direction: Direction::Positive,
        value: 3.1,
        se: Some(0.4),
        n: Some(812),
    });
    e.confidence = Confidence::Suggestive;
    e.evidence_class = EvidenceClass::Derived;
    e.evidence_ids
        .insert("experiment_id".into(), "exp_42".into());
    e.valid_until = Some("2026-09-28".into());
    e.created_by = "agent:opus".into();
    e.conflicts_with = vec!["f_7".into(), "f_9".into()];
    e.corroborated_by = vec!["run_10".into()];
    e.corroborations = 2;
    e.stale = true;
    e.cited_by_chain = true;
    e.extra = serde_json::json!({ "failure_class": "weak_effect" });
    e
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn every_field_survives_the_round_trip() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let user = Uuid::new_v4();

    let written = full_entry("f_round");
    store.put(user, &written).await.expect("the entry stores");

    let back = store
        .by_kind(user, Kind::Finding, 10)
        .await
        .expect("the entry reads back");
    assert_eq!(back.len(), 1, "one written, one read");
    let r = &back[0];

    // Field by field rather than a whole-struct compare, so a failure names the
    // column. A positional INSERT that transposed two of twenty-nine would
    // otherwise report "entries differ" and leave the reader to diff them.
    assert_eq!(r.id, written.id, "entry_id");
    assert_eq!(r.kind, written.kind, "kind");
    assert_eq!(r.subject, written.subject, "subject");
    assert_eq!(r.claim, written.claim, "claim — transposed with subject?");
    assert_eq!(r.confidence, written.confidence, "confidence");
    assert_eq!(r.evidence_class, written.evidence_class, "evidence_class");
    assert_eq!(r.evidence_ids, written.evidence_ids, "evidence_ids");
    assert_eq!(r.data_window, written.data_window, "data_window");
    assert_eq!(r.valid_until, written.valid_until, "valid_until");
    assert_eq!(r.provenance, written.provenance, "provenance — all four parts");
    assert_eq!(r.created_by, written.created_by, "created_by");
    assert_eq!(r.conflicts_with, written.conflicts_with, "conflicts_with");
    assert_eq!(r.corroborated_by, written.corroborated_by, "corroborated_by");
    assert_eq!(r.corroborations, written.corroborations, "corroborations");
    assert_eq!(r.stale, written.stale, "stale");
    assert_eq!(r.cited_by_chain, written.cited_by_chain, "cited_by_chain");
    assert_eq!(r.extra, written.extra, "extra");

    let m = r.measurement.as_ref().expect("the measurement survives");
    let w = written.measurement.as_ref().unwrap();
    assert_eq!(m.quantity, w.quantity, "quantity");
    assert_eq!(m.direction, w.direction, "direction");
    assert!((m.value - w.value).abs() < 1e-12, "value");
    assert_eq!(m.se, w.se, "se");
    assert_eq!(m.n, w.n, "n");
}

/// An entry `Entry::validate` refuses never reaches the database.
///
/// The CHECK would refuse it too — that is asserted in
/// `semantic_memory_refuses.rs`. What this proves is that the caller gets
/// `EntryError::ClaimTooLong(301)` rather than a `sqlx::Error` naming
/// `entry_claim_check`, which is the difference between a message someone can
/// act on and a constraint name they have to go and look up.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_invalid_entry_is_refused_before_the_database_sees_it() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);

    let mut bad = full_entry("f_bad");
    bad.claim = "x".repeat(301);
    let err = store
        .put(Uuid::new_v4(), &bad)
        .await
        .expect_err("a 301-character claim is refused");
    let text = err.to_string();
    assert!(
        text.contains("301") && text.contains("300"),
        "the caller should be told the length and the limit, not handed a constraint name: \
         {text}"
    );
}

/// The store scopes to one user.
///
/// Memory is per-user and there is no cross-tenant read path. A missing
/// `user_id` in the WHERE clause is the kind of omission that works perfectly in
/// every single-user test.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_users_memory_is_not_another_users() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());

    store.put(a, &full_entry("f_a")).await.expect("stores");
    let seen = store
        .by_kind(b, Kind::Finding, 10)
        .await
        .expect("the query runs");
    assert!(
        seen.is_empty(),
        "user b read {} of user a's entries",
        seen.len()
    );

    let mine = store.for_subject(a, "BTC-USD 1h", 10).await.expect("runs");
    assert_eq!(mine.len(), 1, "and user a still sees their own");
    assert_eq!(mine[0].claim, full_entry("f_a").claim, "and it came back whole");
}

/// A user preference stores, lists and archives (S-4.6).
///
/// The acceptance's second half is "the user can delete a preference and the
/// next brief omits it". Archiving is what delete means here — §2.4 — so what
/// must be true is that the next *read* does not return it. A test that only
/// checked the row's `archived` flag would pass while the brief still showed it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_preference_stores_and_a_deleted_one_stops_being_returned() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();

    let today = chrono::Utc::now().date_naive().to_string();
    let mut pref = Entry::finding(
        "pref_1",
        "BTC-USD",
        "never trade through a weekend gap",
        DataWindow::new(today.clone(), today),
        Provenance::new("user", "ask_user", "1", "n/a"),
    );
    pref.kind = Kind::UserPreference;
    pref.confidence = Confidence::Established;
    pref.evidence_class = EvidenceClass::Human;
    pref.created_by = user.to_string();

    // `human` + `established` is the one combination that needs no verification
    // pass: the user is the authority on their own preferences. The CHECK that
    // forbids `model_summary` + `established` must not catch this.
    store.put(user, &pref).await.expect("a preference stores");

    let listed = store
        .by_kind(user, Kind::UserPreference, 10)
        .await
        .expect("preferences list");
    assert_eq!(listed.len(), 1, "one preference, listed");
    assert_eq!(listed[0].confidence, Confidence::Established);
    assert_eq!(listed[0].evidence_class, EvidenceClass::Human);

    // Delete is archive.
    sqlx::query("UPDATE agent_memory.entry SET archived = true WHERE entry_id = 'pref_1'")
        .execute(&pool)
        .await
        .expect("the archive lands");

    let after = store
        .by_kind(user, Kind::UserPreference, 10)
        .await
        .expect("preferences list again");
    assert!(
        after.is_empty(),
        "the preference was deleted and is still being returned, so the next brief would \
         still show it"
    );
}

/// An entry with no kind-specific fields stores.
///
/// `Entry::extra` is `#[serde(default)]`, so it arrives as JSON `null` unless a
/// caller sets it — and the column requires an object. Every
/// `Entry::finding(...)` therefore failed to insert until the store started
/// normalising it.
///
/// This is the regression test, and it is here rather than in the unit tests
/// because no unit test could have caught it: the round-trip fixture sets
/// `extra` explicitly and passed throughout. The defect was in the pairing of a
/// Rust default with a database CHECK, which only a real write exercises.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_entry_with_no_extra_fields_still_stores() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let user = Uuid::new_v4();

    let bare = Entry::finding(
        "f_bare",
        "ETH-USD",
        "nothing kind-specific about this one",
        DataWindow::new("2026-04-01", "2026-06-30"),
        Provenance::new("run_1", "get_bars", "1", "abc"),
    );
    assert!(
        !bare.extra.is_object(),
        "the fixture must exercise the default, or this test proves nothing"
    );

    store
        .put(user, &bare)
        .await
        .expect("an entry with no kind-specific fields stores");

    let back = store.by_kind(user, Kind::Finding, 10).await.expect("reads");
    assert_eq!(back.len(), 1);
    assert!(
        back[0].extra.is_object(),
        "and it reads back as an empty object rather than a null"
    );
}

/// A superseded profile is retained, not overwritten (S-4.7).
///
/// The acceptance's second clause, and the distinction `put` cannot make: `put`
/// is `ON CONFLICT DO UPDATE`, which is right for a correction and wrong for a
/// succession. A weekly profiler that overwrote would leave no way to ask what
/// the instrument looked like last month — and "the profile changed" is often
/// the finding.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_superseded_profile_is_retained_and_the_current_view_shows_one() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();

    let mut january = Entry::finding(
        "profile_jan",
        "BTC-USD",
        "1h coverage complete; spread 8bps; hill alpha 3.4",
        DataWindow::new("2026-01-01", "2026-01-31"),
        Provenance::new("profiler_1", "estimate_liquidity_cost", "1", "h1"),
    );
    january.kind = Kind::InstrumentProfile;
    store.put(user, &january).await.expect("january stores");

    let mut february = Entry::finding(
        "profile_feb",
        "BTC-USD",
        "1h coverage complete; spread 19bps; hill alpha 2.6",
        DataWindow::new("2026-02-01", "2026-02-28"),
        Provenance::new("profiler_2", "estimate_liquidity_cost", "1", "h1"),
    );
    february.kind = Kind::InstrumentProfile;

    store
        .supersede(user, "profile_jan", &february)
        .await
        .expect("february supersedes january");

    // Retained: the January row is still there, unchanged but for the pointer.
    let old = store
        .get(user, "profile_jan")
        .await
        .expect("the audit read runs")
        .expect("the superseded profile is retained, not overwritten");
    assert_eq!(
        old.claim, january.claim,
        "the old profile's content was overwritten rather than kept"
    );
    assert_eq!(
        old.superseded_by.as_deref(),
        Some("profile_feb"),
        "and it points forward to what replaced it"
    );

    // Shown: the current view has one profile, the new one. Retained and shown
    // are different things — a brief listing every profile the instrument ever
    // had is a brief nobody can read.
    let current = store
        .by_kind(user, Kind::InstrumentProfile, 10)
        .await
        .expect("the current view reads");
    assert_eq!(current.len(), 1, "the current view shows the succession twice");
    assert_eq!(current[0].id, "profile_feb");
    assert!(current[0].claim.contains("19bps"));
}

/// A failed succession leaves neither half.
///
/// Half of a succession is worse than neither: a new profile with the old one
/// still current shows two, and an old one marked superseded by a row that never
/// wrote shows none.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_succession_that_cannot_write_marks_nothing() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();

    let mut first = Entry::finding(
        "profile_a",
        "ETH-USD",
        "the first profile",
        DataWindow::new("2026-01-01", "2026-01-31"),
        Provenance::new("profiler_1", "estimate_liquidity_cost", "1", "h1"),
    );
    first.kind = Kind::InstrumentProfile;
    store.put(user, &first).await.expect("stores");

    // A replacement the database will refuse: `model_summary` may never be
    // `established`.
    let mut bad = Entry::finding(
        "profile_b",
        "ETH-USD",
        "a summary the model wrote about itself",
        DataWindow::new("2026-02-01", "2026-02-28"),
        Provenance::new("profiler_2", "estimate_liquidity_cost", "1", "h1"),
    );
    bad.kind = Kind::InstrumentProfile;
    bad.evidence_class = EvidenceClass::ModelSummary;
    bad.confidence = Confidence::Established;

    store
        .supersede(user, "profile_a", &bad)
        .await
        .expect_err("the replacement is refused");

    let still = store
        .get(user, "profile_a")
        .await
        .expect("reads")
        .expect("the original survives");
    assert!(
        still.superseded_by.is_none(),
        "the original was marked superseded by a row that never wrote, so the current view \
         now has no profile at all"
    );
    assert_eq!(
        store
            .by_kind(user, Kind::InstrumentProfile, 10)
            .await
            .expect("reads")
            .len(),
        1,
        "and the current view still shows exactly one"
    );
}

/// A negative result's class must be one the funnel produces (S-4.4).
///
/// This is the mechanism that stops `capacity_insufficient` being used to keep a
/// dead idea alive. `derive_negative_result` already makes the class unforgeable
/// in Rust — it has no class parameter — but that is a property of one function
/// and holds until the second writer. The CHECK is the same rule where a writer
/// cannot route around it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_negative_result_cannot_carry_a_class_the_funnel_never_produced() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let user = Uuid::new_v4();

    let mut base = Entry::finding(
        "neg_1",
        "ema_cross_v1",
        "refused at gate 3 — weak_effect (t = 1.1)",
        DataWindow::new("2026-04-01", "2026-06-30"),
        Provenance::new("run_9", "advance_funnel", "1", "h"),
    );
    base.kind = Kind::NegativeResult;

    // A real class stores.
    base.extra = serde_json::json!({ "failure_class": "weak_effect", "ticket_id": "t_1" });
    store.put(user, &base).await.expect("a real class stores");

    // An invented one does not, however plausible it reads.
    let mut invented = base.clone();
    invented.id = "neg_2".into();
    invented.extra =
        serde_json::json!({ "failure_class": "needs_more_capital", "ticket_id": "t_2" });
    let err = store
        .put(user, &invented)
        .await
        .expect_err("a class the funnel never produced is refused");
    assert!(
        err.to_string()
            .contains("a_failure_class_is_one_the_funnel_produces"),
        "refused by the wrong constraint: {err}"
    );

    // A negative result with no class at all is refused too — without this the
    // constraint above is satisfied vacuously by omitting the field.
    let mut classless = base.clone();
    classless.id = "neg_3".into();
    classless.extra = serde_json::json!({ "ticket_id": "t_3" });
    let err = store
        .put(user, &classless)
        .await
        .expect_err("a negative result with no class is refused");
    assert!(
        err.to_string()
            .contains("a_negative_result_carries_a_class"),
        "refused by the wrong constraint: {err}"
    );

    // And one with no ticket: a refusal nobody can trace to the gate that
    // issued it cannot be checked against its own retry condition.
    let mut ticketless = base.clone();
    ticketless.id = "neg_4".into();
    ticketless.extra = serde_json::json!({ "failure_class": "weak_effect" });
    let err = store
        .put(user, &ticketless)
        .await
        .expect_err("a negative result with no ticket is refused");
    assert!(
        err.to_string()
            .contains("a_negative_result_names_its_ticket"),
        "refused by the wrong constraint: {err}"
    );

    // The constraint says nothing about the other kinds: a finding carries no
    // `failure_class` and must never be asked about one.
    let plain = Entry::finding(
        "f_plain",
        "BTC-USD",
        "an ordinary finding with no class",
        DataWindow::new("2026-04-01", "2026-06-30"),
        Provenance::new("run_9", "get_bars", "1", "h"),
    );
    store
        .put(user, &plain)
        .await
        .expect("a finding is not a negative result and is not asked for a class");
}

/// A refused gate becomes a storable negative result (S-4.4).
///
/// The mapping is unit-tested; this is the half that needs a database — that the
/// derived entry satisfies every constraint 0125 adds. A mapping that produced a
/// class the CHECK rejects would pass its own tests and fail on the first real
/// refusal, which is the worst moment to find out.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn every_gate_refusal_produces_an_entry_the_store_accepts() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let user = Uuid::new_v4();

    let gates = [
        (backtest::gates::Gate::Integrity, vec!["cost.below_floor"]),
        (backtest::gates::Gate::Integrity, vec!["lookahead.higher_tf_open"]),
        (backtest::gates::Gate::SinglePath, vec![]),
        (backtest::gates::Gate::Robustness, vec![]),
        (backtest::gates::Gate::Significance, vec![]),
        (backtest::gates::Gate::Vault, vec![]),
    ];

    for (i, (gate, reasons)) in gates.iter().enumerate() {
        let verdict = backtest::gates::GateVerdict {
            gate: *gate,
            passed: false,
            summary: format!("{gate:?} refused: the evidence did not clear the bar"),
            evidence: vec![],
            profile_id: "strict_v1".into(),
            reasons: reasons.iter().map(|s| (*s).to_string()).collect(),
            at: chrono::Utc::now(),
        };
        let mut entry = api::negative_from_funnel::entry_for(
            &verdict,
            &format!("t_{i}"),
            "ema_cross_v1",
            DataWindow::new("2026-04-01", "2026-06-30"),
            "run_1",
        )
        .expect("a refused gate produces an entry");
        // Ids collide across the two Integrity cases, which share a gate.
        entry.id = format!("{}_{i}", entry.id);

        store.put(user, &entry).await.unwrap_or_else(|e| {
            panic!(
                "the entry derived for {gate:?} is refused by the store: {e}. The mapping and \
                 the CHECK disagree, and the first real refusal would have found out."
            )
        });
    }

    let stored = store
        .by_kind(user, Kind::NegativeResult, 20)
        .await
        .expect("reads back");
    assert_eq!(stored.len(), gates.len(), "one entry per refused gate");

    // Every one carries a class the funnel produces and the ticket it came from.
    for e in &stored {
        let class = e.extra["failure_class"]
            .as_str()
            .expect("every negative result carries a class");
        assert!(
            harness::memory::negative::FailureClass::all()
                .iter()
                .any(|c| c.as_str() == class),
            "`{class}` is not a class the funnel produces"
        );
        assert!(
            e.extra["ticket_id"].as_str().is_some_and(|t| !t.is_empty()),
            "every refusal names the ticket that issued it"
        );
    }

    // And the two integrity failures are told apart, which is the whole reason
    // `GateVerdict` carries typed reasons.
    let classes: Vec<&str> = stored
        .iter()
        .filter_map(|e| e.extra["failure_class"].as_str())
        .collect();
    assert!(
        classes.contains(&"costs_dominate") && classes.contains(&"leakage"),
        "a leak and a cost failure were given the same class, so one of them has the other's \
         retry condition: {classes:?}"
    );
}
