//! The stores Set S phase 11 added (migrations 0161-0164) against live Postgres:
//! the day budget in trials, contradiction pairs, a survey's proposals and
//! absorption, a profile's standing and `judgement`, cross-session dedupe, and
//! regime flips.
//!
//! Ignored by default; run against a scratch database migrated through 0164:
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/fsession_scratch \
//!   cargo test -j 2 -p api --test pg_session_stores -- --ignored --test-threads=1
//! ```
//!
//! **Every statement the platform issues here runs as `app_role`.** The suite
//! connects as the owner to build fixtures, and then hands the code under test a
//! pool whose connections `SET ROLE app_role` - because an owner-only test is
//! how 0145 and 0147 came to be needed: a missing GRANT passes CI and fails live.
//! Rows are committed to the scratch database under random keys; the append-only
//! tables cannot be cleaned up afterwards, and the scratch database is dropped.

use api::{
    session_contradictions, session_day, session_dedupe, session_profile, session_regime,
    session_survey,
};
use research_session::agenda::{AgendaItem, Source};
use research_session::couplings::ProfileHealth;
use research_session::replenish::{absorb, SurveyReport};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/fsession_scratch".into())
}

async fn owner() -> PgPool {
    PgPool::connect(&url()).await.expect("the database is up")
}

/// The platform's pool: every connection is `app_role`.
async fn app() -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|c, _| {
            Box::pin(async move {
                c.execute("SET ROLE app_role").await?;
                Ok(())
            })
        })
        .connect(&url())
        .await
        .expect("the platform pool connects")
}

struct Fixture {
    user: Uuid,
    session: Uuid,
}

/// A user, a conversation and a running session with a log that opens legally.
async fn session(pg: &PgPool, user: Uuid, state: &str) -> Uuid {
    let convo = Uuid::new_v4();
    let session = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model)
         VALUES ($1, $2, 'test-fixture', 'test-fixture')",
    )
    .bind(convo)
    .bind(user)
    .execute(pg)
    .await
    .expect("a conversation");
    sqlx::query(
        "INSERT INTO research_sessions (session_id, conversation_id, user_id, tier, continuous, question)
         VALUES ($1, $2, $3, 'test-fixture', false, 'does the fixture work?')",
    )
    .bind(session)
    .bind(convo)
    .bind(user)
    .execute(pg)
    .await
    .expect("a session");
    for (kind, st) in [
        ("session_created", Some("draft")),
        ("session_started", Some("running")),
    ] {
        sqlx::query("INSERT INTO session_events (session_id, kind, state) VALUES ($1, $2, $3)")
            .bind(session)
            .bind(kind)
            .bind(st)
            .execute(pg)
            .await
            .expect("an event");
    }
    if state == "stopped" {
        sqlx::query(
            "INSERT INTO session_events (session_id, kind, state) VALUES ($1, 'session_stopped', 'stopped')",
        )
        .bind(session)
        .execute(pg)
        .await
        .expect("a stop");
    }
    session
}

async fn fixture(pg: &PgPool) -> Fixture {
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("test-fixture-{user}@test.invalid"))
        .execute(pg)
        .await
        .expect("a user");
    let session = session(pg, user, "running").await;
    Fixture { user, session }
}

async fn item_row(pg: &PgPool, item: &AgendaItem) {
    sqlx::query(
        "INSERT INTO agenda_items (item_id, session_id, question, subject, workflow, dedupe_key, status)
         VALUES ($1, $2, $3, $4, $5, $6, 'queued')",
    )
    .bind(item.id)
    .bind(item.session_id)
    .bind(&item.question)
    .bind(&item.subject)
    .bind(&item.workflow)
    .bind(&item.dedupe_key)
    .execute(pg)
    .await
    .expect("an item");
}

fn charter() -> research_session::Charter {
    use research_session::charter::{ExplorationShare, Objective, Scope};
    research_session::Charter {
        scope: Scope {
            instruments: vec!["SOL-USD".into()],
            ..Scope::default()
        },
        objective: Objective::BuildKnowledge {
            about: vec!["SOL-USD".into()],
        },
        seeds: vec![],
        permitted_workflows: vec![],
        exclusions: vec![],
        standing_constraints: vec![],
        exploration_share: ExplorationShare::default(),
        stop_when: vec![],
        delta_practical_default: None,
        version: 1,
    }
}

