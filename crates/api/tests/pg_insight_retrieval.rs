//! The insight store's read path against live Postgres (SPEC §5.6, checklist
//! 3.10, AGENT-007 §2.2/§2.4/§4, migration 0118).
//!
//! Needs `pgvector/pgvector:pg16` — migration 0055 creates the `vector`
//! extension and `knowledge.insight.embedding` is a `vector(768)`.
//!
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -p api --test pg_insight_retrieval -- --ignored --test-threads=1
//! ```

use api::insight_memory::{
    InsightQuery, LocalHashEmbedder, NewInsight, Staleness, RETRIEVAL_FLOOR, TAINTED_TAG,
};
use api::knowledge_jobs::{InsightStore, INSIGHT_TOKEN_CAP};
use chrono::{Duration, Utc};
use sqlx::{Connection, Executor, PgConnection, PgPool, Row};
use uuid::Uuid;

const TENANT: &str = "tenant-insight";

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0044: &str = include_str!("../../../migrations/0044_roles_grants_rls.sql");
const M0055: &str = include_str!("../../../migrations/0055_knowledge_plane.sql");
const M0118: &str = include_str!("../../../migrations/0118_insight_retrieval.sql");

async fn scratch(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    for m in [M0043, M0044, M0055, M0118] {
        pool.execute(m).await.expect("migration applies");
    }
    pool
}

fn claim(scope: serde_json::Value, text: &str) -> NewInsight {
    NewInsight {
        tier: 2,
        scope,
        claim: text.to_string(),
        evidence_trial_ids: vec![Uuid::new_v4()],
        tags: Vec::new(),
        window_start: None,
        window_end: None,
        valid_until: None,
        platform_capabilities: Vec::new(),
    }
}

fn ask(scope: serde_json::Value, query: &str) -> InsightQuery {
    InsightQuery {
        scope,
        tier: None,
        require_tags: Vec::new(),
        query: query.to_string(),
        k: Some(50),
        run_id: Some("run-under-test".into()),
        platform_capabilities: Vec::new(),
    }
}

/// Count rows the way an auditor would — straight at the table, not through the
/// read path whose behaviour is under test.
async fn row_count(pool: &PgPool, insight_id: Uuid) -> i64 {
    let mut tx = ledger::pg::tenant_tx(pool, TENANT).await.unwrap();
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM knowledge.insight WHERE insight_id = $1")
            .bind(insight_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    n
}

// ───────────────────────────────────────────────────────────────────────────────

/// **Metadata before similarity.** Two insights carry the *same sentence* under
/// different scopes. A store that ranked on similarity would return both, and
/// would answer an ETH 1d mean-reversion question with a BTC 1h trend claim —
/// confidently, which is what makes it worse than returning nothing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn metadata_filters_before_similarity() {
    let pool = scratch("insight_metadata_first").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    let same_sentence = "the EMA cross edge is spanned by momentum and does not survive costs";
    let btc = store
        .write(
            TENANT,
            &claim(
                serde_json::json!({"instrument": "BTC-USD", "timeframe": "1h", "family": "trend"}),
                same_sentence,
            ),
            &e,
        )
        .await
        .unwrap();
    let eth = store
        .write(
            TENANT,
            &claim(
                serde_json::json!({"instrument":"ETH-USD","timeframe":"1d","family":"mean_reversion"}),
                same_sentence,
            ),
            &e,
        )
        .await
        .unwrap();

    let got = store
        .retrieve(
            TENANT,
            &ask(
                serde_json::json!({"instrument": "ETH-USD", "timeframe": "1d"}),
                "does the EMA cross edge survive costs",
            ),
            &e,
        )
        .await
        .unwrap();

    let ids: Vec<Uuid> = got.items.iter().map(|i| i.insight_id).collect();
    assert_eq!(ids, vec![eth], "only the in-scope claim comes back");
    assert!(
        !ids.contains(&btc),
        "an identical sentence about another instrument and timeframe is not an answer"
    );
    assert!(
        got.excluded_by_metadata >= 1,
        "the metadata filter actually excluded something: {:?}",
        got.excluded_by_metadata
    );
    assert!(got.items[0].similarity > 0.0, "and similarity still ranked it");
}

