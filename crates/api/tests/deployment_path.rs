//! Set T Phase 1 and the parts of Phases 2 and 3 that need a database: the
//! deployment path (T-1.1, T-1.2, T-1.4, T-1.5), the fleet's ceilings (T-2.1),
//! and the ramp's review periods and step-down (T-3.1 to T-3.3).
//!
//! Every test makes its own scratch database, applies **every** migration, and
//! drops it. They are `#[ignore]`d because they need a Postgres that can create
//! databases:
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test deployment_path -- --ignored --test-threads=2
//! ```
//!
//! The modules under test are compiled in by path so that this file exercises the
//! exact source the library builds (they name no `crate::` item except each
//! other, so the root of this test crate is a valid home for them).

#![allow(dead_code)]

// The routes file names `crate::auth` and `crate::state`; these re-exports let
// it compile here exactly as it does inside the library.
mod auth {
    pub use api::auth::*;
}
mod state {
    pub use api::state::*;
}
#[path = "../src/deployment_gate.rs"]
mod deployment_gate;
#[path = "../src/deployment_gate_routes.rs"]
mod deployment_gate_routes;
#[path = "../src/legacy_marker.rs"]
mod legacy_marker;
#[path = "../src/ramp_worker.rs"]
mod ramp_worker;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use deployment_gate::{GateError, ProposerKind, HUMAN_SCOPE};
use ledger::deployment_path::{DivergenceThresholds, GateVerdictEntry, ProposalDraft};
use ledger::ramp::{Outcome, Size};
use ledger::ramp_gate::StepThresholds;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

// ── scratch database ─────────────────────────────────────────────────────────

fn base_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL: a cluster that can create databases")
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

struct Scratch {
    pg: PgPool,
    name: String,
}

impl Scratch {
    async fn new() -> Self {
        let name = format!("deploy_path_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
            .await
            .expect("postgres reachable");
        admin
            .execute(format!("CREATE DATABASE {name}").as_str())
            .await
            .unwrap();
        let pg = PgPool::connect(&with_db(&base_url(), &name)).await.unwrap();
        // Not `storage::postgres::run_migrations`: this scratch cluster has no
        // pgvector, so migrations that need it (0055 and what builds on it) cannot
        // apply. Each migration runs in its own transaction and one that fails is
        // skipped and reported; the three this file tests must all apply, and
        // anything they depend on is exercised by them failing if it did not.
        let m = sqlx::migrate!("../../migrations");
        let mut skipped: Vec<(i64, String)> = Vec::new();
        for mig in m.iter() {
            if mig.migration_type.is_down_migration() {
                continue;
            }
            let mut tx = pg.begin().await.unwrap();
            match tx.execute(&*mig.sql).await {
                Ok(_) => tx.commit().await.unwrap(),
                Err(e) => {
                    tx.rollback().await.ok();
                    skipped.push((mig.version, e.to_string()));
                }
            }
        }
        for mine in [180_i64, 181, 182] {
            assert!(
                !skipped.iter().any(|(v, _)| *v == mine),
                "migration {mine} did not apply: {skipped:?}"
            );
        }
        if !skipped.is_empty() {
            eprintln!(
                "scratch database skipped (no pgvector?): {:?}",
                skipped.iter().map(|(v, _)| *v).collect::<Vec<_>>()
            );
        }
        Self { pg, name }
    }

    async fn drop_db(self) {
        self.pg.close().await;
        let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
            .await
            .unwrap();
        admin
            .execute(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name).as_str())
            .await
            .unwrap();
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────────

fn rev(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn human() -> Vec<String> {
    vec![HUMAN_SCOPE.to_owned()]
}

fn draft(revision: &str, instrument: &str, capital: &str) -> ProposalDraft {
    ProposalDraft {
        strategy_name: format!("strat-{instrument}"),
        revision_hash: revision.to_owned(),
        instrument_id: instrument.to_owned(),
        profile_id: "paper_v1".to_owned(),
        verdict_revision_hash: revision.to_owned(),
        verdict: (1..=16)
            .map(|n| GateVerdictEntry { gate_no: n, passed: true, detail: None })
            .collect(),
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 0.02,
            max_cum_return_divergence: 0.05,
            max_position_divergence: 0.0,
            min_signal_match: 0.9,
        },
        starting_capital: capital.to_owned(),
        review_period_days: 14,
    }
}

async fn arm(pg: &PgPool, name: &str) {
    sqlx::query("INSERT INTO deployment_gate_switch (name, armed_by, note) VALUES ($1, 'test', 'test')")
        .bind(name)
        .execute(pg)
        .await
        .unwrap();
}

async fn declare_ceilings(pg: &PgPool, concurrent: &str, aggregate: &str, instrument: &str) {
    for (n, v) in [
        ("max_concurrent", concurrent),
        ("max_aggregate_capital", aggregate),
        ("max_instrument_exposure", instrument),
    ] {
        deployment_gate::declare_ceiling(pg, &human(), "human:test", n, v, "test")
            .await
            .unwrap();
    }
}

async fn raw_deployment(
    pg: &PgPool,
    id: Uuid,
    revision: &str,
    user: &str,
    instrument: &str,
    capital: &str,
    started_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json, instrument_id,
            environment, venue, starting_capital, model_pins, started_by, started_at)
         VALUES ($1,$2,$3,$4,'{}'::jsonb,$5,'paper','platform_paper',$6::numeric,'[]'::jsonb,$2,$7)",
    )
    .bind(id)
    .bind(user)
    .bind(format!("strat-{instrument}"))
    .bind(revision)
    .bind(instrument)
    .bind(capital)
    .bind(started_at)
    .execute(pg)
    .await
    .map(|_| ())
}

/// A deployment with a forward recorder (`forward_deployment_id` set at insert; the
/// column is immutable afterwards). The recorder row itself is not needed: the
/// review reads `mlops.forward_observation` by this id and finds none, which is a
/// *measured but unobserved* deployment rather than an unmeasured one (ADR-T3-04).
async fn raw_measured_deployment(
    pg: &PgPool,
    id: Uuid,
    revision: &str,
    user: &str,
    instrument: &str,
    capital: &str,
    started_at: DateTime<Utc>,
) -> Uuid {
    let forward = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json, instrument_id,
            environment, venue, starting_capital, model_pins, started_by, started_at,
            forward_deployment_id)
         VALUES ($1,$2,$3,$4,'{}'::jsonb,$5,'paper','platform_paper',$6::numeric,'[]'::jsonb,$2,$7,$8)",
    )
    .bind(id)
    .bind(user)
    .bind(format!("strat-{instrument}"))
    .bind(revision)
    .bind(instrument)
    .bind(capital)
    .bind(started_at)
    .bind(forward)
    .execute(pg)
    .await
    .unwrap();
    forward
}

/// A proposal, approved by a person.
async fn approved(pg: &PgPool, user: &str, d: ProposalDraft) -> Uuid {
    let id = deployment_gate::create_proposal(pg, user, "agent:run-1", ProposerKind::Agent, d, None)
        .await
        .unwrap();
    deployment_gate::decide_proposal(pg, &human(), "human:test", id, true, "ok", None)
        .await
        .unwrap();
    id
}

fn err_text(e: &sqlx::Error) -> String {
    e.to_string()
}