// ── the day budget in trials ────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_day_budget_in_trials_is_set_read_and_amended_by_the_platform_role() {
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    assert_eq!(
        session_day::trials_per_day(&pg, f.session).await.unwrap(),
        None,
        "a session with no row is `nobody said`, not a default"
    );
    session_day::set_trials_per_day(&pg, f.session, 120)
        .await
        .unwrap();
    assert_eq!(
        session_day::trials_per_day(&pg, f.session).await.unwrap(),
        Some(120)
    );
    session_day::set_trials_per_day(&pg, f.session, 30)
        .await
        .unwrap();
    assert_eq!(
        session_day::trials_per_day(&pg, f.session).await.unwrap(),
        Some(30)
    );
    assert!(session_day::set_trials_per_day(&pg, f.session, 0)
        .await
        .is_err());
    let delete = sqlx::query("DELETE FROM session_day_budget WHERE session_id = $1")
        .bind(f.session)
        .execute(&pg)
        .await;
    assert!(
        delete.is_err(),
        "the platform may not delete a budget: it was granted no DELETE"
    );
}

// ── contradiction pairs ─────────────────────────────────────────────────────

async fn finding(pg: &PgPool, user: Uuid, id: &str, claim: &str, conflicts: &[&str]) {
    sqlx::query(
        "INSERT INTO agent_memory.entry
             (entry_id, user_id, kind, subject, claim, confidence, evidence_class,
              window_start, window_end, prov_run_id, prov_tool, prov_tool_version,
              prov_template_hash, created_by, conflicts_with)
         VALUES ($1, $2, 'finding', 'BTC-USD', $3, 'established', 'direct_stat',
                 '2025-01-01', '2025-02-01', 'run-1', 'tool', '1', 'h', 'test', $4)",
    )
    .bind(id)
    .bind(user)
    .bind(claim)
    .bind(
        conflicts
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>(),
    )
    .execute(pg)
    .await
    .expect("a finding");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_conflict_becomes_one_pair_whichever_entry_lists_the_other_and_resolves_when_one_leaves()
{
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    let (a, b) = (
        format!("f_{}", Uuid::new_v4()),
        format!("f_{}", Uuid::new_v4()),
    );
    // Both list the other: one pair, not two.
    finding(&own, f.user, &a, "momentum pays on BTC", &[&b]).await;
    finding(&own, f.user, &b, "momentum does not pay on BTC", &[&a]).await;

    let first = session_contradictions::refresh(&pg, f.user).await.unwrap();
    assert_eq!(
        first.found, 1,
        "two entries listing each other are one pair"
    );
    let again = session_contradictions::refresh(&pg, f.user).await.unwrap();
    assert_eq!(again.found, 0, "refreshing twice finds nothing new");

    let open = session_contradictions::open_pairs(&pg, f.user)
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    assert!(open[0].question.contains("momentum pays on BTC"));
    assert!(open[0].key().contains(&a) && open[0].key().contains(&b));

    // Memory says one side is no longer standing: the pair resolves.
    sqlx::query("UPDATE agent_memory.entry SET archived = true WHERE entry_id = $1")
        .bind(&a)
        .execute(&own)
        .await
        .unwrap();
    let resolved = session_contradictions::refresh(&pg, f.user).await.unwrap();
    assert_eq!(resolved.resolved, 1);
    assert!(session_contradictions::open_pairs(&pg, f.user)
        .await
        .unwrap()
        .is_empty());

    // And a resolved pair stays recorded, for "found/resolved" and for the desk.
    let all = session_contradictions::list(&pg, f.user).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].resolved_at.is_some() && all[0].resolved_why.is_some());

    // It cannot be rewritten or deleted by the platform.
    assert!(
        sqlx::query("DELETE FROM session_contradictions WHERE user_id = $1")
            .bind(f.user)
            .execute(&pg)
            .await
            .is_err()
    );
}

