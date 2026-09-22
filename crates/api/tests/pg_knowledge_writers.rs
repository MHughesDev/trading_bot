//! The knowledge plane's writers, against a live database (SPEC §5, checklist
//! 3.1/3.7, INV-01).
//!
//! These are the adversarial half. Every test here asks what happens when the
//! schedule goes wrong rather than when it goes right, because the ways this
//! particular mechanism fails are all invisible from the happy path: a second
//! embedding at the same knowledge time still looks like an embedding, and a
//! rebuild that lost half its facts still returns a report.
//!
//! Everything runs through the production path — submit, claim, execute — so a
//! property proved here is a property of the thing that actually runs, not of a
//! test harness that resembles it.
//!
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//! KNOWLEDGE_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo test -j 2 -p api --test pg_knowledge_writers -- --ignored --test-threads=1
//! ```
//!
//! `MODEL_TRAINER_URL` points the fingerprint tests at a real sidecar. Without
//! it they run against a stub that returns fixed vectors — which is the right
//! default, because what is under test here is the *writing*, and the
//! fingerprint's own arithmetic is covered by `apps/model-trainer/tests`.

use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Duration, TimeZone, Utc};
use jobs::{JobKind, JobState, JobStore, Submission, SubmittedBy, WorkerClass, WorkerPool};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use storage::identity::{IdentityResolver, MemoryIdentity, SourceInfo};

use api::instrument_profile::{
    InstrumentProfileWorker, ProfileManifest, ProfileTarget, EMBEDDING_VERSION_UNWHITENED,
    RETRIEVAL_DIMS,
};
use api::knowledge_jobs::TensorProjection;
use api::knowledge_scheduler::{self, TensorRebuildWorker};

async fn pg() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("postgres"))
}

fn identity() -> Arc<MemoryIdentity> {
    static ID: OnceLock<Arc<MemoryIdentity>> = OnceLock::new();
    ID.get_or_init(|| {
        let id = Arc::new(MemoryIdentity::new());
        id.add_source(
            "rest",
            SourceInfo {
                source_id: 2,
                declared_vendor_lag: std::time::Duration::from_secs(60),
                live: false,
            },
        );
        storage::identity::install(id.clone());
        id
    })
    .clone()
}

/// A throwaway ClickHouse database with the canonical schema and some bars.
///
/// Returns the url plus the `(instrument_id, venue_id)` the writer allocated,
/// because `load_bars_by_key` reads by surrogate key and the knowledge plane
/// stores the same keys — a test that guessed them would be testing its guess.
async fn ch_scratch(name: &str, symbols: &[&str]) -> Option<(String, Vec<(i64, i32)>)> {
    let base = std::env::var("KNOWLEDGE_E2E_CLICKHOUSE_URL")
        .or_else(|_| std::env::var("BACKTEST_E2E_CLICKHOUSE_URL"))
        .or_else(|_| std::env::var("CONSISTENCY_E2E_CLICKHOUSE_URL"))
        .ok()?;
    let admin = storage::clickhouse::connect(&base);
    admin
        .query(&format!("DROP DATABASE IF EXISTS {name}"))
        .execute()
        .await
        .expect("drop scratch");
    admin
        .query(&format!("CREATE DATABASE {name}"))
        .execute()
        .await
        .expect("create scratch");
    let (scheme, rest) = base.split_once("://").expect("a url");
    let host = rest.split('/').next().expect("a host");
    let url = format!("{scheme}://{host}/{name}");
    storage::clickhouse::migrate::run_migrations(&url)
        .await
        .expect("clickhouse migrations");

    let id = identity();
    let store = backtest::store::BarStore::connect(&url);
    let mut keys = Vec::new();
    for (n, symbol) in symbols.iter().enumerate() {
        store
            .write_collected(
                symbol,
                "knowledge_test",
                "rest",
                domain::payloads::bar::Timeframe::Hours1,
                &hourly_bars(300, n as i64),
            )
            .await
            .expect("bars written");
        let (i, v) = id
            .ensure_instrument(
                "knowledge_test",
                symbol,
                storage::identity::AssetClass::Crypto,
                Utc::now(),
            )
            .await
            .expect("identity");
        keys.push((i.0, v.0));
    }
    id.sync_dims(&url).await.expect("dims");
    Some((url, keys))
}