// ── T-1.1: one door, a person at it, an envelope behind it ───────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn a_deployment_exists_only_with_a_decided_proposal_behind_it() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    declare_ceilings(pg, "10", "1000000", "1000000").await;
    let t0 = Utc::now();

    // Before the switch is armed the door is open (ADR-T1-01): a route that has
    // not been taught to ask still works.
    raw_deployment(pg, Uuid::new_v4(), &rev('a'), "alice", "PRE", "100", t0)
        .await
        .expect("unarmed: no proposal required yet");

    arm(pg, "deployments_require_a_proposal").await;

    // Armed: no proposal, no deployment, whoever writes the row.
    let e = raw_deployment(pg, Uuid::new_v4(), &rev('a'), "alice", "BTC", "100", t0)
        .await
        .unwrap_err();
    assert!(err_text(&e).contains("no decided proposal"), "{e}");

    // A decided proposal opens exactly one deployment.
    let pid = approved(pg, "alice", draft(&rev('a'), "BTC", "1000")).await;
    let dep = Uuid::new_v4();
    deployment_gate::authorise_deployment(pg, &human(), "alice", dep, pid)
        .await
        .unwrap();
    raw_deployment(pg, dep, &rev('a'), "alice", "BTC", "1000", t0)
        .await
        .expect("a decided proposal is the door");

    // A decision is spent once.
    let again = deployment_gate::authorise_deployment(pg, &human(), "alice", Uuid::new_v4(), pid)
        .await
        .unwrap_err();
    assert!(matches!(again, GateError::AlreadyUsed), "{again}");

    // It cannot be spent by somebody else's session.
    let pid2 = approved(pg, "alice", draft(&rev('b'), "ETH", "1000")).await;
    let other = deployment_gate::authorise_deployment(pg, &human(), "mallory", Uuid::new_v4(), pid2)
        .await
        .unwrap_err();
    assert!(matches!(other, GateError::WrongOwner), "{other}");

    // And what it authorises is what it said: one revision, one owner, at most
    // its capital, its instrument.
    let dep2 = Uuid::new_v4();
    deployment_gate::authorise_deployment(pg, &human(), "alice", dep2, pid2)
        .await
        .unwrap();
    for (who, revision, instrument, capital, needle) in [
        ("alice", rev('c'), "ETH", "1000", "approved proposal is for revision"),
        ("mallory", rev('b'), "ETH", "1000", "belongs to another user"),
        ("alice", rev('b'), "ETH", "5000", "exceeds the approved"),
        ("alice", rev('b'), "SOL", "1000", "approved proposal is for"),
    ] {
        let e = raw_deployment(pg, dep2, &revision, who, instrument, capital, t0)
            .await
            .unwrap_err();
        assert!(err_text(&e).contains(needle), "{needle}: {e}");
    }

    // A rejected decision authorises nothing: the composite foreign key names
    // an APPROVED decision or none.
    let rejected = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('d'), "ADA", "10"), None,
    )
    .await
    .unwrap();
    deployment_gate::decide_proposal(pg, &human(), "human:test", rejected, false, "no", None)
        .await
        .unwrap();
    let e = deployment_gate::authorise_deployment(pg, &human(), "alice", Uuid::new_v4(), rejected)
        .await
        .unwrap_err();
    assert!(matches!(e, GateError::NotApproved), "{e}");

    // A selection's members run under their parent's authority, and a member
    // of a parent with none has nothing to inherit.
    let member = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json, instrument_id,
            environment, venue, starting_capital, model_pins, started_by, parent_deployment_id)
         VALUES ($1,'alice','m',$2,'{}','SOLM','paper','platform_paper',1,'[]','alice',$3)",
    )
    .bind(member)
    .bind(rev('a'))
    .bind(dep)
    .execute(pg)
    .await
    .expect("a member of an authorised parent");
    let orphan = sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json, instrument_id,
            environment, venue, starting_capital, model_pins, started_by, parent_deployment_id)
         VALUES ($1,'alice','m',$2,'{}','SOLN','paper','platform_paper',1,'[]','alice',$3)",
    )
    .bind(Uuid::new_v4())
    .bind(rev('a'))
    .bind(Uuid::new_v4())
    .execute(pg)
    .await;
    assert!(orphan.is_err());

    s.drop_db().await;
}

/// Mutation check for the test above: with the trigger dropped the same raw
/// insert succeeds, so the test is about the trigger and nothing else.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn without_the_trigger_the_door_is_open() {
    let s = Scratch::new().await;
    arm(&s.pg, "deployments_require_a_proposal").await;
    sqlx::query("DROP TRIGGER trg_strategy_deployment_requires_authority ON strategy_deployment")
        .execute(&s.pg)
        .await
        .unwrap();
    raw_deployment(&s.pg, Uuid::new_v4(), &rev('a'), "alice", "BTC", "1", Utc::now())
        .await
        .expect("the mutation: nothing refuses it");
    s.drop_db().await;
}

/// No agent token can decide or deploy, proven per scope rather than per route:
/// every scope an agent session can hold, and the monitor's.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn no_scope_an_agent_can_hold_decides_or_authorises_a_deployment() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let pid = deployment_gate::create_proposal(
        pg, "alice", "agent:run", ProposerKind::Agent, draft(&rev('a'), "BTC", "10"), None,
    )
    .await
    .unwrap();
    let mut scopes: Vec<&str> = vec![
        "research:data.read", "research:features", "research:jobs", "research:artifacts",
        "research:memory", "research:report", "llm:proxy", "monitor.control",
        "orders.place", "automations.arm", "models.promote", "skills.admit", "web",
        "web:full:impostor",
    ];
    scopes.sort_unstable();
    for scope in scopes {
        let held = vec![scope.to_owned()];
        assert!(!deployment_gate::may_decide(&held), "{scope}");
        let d = deployment_gate::decide_proposal(pg, &held, "x", pid, true, "", None)
            .await
            .unwrap_err();
        assert!(matches!(d, GateError::NotHuman), "{scope}: {d}");
        let a = deployment_gate::authorise_deployment(pg, &held, "alice", Uuid::new_v4(), pid)
            .await
            .unwrap_err();
        assert!(matches!(a, GateError::NotHuman), "{scope}: {a}");
        let c = deployment_gate::declare_ceiling(pg, &held, "x", "max_concurrent", "9", "")
            .await
            .unwrap_err();
        assert!(matches!(c, GateError::NotHuman), "{scope}: {c}");
        assert_eq!(ProposerKind::from_scopes(&held), ProposerKind::Agent);
    }
    assert!(deployment_gate::may_decide(&human()));
    assert_eq!(ProposerKind::from_scopes(&human()), ProposerKind::Human);
    // And the agent's creation did not decide anything.
    let decided: i64 = sqlx::query_scalar("SELECT count(*) FROM deployment_decision")
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(decided, 0);
    s.drop_db().await;
}

