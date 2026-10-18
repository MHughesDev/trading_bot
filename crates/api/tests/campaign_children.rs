//! The campaign's EXPERIMENT and COMPARE workers against PostgreSQL
//! (SPEC §10, §11.5; checklist 2.1; OPEN-TASKS C1.1).
//!
//! `Study` and `EvalTask` had no registered worker, so a campaign that reached
//! EXPERIMENT or COMPARE failed `no_worker`. Each test here defines a real
//! campaign, folds real events, submits the child the *driver* would submit,
//! claims it through the job store, and runs it through `WorkerPool::execute`
//! with a fake evaluation engine - so the campaign logic is exercised end to end
//! and only the backtests are replaced.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p api --test campaign_children -- --ignored --test-threads=1
//! ```
//!
//! The scratch database needs migrations through 0183 (pgvector is not needed:
//! none of these tests touch an embedding).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use api::campaign_driver::compare_worker::{
    campaign_trials_counted, compare_history, standing_incumbent, stop_check, CompareWorker,
    StopCheck,
};
use api::campaign_driver::study_worker::{
    replicate_seed, CampaignEvaluator, EvaluationSpec, ReplicateCall, StudyWorker, SweepCall,
    SweepOutcome,
};
use backtest::run::ParamMap;
use jobs::store::{JobStore, PgTrialCounter, Submission};
use jobs::types::WorkerClass;
use jobs::{JobKind, JobOutput, WorkerPool};
use ledger::campaign::{Budget, CampaignDefinition, ExplorationFloor};
use ledger::phase::{CampaignEvent, CampaignPhase};
use ledger::pg::PgTrialLedger;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

fn params(fast: i64) -> ParamMap {
    let mut m = BTreeMap::new();
    m.insert("fast".to_string(), json!(fast));
    m
}

/// The evaluation block a campaign declares at DEFINE.
fn evaluation() -> Value {
    json!({
        "strategy_ref": "ema_cross",
        "strategy_type": "trend",
        "universe_ref": "BTC-USD",
        "research": { "start": "2024-01-01T00:00:00Z", "end": "2024-09-01T00:00:00Z" },
        "holdout":  { "start": "2024-09-01T00:00:00Z", "end": "2024-12-01T00:00:00Z" },
        "objective": { "primary": "sharpe" },
        "incumbent_params": { "fast": 12 },
        "max_runs": 8,
        "replicates": 29,
        "rope_scale": 4.0
    })
}

fn definition(slug: &str, search_space: Value) -> CampaignDefinition {
    CampaignDefinition {
        slug: slug.into(),
        hypothesis: "a faster fast window earns more after costs".into(),
        objective: json!({ "maximize": ["sharpe"], "subject_to": {} }),
        benchmark: json!({ "kind": "declared_incumbent" }),
        delta_practical: 0.15,
        approval_spend_usd: 5.0,
        budget: Budget {
            max_trials: 5_000,
            gpu_hours: 1.0,
            usd: 10.0,
            wall_clock_hours: 24.0,
        },
        exploration_floor: ExplorationFloor::default_floor(),
        gates_profile: "strict_v1".into(),
        preference_vector: None,
        search_space,
    }
}

/// What a fake engine saw, and what it answers.
#[derive(Default)]
struct Fake {
    sweeps: Mutex<Vec<SweepCall>>,
    replicates: Mutex<Vec<ReplicateCall>>,
    /// Metric for the candidate (`fast == 9`) and for everything else.
    candidate_value: f64,
    incumbent_value: f64,
    candidate: Option<ParamMap>,
}

#[async_trait::async_trait]
impl CampaignEvaluator for Fake {
    async fn ensure_experiment(
        &self,
        _user: Uuid,
        campaign_id: Uuid,
        _spec: &EvaluationSpec,
        _delta: f64,
    ) -> Result<Uuid, String> {
        // Stable per campaign, like the real one.
        Ok(Uuid::from_u128(campaign_id.as_u128() ^ 1))
    }

