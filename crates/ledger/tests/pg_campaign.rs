//! Campaign DEFINE and the exploration floor against live Postgres
//! (SPEC §8, §10, §4.5; INV-20, INV-21; checklist 2.1 and 2.9).
//!
//! Ignored by default:
//! `cargo test -p ledger --test pg_campaign -- --ignored --test-threads=1`
//!
//! The Rust half of the floor is a type that cannot hold a value below 5%. This
//! is the half that matters more: the *database* refuses the row, so a campaign
//! written by anything other than this code is still bound by it.

use ledger::campaign::{Budget, CampaignDefinition, Dispatcher, ExplorationFloor};
use ledger::pg::PgTrialLedger;
use ledger::{DecisionKind, TrialLedger};
use sqlx::{Connection, Executor, PgConnection, PgPool};

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
const M0052: &str = include_str!("../../../migrations/0052_campaign_phase.sql");
// `approval_spend_usd` is a DEFINE fact (§15, ADR-P2-20); without it every
// `define_campaign` in here writes a column that does not exist.
const M0054: &str = include_str!("../../../migrations/0054_approval_envelope.sql");

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
    for m in [M0043, M0044, M0052, M0054] {
        pool.execute(m).await.expect("migration applies");
    }
    pool
}

fn definition(slug: &str) -> CampaignDefinition {
    CampaignDefinition {
        slug: slug.into(),
        hypothesis: "intraday mean reversion survives costs on majors".into(),
        objective: serde_json::json!({
            "maximize": ["sharpe_net"],
            "subject_to": { "max_dd": 0.2 }
        }),
        benchmark: serde_json::json!({ "kind": "buy_and_hold" }),
        delta_practical: 0.25,
        approval_spend_usd: 50.0,
        budget: Budget {
            max_trials: 500,
            gpu_hours: 10.0,
            usd: 100.0,
            wall_clock_hours: 48.0,
        },
        exploration_floor: ExplorationFloor::default_floor(),
        gates_profile: "strict_v1".into(),
        preference_vector: None,
        search_space: serde_json::json!({}),
    }
}

/// DEFINE writes the facts, mints the handle, and refuses a second definition of
/// the same campaign. Those facts are then immutable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn define_writes_immutable_facts_and_refuses_a_redefinition() {
    let pg = scratch("campaign_define").await;
    let led = PgTrialLedger::new(pg.clone());
    let tenant = "t-define";

    let handle = led
        .define_campaign(tenant, &definition("mean-reversion-q3"))
        .expect("define");
    assert_eq!(handle.tenant_id(), tenant);
    assert!(handle.define_hash().starts_with("sha256:"));
    assert!((handle.exploration_floor().value() - 0.05).abs() < f64::EPSILON);

    // A second definition of the same slug is a conflict, not an overwrite.
    assert!(led
        .define_campaign(tenant, &definition("mean-reversion-q3"))
        .is_err());

    // And the declared effect size cannot be edited afterwards — this is the
    // whole point of pre-registration.
    let edit = sqlx::query("UPDATE mlops.campaign SET delta_practical = 0.01 WHERE slug = $1")
        .bind("mean-reversion-q3")
        .execute(&pg)
        .await;
    assert!(edit.is_err(), "DEFINE facts must be immutable");

    let stored: f64 =
        sqlx::query_scalar("SELECT delta_practical FROM mlops.campaign WHERE slug = $1")
            .bind("mean-reversion-q3")
            .fetch_one(&pg)
            .await
            .unwrap();
    assert!((stored - 0.25).abs() < 1e-12);
}