/// A deterministic price path with a per-instrument character, so the population
/// is not degenerate.
fn hourly_bars(n: i64, seed: i64) -> Vec<backtest::store::CollectedBar> {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().expect("an instant");
    (0..n)
        .map(|i| {
            let wave = (i as f64 * (0.13 + seed as f64 * 0.07)).sin() * 4.0;
            let price = 100.0 + wave + i as f64 * 0.01 * (seed as f64 + 1.0);
            let p = format!("{price:.4}");
            backtest::store::CollectedBar {
                available_time: start + Duration::hours(i),
                sequence: i as u64,
                open: p.clone(),
                high: format!("{:.4}", price + 0.5),
                low: format!("{:.4}", price - 0.5),
                close: p,
                volume: format!("{}", 1_000 + i),
                trade_count: 10,
            }
        })
        .collect()
}

/// A sidecar that answers `/fingerprint` with fixed, valid rows.
///
/// It is deliberately not a reimplementation of the fingerprint: it returns the
/// *shape* the writer must handle, including a `None` in `tier1` — because
/// "absent is not zero" is a property of the write path, and a stub that only
/// ever returned complete rows would let a writer that coerced `None` to `0.0`
/// pass.
async fn stub_sidecar() -> String {
    if let Ok(url) = std::env::var("MODEL_TRAINER_URL") {
        return url;
    }
    let app = axum::Router::new().route(
        "/fingerprint",
        axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
            let rows: Vec<Value> = body["instruments"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(n, i)| {
                    let mut vec = vec![0.0_f64; RETRIEVAL_DIMS];
                    vec[n % RETRIEVAL_DIMS] = 1.0;
                    json!({
                        "instrument_id": i["instrument_id"],
                        "venue_id": i["venue_id"],
                        // One hole on purpose: the writer must store SQL NULL.
                        "tier1": [0.1_f64 + n as f64, Value::Null, 0.3_f64],
                        "retrieval_vec": vec,
                        "absent": {"kyle_lambda": "no signed volume published"},
                        "quality_flags": 3,
                    })
                })
                .collect();
            axum::Json(json!({
                "instruments": rows,
                "whitened": false,
                "intrinsic_dim": Value::Null,
                "reason": "whitening needs at least 60 instruments, got 3",
                "dimensions": ["rv_1", "rv_5", "rv_21"],
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

// ── the production path, driven a step at a time ─────────────────────────────

/// A knowledge pool with both writers registered, exactly as the scheduler
/// builds it.
fn knowledge_pool(store: &Arc<JobStore>, pg: &PgPool, ch: &str, trainer: &str) -> WorkerPool {
    WorkerPool::new(Arc::clone(store), WorkerClass::Knowledge, 1)
        .register(Arc::new(InstrumentProfileWorker::with_trainer(
            pg.clone(),
            ch.to_string(),
            trainer.to_string(),
        )))
        .register(Arc::new(TensorRebuildWorker::new(pg.clone())))
}

async fn submit(store: &JobStore, kind: JobKind, manifest: Value) -> jobs::Submitted {
    let mut s = Submission::new(kind, knowledge_scheduler::SYSTEM_PRINCIPAL, manifest);
    s.submitted_by = SubmittedBy::System;
    store.submit(s).await.expect("submitted")
}

/// Claim the job we just submitted and run it the way the pool would.
async fn claim_and_run(pool: &WorkerPool, store: &JobStore, job_id: &str) -> jobs::Job {
    let job = store
        .claim(WorkerClass::Knowledge, "test-worker")
        .await
        .expect("claim")
        .expect("a queued knowledge job");
    assert_eq!(job.job_id, job_id, "claimed somebody else's job");
    pool.execute(job).await;
    store.get(job_id).await.expect("the settled job")
}

/// Clear anything the knowledge scheduler left behind, so a claim in this test
/// can only find this test's work.
async fn clear_jobs(pg: &PgPool) {
    let _ = sqlx::query("DELETE FROM jobs WHERE worker_class = 'knowledge'")
        .execute(pg)
        .await;
}

async fn clear_embeddings(pg: &PgPool, ids: &[i64]) {
    let _ = sqlx::query("DELETE FROM knowledge.asset_embedding WHERE instrument_id = ANY($1)")
        .bind(ids)
        .execute(pg)
        .await;
}

async fn clear_tensor(pg: &PgPool, tenant: &str) {
    let Ok(mut tx) = ledger::pg::tenant_tx(pg, tenant).await else {
        return;
    };
    let _ = sqlx::query("DELETE FROM knowledge.outcome_tensor_fact WHERE tenant_id = $1")
        .bind(tenant)
        .execute(&mut *tx)
        .await;
    let _ = tx.commit().await;
}

fn manifest(as_of: DateTime<Utc>, keys: &[(i64, i32)]) -> Value {
    ProfileManifest {
        as_of,
        period_secs: 3600,
        lookback_days: 400,
        window_spec: "400d@3600s".into(),
        instruments: keys
            .iter()
            .map(|(i, v)| ProfileTarget {
                instrument_id: *i,
                venue_id: *v,
                continuous_venue: true,
            })
            .collect(),
        deflate: true,
    }
    .to_value()
}

async fn embedding_count(pg: &PgPool, ids: &[i64]) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM knowledge.asset_embedding WHERE instrument_id = ANY($1)",
    )
    .bind(ids)
    .fetch_one(pg)
    .await
    .expect("count")
}

// ─────────────────────────────────────────────────────────────────────────────
// The fingerprint writer
// ─────────────────────────────────────────────────────────────────────────────

/// **The adversarial one.** Running the same profile job twice must leave one
/// embedding per instrument per knowledge time — not two.
///
/// Two rows for one instrument at one knowledge time is not a duplicate a reader
/// can spot and skip: `EmbeddingStore::neighbours` takes `DISTINCT ON
/// (instrument_id, venue_id) … ORDER BY knowledge_time DESC`, so with two rows
/// at the same knowledge time the winner is whichever the planner happened to
/// reach first. The neighbour list would be non-deterministic, and nothing about
/// the answer would say so.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres and ClickHouse"]
async fn running_a_profile_job_twice_writes_one_embedding_per_instrument() {
    let Some(pg) = pg().await else { return };
    let Some((ch, keys)) = ch_scratch("knowledge_writer_twice", &["KW-A", "KW-B", "KW-C"]).await
    else {
        pg.close().await;
        return;
    };
    let ids: Vec<i64> = keys.iter().map(|(i, _)| *i).collect();
    clear_jobs(&pg).await;
    clear_embeddings(&pg, &ids).await;

    let trainer = stub_sidecar().await;
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(jobs::RefuseTrials)));
    let pool = knowledge_pool(&store, &pg, &ch, &trainer);
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).single().expect("an instant");
    let m = manifest(as_of, &keys);

    let first = submit(&store, JobKind::InstrumentProfile, m.clone()).await;
    assert!(!first.deduplicated);
    let settled = claim_and_run(&pool, &store, &first.job_id).await;
    assert_eq!(settled.state, JobState::Succeeded, "{:?}", settled.error);
    let after_first = embedding_count(&pg, &ids).await;
    assert_eq!(
        after_first,
        keys.len() as i64,
        "one row per instrument, not {after_first}"
    );

    // Asking again inside the same knowledge day is the same job. The job
    // service will not run it twice, and if something does, the write is an
    // upsert — so this asserts both halves.
    let again = submit(&store, JobKind::InstrumentProfile, m.clone()).await;
    assert!(again.deduplicated, "the same manifest is the same job");
    assert_eq!(again.job_id, first.job_id);

    // Force the second pass anyway, which is what a re-queued lease does.
    sqlx::query("UPDATE jobs SET state='queued', lease_owner=NULL, lease_expires_at=NULL, finished_at=NULL WHERE job_id=$1")
        .bind(&first.job_id)
        .execute(&pg)
        .await
        .expect("re-queue");
    let settled = claim_and_run(&pool, &store, &first.job_id).await;
    assert_eq!(settled.state, JobState::Succeeded, "{:?}", settled.error);
    assert_eq!(
        embedding_count(&pg, &ids).await,
        after_first,
        "the same job run twice must upsert, not accumulate"
    );

    // The rows are stamped with the manifest's horizon, never the clock.
    let (kt, vf, version): (DateTime<Utc>, DateTime<Utc>, String) = sqlx::query_as(
        "SELECT knowledge_time, valid_from, embedding_version
           FROM knowledge.asset_embedding WHERE instrument_id = $1",
    )
    .bind(ids[0])
    .fetch_one(&pg)
    .await
    .expect("the row");
    assert_eq!(kt, as_of, "knowledge_time is the as_of, not now() (INV-01)");
    assert_eq!(vf, as_of);
    assert_eq!(
        version, EMBEDDING_VERSION_UNWHITENED,
        "three instruments cannot be whitened, and the version says so"
    );

    // Absent stayed absent: a NULL in tier1, never a zero.
    let tier1: Vec<Option<f32>> =
        sqlx::query_scalar("SELECT tier1 FROM knowledge.asset_embedding WHERE instrument_id = $1")
            .bind(ids[0])
            .fetch_one(&pg)
            .await
            .expect("tier1");
    assert!(
        tier1.iter().any(Option::is_none),
        "a dimension nothing measured must be SQL NULL, not 0.0: {tier1:?}"
    );

    // Nothing claims hubness correction: `neighbours` takes a plain cosine.
    let hubness: bool = sqlx::query_scalar(
        "SELECT hubness_corrected FROM knowledge.asset_embedding WHERE instrument_id = $1",
    )
    .bind(ids[0])
    .fetch_one(&pg)
    .await
    .expect("hubness");
    assert!(
        !hubness,
        "mutual proximity is not applied anywhere, so the column must not claim it"
    );

    clear_embeddings(&pg, &ids).await;
    clear_jobs(&pg).await;
    pg.close().await;
}