/// The decision's envelope records who decided, when and on what evidence, and
/// the evidence is checkable against the stored proposal.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn the_envelope_records_who_when_and_on_what_evidence() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    declare_ceilings(pg, "5", "100000", "100000").await;
    let pid = approved(pg, "alice", draft(&rev('a'), "BTC", "1000")).await;
    let (by, kind, env): (String, String, serde_json::Value) = sqlx::query_as(
        "SELECT decided_by, decider_kind, envelope FROM deployment_decision WHERE proposal_id = $1",
    )
    .bind(pid)
    .fetch_one(pg)
    .await
    .unwrap();
    assert_eq!(by, "human:test");
    assert_eq!(kind, "human");
    assert_eq!(env["decided_by"], "human:test");
    assert!(env["decided_at"].as_str().is_some());
    let (_, stored) = deployment_gate::load_proposal(pg, pid).await.unwrap();
    assert_eq!(
        env["evidence_hash"].as_str().unwrap(),
        ledger::deployment_path::evidence_hash(&stored),
        "the evidence is reproducible from the stored proposal"
    );
    // The row is immutable, and a machine cannot be the decider.
    assert!(sqlx::query("UPDATE deployment_decision SET note = 'x'")
        .execute(pg)
        .await
        .is_err());
    let forged = sqlx::query(
        "INSERT INTO deployment_decision (proposal_id, decision, decided_by, decider_kind, envelope)
         VALUES ($1,'approved','agent:x','agent','{}')",
    )
    .bind(Uuid::new_v4())
    .execute(pg)
    .await;
    assert!(forged.is_err());
    s.drop_db().await;
}

/// A verdict for one revision cannot authorise another, at creation, at
/// decision (the head moved), and in the table itself.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn a_stale_verdict_is_refused() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    declare_ceilings(pg, "5", "100000", "100000").await;

    let mut d = draft(&rev('a'), "BTC", "10");
    d.verdict_revision_hash = rev('b');
    let e = deployment_gate::create_proposal(pg, "alice", "alice", ProposerKind::Human, d, None)
        .await
        .unwrap_err();
    assert!(matches!(e, GateError::Refused(_)));
    assert_eq!(e.http().1, "stale_verdict");

    // Created fresh, but the head moved before the human decided.
    let pid = deployment_gate::create_proposal(
        pg, "alice", "agent:r", ProposerKind::Agent, draft(&rev('a'), "BTC", "10"), Some(&rev('a')),
    )
    .await
    .unwrap();
    let e = deployment_gate::decide_proposal(pg, &human(), "human:t", pid, true, "", Some(&rev('b')))
        .await
        .unwrap_err();
    assert_eq!(e.http().1, "stale_verdict", "{e}");

    // And created against a head that has already moved.
    let e = deployment_gate::create_proposal(
        pg, "alice", "agent:r", ProposerKind::Agent, draft(&rev('a'), "ETH", "10"), Some(&rev('c')),
    )
    .await
    .unwrap_err();
    assert_eq!(e.http().1, "stale_verdict");

    // The table refuses it whoever writes the row.
    let raw = sqlx::query(
        "INSERT INTO deployment_proposal
           (proposal_id, owner_user_id, proposed_by, proposer_kind, strategy_name, revision_hash,
            instrument_id, profile_id, verdict, verdict_revision_hash, gates_passed, thresholds,
            starting_capital, review_period_days)
         SELECT gen_random_uuid(), 'a','a','human','s',$1,'X','paper_v1', verdict, $2, 16, thresholds, 1, 1
           FROM deployment_proposal LIMIT 1",
    )
    .bind(rev('a'))
    .bind(rev('b'))
    .execute(pg)
    .await;
    assert!(raw.is_err());
    s.drop_db().await;
}

/// T-1.2 in the schema: no `venue_class` but paper exists on a proposal or an
/// authorisation, whoever writes the row.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn paper_v1_cannot_authorise_capital_in_the_schema() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    declare_ceilings(pg, "5", "100000", "100000").await;
    let pid = approved(pg, "alice", draft(&rev('a'), "BTC", "10")).await;
    let r = sqlx::query("UPDATE deployment_proposal SET venue_class = 'live'")
        .execute(pg)
        .await;
    assert!(r.is_err(), "a proposal is immutable, and `live` is not a venue class");
    let capital = sqlx::query(
        "INSERT INTO deployment_authorisation (deployment_id, proposal_id, venue_class) VALUES ($1,$2,'capital')",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(pg)
    .await;
    assert!(err_text(&capital.unwrap_err()).contains("venue_class"), "the CHECK names it");
    // The ramp: a paper_v1 step-up is refused by the review trigger.
    let dep = Uuid::new_v4();
    raw_deployment(pg, dep, &rev('a'), "alice", "BTC", "10", Utc::now() - Duration::days(40))
        .await
        .unwrap();
    let t0 = Utc::now() - Duration::days(30);
    sqlx::query("INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,1,$2,$3)")
        .bind(dep)
        .bind(t0)
        .bind(t0 + Duration::days(14))
        .execute(pg)
        .await
        .unwrap();
    let step_up = review_row(pg, dep, 1, "paper_v1", "step_up", true, 0, 0).await;
    assert!(
        err_text(&step_up.unwrap_err()).contains("does not authorise capital"),
        "a paper_v1 period is evidence, not a rung"
    );
    // strict_v1 may.
    review_row(pg, dep, 1, "strict_v1", "step_up", true, 0, 0).await.unwrap();
    let _ = pid;
    s.drop_db().await;
}

#[allow(clippy::too_many_arguments)]
async fn review_row(
    pg: &PgPool,
    dep: Uuid,
    period: i32,
    profile: &str,
    outcome: &str,
    passed: bool,
    before: i16,
    after: i16,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ramp_review
           (deployment_id, period_no, profile_id, outcome, passed, halvings_before, halvings_after,
            evidence, evidence_hash, reason, decided_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,'{}'::jsonb,$8,'test','test')",
    )
    .bind(dep)
    .bind(period)
    .bind(profile)
    .bind(outcome)
    .bind(passed)
    .bind(before)
    .bind(after)
    .bind("0".repeat(64))
    .execute(pg)
    .await
    .map(|_| ())
}

// ── T-2.1 ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn deploying_past_a_ceiling_is_refused_and_names_which_one_bound() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let p1 = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('a'), "BTC", "15000"), None,
    )
    .await
    .unwrap();

    // Migration 0190 seeds the three ceilings (ADR-T2-04). To exercise the other
    // half of the rule - that an undeclared ceiling refuses rather than permits -
    // start from the state before anybody declared them. The owner connection can
    // truncate; the platform's role cannot.
    sqlx::query("TRUNCATE deployment_ceiling").execute(pg).await.unwrap();

    // Nothing declared: refused, naming the first undeclared ceiling. No default
    // permits everything.
    let e = deployment_gate::decide_proposal(pg, &human(), "human:t", p1, true, "", None)
        .await
        .unwrap_err();
    assert!(matches!(e, GateError::Ceiling(_)), "{e}");
    assert!(e.to_string().contains("not been declared"), "{e}");

    declare_ceilings(pg, "2", "40000", "20000").await;
    deployment_gate::decide_proposal(pg, &human(), "human:t", p1, true, "", None)
        .await
        .expect("fits every ceiling");

    // The same revision on the same instrument: 15k + 15k = 30k > 20k, though
    // each is under it alone. Aggregated across instances, not per instance.
    let p2 = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('a'), "BTC", "15000"), None,
    )
    .await
    .unwrap();
    let e = deployment_gate::decide_proposal(pg, &human(), "human:t", p2, true, "", None)
        .await
        .unwrap_err();
    match &e {
        GateError::Ceiling(c) => assert_eq!(c.binding().as_str(), "max_instrument_exposure", "{e}"),
        other => panic!("{other}"),
    }

    // Aggregate capital: another revision, 30k, total would be 45k > 40k.
    let p3 = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('b'), "ETH", "30000"), None,
    )
    .await
    .unwrap();
    // (30k on one instrument is also over the 20k instrument ceiling; use two
    // proposals under it to isolate the aggregate.)
    let _ = p3;
    let p4 = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('b'), "ETH", "20000"), None,
    )
    .await
    .unwrap();
    deployment_gate::decide_proposal(pg, &human(), "human:t", p4, true, "", None)
        .await
        .expect("35k of 40k");
    let p5 = deployment_gate::create_proposal(
        pg, "alice", "alice", ProposerKind::Human, draft(&rev('c'), "SOL", "10000"), None,
    )
    .await
    .unwrap();
    let e = deployment_gate::decide_proposal(pg, &human(), "human:t", p5, true, "", None)
        .await
        .unwrap_err();
    match &e {
        GateError::Ceiling(c) => {
            // The count binds first: two approved decisions already hold two places.
            assert_eq!(c.binding().as_str(), "max_concurrent", "{e}");
        }
        other => panic!("{other}"),
    }
    // A refusal recorded nothing.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deployment_decision WHERE decision = 'approved'")
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(n, 2);
    // Rejecting is never refused by a ceiling.
    deployment_gate::decide_proposal(pg, &human(), "human:t", p5, false, "full", None)
        .await
        .unwrap();
    s.drop_db().await;
}