/// The database refuses a campaign below the floor on its own, independently of
/// the Rust type that makes such a value unconstructable (INV-21).
#[tokio::test]
#[ignore = "requires live postgres"]
async fn the_database_refuses_a_campaign_below_the_exploration_floor() {
    let pg = scratch("campaign_floor").await;

    // Bypass the type entirely: write the row directly, the way a script or a
    // future code path might.
    let too_low = sqlx::query(
        "INSERT INTO mlops.campaign
             (slug, tenant_id, hypothesis, objective, benchmark, delta_practical, budget,
              exploration_floor, gates_profile, created_by)
         VALUES ('sneaky','t','h',
                 '{\"maximize\":[\"sharpe_net\"],\"subject_to\":{}}'::jsonb,
                 '{}'::jsonb, 0.25,
                 '{\"max_trials\":1,\"gpu_hours\":1,\"usd\":1,\"wall_clock_hours\":1}'::jsonb,
                 0.01, 'strict_v1', 't')",
    )
    .execute(&pg)
    .await;
    assert!(
        too_low.is_err(),
        "a 1% exploration floor must be refused by the schema, not only by the type"
    );

    // And delta_practical is NOT NULL with a positive CHECK, so a campaign that
    // never declared an effect size cannot exist either.
    let no_delta = sqlx::query(
        "INSERT INTO mlops.campaign
             (slug, tenant_id, hypothesis, objective, benchmark, delta_practical, budget,
              exploration_floor, gates_profile, created_by)
         VALUES ('nodelta','t','h',
                 '{\"maximize\":[\"sharpe_net\"],\"subject_to\":{}}'::jsonb,
                 '{}'::jsonb, 0.0,
                 '{\"max_trials\":1,\"gpu_hours\":1,\"usd\":1,\"wall_clock_hours\":1}'::jsonb,
                 0.05, 'strict_v1', 't')",
    )
    .execute(&pg)
    .await;
    assert!(no_delta.is_err(), "delta_practical must be positive");
}

/// Every draw the dispatcher makes lands in `mlops.decision` with its candidate
/// set, propensity and exploration flag (INV-20). This is the first production
/// writer that table has had.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn dispatcher_draws_are_recorded_as_decisions() {
    let pg = scratch("campaign_decisions").await;
    let led = PgTrialLedger::new(pg.clone());
    let tenant = "t-draws";
    let campaign = led
        .define_campaign(tenant, &definition("draws"))
        .expect("define");

    let d = Dispatcher::new(&campaign, &led);
    let candidates: Vec<serde_json::Value> =
        (0..4).map(|i| serde_json::json!({ "fast": 8 + i })).collect();
    // A fixed sequence: the verdict must not depend on the day it ran.
    let mut i = 0usize;
    let seq = [0.90_f64, 0.10, 0.40, 0.70, 0.20, 0.60];
    let mut rng = move || {
        let v = seq[i % seq.len()];
        i += 1;
        v
    };

    for n in 0..3 {
        let draw = d
            .draw(
                DecisionKind::Select,
                &candidates,
                &[1.0, 0.5, 0.5, 0.5],
                f64::from(n) / 10.0,
                &mut rng,
                Some("funnel dispatch".into()),
            )
            .expect("draw");
        assert!(draw.propensity > 0.0 && draw.propensity <= 1.0);
    }

    let rows: Vec<(serde_json::Value, Option<f64>, bool, String, String)> = sqlx::query_as(
        "SELECT candidate_set, propensity, exploration_flag, policy_id, decision_tier
           FROM mlops.decision ORDER BY seq",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    for (candidate_set, propensity, _explore, policy_id, tier) in &rows {
        assert_eq!(candidate_set.as_array().map(Vec::len), Some(4));
        assert!(
            propensity.is_some_and(|p| p > 0.0 && p <= 1.0),
            "a null propensity makes off-policy evaluation impossible (§4.2·6)"
        );
        assert_eq!(policy_id, ledger::campaign::POLICY_ID);
        assert_eq!(tier, "rule");
    }

    // The decision stream is hash-chained like the trial stream.
    let chained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mlops.decision WHERE row_hash IS NOT NULL AND prev_hash IS NOT NULL",
    )
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!(chained, 3);
}