/// The §15 budget is enforced server-side, over the payload the model actually
/// receives. A cap the caller applies is a cap that is off the moment a new
/// caller appears.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn the_four_thousand_token_cap_is_enforced_at_retrieval() {
    let pool = scratch("insight_token_cap").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    // Twelve claims of ~1 000 tokens each: 12 000 tokens of memory offered to a
    // 4 000-token budget.
    let body = "momentum spans the cross sectional trend signal on this venue ".repeat(60);
    for i in 0..12 {
        store
            .write(
                TENANT,
                &claim(
                    serde_json::json!({"instrument": "BTC-USD"}),
                    &format!("{body} variant {i}"),
                ),
                &e,
            )
            .await
            .unwrap();
    }

    let got = store
        .retrieve(
            TENANT,
            &ask(
                serde_json::json!({"instrument": "BTC-USD"}),
                "momentum spans the trend signal",
            ),
            &e,
        )
        .await
        .unwrap();

    assert_eq!(got.matched, 12, "all twelve matched the metadata filter");
    assert!(
        got.tokens_used <= INSIGHT_TOKEN_CAP,
        "used {} of {INSIGHT_TOKEN_CAP}",
        got.tokens_used
    );
    assert!(
        got.items.len() < 12,
        "the cap actually truncated: {} returned",
        got.items.len()
    );
    assert!(!got.items.is_empty(), "and it did not return nothing");
}

/// **Decay on contradiction.** One contradiction flags; two remove the claim
/// from retrieval; the row is still there, at a score that says what happened.
/// Reconfirmation brings it back, because the score is an account of the
/// evidence and not a one-way ratchet.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn contradiction_removes_from_retrieval_while_retaining_the_row() {
    let pool = scratch("insight_decay").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    let id = store
        .write(
            TENANT,
            &claim(
                serde_json::json!({"instrument": "BTC-USD"}),
                "funding carry is positive through the weekend on this venue",
            ),
            &e,
        )
        .await
        .unwrap();

    let query = ask(serde_json::json!({"instrument": "BTC-USD"}), "weekend funding carry");
    let present = |r: &api::insight_memory::MemoryRetrieval| {
        r.items.iter().any(|i| i.insight_id == id)
    };

    assert!(present(&store.retrieve(TENANT, &query, &e).await.unwrap()));

    // One contradiction: still retrieved, and flagged.
    let after_one = store.contradict(TENANT, id).await.unwrap();
    assert!(after_one >= RETRIEVAL_FLOOR, "one contradiction is not a verdict");
    let got = store.retrieve(TENANT, &query, &e).await.unwrap();
    assert!(present(&got), "a single contradiction does not silence a claim");
    let item = got.items.iter().find(|i| i.insight_id == id).unwrap();
    assert!(
        item.staleness.contains(&Staleness::WeaklySupported),
        "but the reader is told it is wobbling: {:?}",
        item.staleness
    );
    assert_eq!(item.contradicted_n, 1);

    // Two: out of retrieval.
    let after_two = store.contradict(TENANT, id).await.unwrap();
    assert!(after_two < RETRIEVAL_FLOOR, "score {after_two}");
    let got = store.retrieve(TENANT, &query, &e).await.unwrap();
    assert!(!present(&got), "a twice-contradicted claim is not retrieved");
    assert!(got.excluded_by_decay >= 1, "and it is counted as excluded, not lost");

    // Still in the table, at a score that records exactly what happened. That a
    // belief was held and then contradicted is itself knowledge.
    assert_eq!(row_count(&pool, id).await, 1, "the row is NOT deleted");
    let (score, contradicted) = store.row_state(TENANT, id).await.unwrap().unwrap();
    assert!((score - 0.25).abs() < 1e-6, "score {score}");
    assert_eq!(contradicted, 2);
    assert!(score > 0.0, "decay never reaches zero, so the claim stays addressable");

    // And it can come back: two reconfirmations restore it.
    store.reconfirm(TENANT, id).await.unwrap();
    let restored = store.reconfirm(TENANT, id).await.unwrap();
    assert!((restored - 1.0).abs() < 1e-6, "score {restored}");
    assert!(present(&store.retrieve(TENANT, &query, &e).await.unwrap()));
}