// ── a survey's proposals ────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_surveys_proposals_are_stored_with_their_estimates_and_a_deduped_survey_is_an_explicit_zero(
) {
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    let survey_item = AgendaItem::new(f.session, "survey the charter", "", Source::Replenish)
        .with_workflow("survey");
    item_row(&own, &survey_item).await;
    let run = Uuid::new_v4();

    let rows = vec![
        session_survey::ProposalRow {
            entry_id: "q1".into(),
            question: "does funding lead returns on SOL-USD?".into(),
            subject: "SOL-USD".into(),
            workflow: None,
            label: Some("decisive".into()),
        },
        session_survey::ProposalRow {
            entry_id: "q2".into(),
            question: "is the spread stable on SOL-USD?".into(),
            subject: "SOL-USD".into(),
            workflow: None,
            label: None,
        },
    ];
    session_survey::store_proposals(&pg, f.session, survey_item.id, run, &rows)
        .await
        .unwrap();
    // Idempotent on (run, question).
    session_survey::store_proposals(&pg, f.session, survey_item.id, run, &rows)
        .await
        .unwrap();
    let stored: Vec<(String, Option<f64>, Option<String>)> = sqlx::query_as(
        "SELECT question, value_estimate, value_label FROM session_survey_proposals
          WHERE run_id = $1 ORDER BY question",
    )
    .bind(run)
    .fetch_all(&own)
    .await
    .unwrap();
    assert_eq!(stored.len(), 2, "written once, not twice");
    assert_eq!(stored[0].1, Some(1.0));
    assert_eq!(
        stored[1].1, None,
        "an unlabelled proposal has no estimate - never 0"
    );

    // Absorb against an agenda that already holds both questions: every proposal
    // is a duplicate, and the absorption says zero *explicitly*.
    let charter = charter();
    let report: SurveyReport = session_survey::report(f.session, survey_item.id, run, &rows);
    let mut live = vec![
        AgendaItem::new(
            f.session,
            &rows[0].question,
            "SOL-USD",
            Source::OpenQuestion,
        ),
        AgendaItem::new(
            f.session,
            &rows[1].question,
            "SOL-USD",
            Source::OpenQuestion,
        ),
    ];
    let r = absorb(&charter, &report, &mut live);
    let counts = session_survey::Counts::of(rows.len(), &r);
    assert_eq!(counts.new_questions, 0, "{counts:?}");
    assert!(session_survey::new_questions(&pg, run)
        .await
        .unwrap()
        .is_none());
    assert!(
        session_survey::record_absorption(&pg, f.session, survey_item.id, run, counts)
            .await
            .unwrap()
    );
    assert!(
        !session_survey::record_absorption(&pg, f.session, survey_item.id, run, counts)
            .await
            .unwrap(),
        "an absorption is written once"
    );
    assert_eq!(
        session_survey::new_questions(&pg, run).await.unwrap(),
        Some(0)
    );
    assert!(sqlx::query(
        "UPDATE session_survey_absorptions SET new_questions = 9 WHERE run_id = $1"
    )
    .bind(run)
    .execute(&pg)
    .await
    .is_err());
}

// ── a profile's standing ────────────────────────────────────────────────────

fn scorecard(health: &str, rate: f64) -> session_profile::ScorecardInput {
    session_profile::ScorecardInput {
        health: health.into(),
        reason: format!("judgement-v1 {health}"),
        autonomous_threshold: Some(0.9),
        judgement: vec![session_profile::JudgementInput {
            workflow: "investigate".into(),
            pass_rate: rate,
            pass_line: 0.8,
            trials: 40,
            suite_version: "judgement-v1".into(),
        }],
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_recorded_scorecard_is_read_back_and_a_later_degraded_one_wins() {
    let pg = app().await;
    let profile = format!("profile-{}", Uuid::new_v4());
    assert!(session_profile::latest_standing(&pg, &profile)
        .await
        .unwrap()
        .is_none());

    session_profile::record_scorecard(&pg, &profile, &scorecard("measured", 0.95), "test")
        .await
        .unwrap();
    let s = session_profile::latest_standing(&pg, &profile)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.health, ProfileHealth::Measured);
    assert_eq!(s.autonomous_threshold, Some(0.9));
    let j = session_profile::latest_judgement(&pg, &profile)
        .await
        .unwrap();
    assert_eq!(j.len(), 1);
    assert!(j[0].verdict().passed());

    // The scorecard falls: the newest record wins, and the pause reads it.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    session_profile::record_scorecard(&pg, &profile, &scorecard("degraded", 0.5), "test")
        .await
        .unwrap();
    let s = session_profile::latest_standing(&pg, &profile)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.health, ProfileHealth::Degraded);
    let (health, why) = session_profile::health_from(ProfileHealth::Measured, None, Some(&s));
    assert_eq!(health, ProfileHealth::Degraded);
    assert!(why.unwrap().contains("degraded"));
    let j = session_profile::latest_judgement(&pg, &profile)
        .await
        .unwrap();
    assert!(
        !j[0].verdict().passed(),
        "the latest rate is the one that counts"
    );

    // Append-only against the platform role.
    assert!(
        sqlx::query("UPDATE profile_standing SET health = 'measured' WHERE profile_id = $1")
            .bind(&profile)
            .execute(&pg)
            .await
            .is_err()
    );
    // A malformed scorecard is refused with a sentence, and nothing is written.
    let bad =
        session_profile::record_scorecard(&pg, &profile, &scorecard("fine", 0.5), "test").await;
    assert!(bad.is_err());
}

