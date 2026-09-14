//! Daily anchors and the chain verifier against live Postgres (SPEC §4.6, INV-19).
//! Ignored by default: `cargo test -p ledger --test pg_anchor -- --ignored --test-threads=1`.

use chrono::NaiveDate;
use ledger::anchor::{AnchorSigner, FsWorm, WormStore};
use ledger::pg::PgTrialLedger;
use ledger::{Decision, DecisionKind, DecisionTier, DispatchContext, OutcomeVector, Registration, TrialEvent, TrialState, TrialSubject};
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0044: &str = include_str!("../../../migrations/0044_roles_grants_rls.sql");
const M0047: &str = include_str!("../../../migrations/0047_ledger_fixation.sql");

async fn scratch(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str()).await.unwrap();
    admin.execute(format!("CREATE DATABASE {name}").as_str()).await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    pool.execute("CREATE TABLE IF NOT EXISTS public._sqlx_migrations (version BIGINT PRIMARY KEY)").await.unwrap();
    for m in [M0043, M0044, M0047] {
        pool.execute(m).await.expect("migration applies");
    }
    pool
}

fn subject(n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({ "n": n }),
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

async fn populate(ledger: &PgTrialLedger, tenant: &str, from: u64, count: u64) {
    let ctx = DispatchContext::human(tenant, "m", 0.1).with_experiment("e");
    for i in from..from + count {
        let trial_subject = subject(i);
        let mut ticket = ledger.register_async(&Registration::new(&ctx, &trial_subject, 0.25)).await.unwrap();
        ledger.transition_async(&mut ticket, TrialEvent::to(TrialState::Running)).await.unwrap();
        // INV-18: a completion reporting a Sharpe chains its persisted return series.
        let start = chrono::Utc::now();
        let series = ledger::neff::ReturnSeries {
            timestamps: (0..3).map(|k| start + chrono::Duration::days(k)).collect(),
            returns: vec![0.01, -0.004, 0.002 * i as f64],
        };
        let returns_uri = ledger.persist_returns_async(&ticket, &series).await.unwrap();
        let outcome = OutcomeVector { sharpe_net: Some(0.5 + i as f64), ..Default::default() };
        ledger.settle_async(ticket, TrialEvent::completed(format!("run-{i}"), Some(outcome)).with_returns(returns_uri)).await.unwrap();
    }
    let decision = Decision {
        kind: DecisionKind::Propose,
        context_hash: "sha256:ctx".into(),
        context_uri: None,
        // Keys in non-canonical order and nested values: the verifier must hash what
        // Postgres rendered, not what the caller sent.
        candidate_set: vec![serde_json::json!({"zeta": 1, "a": [1.50, {"y": null, "b": true}]}), serde_json::json!({"cfg": 2})],
        chosen: serde_json::json!({"zeta": 1, "a": [1.50, {"y": null, "b": true}]}),
        propensity: Some(0.5),
        exploration_flag: true,
        decision_tier: DecisionTier::Rule,
        rationale: None,
    };
    ledger.log_decision_async(&ctx, &decision).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn anchors_verify_and_tampering_is_caught_everywhere() {
    let pool = scratch("ledger_anchor_test").await;
    let l = PgTrialLedger::new(pool.clone());
    let worm_dir = std::env::temp_dir().join(format!("worm-{}", uuid::Uuid::new_v4()));
    let worm = FsWorm::new(&worm_dir);
    let signer = AnchorSigner::from_hex_seed(&"2a".repeat(32)).unwrap();
    let keys = [signer.verifying_key()];

    populate(&l, "tenant-a", 0, 3).await;
    populate(&l, "tenant-b", 100, 1).await;
    assert_eq!(l.tenants_async().await.unwrap(), vec!["tenant-a".to_string(), "tenant-b".to_string()]);

    let day1 = NaiveDate::from_ymd_opt(2026, 9, 11).unwrap();
    let day2 = NaiveDate::from_ymd_opt(2026, 9, 12).unwrap();
    assert!(l.write_anchor_async("tenant-a", day1, &signer, &worm).await.unwrap().is_some());
    assert!(l.write_anchor_async("tenant-a", day1, &signer, &worm).await.unwrap().is_none(), "one anchor per day");
    populate(&l, "tenant-a", 3, 2).await;
    assert!(l.write_anchor_async("tenant-a", day2, &signer, &worm).await.unwrap().is_some());

    let clean = l.verify_and_record_async("tenant-a", &keys, Some(&worm)).await.unwrap();
    assert!(clean.ok(), "{:?}", clean.failures);
    assert_eq!((clean.trials, clean.events, clean.decisions, clean.anchors), (5, 10, 2, 2));

    // An anchor signed by a key the verifier does not accept.
    let stranger = AnchorSigner::from_hex_seed(&"2b".repeat(32)).unwrap();
    let unknown = l.verify_and_record_async("tenant-a", &[stranger.verifying_key()], None).await.unwrap();
    assert_eq!(unknown.failures.len(), 2, "{:?}", unknown.failures);

    // The WORM copy disappears.
    let anchors = l.anchor_rows_async("tenant-a").await.unwrap();
    let path = anchors[0].worm_uri.strip_prefix("file://").unwrap().to_string();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    std::fs::write(&path, b"{}").unwrap();
    let worm_tamper = l.verify_and_record_async("tenant-a", &keys, Some(&worm)).await.unwrap();
    assert!(worm_tamper.failures.iter().any(|f| f.contains("WORM copy unreadable")), "{:?}", worm_tamper.failures);

    // Someone with the owner's credentials bypasses the append-only trigger and
    // rewrites a historical propensity.
    pool.execute("ALTER TABLE mlops.trial DISABLE TRIGGER USER").await.unwrap();
    pool.execute("UPDATE mlops.trial SET propensity = 0.9 WHERE tenant_id = 'tenant-a' AND seq = 1").await.unwrap();
    pool.execute("ALTER TABLE mlops.trial ENABLE TRIGGER USER").await.unwrap();
    let rewritten = l.verify_and_record_async("tenant-a", &keys, None).await.unwrap();
    assert!(rewritten.failures.iter().any(|f| f.contains("trial chain")), "{:?}", rewritten.failures);
    let refused = l.write_anchor_async("tenant-a", NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(), &signer, &worm).await;
    assert!(matches!(refused, Err(ledger::anchor::AnchorError::Broken(_))), "a broken chain is never anchored");

    // …and recomputes every hash to hide it. The chain verifies again on its own,
    // but no longer matches what was signed.
    pool.execute("ALTER TABLE mlops.trial DISABLE TRIGGER USER").await.unwrap();
    let rows = l.trial_rows_async("tenant-a").await.unwrap();
    let mut prev = ledger::GENESIS_HASH.to_vec();
    for mut r in rows {
        r.prev_hash.clone_from(&prev);
        let h = r.recompute_hash();
        sqlx::query("UPDATE mlops.trial SET prev_hash = $1, row_hash = $2 WHERE trial_id = $3")
            .bind(&prev)
            .bind(&h)
            .bind(r.trial_id)
            .execute(&pool)
            .await
            .unwrap();
        prev = h;
    }
    pool.execute("ALTER TABLE mlops.trial ENABLE TRIGGER USER").await.unwrap();
    let rehashed = l.verify_and_record_async("tenant-a", &keys, None).await.unwrap();
    assert!(!rehashed.failures.iter().any(|f| f.contains("trial chain:")), "the forger's chain is self-consistent");
    assert!(rehashed.failures.iter().any(|f| f.contains("trial head no longer matches")), "{:?}", rehashed.failures);

    // Every pass left a row, including the failures, and none can be erased.
    let recorded: Vec<bool> = sqlx::query_scalar("SELECT ok FROM mlops.ledger_verification WHERE tenant_id = 'tenant-a' ORDER BY verified_at")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(recorded, vec![true, false, false, false, false]);
    assert!(pool.execute("DELETE FROM mlops.ledger_verification").await.is_err());
    assert!(pool.execute("UPDATE mlops.ledger_anchor SET key_id = 'x'").await.is_err());

    let other = l.verify_and_record_async("tenant-b", &keys, Some(&worm)).await.unwrap();
    assert!(other.ok(), "tenant-b is untouched: {:?}", other.failures);

    pool.close().await;
    for entry in walk(&worm_dir) {
        let mut p = std::fs::metadata(&entry).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        std::fs::set_permissions(&entry, p).unwrap();
    }
    std::fs::remove_dir_all(&worm_dir).ok();
    let _ = worm.get("file:///nonexistent");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