    async fn sweep(
        &self,
        _user: Uuid,
        call: SweepCall,
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SweepOutcome, String> {
        self.sweeps.lock().unwrap().push(call);
        Ok(SweepOutcome {
            candidate: self.candidate.clone(),
            trials_consumed: 9,
            n_sampled: 8,
            n_feasible: 8,
            plateau: Some(true),
            study_ids: vec!["s1".into()],
            sample_scores: (0..8).map(|i| f64::from(i) * 0.1).collect(),
        })
    }

    async fn replicate_metrics(
        &self,
        _user: Uuid,
        call: ReplicateCall,
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Vec<Option<f64>>, String> {
        let is_candidate = call.params.get("fast") == Some(&json!(9));
        let v = if is_candidate {
            self.candidate_value
        } else {
            self.incumbent_value
        };
        let n = call.windows.len();
        self.replicates.lock().unwrap().push(call);
        Ok(vec![Some(v); n])
    }
}

struct World {
    pg: PgPool,
    store: Arc<JobStore>,
    led: PgTrialLedger,
    tenant: String,
    user: Uuid,
    campaign_id: Uuid,
    driver_job: String,
}

/// A defined campaign with the log folded up to just before `next`, a driver job
/// to be the children's parent, and (optionally) a HYPOTHESIZE child already
/// settled with a proposal.
async fn world(slug_prefix: &str, search_space: Value, proposal: Option<Value>) -> World {
    let pg = pool().await;
    let user = Uuid::new_v4();
    let tenant = user.to_string();
    let led = PgTrialLedger::new(pg.clone());
    let slug = format!("{slug_prefix}-{}", Uuid::new_v4().simple());
    let handle = led
        .define_campaign_async(&tenant, &definition(&slug, search_space))
        .await
        .expect("define");
    let campaign_id = handle.campaign_id();
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(PgTrialCounter)));

    // The driver's own job: every child names it as parent.
    let mut driver = Submission::new(JobKind::Campaign, user, json!({ "campaign_id": campaign_id }));
    driver.project_id = Some(Uuid::new_v4());
    let driver_job = store.submit(driver).await.expect("driver job").job_id;

    async fn append(led: &PgTrialLedger, tenant: &str, campaign: Uuid, phase: CampaignPhase, detail: Value) {
        led.append_campaign_event_async(tenant, campaign, &CampaignEvent::new(phase, detail))
            .await
            .expect("append");
    }
    append(&led, &tenant, campaign_id, CampaignPhase::Baseline, json!({})).await;
    append(&led, &tenant, campaign_id, CampaignPhase::Diagnose, json!({})).await;

    // HYPOTHESIZE: a settled child carrying the proposal, recorded in the log.
    let hyp = {
        let mut sub = Submission::new(
            JobKind::ResearchRun,
            user,
            json!({ "campaign_id": campaign_id, "phase": "hypothesize", "seq": 3 }),
        );
        sub.parent_job_id = Some(driver_job.clone());
        sub.member_index = Some(3);
        store.submit_child_once(sub).await.expect("hyp child").job_id
    };
    sqlx::query("UPDATE jobs SET state='succeeded', result=$2, finished_at=now() WHERE job_id=$1")
        .bind(&hyp)
        .bind(json!({ "proposal": proposal.unwrap_or(Value::Null) }))
        .execute(&pg)
        .await
        .unwrap();
    append(&led, &tenant, campaign_id, CampaignPhase::Hypothesize, json!({ "job_id": hyp })).await;

    World {
        pg,
        store,
        led,
        tenant,
        user,
        campaign_id,
        driver_job,
    }
}