// ── T-1.4 ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn everything_running_carries_the_legacy_marker_and_no_gate_can_read_it() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    declare_ceilings(pg, "9", "1000000", "1000000").await;

    // Running before the path existed: a straggler the sweep finds.
    let old = Uuid::new_v4();
    raw_deployment(pg, old, &rev('a'), "alice", "OLD", "100", Utc::now()).await.unwrap();
    let stopped = Uuid::new_v4();
    raw_deployment(pg, stopped, &rev('a'), "alice", "GONE", "100", Utc::now()).await.unwrap();
    sqlx::query("UPDATE strategy_deployment SET stopped_at = now(), stop_reason = 'x' WHERE deployment_id = $1")
        .bind(stopped)
        .execute(pg)
        .await
        .unwrap();
    assert_eq!(legacy_marker::mark_unmarked(pg).await.unwrap(), 1, "the running one, not the stopped one");
    assert_eq!(legacy_marker::mark_unmarked(pg).await.unwrap(), 0, "idempotent");

    // One with authority is not legacy, and the marker would lie about it.
    arm(pg, "deployments_require_a_proposal").await;
    let pid = approved(pg, "alice", draft(&rev('a'), "NEW", "100")).await;
    let new = Uuid::new_v4();
    deployment_gate::authorise_deployment(pg, &human(), "alice", new, pid).await.unwrap();
    raw_deployment(pg, new, &rev('a'), "alice", "NEW", "100", Utc::now()).await.unwrap();
    assert_eq!(legacy_marker::mark_unmarked(pg).await.unwrap(), 0);
    let lie = sqlx::query("INSERT INTO deployment_legacy_marker (deployment_id, basis) VALUES ($1,'x')")
        .bind(new)
        .execute(pg)
        .await;
    assert!(err_text(&lie.unwrap_err()).contains("is not legacy_ungated"));

    // The surface shows the marker beside the numbers.
    let rows = deployment_gate::overview(pg, "alice").await.unwrap();
    let find = |id: Uuid| rows.iter().find(|r| r["deployment_id"] == id.to_string()).unwrap().clone();
    assert_eq!(find(old)["legacy_ungated"], true);
    assert_eq!(find(new)["legacy_ungated"], false);
    assert_eq!(find(new)["profile_id"], "paper_v1");
    assert_eq!(find(new)["gates_passed"], 16);
    assert_eq!(find(old)["profile_id"], serde_json::Value::Null);

    // The marker is a fact, never an input: nothing that evaluates a strategy
    // holds a grant on it. (As the platform's role it reads and writes.)
    for role in ["agent_role", "backtest_role", "internal_ml_role"] {
        let can: bool = sqlx::query_scalar("SELECT has_table_privilege($1, 'deployment_legacy_marker', 'SELECT')")
            .bind(role)
            .fetch_one(pg)
            .await
            .unwrap();
        assert!(!can, "{role} can read legacy markers");
    }
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deployment_legacy_marker")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 1);
    tx.rollback().await.unwrap();
    s.drop_db().await;
}

