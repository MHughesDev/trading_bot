#![allow(clippy::float_cmp)] // N_eff values are whole cluster counts; exact comparison is the assertion.
//! Return-series persistence and platform N_eff against live Postgres (INV-18,
//! INV-22, AT-26, AT-28). Ignored by default:
//! `cargo test -p ledger --test pg_neff -- --ignored --test-threads=1`.

use chrono::{Duration, TimeZone, Utc};
use ledger::neff::ReturnSeries;
use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, OutcomeVector, Registration, TerminalReason, TrialEvent, TrialLedger, TrialState, TrialSubject};
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

async fn scratch(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str()).await.unwrap();
    admin.execute(format!("CREATE DATABASE {name}").as_str()).await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    pool.execute(include_str!("../../../migrations/0043_trial_ledger.sql")).await.expect("0043 applies");
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

fn series(seed: u64, idea: u64) -> ReturnSeries {
    let t0 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    let mut g = idea.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(3_037_000_493);
    let mut h = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let next = |s: &mut u64| {
        *s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((*s >> 11) as f64 / (1u64 << 53) as f64) - 0.5
    };
    let returns = (0..120).map(|_| next(&mut g) * 0.02 + next(&mut h) * 0.002).collect();
    ReturnSeries { timestamps: (0..120).map(|i| t0 + Duration::days(i)).collect(), returns }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn series_persist_once_and_every_trial_enters_n_eff() {
    let pool = scratch("ledger_neff").await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("tenant-n", "m", 0.1).with_experiment("e");

    // 100 trials over 3 ideas; 90 of them then fail a gate. All are counted.
    for i in 0..100u64 {
        let s = subject(i);
        let mut t = l.register_async(&Registration::new(&ctx, &s, 0.2)).await.unwrap();
        l.transition_async(&mut t, TrialEvent::to(TrialState::Running)).await.unwrap();
        let uri = l.persist_returns_async(&t, &series(i, i % 3)).await.unwrap();
        assert!(l.persist_returns_async(&t, &series(i, i % 3)).await.is_err(), "a series is written once");
        let outcome = Some(OutcomeVector { sharpe_net: Some(1.0), ..Default::default() });
        if i < 90 {
            // A gate failure is a completed, evaluated trial that failed the gate (§9).
            for st in [TrialState::Evaluate, TrialState::Gated] {
                l.transition_async(&mut t, TrialEvent::to(st)).await.unwrap();
            }
            let failed = TrialEvent { outcome, gate_profile: Some("strict_v1".into()), ..TrialEvent::to(TrialState::CompletedFail) };
            l.settle_async(t, failed.with_returns(uri)).await.unwrap();
        } else {
            l.settle_async(t, TrialEvent::completed(format!("run-{i}"), outcome).with_returns(uri)).await.unwrap();
        }
    }
    let n = l.n_eff_async("tenant-n").await.unwrap();
    assert_eq!(n.trials_counted(), 100);
    assert_eq!(n.series(), 100);
    assert_eq!(n.value(), 3.0, "three ideas, however many times swept");
    // A crashed trial is still counted, and contributes no series.
    let s = subject(999);
    let t = l.register_async(&Registration::new(&ctx, &s, 0.2)).await.unwrap();
    l.settle_async(t, TrialEvent::failed(TerminalReason::Oom, "oom")).await.unwrap();
    let n = TrialLedger::n_eff(&l, "tenant-n").unwrap();
    assert_eq!((n.trials_counted(), n.series(), n.value()), (101, 100, 3.0));

    // INV-18: the schema refuses a Sharpe without its series.
    let s = subject(1000);
    let mut t = l.register_async(&Registration::new(&ctx, &s, 0.2)).await.unwrap();
    l.transition_async(&mut t, TrialEvent::to(TrialState::Running)).await.unwrap();
    let err = l
        .settle_async(t, TrialEvent::completed("run-x", Some(OutcomeVector { sharpe_net: Some(2.0), ..Default::default() })))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("chk_event_returns_persisted"), "{err}");

    assert!(pool.execute("UPDATE mlops.trial_return_series SET returns = '{}'").await.is_err());
    pool.close().await;
}