impl World {
    /// Submit the child the driver would for `phase`, claim it, run it.
    async fn run_child(&self, kind: JobKind, class: WorkerClass, phase: &str, seq: i64, worker: Arc<dyn jobs::Worker>) -> String {
        let mut sub = Submission::new(
            kind,
            self.user,
            json!({ "campaign_id": self.campaign_id, "phase": phase, "seq": seq }),
        );
        sub.parent_job_id = Some(self.driver_job.clone());
        sub.member_index = Some(i32::try_from(seq).unwrap());
        if kind.counts_trial() {
            sub.experiment_id = Some(format!("campaign:{}:{phase}", self.campaign_id));
        }
        let submitted = self.store.submit_child_once(sub).await.expect("child");
        let claimed = self
            .store
            .claim(class, "test-worker")
            .await
            .expect("claim")
            .expect("a queued child");
        assert_eq!(claimed.job_id, submitted.job_id, "claimed the child just submitted");
        WorkerPool::new(Arc::clone(&self.store), class, 1)
            .register(worker)
            .execute(claimed)
            .await;
        submitted.job_id
    }

    async fn result(&self, job_id: &str) -> Value {
        let job = self.store.get(job_id).await.unwrap();
        assert_eq!(
            job.state.as_str(),
            "succeeded",
            "child ended {}: {:?}",
            job.state.as_str(),
            job.error
        );
        job.result.unwrap()
    }
}

fn study_worker(w: &World, fake: Arc<Fake>) -> Arc<dyn jobs::Worker> {
    Arc::new(StudyWorker::new(Arc::clone(&w.store), w.pg.clone(), fake))
}

fn compare_worker(w: &World, fake: Arc<Fake>) -> Arc<dyn jobs::Worker> {
    Arc::new(CompareWorker::new(Arc::clone(&w.store), w.pg.clone(), fake))
}