/// **The other adversarial one.** A worker killed mid-pass loses nothing and
/// doubles nothing.
///
/// The kill is real: the job is claimed, its lease is expired underneath it, and
/// the reaper re-queues it exactly as it would for a worker whose process
/// vanished. The partial state is real too — a pass that got as far as one
/// instrument and stopped leaves that instrument's row behind, and nothing rolls
/// it back. What must hold afterwards is that the job is still there to be
/// claimed with the manifest it had, that the recovered pass covers the whole
/// population, and that the instrument already written ends up with one row
/// rather than two.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres and ClickHouse"]
async fn a_killed_profile_run_loses_nothing_and_doubles_nothing() {
    let Some(pg) = pg().await else { return };
    let Some((ch, keys)) = ch_scratch("knowledge_writer_killed", &["KK-A", "KK-B"]).await else {
        pg.close().await;
        return;
    };
    let ids: Vec<i64> = keys.iter().map(|(i, _)| *i).collect();
    clear_jobs(&pg).await;
    clear_embeddings(&pg, &ids).await;

    let trainer = stub_sidecar().await;
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(jobs::RefuseTrials)));
    let pool = knowledge_pool(&store, &pg, &ch, &trainer);
    let as_of = Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).single().expect("an instant");

    // A pass that got as far as the first instrument, then died.
    let partial = submit(
        &store,
        JobKind::InstrumentProfile,
        manifest(as_of, &keys[..1]),
    )
    .await;
    claim_and_run(&pool, &store, &partial.job_id).await;
    assert_eq!(
        embedding_count(&pg, &ids).await,
        1,
        "the half-done pass is on disk, and nothing rolled it back"
    );

    // The full pass is claimed by a worker that then vanishes.
    let full = submit(&store, JobKind::InstrumentProfile, manifest(as_of, &keys)).await;
    let claimed = store
        .claim(WorkerClass::Knowledge, "doomed")
        .await
        .expect("claim")
        .expect("a job to claim");
    assert_eq!(claimed.job_id, full.job_id);

    sqlx::query("UPDATE jobs SET lease_expires_at = now() - interval '5 minutes' WHERE job_id = $1")
        .bind(&full.job_id)
        .execute(&pg)
        .await
        .expect("expire the lease");
    let reaped = store.reap_expired_leases().await.expect("reap");
    assert!(reaped >= 1, "a dead worker's job returns to the queue");

    let settled = claim_and_run(&pool, &store, &full.job_id).await;
    assert_eq!(settled.state, JobState::Succeeded, "{:?}", settled.error);
    assert_eq!(
        settled.manifest,
        manifest(as_of, &keys),
        "the re-queued job carries the manifest it was submitted with, so the recovered pass \
         does the work the killed one was doing"
    );
    assert_eq!(
        embedding_count(&pg, &ids).await,
        keys.len() as i64,
        "every instrument is written exactly once after the recovery"
    );

    // One open validity interval per instrument. Two would make "the embedding
    // as of t" ambiguous, which is the thing `knowledge_time` exists to prevent.
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM knowledge.asset_embedding
          WHERE instrument_id = $1 AND valid_to IS NULL",
    )
    .bind(ids[0])
    .fetch_one(&pg)
    .await
    .expect("open intervals");
    assert_eq!(open, 1);

    clear_embeddings(&pg, &ids).await;
    clear_jobs(&pg).await;
    pg.close().await;
}