/// The marker is also unreachable from the source side. Two scans, because there
/// are two things to keep apart:
///
/// * the **table** (`deployment_legacy_marker`) is named by no gate, ledger, risk
///   or engine source;
/// * the **word** `legacy_ungated` appears in exactly one place outside the API —
///   `backtest::stats::compare::ComparisonFlags`, a display qualifier that already
///   existed (ADR-P2-27: "shown beside the number") — and in no gate evaluator,
///   no gate profile, and nothing under `ledger` or `risk`.
#[test]
fn no_gate_or_evaluator_source_names_the_legacy_marker() {
    fn scan(dir: &std::path::Path, needle: &str, hits: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if e.file_name() != "target" {
                    scan(&p, needle, hits);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                let src = std::fs::read_to_string(&p).unwrap_or_default();
                if src.contains(needle) {
                    hits.push(p.display().to_string().replace('\\', "/"));
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut table = Vec::new();
    for d in [
        "crates/backtest/src",
        "crates/ledger/src",
        "crates/risk/src",
        "crates/strategy-engine/src",
        "crates/features/src",
    ] {
        scan(&root.join(d), "deployment_legacy_marker", &mut table);
    }
    assert!(table.is_empty(), "a gate or evaluator reads the marker table: {table:?}");

    let mut word = Vec::new();
    for d in [
        "crates/backtest/src",
        "crates/ledger/src",
        "crates/risk/src",
        "crates/strategy-engine/src",
        "crates/features/src",
    ] {
        scan(&root.join(d), "legacy_ungated", &mut word);
    }
    assert!(
        word.iter().all(|p| p.ends_with("crates/backtest/src/stats/compare.rs")),
        "`legacy_ungated` appears outside the one display qualifier: {word:?}"
    );
}

// ── T-1.5 ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn once_armed_no_new_trial_registers_as_legacy_unlogged_and_history_stays() {
    use ledger::{pg::PgTrialLedger, ActorKind, DispatchContext, Registration, TrialSubject};
    let s = Scratch::new().await;
    let l = PgTrialLedger::new(s.pg.clone());
    let subject = |n: u64| TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({ "n": n }),
        dataset_id: "slice:sha256:abc".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1".into(),
        seed_set: vec![1],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: Some(1),
    };
    let ctx = DispatchContext::legacy("tenant-a", ActorKind::Human, "ui");
    let s1 = subject(1);
    l.register_async(&Registration::unlogged(&ctx, &s1))
        .await
        .expect("before the switch: the legacy path still works");

    arm(&s.pg, "legacy_unlogged_retired").await;
    let s2 = subject(2);
    let e = l.register_async(&Registration::unlogged(&ctx, &s2)).await.unwrap_err();
    assert!(e.to_string().contains("legacy_unlogged is retired"), "{e}");

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE policy_id = 'legacy_unlogged'")
        .fetch_one(&s.pg)
        .await
        .unwrap();
    assert_eq!(n, 1, "the existing row is untouched, and nothing new was written");
    // The switch cannot be turned back off, by anyone.
    assert!(sqlx::query("DELETE FROM deployment_gate_switch").execute(&s.pg).await.is_err());
    assert!(sqlx::query("UPDATE deployment_gate_switch SET note = 'x'").execute(&s.pg).await.is_err());
    s.drop_db().await;
}

// ── grants (the owner connection cannot see a missing one) ───────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn the_platforms_role_can_use_the_path_and_cannot_arm_itself() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    for table in [
        "deployment_proposal", "deployment_decision", "deployment_authorisation",
        "deployment_legacy_marker", "deployment_gate_switch", "deployment_ceiling",
        "current_deployment_ceiling", "ramp_period", "ramp_review", "deployment_size",
    ] {
        sqlx::query(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("app_role cannot read {table}: {e}"));
    }
    // Writes the platform does (a savepoint per probe: a permission error aborts
    // the transaction).
    for (sql, allowed) in [
        ("INSERT INTO deployment_ceiling (name, value, declared_by) VALUES ('max_concurrent', 1, 't')", true),
        ("INSERT INTO deployment_gate_switch (name, armed_by, note) VALUES ('deployments_require_a_proposal','t','t')", false),
        ("INSERT INTO deployment_ceiling (name, value, declared_by) VALUES ('nonsense', 1, 't')", false),
    ] {
        sqlx::query("SAVEPOINT p").execute(&mut *tx).await.unwrap();
        let r = sqlx::query(sql).execute(&mut *tx).await;
        assert_eq!(r.is_ok(), allowed, "{sql}: {r:?}");
        sqlx::query("ROLLBACK TO SAVEPOINT p").execute(&mut *tx).await.unwrap();
    }
    tx.rollback().await.unwrap();
    // The research agent reads proposals and nothing it could use to decide one.
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE agent_role").execute(&mut *tx).await.unwrap();
    assert!(sqlx::query("SELECT count(*) FROM deployment_proposal").fetch_one(&mut *tx).await.is_ok());
    sqlx::query("SAVEPOINT p").execute(&mut *tx).await.unwrap();
    assert!(sqlx::query("SELECT count(*) FROM deployment_decision").fetch_one(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    s.drop_db().await;
}

// ── T-1.3: stopping ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn stop_all_is_the_kill_switch_action_and_stopping_one_halts_its_scope() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let kill = risk::KillSwitch::new(false);
    let store = monitor::store::MonitorStore::new(pg.clone());

    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    raw_deployment(pg, a, &rev('a'), "alice", "AAA", "1", Utc::now()).await.unwrap();
    raw_deployment(pg, b, &rev('a'), "alice", "BBB", "1", Utc::now()).await.unwrap();

    // One: its own scope halts at the gate now, and the record closes.
    let r = deployment_gate::revoke_one(pg, &kill, a, "human:t", "test stop").await.unwrap();
    assert!(r.closed);
    assert!(kill.refuses(None, Some(&a.to_string())), "the gate refuses its orders before the next tick");
    assert!(!kill.refuses(None, Some(&b.to_string())), "and only its own");
    assert!(store.halted_scopes().await.unwrap().contains(&risk::Scope::strategy(a.to_string())));

    // All: the kill switch's own action — the global scope, in the monitor's
    // table and in the gate's mirror — and every record closed.
    let stopped = deployment_gate::revoke_all(pg, &kill, "human:t", "test stop all").await.unwrap();
    assert_eq!(stopped, vec![b]);
    assert!(kill.is_active(), "the global kill switch is tripped");
    assert!(kill.refuses(Some("any"), Some("anything")));
    assert!(store.halted_scopes().await.unwrap().contains(&risk::Scope::Global));
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_deployment WHERE stopped_at IS NULL")
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(open, 0);

    // An agent is not an actor the monitor accepts.
    let e = deployment_gate::revoke_one(pg, &kill, a, "agent:run", "x").await.unwrap_err();
    assert!(matches!(e, GateError::Monitor(_)), "{e}");
    s.drop_db().await;
}

// ── T-3.x: the review worker ─────────────────────────────────────────────────

#[derive(Default)]
struct Recorder(Mutex<Vec<(Uuid, Size)>>);

#[async_trait]
impl ramp_worker::RampEffects for Arc<Recorder> {
    async fn apply_size(&self, id: Uuid, size: Size, _reason: &str) -> Result<(), String> {
        self.0.lock().unwrap().push((id, size));
        Ok(())
    }
}

fn policy() -> ramp_worker::RampPolicy {
    ramp_worker::RampPolicy {
        legacy_review_period_days: 14,
        thresholds: StepThresholds {
            min_observed_bars: 20,
            paper_signal_match_gte: 0.9,
            paper_slippage_ratio_lte: 1.5,
            paper_turnover_tolerance: 0.25,
            paper_model_rejects_max: 0,
        },
    }
}

async fn halvings(pg: &PgPool, dep: Uuid) -> Vec<(i32, String, i16)> {
    sqlx::query_as(
        "SELECT period_no, outcome, halvings_after FROM ramp_review WHERE deployment_id = $1 ORDER BY period_no",
    )
    .bind(dep)
    .fetch_all(pg)
    .await
    .unwrap()
}

/// T-3.3 — a deployment whose recorder observed nothing is halved at each recorded
/// boundary and is zero after four periods; the schedule chains from recorded ends.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn a_failing_deployment_is_halved_each_period_and_reaches_zero() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let rec = Arc::new(Recorder::default());
    let now = Utc::now();

    // 30 days old, 14-day periods: periods 1 and 2 have ended, 3 has not.
    let dep = Uuid::new_v4();
    raw_measured_deployment(pg, dep, &rev('a'), "alice", "AAA", "100", now - Duration::days(30)).await;
    sqlx::query("INSERT INTO deployment_legacy_marker (deployment_id, basis, marked_at) VALUES ($1,'t',$2)")
        .bind(dep)
        .bind(now - Duration::days(30))
        .execute(pg)
        .await
        .unwrap();

    let report = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        halvings(pg, dep).await,
        vec![(1, "halve".into(), 1), (2, "halve".into(), 2)],
        "a recorder with nothing observed in the period: not computable, which steps down like a failure"
    );
    assert_eq!(rec.0.lock().unwrap().last(), Some(&(dep, Size::from_halvings(2).unwrap())));
    let periods: Vec<(i32, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT period_no, starts_at, ends_at FROM ramp_period WHERE deployment_id = $1 ORDER BY period_no",
    )
    .bind(dep)
    .fetch_all(pg)
    .await
    .unwrap();
    assert_eq!(periods.len(), 3);
    assert_eq!(periods[1].1, periods[0].2, "each period starts where the last one ended");
    assert_eq!(periods[2].1, periods[1].2);
    assert!(periods[2].2 > now, "the third has not ended and is not reviewed");

    // Idempotent: a second pass at the same moment changes no record.
    let again = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert!(again.reviewed.is_empty());
    assert_eq!(halvings(pg, dep).await.len(), 2);
    // A deployment neglected for 70 days (five 14-day periods; the first four
    // have ended): one pass catches up, every period judged on its own window,
    // and the fourth failing one takes it to zero. The schedule chains from the
    // recorded ends, not from when the worker ran.
    let old = Uuid::new_v4();
    raw_measured_deployment(pg, old, &rev('b'), "alice", "OLD", "100", now - Duration::days(70)).await;
    sqlx::query("INSERT INTO deployment_legacy_marker (deployment_id, basis, marked_at) VALUES ($1,'t',$2)")
        .bind(old)
        .bind(now - Duration::days(70))
        .execute(pg)
        .await
        .unwrap();
    let late = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert!(late.errors.is_empty(), "{:?}", late.errors);
    let h = halvings(pg, old).await;
    assert_eq!(
        h.iter().map(|r| (r.1.as_str(), r.2)).collect::<Vec<_>>(),
        vec![("halve", 1), ("halve", 2), ("halve", 3), ("zero", 4)]
    );
    assert!(late.reviewed.iter().any(|(id, o)| *id == old && *o == Outcome::Zero));
    assert!(rec.0.lock().unwrap().contains(&(old, Size::ZERO)));
    let after_zero: i64 = sqlx::query_scalar("SELECT count(*) FROM ramp_period WHERE deployment_id = $1")
        .bind(old)
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(after_zero, 4, "no period opens after zero");
    // The reviews carry their stored evidence, and it re-hashes to its hash.
    let (ev, hash): (serde_json::Value, String) =
        sqlx::query_as("SELECT evidence, evidence_hash FROM ramp_review WHERE deployment_id = $1 AND period_no = 1")
            .bind(old)
            .fetch_one(pg)
            .await
            .unwrap();
    let inputs: ledger::ramp_gate::StepInputs = serde_json::from_value(ev["inputs"].clone()).unwrap();
    let evidence: ledger::ramp_gate::PeriodEvidence = serde_json::from_value(ev["evidence"].clone()).unwrap();
    assert_eq!(ledger::ramp_gate::evidence_hash(&inputs, &evidence), hash);
    let rejudged = ledger::ramp_gate::judge(&inputs, &evidence);
    assert_eq!(serde_json::to_value(&rejudged).unwrap(), ev["judgement"], "the verdict reproduces from its evidence");
    assert!(rejudged.not_computable_reason().is_some());
    // The first dep is untouched by the second's catch-up.
    assert_eq!(halvings(pg, dep).await.len(), 2);
    s.drop_db().await;
}