async fn record_experiment(w: &World, job_id: &str) {
    w.led
        .append_campaign_event_async(
            &w.tenant,
            w.campaign_id,
            &CampaignEvent::new(CampaignPhase::Experiment, json!({ "job_id": job_id })),
        )
        .await
        .expect("experiment event");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn experiment_runs_the_proposed_ranges_around_the_standing_incumbent() {
    let proposal = json!({
        "move": "change_sizing",
        "hypothesis": "narrower fast window",
        "ranges": [{ "name": "fast", "kind": { "kind": "int", "min": 6, "max": 14, "step": 1 } }]
    });
    let w = world("exp", json!({ "evaluation": evaluation() }), Some(proposal)).await;
    let fake = Arc::new(Fake {
        candidate: Some(params(9)),
        ..Default::default()
    });
    let job = w
        .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, Arc::clone(&fake)))
        .await;
    let result = w.result(&job).await;

    // The candidate is the sweep's carried-forward member, nothing else.
    assert_eq!(result["candidate_params"], json!({ "fast": 9 }));
    assert_eq!(result["incumbent_params"], json!({ "fast": 12 }), "the declared start");
    // Ranges are applied; the structural half of the move is not, and says so.
    assert_eq!(result["structural_move_applied"], json!(false));
    assert_eq!(result["move"], json!("change_sizing"));

    let call = fake.sweeps.lock().unwrap()[0].clone();
    assert_eq!(call.narrowing["fast"].min, Some(6.0));
    assert_eq!(call.narrowing["fast"].max, Some(14.0));
    assert_eq!(call.base_params, params(12));
    assert_eq!(call.seed, replicate_seed(w.campaign_id, 4), "the platform's seed, not a caller's");
    assert_eq!(call.max_runs, 8);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn a_campaign_without_an_evaluation_block_fails_its_experiment_naming_the_fix() {
    let w = world("noeval", json!({}), None).await;
    let mut sub = Submission::new(
        JobKind::Study,
        w.user,
        json!({ "campaign_id": w.campaign_id, "phase": "experiment", "seq": 4 }),
    );
    sub.parent_job_id = Some(w.driver_job.clone());
    sub.member_index = Some(4);
    sub.experiment_id = Some(format!("campaign:{}:experiment", w.campaign_id));
    let submitted = w.store.submit_child_once(sub).await.unwrap();
    let claimed = w.store.claim(WorkerClass::Backtest, "t").await.unwrap().unwrap();
    WorkerPool::new(Arc::clone(&w.store), WorkerClass::Backtest, 1)
        .register(study_worker(&w, Arc::new(Fake::default())))
        .execute(claimed)
        .await;
    let job = w.store.get(&submitted.job_id).await.unwrap();
    assert_eq!(job.state.as_str(), "failed");
    let err = job.error.unwrap().to_string();
    assert!(err.contains("no_evaluation_spec"), "{err}");
    assert!(err.contains("redefined"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn compare_promotes_a_clear_winner_and_it_becomes_the_incumbent() {
    let w = world("cmp", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    // A difference of 3.0 on a declared half-width of 4.0: the confidence
    // sequence over 29 identical windows needs most of the scale to clear delta.
    let fake = Arc::new(Fake {
        candidate: Some(params(9)),
        candidate_value: 3.0,
        incumbent_value: 0.0,
        ..Default::default()
    });
    let exp = w
        .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, Arc::clone(&fake)))
        .await;
    record_experiment(&w, &exp).await;

    let cmp = w
        .run_child(JobKind::EvalTask, WorkerClass::Eval, "compare", 5, compare_worker(&w, Arc::clone(&fake)))
        .await;
    let result = w.result(&cmp).await;
    assert_eq!(result["verdict"], json!("promote"));
    assert_eq!(result["promoted"], json!(true));

    // Both sides were measured on the same 29 windows.
    let reps = fake.replicates.lock().unwrap().clone();
    assert_eq!(reps.len(), 2);
    assert_eq!(reps[0].windows, reps[1].windows, "paired on identical windows");
    assert_eq!(reps[0].windows.len(), 29);

    // The decision is on the append-only log, scoped to the campaign.
    let history = compare_history(&w.pg, &w.tenant, w.campaign_id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].0.promoted);
    let spec = EvaluationSpec::from_search_space(&json!({ "evaluation": evaluation() })).unwrap();
    assert_eq!(
        standing_incumbent(&w.pg, &w.tenant, w.campaign_id, &spec).await.unwrap(),
        params(9),
        "a promoted candidate is the next generation's incumbent"
    );
    let (kind,): (String,) = sqlx::query_as(
        "SELECT decision_kind FROM mlops.decision WHERE campaign_id = $1 AND policy_id = 'campaign_compare'",
    )
    .bind(w.campaign_id)
    .fetch_one(&w.pg)
    .await
    .unwrap();
    assert_eq!(kind, "promote");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn identical_candidates_are_not_promoted_and_leave_the_incumbent_alone() {
    let w = world("eq", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    let fake = Arc::new(Fake {
        candidate: Some(params(9)),
        candidate_value: 1.0,
        incumbent_value: 1.0,
        ..Default::default()
    });
    let exp = w
        .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, Arc::clone(&fake)))
        .await;
    record_experiment(&w, &exp).await;
    let cmp = w
        .run_child(JobKind::EvalTask, WorkerClass::Eval, "compare", 5, compare_worker(&w, fake))
        .await;
    let result = w.result(&cmp).await;
    assert_eq!(result["verdict"], json!("practically_equivalent"));
    assert_eq!(result["promoted"], json!(false));
    let spec = EvaluationSpec::from_search_space(&json!({ "evaluation": evaluation() })).unwrap();
    assert_eq!(
        standing_incumbent(&w.pg, &w.tenant, w.campaign_id, &spec).await.unwrap(),
        params(12)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn a_sweep_with_no_feasible_candidate_is_recorded_as_a_rejection_not_a_failure() {
    let w = world("none", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    let fake = Arc::new(Fake::default()); // candidate: None
    let exp = w
        .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, Arc::clone(&fake)))
        .await;
    record_experiment(&w, &exp).await;
    let cmp = w
        .run_child(JobKind::EvalTask, WorkerClass::Eval, "compare", 5, compare_worker(&w, fake))
        .await;
    let result = w.result(&cmp).await;
    assert_eq!(result["verdict"], json!("no_candidate"));
    let history = compare_history(&w.pg, &w.tenant, w.campaign_id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].0.could_exceed, "a search that found nothing cannot exceed delta");
}

/// The stopping rules read what the COMPARE worker wrote.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn a_dry_run_of_rejections_ends_the_campaign_as_diminishing_returns() {
    let w = world("dry", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    let fake = Arc::new(Fake::default());
    // Ten generations that found nothing: the log the rule reads.
    for i in 0..10_i64 {
        let seq = 4 + 2 * i;
        let exp = w
            .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", seq, study_worker(&w, Arc::clone(&fake)))
            .await;
        record_experiment(&w, &exp).await;
        let cmp = w
            .run_child(JobKind::EvalTask, WorkerClass::Eval, "compare", seq + 1, compare_worker(&w, Arc::clone(&fake)))
            .await;
        // The rest of the cycle the driver records, so the next EXPERIMENT is a
        // legal transition: compare -> gate -> prune -> reallocate -> hypothesize.
        for (phase, detail) in [
            (CampaignPhase::Compare, json!({ "job_id": cmp })),
            (CampaignPhase::Gate, json!({})),
            (CampaignPhase::Prune, json!({})),
            (CampaignPhase::Reallocate, json!({})),
            (CampaignPhase::Hypothesize, json!({})),
        ] {
            w.led
                .append_campaign_event_async(&w.tenant, w.campaign_id, &CampaignEvent::new(phase, detail))
                .await
                .expect("cycle event");
        }
    }
    let history: Vec<_> = compare_history(&w.pg, &w.tenant, w.campaign_id)
        .await
        .unwrap()
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert_eq!(history.len(), 10);
    let counted = campaign_trials_counted(&w.pg, &w.tenant, w.campaign_id).await.unwrap();
    match stop_check(&history, counted) {
        StopCheck::End(phase, why) => {
            assert!(
                matches!(phase, CampaignPhase::Converged | CampaignPhase::DiminishingReturns),
                "{phase:?}: {why}"
            );
        }
        other => panic!("ten dry generations should end the campaign, got {other:?}"),
    }
}

/// The budget rule counts what the job store registers. Until the store stamped
/// the campaign on its children, this was 0 for every campaign, so no budget
/// could ever run out.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn children_are_counted_against_their_campaign() {
    let w = world("count", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    assert_eq!(
        campaign_trials_counted(&w.pg, &w.tenant, w.campaign_id).await.unwrap(),
        0,
        "nothing counted yet (the driver and the hypothesize child count no trial)"
    );
    let fake = Arc::new(Fake::default());
    w.run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, fake))
        .await;
    assert_eq!(
        campaign_trials_counted(&w.pg, &w.tenant, w.campaign_id).await.unwrap(),
        1,
        "a Study child is a counted look, and it is this campaign's"
    );
    // The stamp itself, not just the experiment-name convention: the job store
    // registers a child with the campaign's id in `mlops.trial.campaign_id`.
    let (stamped,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM mlops.trial WHERE campaign_id = $1")
            .bind(w.campaign_id)
            .fetch_one(&w.pg)
            .await
            .unwrap();
    assert_eq!(stamped, 1, "the child's trial carries the campaign id");
    // And a trial the Backtest Suite registers under the campaign's experiment
    // (it cannot tag a campaign) is found by name.
    sqlx::query(
        "INSERT INTO mlops.trial (seq, prev_hash, row_hash, tenant_id, experiment_id, config_hash, config,
             dataset_id, code_hash, image_digest, seed_set, actor_kind, actor_id, policy_id, policy_version,
             exploration_flag, propensity, delta_practical, prereg_hash, non_reproducible,
             overlapping_labels_unweighted, split_overrides)
         VALUES (0, '\\x'::bytea, '\\x'::bytea, $1, $2, 'h', '{}'::jsonb, 'd', 'c', 'i', '{}', 'human', 'u',
                 'suite_manual', 1, false, 1.0, 0.15, 'p', false, false, '[]'::jsonb)",
    )
    .bind(&w.tenant)
    .bind(format!("campaign-{}", w.campaign_id))
    .execute(&w.pg)
    .await
    .expect("a suite trial");
    assert_eq!(
        campaign_trials_counted(&w.pg, &w.tenant, w.campaign_id).await.unwrap(),
        2
    );
}