/// A later pass closes the earlier interval rather than leaving two open.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres and ClickHouse"]
async fn a_later_knowledge_time_closes_the_earlier_interval() {
    let Some(pg) = pg().await else { return };
    let Some((ch, keys)) = ch_scratch("knowledge_writer_interval", &["KI-A"]).await else {
        pg.close().await;
        return;
    };
    let ids: Vec<i64> = keys.iter().map(|(i, _)| *i).collect();
    clear_jobs(&pg).await;
    clear_embeddings(&pg, &ids).await;

    let trainer = stub_sidecar().await;
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(jobs::RefuseTrials)));
    let pool = knowledge_pool(&store, &pg, &ch, &trainer);
    let day1 = Utc.with_ymd_and_hms(2026, 6, 3, 0, 0, 0).single().expect("an instant");
    let day2 = day1 + Duration::days(1);

    for day in [day1, day2] {
        let s = submit(&store, JobKind::InstrumentProfile, manifest(day, &keys)).await;
        let settled = claim_and_run(&pool, &store, &s.job_id).await;
        assert_eq!(settled.state, JobState::Succeeded, "{:?}", settled.error);
    }

    let rows: Vec<(DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT valid_from, valid_to FROM knowledge.asset_embedding
          WHERE instrument_id = $1 ORDER BY valid_from",
    )
    .bind(ids[0])
    .fetch_all(&pg)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "a new knowledge day is a new row, not an edit");
    assert_eq!(rows[0].1, Some(day2), "yesterday's interval closes at today");
    assert_eq!(rows[1].1, None, "exactly one interval is open");

    clear_embeddings(&pg, &ids).await;
    clear_jobs(&pg).await;
    pg.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// The tensor projection
