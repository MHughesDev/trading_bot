//! Platform self-monitoring, gate verdicts and the second gate profile, against
//! live Postgres (SPEC §12.3, §12.7, §16.2; AT-58, AT-63, AT-65, AT-69).
//!
//! Ignored by default:
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//! PLATFORM_DB_APP_PASSWORD=platform_app_dev \
//!   cargo test -p api --test pg_self_monitor -- --ignored --test-threads=1
//! ```

use ledger::gates::{GateLog, GateRecord};
use ledger::pg::PgTrialLedger;
use sqlx::{Connection, Executor, PgConnection, PgPool, Row};

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const M0002: &str = include_str!("../../../migrations/0002_instruments.sql");
const M0037: &str = include_str!("../../../migrations/0037_jobs_artifacts.sql");
const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0044: &str = include_str!("../../../migrations/0044_roles_grants_rls.sql");
const M0046: &str = include_str!("../../../migrations/0046_dataplane.sql");
const M0047: &str = include_str!("../../../migrations/0047_ledger_fixation.sql");
const M0048: &str = include_str!("../../../migrations/0048_feature_consistency.sql");
const M0049: &str = include_str!("../../../migrations/0049_leakage_suite.sql");
const M0050: &str = include_str!("../../../migrations/0050_gates_v2.sql");

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
    pool.execute("CREATE TABLE IF NOT EXISTS public._sqlx_migrations (version BIGINT PRIMARY KEY)")
        .await
        .unwrap();
    for m in [M0002, M0037, M0043, M0044, M0046, M0047, M0048, M0049, M0050] {
        pool.execute(m).await.expect("migration applies");
    }
    pool
}

/// AT-58 / AT-69: every §16.2 signal is present, and a signal whose model does
/// not exist reports `not_fitted` **with no number** rather than a zero.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn every_spec_signal_is_reported_and_unfitted_ones_carry_no_number() {
    let pg = scratch("self_monitor_signals").await;
    let report = api::self_monitor::health(&pg, "t-health")
        .await
        .expect("health report");

    let ids: Vec<&str> = report.signals.iter().map(|s| s.id).collect();
    for expected in [
        "gate_pass_rate",
        "exploration_fraction",
        "feature_consistency_p99",
        "leakage_suite",
        "sealed_holdout_calls",
        "neff_vs_trials",
        "policy_entropy",
        "artifact_pin_coverage",
        "trajectory_corpus_half_life",
        "m5_calibration_ece",
        "mnar_b1",
    ] {
        assert!(ids.contains(&expected), "§16.2 signal {expected} is missing from the report");
    }
    assert_eq!(ids.len(), 11, "no signal may be added without a spec reference");

    // On an empty platform nothing is in alarm and nothing is broken — but
    // nothing pretends to a value either.
    for s in &report.signals {
        assert!(
            s.state != api::self_monitor::SignalState::Unavailable,
            "{} could not be computed: {}",
            s.id,
            s.detail
        );
        if !s.state.has_value() {
            assert!(
                s.value.is_none(),
                "{} is {:?} and must not carry a number",
                s.id,
                s.state
            );
            assert!(!s.detail.is_empty(), "{} must say why it has no value", s.id);
        }
    }

    // M5 and the MNAR coefficient are the two signals whose models do not exist.
    let m5 = report.signals.iter().find(|s| s.id == "m5_calibration_ece").unwrap();
    assert_eq!(m5.state, api::self_monitor::SignalState::NotFitted);
    assert!(m5.value.is_none());
    assert_eq!(m5.threshold, Some(0.08), "the bar is shown even before there is a value");

    assert!(report.healthy(), "an empty platform has nothing wrong with it");
}

/// Gate verdicts are durable, carry their profile, and aggregate to a
/// candidate-level pass rate the self-monitor can read.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn gate_verdicts_persist_and_aggregate_by_profile() {
    let pg = scratch("self_monitor_gates").await;
    let led = PgTrialLedger::new(pg.clone());
    let tenant = "t-gates";

    // Two candidates: one clears both gates, one fails the second.
    for (exp, g13, g14) in [("exp-pass", true, true), ("exp-fail", true, false)] {
        led.record_gate(
            tenant,
            &GateRecord::measured("strict_v1", 13, "stationary_bootstrap", g13, 0.4, 0.0, "p05 > 0")
                .for_experiment(exp),
        )
        .expect("record gate 13");
        led.record_gate(
            tenant,
            &GateRecord::measured("strict_v1", 14, "romano_wolf", g14, 0.02, 0.05, "stepdown")
                .for_experiment(exp)
                .with_significance(9.0, 140),
        )
        .expect("record gate 14");
    }

    let rate = led.pass_rate(tenant, "strict_v1", 30).expect("pass rate");
    assert_eq!(rate.recent_decided, 2, "two candidates were decided");
    assert_eq!(
        rate.recent_passed, 1,
        "a candidate that failed one gate did not pass — verdicts are not averaged"
    );
    assert_eq!(rate.baseline_decided, 0);
    assert_eq!(rate.drifted(), None, "no baseline is not a drift");

    // The verdict is immutable, like every other ledger row.
    let edit = sqlx::query("UPDATE mlops.gate_verdict SET passed = true WHERE passed = false")
        .execute(&pg)
        .await;
    assert!(edit.is_err(), "a recorded verdict must not be editable");

    // And INV-3 holds in the schema, not only in the type: a significance
    // verdict without its trial count is refused by the database.
    let naked = sqlx::query(
        "INSERT INTO mlops.gate_verdict
             (tenant_id, experiment_id, profile_id, gate_no, gate_name, passed, detail)
         VALUES ($1, 'exp-x', 'strict_v1', 14, 'romano_wolf', true, 'no context')",
    )
    .bind(tenant)
    .execute(&pg)
    .await;
    assert!(naked.is_err(), "significance is never naked (INV-3)");

    // The self-monitor reads it back.
    let report = api::self_monitor::health(&pg, tenant).await.expect("health");
    let s = report.signals.iter().find(|s| s.id == "gate_pass_rate").unwrap();
    assert!(s.state.has_value(), "{}", s.detail);
    assert!(s.detail.contains("strict_v1"));
}

