//! Training-run resumption against PostgreSQL (migration 0185; checklist 2.4/2.5;
//! ADR-C-08): checkpoints are persisted from the trainer's progress frames, a
//! partial one is refused, and boot recovery applies its policy to runs the
//! platform left `running`.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p model-registry --test pg_training_resume -- --ignored --test-threads=1
//! ```
//!
//! The scratch database needs migrations through 0185 (pgvector stubbed is fine).

use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, Registration, TrialEvent, TrialState, TrialSubject};
use model_registry::manager::ModelManager;
use serde_json::{json, Value};
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

fn manifest(step: i64) -> Value {
    json!({
        "step": step,
        "rng_state": { "python": "a", "numpy": "b", "torch_cpu": "c", "torch_cuda": [] },
        "optimizer_state_ref": "file:///x/state.pt",
        "lr_scheduler_state_ref": "file:///x/s.json",
        "dataloader_position": 72,
        "amp_scaler_state": "file:///x/a.json",
        "code_hash": "sha256:code",
        "image_digest": "img",
        "dataset_id": "ds",
        "resume_support": "bit_identical"
    })
}

/// A run the platform "left running": model, run row, a running trial, and
/// (optionally) its recorded spec.
async fn interrupted_run(pg: &sqlx::PgPool, with_spec: bool) -> (Uuid, Uuid, Uuid) {
    let user = Uuid::new_v4();
    let model_id = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by)
         VALUES ($1, $1, 'm', 'classifier', 'crypto', '{}'::jsonb, $2)",
    )
    .bind(&model_id)
    .bind(user)
    .execute(pg)
    .await
    .unwrap();
    let run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO training_runs (run_id, model_id, status, phase, created_by) VALUES ($1,$2,'running','fitting',$3)",
    )
    .bind(run)
    .bind(&model_id)
    .bind(user)
    .execute(pg)
    .await
    .unwrap();

    let led = PgTrialLedger::new(pg.clone());
    let tenant = user.to_string();
    let ctx = DispatchContext::legacy(&tenant, ledger::ActorKind::Human, &tenant);
    let subject = TrialSubject {
        config_hash: format!("sha256:{}", Uuid::new_v4().simple()),
        config: json!({}),
        dataset_id: "ds".into(),
        split_spec_id: None,
        code_hash: format!("model:{model_id}"),
        image_digest: "img".into(),
        seed_set: vec![],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: json!([]),
        planned_steps: None,
    };
    let mut ticket = led
        .register_async(&Registration::unlogged(&ctx, &subject))
        .await
        .unwrap();
    let trial = ticket.trial_id();
    for st in [
        TrialState::Queued,
        TrialState::Provision,
        TrialState::Running,
    ] {
        led.transition_async(&mut ticket, TrialEvent::to(st))
            .await
            .unwrap();
    }
    if with_spec {
        sqlx::query(
            "INSERT INTO mlops.training_run_spec (run_id, model_id, user_id, trial_id, hyperparameters, data, window_start, window_end)
             VALUES ($1,$2,$3,$4,'null'::jsonb,'null'::jsonb, now() - interval '30 days', now())",
        )
        .bind(run)
        .bind(&model_id)
        .bind(user)
        .bind(trial)
        .execute(pg)
        .await
        .unwrap();
    }
    (run, user, trial)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0185"]