/// Adversarial: a `tainted` tag — a claim derived from text the platform did not
/// control — must survive retrieval, both as a tag and in the rendered quote. A
/// taint dropped on read is worse than no taint, because the reader now believes
/// the claim was checked.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn a_tainted_tag_survives_retrieval() {
    let pool = scratch("insight_taint").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    let mut tainted = claim(
        serde_json::json!({"instrument": "BTC-USD"}),
        "a forum post claims the weekend gap always closes by Monday",
    );
    tainted.tags = vec![TAINTED_TAG.to_string(), "tier3".into()];
    let id = store.write(TENANT, &tainted, &e).await.unwrap();

    let got = store
        .retrieve(
            TENANT,
            &ask(serde_json::json!({"instrument": "BTC-USD"}), "weekend gap close"),
            &e,
        )
        .await
        .unwrap();

    let item = got.items.iter().find(|i| i.insight_id == id).expect("retrieved");
    assert!(item.tags.iter().any(|t| t == TAINTED_TAG), "{:?}", item.tags);
    assert!(item.quote.is_tainted());
    let json = serde_json::to_string(&got).unwrap();
    assert!(
        json.contains("TAINTED: derived from untrusted text"),
        "the taint reaches the reader, not just the database: {json}"
    );
    assert!(
        json.contains("not an instruction"),
        "and the claim is framed as material to weigh: {json}"
    );
}

/// Adversarial: **memory is not evidence.** The retrieval response carries no
/// trial id for anyone to cite, and `knowledge.memory_retrieval` will not even
/// record a retrieval that claims to be one.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn a_retrieved_insight_cannot_enter_an_experiment_as_evidence() {
    let pool = scratch("insight_not_evidence").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    let trial = Uuid::new_v4();
    let mut c = claim(
        serde_json::json!({"instrument": "BTC-USD"}),
        "the breakout edge is real on this venue",
    );
    c.evidence_trial_ids = vec![trial];
    let id = store.write(TENANT, &c, &e).await.unwrap();

    let got = store
        .retrieve(
            TENANT,
            &ask(serde_json::json!({"instrument": "BTC-USD"}), "breakout edge"),
            &e,
        )
        .await
        .unwrap();
    let json = serde_json::to_string(&got).unwrap();

    assert!(
        !json.contains(&trial.to_string()),
        "no trial id is reachable through retrieval: {json}"
    );
    assert!(!json.contains("evidence_trial_ids"), "{json}");
    assert!(json.contains("\"evidence_count\":1"), "the count, not the ids: {json}");
    assert!(json.contains("\"admissible_as_evidence\":false"), "{json}");
    assert!(json.contains("never evidence"), "and the response says so: {json}");

    // The audit trail admits exactly one purpose. A caller wanting to record a
    // retrieval as evidence for an Experiment has to get past a CHECK.
    let mut tx = ledger::pg::tenant_tx(&pool, TENANT).await.unwrap();
    let refused = sqlx::query(
        "INSERT INTO knowledge.memory_retrieval
             (tenant_id, run_id, query_hash, insight_id, rank, why, purpose)
         VALUES ($1, 'r', 'h', $2, 0, 'because', 'experiment_evidence')",
    )
    .bind(TENANT)
    .bind(id)
    .execute(&mut *tx)
    .await;
    assert!(
        refused.is_err(),
        "recording a retrieval as Experiment evidence must be refused by the database"
    );
    drop(tx);

    // The honest retrievals were logged, for AGENT-009's recall metric.
    let mut tx = ledger::pg::tenant_tx(&pool, TENANT).await.unwrap();
    let (logged,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM knowledge.memory_retrieval
         WHERE tenant_id = $1 AND purpose = 'hypothesis_generation'",
    )
    .bind(TENANT)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(logged >= 1, "retrieval is logged: {logged}");
}