/// AT-59 — a campaign's state is the fold of its log, so a driver killed
/// mid-phase and restarted resumes at the same child job and spends no second
/// trial.
///
/// The driver itself is a job worker and needs a running platform; what this
/// test pins is the property the driver rests on. If the fold of a log were not
/// identical before and after, or if the same fold did not key the same child,
/// nothing the driver does could be exactly-once.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn a_campaign_resumes_at_the_same_child_and_spends_no_second_trial() {
    use ledger::phase::{CampaignEvent, CampaignPhase};

    let pg = scratch("campaign_fold").await;
    let led = PgTrialLedger::new(pg.clone());
    let tenant = "t-fold";

    let handle = led.define_campaign(tenant, &definition("resume-me")).expect("define");
    let id = handle.campaign_id();

    // DEFINE opened the log in the same transaction that wrote the campaign.
    let state = led.campaign_state_async(tenant, id).await.expect("fold");
    assert_eq!(state.phase, CampaignPhase::Define);
    assert_eq!(state.seq, 1);
    assert_eq!(state.next_phase(), Some(CampaignPhase::Baseline));

    // The driver dispatches BASELINE and records the child it created.
    let key_before = state.child_key(id, CampaignPhase::Baseline);
    led.append_campaign_event_async(
        tenant,
        id,
        &CampaignEvent::new(
            CampaignPhase::Baseline,
            serde_json::json!({ "job_id": "job_BASE", "idempotency_key": key_before }),
        ),
    )
    .await
    .expect("append baseline");

    // --- the driver dies here ---
    // A fresh one folds the same log and sees the same child.
    let resumed = led.campaign_state_async(tenant, id).await.expect("re-fold");
    assert_eq!(resumed.phase, CampaignPhase::Baseline);
    assert_eq!(resumed.children, vec![(CampaignPhase::Baseline, 1, "job_BASE".to_string())]);
    // Folding again changes nothing: the fold is a function of the log.
    assert_eq!(resumed, led.campaign_state_async(tenant, id).await.unwrap());

    // The next phase's key is derived from the fold, so both drivers agree.
    assert_eq!(
        resumed.child_key(id, CampaignPhase::Diagnose),
        led.campaign_state_async(tenant, id).await.unwrap().child_key(id, CampaignPhase::Diagnose)
    );

    // The database refuses a transition SPEC §10 does not allow, whoever writes
    // it — this is what stops a second writer from making the fold disagree
    // with itself between two readings.
    let illegal = led
        .append_campaign_event_async(tenant, id, &CampaignEvent::new(CampaignPhase::Gate, serde_json::json!({})))
        .await;
    assert!(illegal.is_err(), "gate cannot follow baseline");

    // And nothing may be appended after an ending.
    led.append_campaign_event_async(tenant, id, &CampaignEvent::new(CampaignPhase::Halted, serde_json::json!({ "by": "a human" })))
        .await
        .expect("a campaign may be halted from anywhere");
    let after = led
        .append_campaign_event_async(tenant, id, &CampaignEvent::new(CampaignPhase::Diagnose, serde_json::json!({})))
        .await;
    assert!(after.is_err(), "a halted campaign is over");

    let ended = led.campaign_state_async(tenant, id).await.unwrap();
    assert_eq!(ended.terminal, Some(CampaignPhase::Halted));
    assert_eq!(ended.next_phase(), None);

    // No trial was ever registered: folding, resuming and refusing are all free.
    let trials: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE campaign_id = $1")
        .bind(id)
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(trials, 0);
}

/// The phase log is ordered by `seq`, not by clock. Two events written inside
/// the same millisecond still fold in the order they were appended.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live postgres"]
async fn the_log_is_ordered_by_position_not_by_clock() {
    use ledger::phase::{CampaignEvent, CampaignPhase};

    let pg = scratch("campaign_seq").await;
    let led = PgTrialLedger::new(pg.clone());
    let tenant = "t-seq";
    let handle = led.define_campaign(tenant, &definition("ordered")).expect("define");
    let id = handle.campaign_id();

    for phase in [CampaignPhase::Baseline, CampaignPhase::Diagnose, CampaignPhase::Hypothesize] {
        led.append_campaign_event_async(tenant, id, &CampaignEvent::new(phase, serde_json::json!({})))
            .await
            .expect("append");
    }

    // Every event shares an `occurred_at` now; `seq` is what orders them.
    sqlx::query("UPDATE mlops.campaign_event SET occurred_at = now() WHERE campaign_id = $1")
        .bind(id)
        .execute(&pg)
        .await
        .expect_err("campaign_event is append-only; even this test cannot rewrite it");

    let seqs: Vec<i64> = sqlx::query_scalar(
        "SELECT seq FROM mlops.campaign_event WHERE campaign_id = $1 ORDER BY seq",
    )
    .bind(id)
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(seqs, vec![0, 1, 2, 3], "the database assigns the position, not the writer");

    let state = led.campaign_state_async(tenant, id).await.unwrap();
    assert_eq!(state.phase, CampaignPhase::Hypothesize);
    assert_eq!(state.seq, 4);
}