// ─────────────────────────────────────────────────────────────────────────────

/// The projection writes facts, and writing them twice writes them once.
///
/// It also pins what the fact carries: the propensity and the censoring, which
/// are the two inputs the completion model needs (§5.5), and a `knowledge_time`
/// that is the trial's settlement rather than the projection's own clock. The
/// last of those is what makes a re-run a genuine no-op — with `now()` every
/// rebuild would rewrite every row and the projection would quietly become a log.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn the_tensor_projection_writes_facts_and_a_rebuild_changes_nothing() {
    let Some(pg) = pg().await else { return };
    let tenant = "t_knowledge_tensor_test";
    clear_tensor(&pg, tenant).await;

    // A real trial, registered and settled through the ledger. There is no side
    // door that writes a fact without one (INV-16), and this test does not want
    // one.
    let l = ledger::pg::PgTrialLedger::new(pg.clone());
    let ctx = ledger::DispatchContext::human(tenant, "knowledge-test", 0.25)
        .with_experiment("knowledge-writers");
    let subject = ledger::TrialSubject {
        config_hash: format!("sha256:knowledge-test-{}", uuid::Uuid::new_v4()),
        config: json!({"kind": "test"}),
        dataset_id: "snapshot:test".into(),
        split_spec_id: None,
        code_hash: "family_under_test".into(),
        image_digest: "test@1".into(),
        seed_set: vec![7],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: json!([]),
        planned_steps: Some(100),
    };
    let t = l
        .register_async(&ledger::Registration::new(&ctx, &subject, 0.5))
        .await
        .expect("registered");
    let trial_id = t.trial_id();
    l.settle_async(
        t,
        ledger::TrialEvent {
            outcome: Some(ledger::OutcomeVector {
                sharpe_net: Some(1.5),
                ..Default::default()
            }),
            ..ledger::TrialEvent::to(ledger::TrialState::CompletedPass)
        },
    )
    .await
    .expect("settled");

    let projection = TensorProjection::new(pg.clone());
    let first = projection.rebuild(tenant).await.expect("first rebuild");
    assert!(
        first.facts_written >= 1,
        "a settled trial with a sharpe is a fact: {first:?}"
    );

    let mut tx = ledger::pg::tenant_tx(&pg, tenant).await.expect("tenant tx");
    let (count, value, propensity, censoring, kt): (
        i64,
        Option<f64>,
        Option<f64>,
        Option<String>,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT count(*), max(metric_value), max(propensity), max(censoring), max(knowledge_time)
           FROM knowledge.outcome_tensor_fact WHERE tenant_id = $1 AND trial_id = $2",
    )
    .bind(tenant)
    .bind(trial_id)
    .fetch_one(&mut *tx)
    .await
    .expect("the fact");
    tx.commit().await.expect("commit");
    assert_eq!(count, 1);
    assert_eq!(value, Some(1.5));
    assert_eq!(
        propensity,
        Some(0.5),
        "the logged propensity travels with the fact (§5.5·1)"
    );
    assert_eq!(censoring.as_deref(), Some("none"));

    // The second pass writes the same thing, including the same knowledge time.
    let second = projection.rebuild(tenant).await.expect("second rebuild");
    assert_eq!(second.facts_written, first.facts_written);
    let mut tx = ledger::pg::tenant_tx(&pg, tenant).await.expect("tenant tx");
    let (count2, kt2): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT count(*), max(knowledge_time) FROM knowledge.outcome_tensor_fact
          WHERE tenant_id = $1 AND trial_id = $2",
    )
    .bind(tenant)
    .bind(trial_id)
    .fetch_one(&mut *tx)
    .await
    .expect("the fact again");
    tx.commit().await.expect("commit");
    assert_eq!(count2, 1, "a rebuild upserts; it does not accumulate");
    assert_eq!(
        kt2, kt,
        "knowledge_time is when the trial settled, so a rebuild does not move it (INV-01)"
    );

    clear_tensor(&pg, tenant).await;
    pg.close().await;
}