async fn the_platform_role_can_write_checkpoints_and_never_edit_them() {
    let pg = pool().await;
    for role in ["app_role"] {
        let mut tx = pg.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let run = Uuid::new_v4();
        assert!(sqlx::query("INSERT INTO mlops.training_checkpoint (run_id, step, manifest) VALUES ($1, 1, '{}')")
            .bind(run)
            .execute(&mut *tx)
            .await
            .is_ok(), "{role} cannot append a checkpoint (sequence grant?)");
        assert!(sqlx::query("SELECT 1 FROM mlops.training_checkpoint")
            .fetch_all(&mut *tx)
            .await
            .is_ok());
        assert!(sqlx::query("UPDATE mlops.training_checkpoint SET step = 2")
            .execute(&mut *tx)
            .await
            .is_err());
        tx.rollback().await.unwrap();
        let mut tx = pg.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        assert!(sqlx::query("DELETE FROM mlops.training_checkpoint")
            .execute(&mut *tx)
            .await
            .is_err());
        assert!(sqlx::query("DELETE FROM mlops.training_run_spec")
            .execute(&mut *tx)
            .await
            .is_err());
        tx.rollback().await.unwrap();
    }
    // A non-object manifest is refused by the table, not by a code path.
    assert!(sqlx::query(
        "INSERT INTO mlops.training_checkpoint (run_id, step, manifest) VALUES ($1, 1, '[]')"
    )
    .bind(Uuid::new_v4())
    .execute(&pg)
    .await
    .is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0185"]
async fn a_progress_frame_stores_a_good_checkpoint_and_refuses_a_partial_one() {
    let pg = pool().await;
    let (run, _, _) = interrupted_run(&pg, true).await;
    let mgr = ModelManager::from_env(pg.clone());
    mgr.apply_nats_progress(
        run,
        "checkpoint",
        40.0,
        Some(json!({ "checkpoint": manifest(3) })),
    )
    .await;
    let mut partial = manifest(4);
    partial.as_object_mut().unwrap().remove("rng_state");
    mgr.apply_nats_progress(
        run,
        "checkpoint",
        50.0,
        Some(json!({ "checkpoint": partial })),
    )
    .await;
    // An ordinary frame stores nothing.
    mgr.apply_nats_progress(run, "fitting", 60.0, Some(json!({ "epoch": 6 })))
        .await;
    let steps: Vec<(i64,)> =
        sqlx::query_as("SELECT step FROM mlops.training_checkpoint WHERE run_id = $1 ORDER BY id")
            .bind(run)
            .fetch_all(&pg)
            .await
            .unwrap();
    assert_eq!(steps, vec![(3,)], "the partial manifest never got in");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0185"]
async fn boot_recovery_fails_runs_it_cannot_resume_with_a_named_reason() {
    let pg = pool().await;
    let (no_ckpt, _, trial) = interrupted_run(&pg, true).await;
    let (no_spec, _, _) = interrupted_run(&pg, false).await;
    // Has a checkpoint and a spec, but the plan cannot be rebuilt here (no model
    // definition, no bars): the redrive fails and says so, rather than hanging.
    let (unrebuildable, _, _) = interrupted_run(&pg, true).await;
    sqlx::query(
        "INSERT INTO mlops.training_checkpoint (run_id, step, manifest) VALUES ($1, 2, $2)",
    )
    .bind(unrebuildable)
    .bind(manifest(2))
    .execute(&pg)
    .await
    .unwrap();

    let mgr = ModelManager::from_env(pg.clone());
    let report = mgr.recover_interrupted_runs().await;
    assert!(report.failed_no_checkpoint.contains(&no_ckpt));
    assert!(report.failed_no_checkpoint.contains(&unrebuildable));
    assert!(report.failed_no_spec.contains(&no_spec));
    assert!(!report.resumed.contains(&unrebuildable));

    let err = |run: Uuid| {
        let pg = pg.clone();
        async move {
            let (status, error): (String, Option<String>) =
                sqlx::query_as("SELECT status, error FROM training_runs WHERE run_id = $1")
                    .bind(run)
                    .fetch_one(&pg)
                    .await
                    .unwrap();
            (status, error.unwrap_or_default())
        }
    };
    let (s, e) = err(no_ckpt).await;
    assert_eq!(s, "failed");
    assert!(
        e.contains("interrupted_no_checkpoint") && e.contains("not restarted from step 0"),
        "{e}"
    );
    let (_, e) = err(no_spec).await;
    assert!(e.contains("interrupted_no_spec"), "{e}");
    let (_, e) = err(unrebuildable).await;
    assert!(e.contains("interrupted_unrecoverable"), "{e}");

    // The trial is settled as abandoned-after-preemption, not left running.
    let (state, reason): (String, Option<String>) =
        sqlx::query_as("SELECT state, terminal_reason FROM mlops.trial_state WHERE trial_id = $1")
            .bind(trial)
            .fetch_one(&pg)
            .await
            .unwrap();
    assert_eq!(state, "failed");
    assert_eq!(reason.as_deref(), Some("preempted_abandoned"));

    // A second pass finds nothing left to recover.
    let again = mgr.recover_interrupted_runs().await;
    assert!(!again.failed_no_checkpoint.contains(&no_ckpt));
}