// ── two sessions, one question ──────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn two_sessions_with_one_question_produce_one_item_with_two_sources() {
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    let other = session(&own, f.user, "running").await;

    let q = "does funding lead returns on ETH-USD?";
    let first = AgendaItem::new(f.session, q, "ETH-USD", Source::OpenQuestion);
    assert!(matches!(
        session_dedupe::offer(&pg, f.user, &first).await.unwrap(),
        session_dedupe::Offered::Here(Some(_))
    ));

    // The same question from a retry condition in the second session.
    let second = AgendaItem::new(other, q, "ETH-USD", Source::RetryConditionMet);
    let placed = session_dedupe::offer(&pg, f.user, &second).await.unwrap();
    assert_eq!(
        placed,
        session_dedupe::Offered::Elsewhere {
            item: first.id,
            owner: f.session
        },
        "merged onto the first session's item"
    );
    let live_in_second: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM agenda_items WHERE session_id = $1")
            .bind(other)
            .fetch_one(&own)
            .await
            .unwrap();
    assert_eq!(
        live_in_second.0, 0,
        "the second session holds no copy: the question runs once"
    );
    let (sources,): (serde_json::Value,) =
        sqlx::query_as("SELECT sources FROM agenda_items WHERE item_id = $1")
            .bind(first.id)
            .fetch_one(&own)
            .await
            .unwrap();
    assert_eq!(
        sources,
        json!(["open_question", "retry_condition_met"]),
        "one item, two sources"
    );
    assert_eq!(
        session_dedupe::untold_sponsors(&pg, first.id)
            .await
            .unwrap(),
        vec![other]
    );
    assert!(session_dedupe::mark_told(&pg, first.id, other)
        .await
        .unwrap());
    assert!(
        !session_dedupe::mark_told(&pg, first.id, other)
            .await
            .unwrap(),
        "told once"
    );

    // A human's own question in the second session is never rerouted.
    let asked = AgendaItem::new(other, q, "ETH-USD", Source::User);
    assert!(matches!(
        session_dedupe::offer(&pg, f.user, &asked).await.unwrap(),
        session_dedupe::Offered::Here(Some(_))
    ));

    // Another user's identical question never merges.
    let stranger = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(stranger)
        .bind(format!("test-fixture-{stranger}@test.invalid"))
        .execute(&own)
        .await
        .unwrap();
    let theirs = session(&own, stranger, "running").await;
    let alien = AgendaItem::new(theirs, q, "ETH-USD", Source::OpenQuestion);
    assert!(matches!(
        session_dedupe::offer(&pg, stranger, &alien).await.unwrap(),
        session_dedupe::Offered::Here(Some(_))
    ));
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_stopped_sessions_board_is_not_a_place_a_question_is_answered() {
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    let dead = session(&own, f.user, "stopped").await;
    let q = "is the basis stable on BTC-USD?";
    let there = AgendaItem::new(dead, q, "BTC-USD", Source::OpenQuestion);
    item_row(&own, &there).await;
    let here = AgendaItem::new(f.session, q, "BTC-USD", Source::OpenQuestion);
    assert!(matches!(
        session_dedupe::offer(&pg, f.user, &here).await.unwrap(),
        session_dedupe::Offered::Here(Some(_))
    ));
}

