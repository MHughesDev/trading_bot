//! `lifecycle_evaluate` against live Postgres, **as the platform's role**
//! (S-10.4, S-10.8, S-10.15 server side; ADR-S10-21).
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/lifecycle_scratch \
//!   cargo test -j 2 -p api --test pg_lifecycle_evaluate -- --ignored --test-threads=1
//! ```
//!
//! The statements under test run on a pool whose connections `SET ROLE app_role`.
//! That is the whole point of the first test: `mlops.trial` and
//! `mlops.trial_statistic` FORCE row-level security keyed on `app.tenant_id`, and
//! `evidence_for` used to read them with no context set. As the platform's role
//! that returned no rows, so every hypothesis would have resolved
//! `insufficient_data`; an owner-connected test cannot see it.

use api::lifecycle_evaluate::decide_run;
use harness::lifecycle::evaluate::{Verdict, Why};
use ledger::pg::PgTrialLedger;
use ledger::reproduction::ReproductionRecord;
use ledger::{DispatchContext, Registration, TrialEvent, TrialState, TrialSubject};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@127.0.0.1:5433/lifecycle_scratch".into()
    })
}

async fn owner() -> PgPool {
    PgPool::connect(&url()).await.expect("the database is up")
}

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

fn subject(tag: &str, n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:{tag}-{n}"),
        config: serde_json::json!({ "n": n, "tag": tag }),
        dataset_id: "slice:sha256:abc".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1".into(),
        seed_set: vec![n as i64],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    }
}

async fn hypothesis(pg: &PgPool, run: Uuid, falsifier: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_lifecycle.hypothesis (hypothesis_id, run_id, statement, falsifier, role)
         VALUES ($1, $2, 'momentum survives costs', $3, 'primary')",
    )
    .bind(id)
    .bind(run)
    .bind(falsifier)
    .execute(pg)
    .await
    .expect("app_role may insert a hypothesis");
    id
}

/// A completed trial for `h`, with one statistic recorded on it.
async fn trial(
    led: &PgTrialLedger,
    tenant: &str,
    h: Uuid,
    tag: &str,
    n: u64,
    stat: (&str, f64),
) -> (Uuid, String) {
    let ctx = DispatchContext::human(tenant, "mason", 0.1).with_experiment("exp-e");
    let s = subject(tag, n);
    let mut reg = Registration::new(&ctx, &s, 1.0);
    reg.hypothesis_id = Some(h);
    let mut t = led.register_async(&reg).await.expect("registered");
    let id = t.trial_id();
    led.transition_async(&mut t, TrialEvent::to(TrialState::Running))
        .await
        .unwrap();
    led.settle_async(t, TrialEvent::completed(s.config_hash.clone(), None))
        .await
        .unwrap();
    led.record_statistic_async(tenant, id, stat.0, stat.1, "test")
        .await
        .expect("statistic recorded");
    (id, s.config_hash)
}

/// The first test is the regression for the RLS read, and the whole battery.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres migrated through 0242"]
async fn evidence_is_read_as_the_platforms_role_and_a_verdict_keeps_its_inputs() {
    let pg = app().await;
    let tenant = format!("tenant-{}", Uuid::new_v4().simple());
    let led = PgTrialLedger::new(pg.clone());
    let run = Uuid::new_v4();
    let h = hypothesis(&pg, run, "sharpe <= 0").await;

    // Three out-of-sample measurements that survive the falsifier, a cost study and
    // a null study, all on this hypothesis's own trials.
    let (t1, _) = trial(&led, &tenant, h, "a", 1, ("sharpe", 1.1)).await;
    let (_t2, _) = trial(&led, &tenant, h, "a", 2, ("sharpe", 1.4)).await;
    let (_t3, _) = trial(&led, &tenant, h, "a", 3, ("sharpe", 0.9)).await;
    let (cost, _) = trial(&led, &tenant, h, "a", 4, ("breakeven_cost_multiple", 3.4)).await;
    let (bench, _) = trial(&led, &tenant, h, "a", 5, ("permutation_p_value", 0.03)).await;

    let (decided, undecidable) = decide_run(&pg, &tenant, run).await.expect("decides");
    assert!(undecidable.is_empty(), "{undecidable:?}");
    assert_eq!(decided.len(), 1);
    let d = &decided[0];

    // Read as app_role the evidence is there at all: before the fix this was
    // `insufficient_data` for every hypothesis on a live platform.
    assert_eq!(
        d.verdict,
        Verdict::Inconclusive {
            why: Why::EvidenceIncomplete
        },
        "three measurements, a cost ticket and a benchmark ticket, and no receipt yet: {:?}",
        d.inputs
    );
    assert_eq!(d.missing, vec!["a reproducibility receipt"], "{:?}", d.missing);
    assert_eq!(d.inputs["battery"]["cost_model_ticket"], cost.to_string());
    assert_eq!(d.inputs["battery"]["benchmark_ticket"], bench.to_string());
    assert_eq!(d.inputs["battery"]["out_of_sample_ticket"], t1.to_string());
    assert_eq!(d.inputs["n"], 3);

    // The row keeps what decided it, so a card can show the number.
    let stored: (String, serde_json::Value) = sqlx::query_as(
        "SELECT status, verdict_inputs FROM agent_lifecycle.hypothesis WHERE hypothesis_id = $1",
    )
    .bind(h)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!(stored.0, "inconclusive");
    assert_eq!(stored.1["metric"], "sharpe");
    assert_eq!(stored.1["threshold"], 0.0);
    assert_eq!(stored.1["measurements"].as_array().unwrap().len(), 3);
}