/// ADR-T3-04 — a deployment with **no** forward recorder cannot be measured, so it
/// is flagged on every pass and is not stepped down: no period opens, no review is
/// written, no size is applied. A measured one beside it is still reviewed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn an_unmeasured_deployment_is_flagged_and_not_halved() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let rec = Arc::new(Recorder::default());
    let now = Utc::now();

    let unmeasured = Uuid::new_v4();
    raw_deployment(pg, unmeasured, &rev('a'), "alice", "UNM", "100", now - Duration::days(60)).await.unwrap();
    let measured = Uuid::new_v4();
    raw_measured_deployment(pg, measured, &rev('b'), "alice", "MEA", "100", now - Duration::days(20)).await;

    let report = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.unmeasured, vec![unmeasured], "flagged, by id");
    let periods: i64 = sqlx::query_scalar("SELECT count(*) FROM ramp_period WHERE deployment_id = $1")
        .bind(unmeasured)
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(periods, 0, "no period is opened for a deployment nothing can measure");
    assert!(halvings(pg, unmeasured).await.is_empty(), "and none is halved");
    assert!(
        !rec.0.lock().unwrap().iter().any(|(id, _)| *id == unmeasured),
        "no size is applied to it"
    );
    // The measured one is reviewed as before: a 20-day-old deployment has one
    // ended 14-day period with nothing observed.
    assert_eq!(halvings(pg, measured).await, vec![(1, "halve".into(), 1)]);
    // Flagged again on the next pass: the flag does not expire.
    let again = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert_eq!(again.unmeasured, vec![unmeasured]);
    s.drop_db().await;
}

/// ADR-T3-04 — the two chosen numbers are a row, read by the pass; `app_role` reads
/// them and cannot write them (0044's default privileges would otherwise let it).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn the_ramp_policy_is_a_row_the_platform_reads_and_cannot_write() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let policy = ramp_worker::RampPolicy::load(pg, &backtest::gates::profile::GateProfile::paper_v1())
        .await
        .expect("a policy row exists");
    assert_eq!(policy.legacy_review_period_days, 14);
    assert_eq!(policy.thresholds.min_observed_bars, 20);

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    assert!(sqlx::query("SELECT count(*) FROM current_ramp_policy").fetch_one(&mut *tx).await.is_ok());
    for sql in [
        "INSERT INTO ramp_policy (legacy_review_period_days, min_observed_bars, decided_by, basis) VALUES (1, 1, 't', 't')",
        "UPDATE ramp_policy SET min_observed_bars = 1",
        "DELETE FROM ramp_policy",
    ] {
        sqlx::query("SAVEPOINT p").execute(&mut *tx).await.unwrap();
        assert!(sqlx::query(sql).execute(&mut *tx).await.is_err(), "app_role must not: {sql}");
        sqlx::query("ROLLBACK TO SAVEPOINT p").execute(&mut *tx).await.unwrap();
    }
    tx.rollback().await.unwrap();
    // The owner can supersede it, and the newest row wins; the old one stays.
    sqlx::query(
        "INSERT INTO ramp_policy (legacy_review_period_days, min_observed_bars, decided_by, basis)
         VALUES (7, 10, 'owner', 'test')",
    )
    .execute(pg)
    .await
    .unwrap();
    let p2 = ramp_worker::RampPolicy::load(pg, &backtest::gates::profile::GateProfile::paper_v1())
        .await
        .unwrap();
    assert_eq!((p2.legacy_review_period_days, p2.thresholds.min_observed_bars), (7, 10));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM ramp_policy").fetch_one(pg).await.unwrap();
    assert_eq!(n, 2);
    s.drop_db().await;
}

/// ADR-T2-04 — the three ceilings are declared by 0190, so a first approval is not
/// refused for want of a number; each can be raised by a new row.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn the_seeded_ceilings_are_declared_with_their_adr() {
    let s = Scratch::new().await;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT name, value::text, declared_by FROM current_deployment_ceiling ORDER BY name",
    )
    .fetch_all(&s.pg)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|r| r.2 == "adr:ADR-T2-04"), "{rows:?}");
    s.drop_db().await;
}

/// The database refuses what no configuration may permit: skipping a step-down,
/// holding through a failure, reviewing a period early, a gap in the schedule.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn no_configuration_skips_a_step_down() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let dep = Uuid::new_v4();
    raw_deployment(pg, dep, &rev('a'), "alice", "AAA", "1", Utc::now() - Duration::days(60)).await.unwrap();
    let t0 = Utc::now() - Duration::days(50);
    let period = |no: i32, start: DateTime<Utc>, end: DateTime<Utc>| {
        let pg = pg.clone();
        async move {
            sqlx::query("INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,$2,$3,$4)")
                .bind(dep)
                .bind(no)
                .bind(start)
                .bind(end)
                .execute(&pg)
                .await
                .map(|_| ())
        }
    };
    period(1, t0, t0 + Duration::days(14)).await.unwrap();

    // A gap: period 3 before 2.
    assert!(err_text(&period(3, t0, t0 + Duration::days(14)).await.unwrap_err()).contains("cannot open before"));
    // Period 2 before period 1 was reviewed.
    assert!(err_text(&period(2, t0 + Duration::days(14), t0 + Duration::days(28)).await.unwrap_err())
        .contains("has not been reviewed"));

    // A failing period cannot hold, or step up; a passing one cannot halve.
    for (outcome, passed, after) in [("hold", false, 0), ("step_up", false, 0), ("halve", true, 1)] {
        assert!(
            review_row(pg, dep, 1, "paper_v1", outcome, passed, 0, after).await.is_err(),
            "{outcome} passed={passed}"
        );
    }
    // The arithmetic: a halve that does not halve, a zero from full size.
    assert!(review_row(pg, dep, 1, "paper_v1", "halve", false, 0, 2).await.is_err());
    assert!(review_row(pg, dep, 1, "paper_v1", "zero", false, 0, 4).await.is_err());
    review_row(pg, dep, 1, "paper_v1", "halve", false, 0, 1).await.unwrap();

    // The next period must start where the last ended.
    assert!(err_text(&period(2, t0 + Duration::days(15), t0 + Duration::days(29)).await.unwrap_err())
        .contains("must start where period 1 ended"));
    period(2, t0 + Duration::days(14), t0 + Duration::days(28)).await.unwrap();

    // A review of a period that has not ended.
    let future = Uuid::new_v4();
    raw_deployment(pg, future, &rev('a'), "alice", "BBB", "1", Utc::now()).await.unwrap();
    sqlx::query("INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,1,now(),now() + interval '14 days')")
        .bind(future)
        .execute(pg)
        .await
        .unwrap();
    let e = review_row(pg, future, 1, "paper_v1", "halve", false, 0, 1).await.unwrap_err();
    assert!(err_text(&e).contains("not reviewed before it has ended"), "{e}");

    // The ladder is continuous: period 2 starts from period 1's halvings.
    let e = review_row(pg, dep, 2, "paper_v1", "halve", false, 0, 1).await.unwrap_err();
    assert!(err_text(&e).contains("is at 1 halvings"), "{e}");
    review_row(pg, dep, 2, "paper_v1", "halve", false, 1, 2).await.unwrap();

    // Immutable, both.
    assert!(sqlx::query("DELETE FROM ramp_review").execute(pg).await.is_err());
    assert!(sqlx::query("UPDATE ramp_period SET ends_at = now()").execute(pg).await.is_err());
    s.drop_db().await;
}