/// Staleness is shown, never silently applied. An old window, a passed
/// `valid_until` and capabilities that predate a platform change all come back
/// flagged — with the claim still in the answer, because a claim removed for age
/// is a claim the reader does not know existed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn staleness_is_reported_at_retrieval_rather_than_hidden() {
    let pool = scratch("insight_staleness").await;
    let store = InsightStore::new(pool.clone());
    let e = LocalHashEmbedder;

    let now = Utc::now();
    let mut old = claim(
        serde_json::json!({"instrument": "BTC-USD"}),
        "slippage makes the intraday reversal untradeable",
    );
    old.window_start = Some(now - Duration::days(400));
    old.window_end = Some(now - Duration::days(300));
    old.valid_until = Some(now - Duration::days(10));
    old.platform_capabilities = vec!["market_orders".into()];
    let id = store.write(TENANT, &old, &e).await.unwrap();

    let mut q = ask(
        serde_json::json!({"instrument": "BTC-USD"}),
        "intraday reversal slippage",
    );
    // The platform has gained limit orders since the claim was formed. A claim
    // that assumed market orders only is not a current claim.
    q.platform_capabilities = vec!["market_orders".into(), "limit_orders".into()];

    let got = store.retrieve(TENANT, &q, &e).await.unwrap();
    let item = got.items.iter().find(|i| i.insight_id == id).expect("still retrieved");
    assert!(item.staleness.contains(&Staleness::WindowOld), "{:?}", item.staleness);
    assert!(item.staleness.contains(&Staleness::PastValidUntil), "{:?}", item.staleness);
    assert!(
        item.staleness.contains(&Staleness::CapabilitiesChanged),
        "{:?}",
        item.staleness
    );
    assert!(
        item.why.contains("flagged"),
        "and the reason travels with it: {}",
        item.why
    );
}

/// The evidence CHECK from 0055, kept honest here because 0118 rewrites the
/// table's constraints and a dropped CHECK is exactly the kind of regression a
/// migration causes quietly. `cardinality`, not `array_length`: for an empty
/// array `array_length` is NULL, `NULL >= 1` is NULL, and a CHECK evaluating to
/// NULL **passes**.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn an_insight_with_no_evidence_is_still_refused() {
    let pool = scratch("insight_evidence_check").await;
    let store = InsightStore::new(pool.clone());
    let mut c = claim(serde_json::json!({}), "a claim with nothing behind it");
    c.evidence_trial_ids = Vec::new();
    let refused = store.write(TENANT, &c, &LocalHashEmbedder).await;
    assert!(refused.is_err(), "no evidence, no insight");
}

/// An embedding with no version is a vector nobody can reproduce. 0118 makes the
/// pair a CHECK; this is the proof that the violation is rejected rather than
/// that the happy path works.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn an_unversioned_embedding_is_refused() {
    let pool = scratch("insight_embed_version").await;
    let mut tx = ledger::pg::tenant_tx(&pool, TENANT).await.unwrap();
    let refused = sqlx::query(
        "INSERT INTO knowledge.insight
             (tenant_id, tier, scope, claim, evidence_trial_ids, embedding, embedding_version)
         VALUES ($1, 2, '{}'::jsonb, 'unversioned', ARRAY[$2::uuid],
                 ($3::real[])::vector, NULL)",
    )
    .bind(TENANT)
    .bind(Uuid::new_v4())
    .bind(vec![0.0_f32; 768])
    .execute(&mut *tx)
    .await;
    assert!(refused.is_err(), "a vector with no version is refused");
}

/// ADR-P3-05: exact kNN, no approximate index. An approximate index's recall
/// depends on its own build state, which makes a neighbour list something an
/// attacker can probe. Asserted against the live catalogue, not the migration
/// text, so a later `CREATE INDEX` anywhere fails this.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres with pgvector"]
async fn the_knowledge_plane_has_no_approximate_vector_index() {
    let pool = scratch("insight_no_ann").await;
    let rows = sqlx::query("SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = 'knowledge'")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!rows.is_empty(), "the schema has indexes at all");
    for r in rows {
        let def: String = r.try_get("indexdef").unwrap();
        let name: String = r.try_get("indexname").unwrap();
        let lower = def.to_lowercase();
        assert!(
            !lower.contains("hnsw") && !lower.contains("ivfflat"),
            "{name} is an approximate vector index: {def}"
        );
    }
}