/// A GATE with no funnel evidence is told to run the funnel, not stopped.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn gate_without_funnel_evidence_asks_for_the_funnel() {
    let w = world("gate", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    let driver = api::campaign_driver::CampaignWorker::new(Arc::clone(&w.store), w.pg.clone());
    let m = driver.gate_manifest(&w.tenant, w.campaign_id).await.unwrap();
    assert_eq!(m["run_funnel"], json!(true));
    assert!(m.get("trial_id").is_none(), "no funnel trial exists yet: {m}");
    assert_eq!(m["experiment_id"], json!(format!("campaign-{}", w.campaign_id)));
    assert_eq!(m["instrument"], json!("BTC-USD"));
}

/// Records what the funnel was asked, and writes the statistic only the real one
/// writes, on a trial of the campaign's experiment.
struct FakeFunnel {
    led: PgTrialLedger,
    asked: Mutex<Vec<(Uuid, f64, ParamMap)>>,
}

#[async_trait::async_trait]
impl api::campaign_driver::funnel_worker::CampaignFunnel for FakeFunnel {
    async fn run(
        &self,
        user: Uuid,
        campaign_id: Uuid,
        _spec: &EvaluationSpec,
        delta: f64,
        incumbent: &ParamMap,
    ) -> Result<api::campaign_driver::funnel_worker::FunnelRan, String> {
        self.asked.lock().unwrap().push((campaign_id, delta, incumbent.clone()));
        let tenant = user.to_string();
        let ctx = ledger::DispatchContext::human(&tenant, "t", delta)
            .with_experiment(format!("campaign-{campaign_id}"));
        let subject = ledger::TrialSubject {
            config_hash: format!("sha256:{}", Uuid::new_v4().simple()),
            config: json!({}),
            dataset_id: "slice:funnel".into(),
            split_spec_id: None,
            code_hash: "c".into(),
            image_digest: "i".into(),
            seed_set: vec![],
            non_reproducible: false,
            overlapping_labels_unweighted: false,
            split_overrides: json!([]),
            planned_steps: None,
        };
        let ticket = self
            .led
            .register_async(&ledger::Registration::new(&ctx, &subject, 1.0))
            .await
            .map_err(|e| e.to_string())?;
        self.led
            .record_statistic_async(&tenant, ticket.trial_id(), "deflated_sharpe", 0.4, "funnel_advance")
            .await
            .map_err(|e| e.to_string())?;
        Ok(api::campaign_driver::funnel_worker::FunnelRan {
            experiment: Uuid::new_v4(),
            axis: backtest::suite::SweepAxis { param: "fast".into(), center: 9.0, step: 1.0 },
        })
    }
}