/// A refutation records the measurement that refuted it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres migrated through 0242"]
async fn a_refutation_stores_the_ticket_and_value_that_fired() {
    let pg = app().await;
    let tenant = format!("tenant-{}", Uuid::new_v4().simple());
    let led = PgTrialLedger::new(pg.clone());
    let run = Uuid::new_v4();
    let h = hypothesis(&pg, run, "sharpe <= 0").await;
    trial(&led, &tenant, h, "r", 1, ("sharpe", 0.8)).await;
    trial(&led, &tenant, h, "r", 2, ("sharpe", 0.6)).await;
    let (bad, _) = trial(&led, &tenant, h, "r", 3, ("sharpe", -0.3)).await;

    let (decided, _) = decide_run(&pg, &tenant, run).await.unwrap();
    assert_eq!(decided[0].verdict, Verdict::Refuted);
    assert_eq!(decided[0].inputs["fired_by"]["ticket_id"], bad.to_string());
    assert_eq!(decided[0].inputs["fired_by"]["value"], -0.3);
    // The first verdict stands: deciding again changes nothing and returns nothing.
    let (again, _) = decide_run(&pg, &tenant, run).await.unwrap();
    assert!(again.is_empty(), "only open hypotheses are decided");
}

/// §6.9's receipt is a bit-identical reproduction under a receipt the original
/// carried, and with it `supported` is reachable from real rows. A reproduction
/// with no receipt on record does not count.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres migrated through 0242"]
async fn supported_needs_a_reproduction_under_a_receipt_the_original_carried() {
    let pg = app().await;
    let tenant = format!("tenant-{}", Uuid::new_v4().simple());
    let led = PgTrialLedger::new(pg.clone());

    // A hypothesis with three surviving measurements, a cost study and a null study
    // on its own trials. Returns the config hash of the first trial.
    async fn build(pg: &PgPool, tenant: &str, run: Uuid, tag: &str) -> String {
        let h = hypothesis(pg, run, "sharpe <= 0").await;
        let l = PgTrialLedger::new(pg.clone());
        let (_, first) = trial(&l, tenant, h, tag, 1, ("sharpe", 1.1)).await;
        trial(&l, tenant, h, tag, 2, ("sharpe", 1.4)).await;
        trial(&l, tenant, h, tag, 3, ("sharpe", 0.9)).await;
        trial(&l, tenant, h, tag, 4, ("breakeven_cost_multiple", 3.4)).await;
        trial(&l, tenant, h, tag, 5, ("permutation_p_value", 0.03)).await;
        first
    }

    // (a) reproduced, but the original carried no receipt: the number only.
    let run_a = Uuid::new_v4();
    let config_a = build(&pg, &tenant, run_a, "ra").await;
    let ticket = led.begin_reproduction_async(&tenant, &config_a).await.unwrap();
    led.record_reproduction_async(
        ticket,
        &ReproductionRecord {
            run_id: config_a.clone(),
            outcome: "bit_identical".into(),
            receipt_stored: None,
            receipt_rerun: serde_json::json!({"calendar_version": "sessions@1"}),
            differences: serde_json::json!([]),
            was_value: None,
            now_value: None,
            reproduced_by: "test".into(),
        },
    )
    .await
    .unwrap();
    let (decided, _) = decide_run(&pg, &tenant, run_a).await.unwrap();
    assert_eq!(
        decided[0].verdict,
        Verdict::Inconclusive {
            why: Why::EvidenceIncomplete
        },
        "a reproduction with no receipt on record proves the number and not the pipeline"
    );

    // (b) reproduced bit-identical under a receipt the original carried.
    let run_b = Uuid::new_v4();
    let config_b = build(&pg, &tenant, run_b, "rb").await;
    let ticket = led.begin_reproduction_async(&tenant, &config_b).await.unwrap();
    let receipt = serde_json::json!({"calendar_version": "sessions@1", "dataset_hash": "sha256:d"});
    led.record_reproduction_async(
        ticket,
        &ReproductionRecord {
            run_id: config_b.clone(),
            outcome: "bit_identical".into(),
            receipt_stored: Some(receipt.clone()),
            receipt_rerun: receipt,
            differences: serde_json::json!([]),
            was_value: None,
            now_value: None,
            reproduced_by: "test".into(),
        },
    )
    .await
    .unwrap();
    let (decided, _) = decide_run(&pg, &tenant, run_b).await.unwrap();
    assert_eq!(
        decided[0].verdict,
        Verdict::Supported,
        "{:?}",
        decided[0].inputs
    );
    assert!(decided[0].inputs["battery"]["repro_spec"].is_string());
    let _ = owner().await;
}