/// The rebuild worker runs the projection, running the job twice is the same as
/// running it once, and a manifest with no tenant is refused.
///
/// The refusal matters more than it looks: under row-level security a rebuild
/// with no tenant reads zero rows and reports a clean pass that wrote nothing,
/// which is indistinguishable from a tenant that genuinely has no trials.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn the_tensor_rebuild_job_is_idempotent_and_refuses_a_tenantless_manifest() {
    let Some(pg) = pg().await else { return };
    let tenant = "t_knowledge_tensor_job";
    clear_jobs(&pg).await;
    clear_tensor(&pg, tenant).await;

    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(jobs::RefuseTrials)));
    let pool = knowledge_pool(&store, &pg, "", "");

    let m = json!({ "tenant_id": tenant, "as_of": "2026-06-01T00:00:00Z" });
    let first = submit(&store, JobKind::TensorRebuild, m.clone()).await;
    let a = claim_and_run(&pool, &store, &first.job_id).await;
    assert_eq!(a.state, JobState::Succeeded, "{:?}", a.error);

    let again = submit(&store, JobKind::TensorRebuild, m).await;
    assert!(again.deduplicated, "the same hour is the same rebuild");
    assert_eq!(again.job_id, first.job_id);

    let bad = submit(
        &store,
        JobKind::TensorRebuild,
        json!({ "as_of": "2026-06-01T00:00:00Z" }),
    )
    .await;
    let settled = claim_and_run(&pool, &store, &bad.job_id).await;
    assert_eq!(settled.state, JobState::Failed);
    assert_eq!(
        settled.error.as_ref().and_then(|e| e["code"].as_str()),
        Some("invalid_manifest"),
        "a tenantless rebuild is refused, not run against nothing"
    );

    clear_tensor(&pg, tenant).await;
    clear_jobs(&pg).await;
    pg.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// The schedule