// ── regime flips ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_regime_flip_has_one_identity_across_three_reads() {
    let (own, pg) = (owner().await, app().await);
    let scope = format!("scope-{}", Uuid::new_v4());
    let version = "test-v1";
    // `regime_vec` is `vector(16)` on a pgvector server and `float8[]` on the
    // scratch database that has none.
    let udt: (String,) = sqlx::query_as(
        "SELECT udt_name FROM information_schema.columns
          WHERE table_schema = 'regime_causal' AND table_name = 'regime_state'
            AND column_name = 'regime_vec'",
    )
    .fetch_one(&own)
    .await
    .unwrap();
    let vec_literal = if udt.0 == "vector" {
        format!("[{}]", vec!["0"; 16].join(","))
    } else {
        format!("{{{}}}", vec!["0"; 16].join(","))
    };
    for (days_ago, label) in [
        (4, "low_vol"),
        (3, "low_vol"),
        (2, "high_vol"),
        (1, "high_vol"),
    ] {
        sqlx::query(&format!(
            "INSERT INTO regime_causal.regime_state
                 (market_scope, event_time, model_version, p_filtered, regime_vec, knowledge_time, label)
             VALUES ($1, now() - interval '{days_ago} days', $2, ARRAY[0.5, 0.5]::real[], '{vec_literal}', now(), $3)"
        ))
        .bind(&scope)
        .bind(version)
        .bind(label)
        .execute(&own)
        .await
        .expect("a regime row");
    }
    let mine = |all: Vec<research_session::sources::observe::Observation>| {
        all.into_iter()
            .filter(|o| o.subject() == scope)
            .collect::<Vec<_>>()
    };
    let one = mine(session_regime::flips(&pg, 500).await.unwrap());
    let two = mine(session_regime::flips(&pg, 500).await.unwrap());
    let three = mine(session_regime::flips(&pg, 500).await.unwrap());
    assert_eq!(one.len(), 1, "low_vol -> high_vol is one flip: {one:?}");
    assert_eq!(one, two);
    assert_eq!(two, three);
    assert!(
        one[0].fingerprint().contains("low_vol->high_vol"),
        "{}",
        one[0].fingerprint()
    );
}

// ── pushed, not polled ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn a_monitor_notification_wakes_a_session_within_one_event_and_the_heartbeat_does_not() {
    let (own, pg) = (owner().await, app().await);
    let mut wake = api::session_wake::Wake::open(&pg).await;
    assert!(
        wake.is_listening(),
        "the platform role can LISTEN: {:?}",
        wake.degraded()
    );

    // The heartbeat is not a change: the sleep runs to its ceiling.
    sqlx::query("SELECT pg_notify('monitor_lease', 'beat')")
        .execute(&own)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        wake.sleep(std::time::Duration::from_millis(400)).await,
        api::session_wake::Woke::Timer
    );
    assert!(started.elapsed() >= std::time::Duration::from_millis(350));

    // A kill-switch notification ends a 30 s sleep at once.
    let started = std::time::Instant::now();
    let notifier = own.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        sqlx::query("SELECT pg_notify('monitor_kill_switch', 'tripped')")
            .execute(&notifier)
            .await
            .unwrap();
    });
    let woke = wake.sleep(std::time::Duration::from_secs(30)).await;
    assert_eq!(
        woke,
        api::session_wake::Woke::Event("monitor_kill_switch".into())
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0164"]
async fn every_session_event_publishes_its_session_id_for_the_stream_to_wake_on() {
    let (own, pg) = (owner().await, app().await);
    let f = fixture(&own).await;
    let mut listener = sqlx::postgres::PgListener::connect_with(&pg).await.unwrap();
    listener.listen("session_events").await.unwrap();
    // As the platform role, through the driver's own writer.
    api::sessions::append(
        &pg,
        f.session,
        research_session::EventKind::Note,
        json!({"x": 1}),
    )
    .await
    .unwrap();
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv())
        .await
        .expect("a notification within one event")
        .unwrap();
    assert_eq!(n.channel(), "session_events");
    assert_eq!(n.payload(), f.session.to_string());
}