/// The GATE child runs the funnel on the **standing incumbent** with the
/// campaign's declared effect size, then judges the trial the funnel wrote.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn the_gate_child_runs_the_funnel_on_the_incumbent_and_judges_what_it_wrote() {
    let w = world("gatefunnel", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    // A promoted candidate becomes the incumbent the funnel must be run on.
    let promoted = Fake {
        candidate: Some(params(9)),
        candidate_value: 3.0,
        incumbent_value: 0.0,
        ..Default::default()
    };
    let fake = Arc::new(promoted);
    let exp = w
        .run_child(JobKind::Study, WorkerClass::Backtest, "experiment", 4, study_worker(&w, Arc::clone(&fake)))
        .await;
    record_experiment(&w, &exp).await;
    w.run_child(JobKind::EvalTask, WorkerClass::Eval, "compare", 5, compare_worker(&w, fake)).await;

    let funnel = Arc::new(FakeFunnel { led: PgTrialLedger::new(w.pg.clone()), asked: Mutex::new(Vec::new()) });
    let driver = api::campaign_driver::CampaignWorker::new(Arc::clone(&w.store), w.pg.clone());
    let mut manifest = driver.gate_manifest(&w.tenant, w.campaign_id).await.unwrap();
    manifest["campaign_id"] = json!(w.campaign_id);
    let mut sub = Submission::new(JobKind::GateAdvance, w.user, manifest);
    sub.parent_job_id = Some(w.driver_job.clone());
    sub.member_index = Some(7);
    sub.experiment_id = Some(format!("campaign:{}:gate", w.campaign_id));
    let submitted = w.store.submit_child_once(sub).await.expect("gate child");
    let claimed = w.store.claim(WorkerClass::Backtest, "t").await.unwrap().unwrap();
    assert_eq!(claimed.job_id, submitted.job_id);
    let worker = api::gate_worker::GateAdvanceWorker::new(w.pg.clone())
        .with_funnel(Arc::clone(&funnel) as Arc<dyn api::campaign_driver::funnel_worker::CampaignFunnel>);
    WorkerPool::new(Arc::clone(&w.store), WorkerClass::Backtest, 1)
        .register(Arc::new(worker))
        .execute(claimed)
        .await;

    let result = w.result(&submitted.job_id).await;
    assert_eq!(result["of"], json!(16), "the whole stack was evaluated: {result}");
    let asked = funnel.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, w.campaign_id);
    assert!((asked[0].1 - 0.15).abs() < 1e-12, "the campaign's own delta");
    assert_eq!(asked[0].2, params(9), "the funnel ran on the promoted incumbent, not the defaults");
    // And it judged the trial the funnel wrote.
    let (trial,): (Uuid,) = sqlx::query_as(
        "SELECT trial_id FROM mlops.trial WHERE experiment_id = $1 AND tenant_id = $2 LIMIT 1",
    )
    .bind(format!("campaign-{}", w.campaign_id))
    .bind(&w.tenant)
    .fetch_one(&w.pg)
    .await
    .unwrap();
    assert_eq!(result["trial_id"], json!(trial));
}