/// T-3.1/T-3.2 in the schema: a raise is the consequence of exactly one passed
/// step_up review, for that deployment and profile.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn a_capital_raise_needs_a_passed_review_and_spends_it_once() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let dep = Uuid::new_v4();
    raw_deployment(pg, dep, &rev('a'), "alice", "AAA", "1", Utc::now() - Duration::days(60)).await.unwrap();
    let t0 = Utc::now() - Duration::days(50);
    sqlx::query("INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,1,$2,$3)")
        .bind(dep)
        .bind(t0)
        .bind(t0 + Duration::days(14))
        .execute(pg)
        .await
        .unwrap();
    let raise = |review: Option<Uuid>| {
        let pg = pg.clone();
        async move {
            sqlx::query(
                "INSERT INTO mlops.capital_authorisation
                   (tenant_id, subject, profile_id, previous_fraction, allowed_fraction, direction,
                    gates_passed, reason, decided_by, ramp_review_id)
                 VALUES ('alice', $1, 'strict_v1', 0, 0.100, 'raise', 16, 'r', 't', $2)",
            )
            .bind(dep.to_string())
            .bind(review)
            .execute(&pg)
            .await
            .map(|_| ())
        }
    };
    // Without a review.
    assert!(err_text(&raise(None).await.unwrap_err()).contains("names none"));
    // With one that is not a step_up: a failed period.
    review_row(pg, dep, 1, "strict_v1", "halve", false, 0, 1).await.unwrap();
    let failed: Uuid = sqlx::query_scalar("SELECT review_id FROM ramp_review WHERE deployment_id = $1")
        .bind(dep)
        .fetch_one(pg)
        .await
        .unwrap();
    assert!(err_text(&raise(Some(failed)).await.unwrap_err()).contains("not a passed step_up"));

    // A passed step_up on a fresh deployment, spent once.
    let dep2 = Uuid::new_v4();
    raw_deployment(pg, dep2, &rev('a'), "alice", "BBB", "1", Utc::now() - Duration::days(60)).await.unwrap();
    sqlx::query("INSERT INTO ramp_period (deployment_id, period_no, starts_at, ends_at) VALUES ($1,1,$2,$3)")
        .bind(dep2)
        .bind(t0)
        .bind(t0 + Duration::days(14))
        .execute(pg)
        .await
        .unwrap();
    review_row(pg, dep2, 1, "strict_v1", "step_up", true, 0, 0).await.unwrap();
    let good: Uuid = sqlx::query_scalar("SELECT review_id FROM ramp_review WHERE deployment_id = $1")
        .bind(dep2)
        .fetch_one(pg)
        .await
        .unwrap();
    // For another deployment's subject it is refused.
    assert!(err_text(&raise(Some(good)).await.unwrap_err()).contains("another deployment"));
    sqlx::query(
        "INSERT INTO mlops.capital_authorisation
           (tenant_id, subject, profile_id, previous_fraction, allowed_fraction, direction,
            gates_passed, reason, decided_by, ramp_review_id)
         VALUES ('alice', $1, 'strict_v1', 0, 0.100, 'raise', 16, 'r', 't', $2)",
    )
    .bind(dep2.to_string())
    .bind(good)
    .execute(pg)
    .await
    .expect("one passed step_up, one raise");
    let spent = sqlx::query(
        "INSERT INTO mlops.capital_authorisation
           (tenant_id, subject, profile_id, previous_fraction, allowed_fraction, direction,
            gates_passed, reason, decided_by, ramp_review_id)
         VALUES ('alice', $1, 'strict_v1', 0.100, 0.250, 'raise', 16, 'r', 't', $2)",
    )
    .bind(dep2.to_string())
    .bind(good)
    .execute(pg)
    .await;
    assert!(spent.is_err(), "a review is spent once");
    // And a paper_v1 pass still cannot raise whatever the review says (AT-65).
    let paper = sqlx::query(
        "INSERT INTO mlops.capital_authorisation
           (tenant_id, subject, profile_id, previous_fraction, allowed_fraction, direction,
            gates_passed, reason, decided_by, ramp_review_id)
         VALUES ('alice', 'x', 'paper_v1', 0, 0.100, 'raise', 16, 'r', 't', NULL)",
    )
    .execute(pg)
    .await;
    assert!(paper.is_err());
    s.drop_db().await;
}

// ── T-3.1..T-3.3 on forward rows (fixtures) ──────────────────────────────────

/// One `mlops.forward_observation` row, as the recorder writes it.
struct Obs {
    bar: DateTime<Utc>,
    source: &'static str,
    matched: bool,
}

