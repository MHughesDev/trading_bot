//! The Set R-a / R-b hand-offs that had no production caller, against PostgreSQL
//! (the R-WIRE track; RB-1.3, RB-3.1, RB-3.3, RB-4.5, RB-5.8).
//!
//! Each of these was built and tested in isolation and reached by nothing: the move veto and
//! the move record, the matched random arm and the gate that needs it, the regime prior and
//! the calibration-drift inbox source. Here the *driver and workers* are run against a real
//! database and the thing each one is supposed to leave behind is read back.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -p api --test campaign_wiring -- --ignored --test-threads=1
//! ```
//!
//! The scratch database needs the migrations through 0245 (pgvector is not needed: none of
//! these touches an embedding; `vector(N)` can be rewritten to `float8[]` to apply 0055).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use api::campaign_driver::study_worker::{
    fold_regime_prior, CampaignEvaluator, EvaluationSpec, ReplicateCall, StudyWorker, SweepCall, SweepOutcome,
};
use api::campaign_driver::{move_run_ref, CampaignWorker};
use api::knowledge_routes::{arm_gate, ArmStore, MoveStore, PriorStore};
use jobs::store::{JobStore, PgTrialCounter, Submission};
use jobs::types::WorkerClass;
use jobs::{JobKind, WorkerPool};
use ledger::campaign::{Budget, CampaignDefinition, ExplorationFloor};
use ledger::phase::{CampaignEvent, CampaignPhase};
use ledger::pg::PgTrialLedger;
use research::move_memory::Outcome;
use research::propose::{ExitMode, Move};
use research::sampler::SamplerKind;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

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
        hypothesis: "a different exit earns more after costs".into(),
        objective: json!({ "maximize": ["sharpe"], "subject_to": {} }),
        benchmark: json!({ "kind": "declared_incumbent" }),
        delta_practical: 0.15,
        approval_spend_usd: 5.0,
        budget: Budget { max_trials: 5_000, gpu_hours: 1.0, usd: 10.0, wall_clock_hours: 24.0 },
        exploration_floor: ExplorationFloor::default_floor(),
        gates_profile: "strict_v1".into(),
        preference_vector: None,
        search_space,
    }
}

/// A fake engine whose sweeps return one score per sampled point; the random arm's sweep (the
/// `Random` sampler with the declared space) scores differently from the agent's, so the two
/// arms are told apart in the table.
#[derive(Default)]
struct Fake {
    sweeps: Mutex<Vec<SweepCall>>,
}