// ─────────────────────────────────────────────────────────────────────────────

/// Two ticks in one knowledge day ask for one job.
///
/// This is the restart property as well as the double-run one: the scheduler
/// keeps no state, so a process that restarts mid-day submits the same manifest
/// and is handed back the job that is already queued or already done. There is
/// no "last run at" to lose.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn two_ticks_in_one_day_ask_for_one_job() {
    let Some(pg) = pg().await else { return };
    let store = JobStore::new(pg.clone(), Arc::new(jobs::RefuseTrials));
    clear_jobs(&pg).await;

    let morning = Utc.with_ymd_and_hms(2026, 6, 10, 0, 5, 0).single().expect("an instant");
    let evening = Utc
        .with_ymd_and_hms(2026, 6, 10, 23, 30, 0)
        .single()
        .expect("an instant");
    let tomorrow = Utc.with_ymd_and_hms(2026, 6, 11, 0, 5, 0).single().expect("an instant");

    let first = knowledge_scheduler::submit_due(&pg, &store, morning)
        .await
        .expect("first tick");
    let second = knowledge_scheduler::submit_due(&pg, &store, evening)
        .await
        .expect("second tick");

    if first.no_population {
        // Nothing has an identity row as of the horizon. Say so loudly rather
        // than passing quietly: a green test here would mean "the scheduler is
        // idempotent about doing nothing".
        eprintln!(
            "no instrument had an identity in dataplane.instrument_symbol as of {morning}; the \
             fingerprint half of this test proved nothing"
        );
    } else {
        assert_eq!(
            first.profile_job, second.profile_job,
            "the second tick of a knowledge day must return the first tick's job"
        );
        assert!(!first.profile_deduplicated);
        assert!(second.profile_deduplicated);

        let third = knowledge_scheduler::submit_due(&pg, &store, tomorrow)
            .await
            .expect("next day");
        assert_ne!(
            third.profile_job, first.profile_job,
            "a new knowledge day is new work"
        );
    }

    // The tensor half runs per tenant per hour, and the same hour deduplicates.
    assert!(
        !first.tensor_jobs.is_empty(),
        "no tenant to project: the scheduler would never rebuild anything"
    );
    assert_eq!(first.tensor_jobs, second.tensor_jobs);
    assert_eq!(second.tensor_deduplicated, second.tensor_jobs.len());

    // Everything the scheduler asked for is queued for the knowledge pool, and
    // for no other — a job in the data pool's queue would be claimed by a pool
    // with no worker for it and failed `no_worker`, permanently.
    let classes: Vec<String> =
        sqlx::query("SELECT DISTINCT worker_class FROM jobs WHERE user_id = $1")
            .bind(knowledge_scheduler::SYSTEM_PRINCIPAL)
            .fetch_all(&pg)
            .await
            .expect("classes")
            .into_iter()
            .map(|r| r.get::<String, _>("worker_class"))
            .collect();
    assert!(!classes.is_empty());
    for class in &classes {
        assert_eq!(class, "knowledge", "scheduled onto the wrong pool");
    }

    clear_jobs(&pg).await;
    pg.close().await;
}