/// A measured deployment *with a proposal behind it* (so its gate verdict is on
/// record) and the forward rows a recorder would have written for it. The scratch
/// owner replays the recorder's inserts with foreign keys relaxed: the trial the
/// forward record compares against is not the subject here, the period's rows are.
async fn deployment_with_forward_rows(
    pg: &PgPool,
    revision: &str,
    instrument: &str,
    started_at: DateTime<Utc>,
    rows: &[Obs],
    halts_at: &[DateTime<Utc>],
) -> Uuid {
    let pid = approved(pg, "alice", draft(revision, instrument, "100")).await;
    let dep = Uuid::new_v4();
    deployment_gate::authorise_deployment(pg, &human(), "alice", dep, pid)
        .await
        .unwrap();
    let forward =
        raw_measured_deployment(pg, dep, revision, "alice", instrument, "100", started_at).await;
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO mlops.paper_deployment
           (deployment_id, tenant_id, strategy_id, trial_id, venue, numeraire, bar_seconds,
            max_abs_return_divergence, max_cum_return_divergence, max_position_divergence,
            min_signal_match, kill_criteria, deployed_by)
         VALUES ($1,'alice',$2,$3,'platform_paper','usd',3600,0.02,0.05,0,0.9,'fixture','alice')",
    )
    .bind(forward)
    .bind(dep.to_string())
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .unwrap();
    for o in rows {
        sqlx::query(
            "INSERT INTO mlops.forward_observation
               (tenant_id, deployment_id, strategy_id, bar, realised_return, expected_return,
                numeraire, source, signal_matched, realised_slippage_bps, modelled_slippage_bps,
                realised_turnover, expected_turnover, model_attributable_rejects)
             VALUES ('alice',$1,$2,$3,0.001,0.001,'usd',$4,$5,1.0,1.0,1.0,1.0,0)",
        )
        .bind(forward)
        .bind(dep.to_string())
        .bind(o.bar)
        .bind(o.source)
        .bind(o.matched)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    for at in halts_at {
        sqlx::query(
            "INSERT INTO mlops.divergence_halt (halt_id, tenant_id, strategy_id, deployment_id, kind, reason, raised_at)
             VALUES ($1,'alice',$2,$3,'return','fixture divergence',$4)",
        )
        .bind(Uuid::new_v4())
        .bind(dep.to_string())
        .bind(forward)
        .bind(at)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    dep
}

fn rows_in(
    start: DateTime<Utc>,
    n: i64,
    source: &'static str,
    matched: impl Fn(i64) -> bool,
) -> Vec<Obs> {
    (1..=n)
        .map(|i| Obs {
            bar: start + Duration::hours(i),
            source,
            matched: matched(i),
        })
        .collect()
}

async fn review(pg: &PgPool, dep: Uuid) -> (String, bool, Option<String>, serde_json::Value) {
    sqlx::query_as(
        "SELECT outcome, passed, not_computable_reason, evidence FROM ramp_review
          WHERE deployment_id = $1 AND period_no = 1",
    )
    .bind(dep)
    .fetch_one(pg)
    .await
    .unwrap()
}

/// T-3.1/T-3.2/T-3.3 on **rows**, not on empty recorders: a review period is judged
/// on the forward observations the recorder wrote inside it (bars `> start` and
/// `<= end`, observed only), the divergence halts raised inside it, and the gate
/// verdict on record, and the outcome follows. Every earlier test of the worker used
/// a deployment whose recorder had observed nothing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Postgres that can create databases"]
async fn a_period_is_judged_on_the_forward_rows_inside_it() {
    let s = Scratch::new().await;
    let pg = &s.pg;
    let rec = Arc::new(Recorder::default());
    let now = Utc::now();
    let started =
        chrono::TimeZone::timestamp_opt(&Utc, (now - Duration::days(20)).timestamp(), 0).unwrap();
    let end = started + Duration::days(14);

    // Healthy: 29 observed bars inside, one exactly at the period end (counted), one
    // exactly at the start and one after the end (neither counted).
    let mut healthy_rows = rows_in(started, 29, "observed", |_| true);
    healthy_rows.push(Obs { bar: end, source: "observed", matched: true });
    healthy_rows.push(Obs { bar: started, source: "observed", matched: false });
    healthy_rows.push(Obs { bar: end + Duration::hours(1), source: "observed", matched: false });
    let healthy =
        deployment_with_forward_rows(pg, &rev('a'), "HEA", started, &healthy_rows, &[]).await;

    // The signals disagree with the backtest on 20 of 30 bars.
    let mismatch_rows = rows_in(started, 30, "observed", |i| i % 3 == 0);
    let mismatch =
        deployment_with_forward_rows(pg, &rev('b'), "MIS", started, &mismatch_rows, &[]).await;

    // Healthy rows, but a divergence halt was raised inside the period.
    let halted_rows = rows_in(started, 30, "observed", |_| true);
    let halted = deployment_with_forward_rows(
        pg,
        &rev('c'),
        "HAL",
        started,
        &halted_rows,
        &[started + Duration::days(3)],
    )
    .await;

    // Plenty of rows, none of them observed: reconstructed history is never forward
    // evidence (D-T1).
    let backfill_rows = rows_in(started, 30, "backfill", |_| true);
    let backfill =
        deployment_with_forward_rows(pg, &rev('d'), "BAC", started, &backfill_rows, &[]).await;

    // Too few observed bars to judge.
    let short_rows = rows_in(started, 10, "observed", |_| true);
    let short = deployment_with_forward_rows(pg, &rev('e'), "SHO", started, &short_rows, &[]).await;

    // A halt raised in some *other* period does not count against this one.
    let other_halt = deployment_with_forward_rows(
        pg,
        &rev('f'),
        "OTH",
        started,
        &rows_in(started, 30, "observed", |_| true),
        &[end + Duration::days(1)],
    )
    .await;

    let report = ramp_worker::run_pass(pg, now, &policy(), &rec).await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.unmeasured.is_empty());

    // Healthy: holds, full size, and nothing is applied to it.
    let (outcome, passed, nc, ev) = review(pg, healthy).await;
    assert_eq!((outcome.as_str(), passed, nc), ("hold", true, None));
    assert_eq!(
        ev["evidence"]["observed_bars"], 30,
        "the boundary rows are the ones that decide"
    );
    assert_eq!(ev["evidence"]["divergence_halts"], 0);
    assert_eq!(ev["evidence"]["profile_passed"], true);
    assert!(
        ev["gate_16"]["inconclusive"].is_boolean(),
        "Gate 16's own verdict is stored beside the judgement"
    );
    assert!(
        !rec.0.lock().unwrap().iter().any(|(id, _)| *id == healthy),
        "full size: no effect applied"
    );
    assert_eq!(halvings(pg, healthy).await, vec![(1, "hold".into(), 0)]);

    // Another period's halt is not this period's.
    assert_eq!(review(pg, other_halt).await.0, "hold");

    // Each failure steps the size down by half, with its reason on the record.
    let half = Size::from_halvings(1).unwrap();
    for (dep, why) in [(mismatch, "signal match"), (halted, "divergence halt")] {
        let (outcome, passed, nc, ev) = review(pg, dep).await;
        assert_eq!((outcome.as_str(), passed, nc), ("halve", false, None), "{why}");
        assert!(
            ev["judgement"].to_string().contains(why),
            "the stored judgement names why: {why} in {}",
            ev["judgement"]
        );
        assert!(rec.0.lock().unwrap().iter().any(|(id, sz)| *id == dep && *sz == half));
    }

    // Not computable steps down too, and says why. Backfill rows count for nothing.
    let (o, p, nc, ev) = review(pg, backfill).await;
    assert_eq!((o.as_str(), p), ("halve", false));
    assert!(
        nc.unwrap().contains("0 observed forward bars"),
        "backfilled rows are not forward evidence"
    );
    assert_eq!(ev["evidence"]["observed_bars"], 0);
    let (o, _, nc, _) = review(pg, short).await;
    assert_eq!(o, "halve");
    assert!(nc.unwrap().contains("10 observed forward bars"));

    // The platform's own role holds the table privileges the review reads (a missing
    // GRANT passes as the owner and fails live). Table-level: schema USAGE comes from
    // 0044, which this scratch harness skips (no `_sqlx_migrations`).
    for table in ["mlops.forward_observation", "mlops.divergence_halt"] {
        let ok: bool = sqlx::query_scalar("SELECT has_table_privilege('app_role', $1, 'SELECT')")
            .bind(table)
            .fetch_one(pg)
            .await
            .unwrap();
        assert!(ok, "app_role cannot SELECT {table}");
    }

    s.drop_db().await;
}