/// Without a funnel runner the GATE says so by name.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn a_gate_that_needs_the_funnel_and_has_none_fails_by_name() {
    let w = world("nofunnel", json!({ "evaluation": evaluation() }), Some(Value::Null)).await;
    let driver = api::campaign_driver::CampaignWorker::new(Arc::clone(&w.store), w.pg.clone());
    let mut manifest = driver.gate_manifest(&w.tenant, w.campaign_id).await.unwrap();
    manifest["campaign_id"] = json!(w.campaign_id);
    let mut sub = Submission::new(JobKind::GateAdvance, w.user, manifest);
    sub.parent_job_id = Some(w.driver_job.clone());
    sub.member_index = Some(7);
    sub.experiment_id = Some(format!("campaign:{}:gate", w.campaign_id));
    let submitted = w.store.submit_child_once(sub).await.unwrap();
    let claimed = w.store.claim(WorkerClass::Backtest, "t").await.unwrap().unwrap();
    WorkerPool::new(Arc::clone(&w.store), WorkerClass::Backtest, 1)
        .register(Arc::new(api::gate_worker::GateAdvanceWorker::new(w.pg.clone())))
        .execute(claimed)
        .await;
    let job = w.store.get(&submitted.job_id).await.unwrap();
    assert_eq!(job.state.as_str(), "failed");
    assert!(job.error.unwrap().to_string().contains("no_funnel_runner"));
    let _ = JobOutput::default();
}