#[async_trait::async_trait]
impl CampaignEvaluator for Fake {
    async fn ensure_experiment(&self, _u: Uuid, c: Uuid, _s: &EvaluationSpec, _d: f64) -> Result<Uuid, String> {
        Ok(Uuid::from_u128(c.as_u128() ^ 1))
    }
    async fn sweep(
        &self,
        _user: Uuid,
        call: SweepCall,
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SweepOutcome, String> {
        let random = call.sampler == SamplerKind::Random && call.narrowing.is_empty();
        let n = call.max_runs;
        self.sweeps.lock().unwrap().push(call);
        Ok(SweepOutcome {
            candidate: Some(BTreeMap::from([("fast".to_string(), json!(9))])),
            trials_consumed: n + 27,
            n_sampled: n,
            n_feasible: n,
            plateau: Some(true),
            study_ids: vec!["s".into()],
            sample_scores: (0..n).map(|i| if random { -f64::from(i) } else { f64::from(i) }).collect(),
        })
    }
    async fn replicate_metrics(
        &self,
        _u: Uuid,
        c: ReplicateCall,
        _x: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Vec<Option<f64>>, String> {
        Ok(vec![Some(0.0); c.windows.len()])
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

/// A defined campaign folded up to a settled HYPOTHESIZE carrying `proposal`.
async fn world(search_space: Value, proposal: Value) -> World {
    let pg = pool().await;
    let user = Uuid::new_v4();
    let tenant = user.to_string();
    let led = PgTrialLedger::new(pg.clone());
    let slug = format!("wire-{}", Uuid::new_v4().simple());
    let campaign_id = led
        .define_campaign_async(&tenant, &definition(&slug, search_space))
        .await
        .expect("define")
        .campaign_id();
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(PgTrialCounter)));
    let mut driver = Submission::new(JobKind::Campaign, user, json!({ "campaign_id": campaign_id }));
    driver.project_id = Some(Uuid::new_v4());
    let driver_job = store.submit(driver).await.expect("driver job").job_id;
    let w = World { pg, store, led, tenant, user, campaign_id, driver_job };
    w.event(CampaignPhase::Baseline, json!({})).await;
    w.event(CampaignPhase::Diagnose, json!({})).await;
    let hyp = w.settled_child(JobKind::ResearchRun, 3, json!({ "proposal": proposal })).await;
    w.event(CampaignPhase::Hypothesize, json!({ "job_id": hyp })).await;
    w
}

impl World {
    async fn event(&self, phase: CampaignPhase, detail: Value) {
        self.led
            .append_campaign_event_async(&self.tenant, self.campaign_id, &CampaignEvent::new(phase, detail))
            .await
            .expect("append");
    }

    /// A child job that already succeeded with `result`.
    async fn settled_child(&self, kind: JobKind, seq: i32, result: Value) -> String {
        let mut sub = Submission::new(kind, self.user, json!({ "campaign_id": self.campaign_id, "seq": seq }));
        sub.parent_job_id = Some(self.driver_job.clone());
        sub.member_index = Some(seq);
        if kind.counts_trial() {
            sub.experiment_id = Some(format!("campaign:{}:{seq}", self.campaign_id));
        }
        let id = self.store.submit_child_once(sub).await.expect("child").job_id;
        sqlx::query("UPDATE jobs SET state='succeeded', result=$2, finished_at=now() WHERE job_id=$1")
            .bind(&id)
            .bind(result)
            .execute(&self.pg)
            .await
            .unwrap();
        id
    }

    fn driver(&self) -> CampaignWorker {
        CampaignWorker::new(Arc::clone(&self.store), self.pg.clone())
    }

    async fn state(&self) -> ledger::phase::CampaignState {
        self.led.campaign_state_async(&self.tenant, self.campaign_id).await.expect("state")
    }

    /// Submit the EXPERIMENT child the driver would, claim it and run it.
    async fn run_experiment(&self, fake: Arc<Fake>, seq: i64) -> Value {
        let mut sub = Submission::new(
            JobKind::Study,
            self.user,
            json!({ "campaign_id": self.campaign_id, "phase": "experiment", "seq": seq }),
        );
        sub.parent_job_id = Some(self.driver_job.clone());
        sub.member_index = Some(i32::try_from(seq).unwrap());
        sub.experiment_id = Some(format!("campaign:{}:experiment", self.campaign_id));
        let submitted = self.store.submit_child_once(sub).await.expect("child");
        let claimed = self.store.claim(WorkerClass::Backtest, "t").await.unwrap().unwrap();
        assert_eq!(claimed.job_id, submitted.job_id);
        WorkerPool::new(Arc::clone(&self.store), WorkerClass::Backtest, 1)
            .register(Arc::new(StudyWorker::new(Arc::clone(&self.store), self.pg.clone(), fake)))
            .execute(claimed)
            .await;
        let job = self.store.get(&submitted.job_id).await.unwrap();
        assert_eq!(job.state.as_str(), "succeeded", "{:?}", job.error);
        job.result.unwrap()
    }
}

fn trail() -> Value {
    json!({ "move": "change_exit", "mode": "trail", "hypothesis": "a trailing exit keeps more of a trend", "ranges": [] })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn the_driver_creates_the_arm_and_the_first_experiment_records_both_matched_arms() {
    let w = world(json!({ "evaluation": evaluation() }), trail()).await;
    let driver = w.driver();

    assert_eq!(driver.ensure_arm(w.user, w.campaign_id).await.unwrap(), Some(8), "the declared sweep size");
    assert_eq!(driver.ensure_arm(w.user, w.campaign_id).await.unwrap(), None, "created once");

    // Gate 3's predicate, before anything has run: the refusal names the arm.
    let why = arm_gate(&w.pg, w.campaign_id).await.unwrap_err();
    assert!(why.contains("0 of 8"), "{why}");

    let fake = Arc::new(Fake::default());
    let result = w.run_experiment(Arc::clone(&fake), 4).await;
    assert_eq!(result["arms"], json!("recorded"));
    assert!(
        !result.to_string().contains("sample_scores"),
        "a score per sample must not be in a result the agent can read (INV-2)"
    );

    let (arm, agent) = ArmStore::new(w.pg.clone()).load(w.campaign_id).await.unwrap().expect("an arm");
    assert_eq!((agent.len(), arm.scores.len(), arm.budget), (8, 8, 8), "equal trial counts");
    assert_eq!(agent[3], 3.0);
    assert_eq!(arm.scores[3], -3.0, "the random arm's scores are its own");
    assert_eq!(arm.seed, research::random_arm::arm_seed(&w.campaign_id.to_string()), "the platform's seed");
    assert!(arm_gate(&w.pg, w.campaign_id).await.is_ok(), "the arm completed, so Gate 3 may proceed");

    // The random arm's sweep: declared space (no narrowing), random sampler, the arm's seed,
    // the same budget and the same fixed parameters as the agent's.
    let calls = fake.sweeps.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    let r = &calls[1];
    assert!(r.narrowing.is_empty());
    assert_eq!(r.sampler, SamplerKind::Random);
    assert_eq!((r.seed, r.max_runs), (arm.seed, 8));
    assert_eq!(r.base_params, calls[0].base_params);

    // A later generation is outside the matched comparison and records nothing more.
    let again = w.run_experiment(Arc::clone(&fake), 6).await;
    assert_eq!(again["arms"], json!("already_matched"));
    assert_eq!(fake.sweeps.lock().unwrap().len(), 3, "only the agent's sweep ran");
    let (_, agent) = ArmStore::new(w.pg.clone()).load(w.campaign_id).await.unwrap().unwrap();
    assert_eq!(agent.len(), 8);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn a_retried_experiment_completes_the_arm_and_never_doubles_it() {
    let w = world(json!({ "evaluation": evaluation() }), trail()).await;
    w.driver().ensure_arm(w.user, w.campaign_id).await.unwrap();
    // The agent's side was written by an attempt that died before the random sweep.
    let store = ArmStore::new(w.pg.clone());
    store.record_scores(w.campaign_id, "agent", &[1.0; 8]).await.unwrap();
    assert_eq!(store.record_scores(w.campaign_id, "agent", &[9.0; 8]).await.unwrap(), 0, "a seq is never rewritten");

    let fake = Arc::new(Fake::default());
    let result = w.run_experiment(Arc::clone(&fake), 4).await;
    assert_eq!(result["arms"], json!("recorded"));
    let (arm, agent) = store.load(w.campaign_id).await.unwrap().unwrap();
    assert_eq!(agent, vec![1.0; 8], "the first record stands");
    assert_eq!(arm.scores.len(), 8);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn a_campaign_with_no_arm_records_nothing_and_a_blocked_move_stops_until_a_person_overrides() {
    // No arm row: the experiment runs, records no arm, and says so.
    let w = world(json!({ "evaluation": evaluation() }), trail()).await;
    let result = w.run_experiment(Arc::new(Fake::default()), 4).await;
    assert_eq!(result["arms"], json!("no_arm"));

    // The veto. Five recorded failures of this motif for this user block it.
    let driver = w.driver();
    let moves = MoveStore::new(w.pg.clone());
    let mv = Move::ChangeExit { mode: ExitMode::Trail };
    let state = w.state().await;
    let allowed = driver.consult_veto(w.user, w.campaign_id, &state).await.unwrap().expect("a typed move was proposed");
    assert_eq!(allowed["veto"]["verdict"], json!("allow"), "no history yet");
    for i in 0..5 {
        moves.record(w.user, "BTC-USD", &mv, Outcome::Failed, &format!("old:{i}")).await.unwrap();
    }
    let err = driver.consult_veto(w.user, w.campaign_id, &state).await.unwrap_err();
    assert_eq!(err.code, "move_vetoed");
    let fix = err.fix.clone().unwrap_or_default();
    assert!(fix.contains("old:0"), "the refusal cites the records: {fix}");

    // Another user's history is not this user's.
    let mine = moves.history(Uuid::new_v4(), &research::move_memory::motif(&mv)).await.unwrap();
    assert!(mine.is_empty());

    // A person overrides; the same call now proceeds and says it was overridden.
    sqlx::query("INSERT INTO agent_memory.move_veto_override (user_id, motif, reason) VALUES ($1, $2, 'reviewed')")
        .bind(w.user)
        .bind(research::move_memory::motif(&mv))
        .execute(&w.pg)
        .await
        .unwrap();
    let v = driver.consult_veto(w.user, w.campaign_id, &state).await.unwrap().unwrap();
    assert_eq!(v["overridden"], json!(true));
    assert_eq!(v["veto"]["verdict"], json!("block"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn a_generation_with_no_typed_move_is_not_vetoed() {
    let w = world(json!({ "evaluation": evaluation() }), Value::Null).await;
    let state = w.state().await;
    assert!(w.driver().consult_veto(w.user, w.campaign_id, &state).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn each_judged_generation_records_what_its_move_did_once() {
    let w = world(json!({ "evaluation": evaluation() }), trail()).await;
    let exp = w.settled_child(JobKind::Study, 4, json!({})).await;
    w.event(CampaignPhase::Experiment, json!({ "job_id": exp })).await;
    let cmp = w
        .settled_child(JobKind::EvalTask, 5, json!({ "verdict": "reject", "promoted": false, "could_exceed": false }))
        .await;
    w.event(CampaignPhase::Compare, json!({ "job_id": cmp })).await;

    let driver = w.driver();
    let state = w.state().await;
    assert_eq!(driver.record_move_outcomes(w.user, w.campaign_id, &state).await.unwrap(), 1);
    assert_eq!(driver.record_move_outcomes(w.user, w.campaign_id, &state).await.unwrap(), 0, "idempotent");

    let key = research::move_memory::motif(&Move::ChangeExit { mode: ExitMode::Trail });
    let history = MoveStore::new(w.pg.clone()).history(w.user, &key).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, Outcome::Failed, "a rejected candidate is a failed move");
    assert!(history[0].run_ref.starts_with(&move_run_ref(w.campaign_id, 0)[..45]), "{}", history[0].run_ref);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn windows_are_folded_into_the_prior_by_the_regime_in_force_at_their_start() {
    use chrono::TimeZone;
    let pg = pool().await;
    let n = Uuid::new_v4().simple().to_string();
    let (class, sym, scope) = (format!("cls-{n}"), format!("SYM-{n}"), format!("scope-{n}"));
    sqlx::query("INSERT INTO asset_lifecycle (symbol, asset_class, venue_id) VALUES ($1, $2, 'v')")
        .bind(&sym)
        .bind(&class)
        .execute(&pg)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO regime_causal.regime_scope
           (market_scope, asset_class, series_symbol, series_base_period_secs, series_period_secs, applies_to, rationale, configured_by)
         VALUES ($1, $2, $3, 86400, 86400, ARRAY[$3], $4, 'test')",
    )
    .bind(&scope)
    .bind(&class)
    .bind(&sym)
    .bind("a rationale of at least eighty characters so that the scope registry's own CHECK is satisfied here")
    .execute(&pg)
    .await
    .unwrap();
    let at = |m: u32, d: u32| chrono::Utc.with_ymd_and_hms(2024, m, d, 0, 0, 0).unwrap();
    for (when, label) in [(at(1, 1), "low_vol"), (at(2, 1), "high_vol")] {
        sqlx::query(
            "INSERT INTO regime_causal.regime_state
               (market_scope, event_time, model_version, p_filtered, regime_vec, knowledge_time, label)
             VALUES ($1, $2, $3, '{0.3,0.3,0.4}', '{0,0,1}', $2, $4)",
        )
        .bind(&scope)
        .bind(when)
        .bind(api::knowledge::RULE_MODEL_VERSION)
        .bind(label)
        .execute(&pg)
        .await
        .unwrap();
    }
    let mut spec: EvaluationSpec = serde_json::from_value(evaluation()).unwrap();
    spec.universe_ref = sym.clone();
    let tenant = format!("t-{n}");
    let windows = [
        (at(1, 15), at(1, 25)), // low_vol is in force
        (at(2, 15), at(2, 25)), // high_vol is in force
        (at(3, 15), at(3, 25)), // in force too, but this window's run failed
        (at(12, 1), at(12, 5)), // (nothing special: still the latest label)
    ];
    let values = [Some(0.5), Some(-0.2), None, Some(f64::NAN)];
    let folded = fold_regime_prior(&pg, &tenant, &spec, &windows, &values).await.unwrap();
    assert_eq!(folded, 2, "a failed run and a non-finite score contribute nothing");

    let store = PriorStore::new(pg.clone());
    let low = store.cells(&tenant, &sym, "low_vol").await.unwrap();
    assert_eq!((low["trend"].n_obs, low["trend"].n_success), (1, 1));
    let high = store.cells(&tenant, &sym, "high_vol").await.unwrap();
    assert_eq!((high["trend"].n_obs, high["trend"].n_success), (1, 0));

    // A window before the first label has no regime and is dropped, never given the nearest one.
    let early = [(at(1, 1) - chrono::Duration::days(30), at(1, 1) - chrono::Duration::days(20))];
    assert_eq!(fold_regime_prior(&pg, &tenant, &spec, &early, &[Some(1.0)]).await.unwrap(), 0);

    // An instrument with no configured scope folds nothing, quietly.
    let mut unscoped = spec.clone();
    unscoped.universe_ref = format!("NOPE-{n}");
    assert_eq!(fold_regime_prior(&pg, &tenant, &unscoped, &windows, &values).await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0245"]
async fn a_forecaster_whose_coverage_left_tolerance_twice_is_a_calibration_item() {
    use chrono::TimeZone;
    let pg = pool().await;
    let model = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by)
         VALUES ($1, $1, 'wire test', 'forecaster', 'crypto', '{}', $2)",
    )
    .bind(&model)
    .bind(Uuid::new_v4())
    .execute(&pg)
    .await
    .unwrap();
    let day = |d: u32| chrono::Utc.with_ymd_and_hms(2024, 6, d, 0, 0, 0).unwrap();
    for (d, c50, c90) in [(1, 0.5, 0.9), (2, 0.85, 0.99), (3, 0.9, 0.99), (4, 0.9, 1.0)] {
        sqlx::query(
            "INSERT INTO model_quality_series (model_id, window_end, coverage_50, coverage_90, n_forecasts)
             VALUES ($1, $2, $3, $4, 100)",
        )
        .bind(&model)
        .bind(day(d))
        .bind(c50)
        .bind(c90)
        .execute(&pg)
        .await
        .unwrap();
    }
    let drifts = api::routes::alerts_inbox::calibration_drifts(&pg).await.unwrap();
    let mine: Vec<_> = drifts.iter().filter(|d| d.model_id == model).collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, api::routes::alerts_inbox::calibration_item_id(&model, day(3)));
    sqlx::query("DELETE FROM ai_models WHERE model_id = $1").bind(&model).execute(&pg).await.unwrap();
}