/// AT-65: `paper_v1` exists as a *new* profile, keeps every statistical floor,
/// and cannot authorise capital.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn paper_v1_exists_beside_strict_v1_and_cannot_authorise_capital() {
    let pg = scratch("self_monitor_paper").await;

    let rows = sqlx::query(
        "SELECT profile_id, supersedes, authorises_capital, thresholds
           FROM mlops.gate_profile ORDER BY profile_id",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "strict_v1 and paper_v1");

    let paper = rows
        .iter()
        .find(|r| r.get::<String, _>("profile_id") == "paper_v1")
        .expect("paper_v1 exists");
    assert!(!paper.get::<bool, _>("authorises_capital"), "paper_v1 must not authorise capital");
    assert_eq!(paper.get::<Option<String>, _>("supersedes").as_deref(), Some("strict_v1"));

    let strict = rows
        .iter()
        .find(|r| r.get::<String, _>("profile_id") == "strict_v1")
        .expect("strict_v1 exists");
    assert!(strict.get::<bool, _>("authorises_capital"));

    let pt: serde_json::Value = paper.get("thresholds");
    let st: serde_json::Value = strict.get("thresholds");
    assert_eq!(pt["min_track_record_years"], serde_json::json!(0.0));
    assert_eq!(st["min_track_record_years"], serde_json::json!(5.0));
    // Every statistical floor P-01 is about is kept.
    for k in ["dsr_gte", "pbo_lt", "min_independent_events", "romano_wolf_p_lt", "alpha_t_stat_gte"] {
        assert_eq!(pt[k], st[k], "paper_v1 must not relax {k}");
    }

    // Both profiles declare their per-asset-class gate facts.
    let ac: Vec<(String, String, i32)> = sqlx::query_as(
        "SELECT profile_id, asset_class, jsonb_array_length(crisis_windows)
           FROM mlops.gate_profile_asset_class ORDER BY profile_id, asset_class",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(ac.len(), 4, "two profiles x two asset classes");
    for (_, _, windows) in &ac {
        assert!(*windows >= 2, "Gate 11 needs at least two crisis windows declared");
    }
}

/// AT-63: the platform-held seed is generated by the database and unreadable by
/// the agent role. A seed the agent can read is a seed it can tune against.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn the_platform_seed_is_database_generated_and_unreadable_by_agents() {
    let pg = scratch("self_monitor_seed").await;

    sqlx::query(
        "INSERT INTO mlops.campaign
             (slug, tenant_id, hypothesis, objective, benchmark, delta_practical, budget,
              exploration_floor, gates_profile, created_by)
         VALUES ('seeded','t','h',
                 '{\"maximize\":[\"sharpe_net\"],\"subject_to\":{}}'::jsonb, '{}'::jsonb, 0.25,
                 '{\"max_trials\":1,\"gpu_hours\":1,\"usd\":1,\"wall_clock_hours\":1}'::jsonb,
                 0.05, 'strict_v1', 't')",
    )
    .execute(&pg)
    .await
    .expect("campaign insert without a seed");

    let seed: i64 = sqlx::query_scalar("SELECT platform_seed FROM mlops.campaign WHERE slug='seeded'")
        .fetch_one(&pg)
        .await
        .unwrap();
    assert!(seed > 0, "the database supplies a seed nobody asked for");

    // Two campaigns get two seeds: a seed reused across campaigns is learnable.
    sqlx::query(
        "INSERT INTO mlops.campaign
             (slug, tenant_id, hypothesis, objective, benchmark, delta_practical, budget,
              exploration_floor, gates_profile, created_by)
         VALUES ('seeded2','t','h',
                 '{\"maximize\":[\"sharpe_net\"],\"subject_to\":{}}'::jsonb, '{}'::jsonb, 0.25,
                 '{\"max_trials\":1,\"gpu_hours\":1,\"usd\":1,\"wall_clock_hours\":1}'::jsonb,
                 0.05, 'strict_v1', 't')",
    )
    .execute(&pg)
    .await
    .unwrap();
    let seeds: Vec<i64> = sqlx::query_scalar("SELECT platform_seed FROM mlops.campaign ORDER BY slug")
        .fetch_all(&pg)
        .await
        .unwrap();
    assert_ne!(seeds[0], seeds[1], "each campaign draws its own seed");

    // The grant is the mechanism: agent_role may read its campaigns and not this
    // column. Checked against the catalogue rather than by attempting a read,
    // because `agent_role` is NOLOGIN.
    let granted: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.column_privileges
          WHERE table_schema = 'mlops' AND table_name = 'campaign'
            AND grantee = 'agent_role' AND privilege_type = 'SELECT'
          ORDER BY column_name",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    assert!(!granted.is_empty(), "agent_role still reads its campaigns");
    assert!(
        !granted.iter().any(|c| c == "platform_seed"),
        "agent_role must hold no grant on platform_seed; granted: {granted:?}"
    );
    for expected in ["slug", "hypothesis", "delta_practical", "exploration_floor"] {
        assert!(granted.iter().any(|c| c == expected), "agent_role lost {expected}");
    }
}
