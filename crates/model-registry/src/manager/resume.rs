//! Resuming a training run after the platform dies (SPEC §9, checklist 2.4/2.5,
//! ADR-P2-07, ADR-C-08; OPEN-TASKS C1.2).
//!
//! # What was missing
//!
//! `jobs::checkpoint::plan_resume` and the trainer's checkpoint writer both
//! existed, and nothing joined them: a training run is an in-process task around
//! one HTTP call, so a restart left a `training_runs` row saying `running` for
//! ever, no checkpoint was stored anywhere the platform could find it, and the
//! trainer was never told to resume from one. This module is the join.
//!
//! * **Persist.** The trainer publishes each checkpoint manifest as a progress
//!   frame; [`ModelManager::record_checkpoint`] refuses a manifest that does not
//!   satisfy the contract and appends the rest to `mlops.training_checkpoint`.
//! * **Decide.** [`ModelManager::resume_for`] asks the trainer which code and
//!   image it is running, takes the run's newest checkpoint and hands both to
//!   `plan_resume`. The decision stays in Rust; the trainer executes it.
//! * **Recover.** [`ModelManager::recover_interrupted_runs`] runs at boot, finds
//!   runs left `queued`/`running`, and applies the policy below.
//!
//! # The policy (ADR-C-08)
//!
//! A run found interrupted is **re-driven only if a checkpoint exists**; otherwise
//! it is marked failed with a named reason. *Rejected alternative:* restarting
//! every interrupted run from step 0. A restart is correct under the same trial,
//! but it silently re-spends hours of compute after every deploy and nobody asked
//! for it; "no checkpoint, so not resumed" is a statement an operator can act on.
//! A re-drive keeps the **same trial** (the trial is the look, not the process),
//! and the data window comes from `mlops.training_run_spec`, because "now minus N
//! days" rebuilt after a restart would be different data.

use serde_json::Value;

#[allow(clippy::wildcard_imports)] // a child module of `manager`, sharing its imports
use super::*;

/// What boot recovery does with one interrupted run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Rebuild the plan and drive it again, resuming from the checkpoint.
    Resume,
    /// Interrupted with nothing to resume from. Failed, with that reason.
    FailNoCheckpoint,
    /// Started before the spec was recorded, so the request cannot be rebuilt.
    FailNoSpec,
}

/// The recovery policy as a pure function, so it is tested without a database.
#[must_use]
pub fn recovery_action(has_spec: bool, has_checkpoint: bool) -> RecoveryAction {
    match (has_spec, has_checkpoint) {
        (false, _) => RecoveryAction::FailNoSpec,
        (true, false) => RecoveryAction::FailNoCheckpoint,
        (true, true) => RecoveryAction::Resume,
    }
}

/// `plan_resume`'s answer, in the shape `drive_train` acts on.
#[derive(Debug, Clone, PartialEq)]
pub enum ResumeDecision {
    /// Train from step 0 (no checkpoint, or one that does not apply).
    Start,
    /// Continue from this manifest.
    Resume(Value),
    /// A stored checkpoint that fails the contract; the run fails, it is not
    /// quietly restarted.
    Abandon(String),
}

/// Judge a run's newest checkpoint against the computation about to run.
#[must_use]
pub fn resume_decision(
    latest: Option<&Value>,
    code_hash: &str,
    image_digest: &str,
    dataset_id: &str,
) -> ResumeDecision {
    use jobs::checkpoint::ResumePlan;
    match jobs::checkpoint::plan_resume(latest, code_hash, image_digest, dataset_id) {
        ResumePlan::Resume { .. } => latest
            .cloned()
            .map_or(ResumeDecision::Start, ResumeDecision::Resume),
        ResumePlan::Restart { reason } => {
            tracing::info!(%reason, "training starts from step 0 under the same trial");
            ResumeDecision::Start
        }
        ResumePlan::Abandon { reason } => ResumeDecision::Abandon(reason),
    }
}

/// What a recovery pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub resumed: Vec<Uuid>,
    pub failed_no_checkpoint: Vec<Uuid>,
    pub failed_no_spec: Vec<Uuid>,
}

impl ModelManager {
    /// Record the request that started a run, with its window resolved.
    pub(super) async fn record_spec(
        &self,
        run_id: Uuid,
        model_id: &str,
        user_id: Uuid,
        trial_id: Uuid,
        hyperparameters: &Value,
        data: &Value,
        version_note: Option<&str>,
        window: (chrono::DateTime<Utc>, chrono::DateTime<Utc>),
    ) {
        let r = sqlx::query(
            "INSERT INTO mlops.training_run_spec
                 (run_id, model_id, user_id, trial_id, hyperparameters, data, version_note,
                  window_start, window_end)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT (run_id) DO NOTHING",
        )
        .bind(run_id)
        .bind(model_id)
        .bind(user_id)
        .bind(trial_id)
        .bind(hyperparameters)
        .bind(data)
        .bind(version_note)
        .bind(window.0)
        .bind(window.1)
        .execute(&self.pg)
        .await;
        if let Err(e) = r {
            // Not fatal: the run proceeds and is simply not recoverable.
            tracing::warn!(error = %e, %run_id, "training run spec not recorded; the run cannot be resumed after a restart");
        }
    }

    /// Store a checkpoint manifest the trainer published. A manifest that fails
    /// the contract is refused and logged: a partial checkpoint is the
    /// silent-divergence bug, and only the door can stop it.
    pub(super) async fn record_checkpoint(&self, run_id: Uuid, manifest: &Value) {
        if let Err(e) = jobs::checkpoint::CheckpointManifest::parse(manifest) {
            tracing::error!(%run_id, error = %e, "refusing a checkpoint that fails the contract");
            return;
        }
        let step = manifest.get("step").and_then(Value::as_i64).unwrap_or(0);
        if let Err(e) = sqlx::query(
            "INSERT INTO mlops.training_checkpoint (run_id, step, manifest) VALUES ($1,$2,$3)",
        )
        .bind(run_id)
        .bind(step)
        .bind(manifest)
        .execute(&self.pg)
        .await
        {
            tracing::warn!(%run_id, error = %e, "checkpoint not stored");
        }
    }

    pub(super) async fn latest_checkpoint(&self, run_id: Uuid) -> Option<Value> {
        sqlx::query_scalar(
            "SELECT manifest FROM mlops.training_checkpoint WHERE run_id = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
    }

    /// The manifest to resume from, if `plan_resume` says to.
    ///
    /// `Err(reason)` means a stored checkpoint failed the contract and the run
    /// must fail. With no checkpoint, or no way to learn the trainer's identity,
    /// the answer is `Ok(None)`: start from step 0.
    pub(super) async fn resume_for(
        &self,
        run_id: Uuid,
        dataset_id: &str,
    ) -> Result<Option<Value>, String> {
        let Some(latest) = self.latest_checkpoint(run_id).await else {
            return Ok(None);
        };
        let Some((code_hash, image_digest)) = self.sidecar.identity().await else {
            tracing::warn!(%run_id, "the trainer did not say which code it runs; a checkpoint exists and is not used");
            return Ok(None);
        };
        match resume_decision(Some(&latest), &code_hash, &image_digest, dataset_id) {
            ResumeDecision::Start => Ok(None),
            ResumeDecision::Resume(m) => Ok(Some(m)),
            ResumeDecision::Abandon(why) => Err(why),
        }
    }

    /// Boot recovery: apply the policy to every run left `queued` or `running`.
    pub async fn recover_interrupted_runs(self: &Arc<Self>) -> RecoveryReport {
        let mut report = RecoveryReport::default();
        let rows: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
            "SELECT run_id, model_id, created_by FROM training_runs
              WHERE status IN ('queued','running') ORDER BY created_at",
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();
        for (run_id, model_id, user_id) in rows {
            if self.jobs.read().await.contains_key(&run_id) {
                continue; // alive in this process
            }
            let spec: Option<(
                Uuid,
                Value,
                Value,
                Option<String>,
                chrono::DateTime<Utc>,
                chrono::DateTime<Utc>,
            )> = sqlx::query_as(
                "SELECT trial_id, hyperparameters, data, version_note, window_start, window_end
                       FROM mlops.training_run_spec WHERE run_id = $1",
            )
            .bind(run_id)
            .fetch_optional(&self.pg)
            .await
            .ok()
            .flatten();
            let has_checkpoint = self.latest_checkpoint(run_id).await.is_some();
            match recovery_action(spec.is_some(), has_checkpoint) {
                RecoveryAction::FailNoSpec => {
                    self.mark_interrupted(run_id, None, "interrupted_no_spec: the run predates the recorded request, so it cannot be rebuilt").await;
                    report.failed_no_spec.push(run_id);
                }
                RecoveryAction::FailNoCheckpoint => {
                    let trial = spec.as_ref().map(|s| s.0);
                    self.mark_interrupted(run_id, trial.map(|t| (user_id, t)), "interrupted_no_checkpoint: the platform stopped mid-run and no checkpoint had been written; not restarted from step 0").await;
                    report.failed_no_checkpoint.push(run_id);
                }
                RecoveryAction::Resume => {
                    let (trial_id, hp, data, note, ws, we) = spec.expect("Resume implies a spec");
                    match self
                        .redrive(
                            run_id,
                            &model_id,
                            user_id,
                            trial_id,
                            hp,
                            data,
                            note,
                            (ws, we),
                        )
                        .await
                    {
                        Ok(()) => report.resumed.push(run_id),
                        Err(why) => {
                            self.mark_interrupted(
                                run_id,
                                Some((user_id, trial_id)),
                                &format!("interrupted_unrecoverable: {why}"),
                            )
                            .await;
                            report.failed_no_checkpoint.push(run_id);
                        }
                    }
                }
            }
        }
        report
    }

    /// Mark an interrupted run failed with a named reason, and settle its trial
    /// as `preempted_abandoned` when it has one.
    async fn mark_interrupted(&self, run_id: Uuid, trial: Option<(Uuid, Uuid)>, reason: &str) {
        let _ = sqlx::query(
            "UPDATE training_runs SET status='failed', phase='failed', error=$2, finished_at=now()
              WHERE run_id=$1",
        )
        .bind(run_id)
        .bind(reason)
        .execute(&self.pg)
        .await;
        if let Some((user_id, trial_id)) = trial {
            match self
                .ledger
                .resume_ticket_async(&user_id.to_string(), trial_id)
                .await
            {
                Ok(mut ticket) => {
                    let _ = self
                        .ledger
                        .transition_async(
                            &mut ticket,
                            ledger::TrialEvent::to(ledger::TrialState::Preempted),
                        )
                        .await;
                    if let Err(e) = self
                        .ledger
                        .settle_async(
                            ticket,
                            ledger::TrialEvent::failed(
                                ledger::TerminalReason::PreemptedAbandoned,
                                reason.to_string(),
                            ),
                        )
                        .await
                    {
                        tracing::warn!(%run_id, error = %e, "could not settle an interrupted run's trial");
                    }
                }
                Err(e) => {
                    tracing::warn!(%run_id, error = %e, "interrupted run's trial is not resumable")
                }
            }
        }
        tracing::warn!(%run_id, reason, "an interrupted training run was failed at boot");
    }

    /// Rebuild a run's plan from its recorded spec and drive it again under the
    /// same trial.
    async fn redrive(
        self: &Arc<Self>,
        run_id: Uuid,
        model_id: &str,
        user_id: Uuid,
        trial_id: Uuid,
        hp: Value,
        data: Value,
        version_note: Option<String>,
        window: (chrono::DateTime<Utc>, chrono::DateTime<Utc>),
    ) -> anyhow::Result<()> {
        let definition = self.load_definition(model_id, &hp).await?;
        let selection: Option<crate::types::TrainDataSelection> = if data.is_null() {
            None
        } else {
            serde_json::from_value(data)?
        };
        let artifacts_prefix =
            std::env::var("ARTIFACT_STORE_PATH").unwrap_or_else(|_| "./artifacts".to_string());
        let plan = self
            .datasets
            .plan(
                &user_id.to_string(),
                dataset_request(&definition, selection.as_ref(), window, &artifacts_prefix)?,
            )
            .await?;
        let mut ticket = self
            .ledger
            .resume_ticket_async(&user_id.to_string(), trial_id)
            .await
            .map_err(|e| anyhow::anyhow!("the run's trial cannot be resumed: {e}"))?;
        // The trial's own history says what happened: it was running, it was
        // preempted, it recovered. Same trial throughout.
        for st in [
            ledger::TrialState::Preempted,
            ledger::TrialState::Recovering,
            ledger::TrialState::Running,
        ] {
            if let Err(e) = self
                .ledger
                .transition_async(&mut ticket, ledger::TrialEvent::to(st))
                .await
            {
                tracing::warn!(%run_id, error = %e, state = ?st, "recovery transition refused");
            }
        }
        let job = Job::new(
            run_id,
            model_id.to_string(),
            user_id,
            ModelRunKind::Train,
            Utc::now(),
        );
        self.jobs.write().await.insert(run_id, Arc::clone(&job));
        let manager = Arc::clone(self);
        let mid = model_id.to_string();
        tokio::spawn(async move {
            manager
                .drive_train(Arc::clone(&job), mid, definition, plan, version_note)
                .await;
            manager.settle_trial(ticket, &job).await;
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(code: &str) -> Value {
        json!({
            "step": 3,
            "rng_state": { "python": "a", "numpy": "b", "torch_cpu": "c", "torch_cuda": [] },
            "optimizer_state_ref": "file:///x/state.pt",
            "lr_scheduler_state_ref": "file:///x/s.json",
            "dataloader_position": 72,
            "amp_scaler_state": "file:///x/a.json",
            "code_hash": code,
            "image_digest": "img",
            "dataset_id": "ds",
            "resume_support": "bit_identical"
        })
    }

    /// The recovery policy: re-drive only when there is something to resume from.
    #[test]
    fn an_interrupted_run_is_resumed_only_if_it_has_a_checkpoint() {
        assert_eq!(recovery_action(true, true), RecoveryAction::Resume);
        assert_eq!(
            recovery_action(true, false),
            RecoveryAction::FailNoCheckpoint
        );
        assert_eq!(recovery_action(false, true), RecoveryAction::FailNoSpec);
        assert_eq!(recovery_action(false, false), RecoveryAction::FailNoSpec);
    }

    #[test]
    fn a_matching_checkpoint_is_resumed_and_the_manifest_travels() {
        let m = manifest("code");
        assert_eq!(
            resume_decision(Some(&m), "code", "img", "ds"),
            ResumeDecision::Resume(m.clone())
        );
    }

    /// A checkpoint from other code, another image or another dataset is not this
    /// computation's: start from 0, under the same trial.
    #[test]
    fn a_checkpoint_from_a_different_computation_starts_over() {
        let m = manifest("old-code");
        assert_eq!(
            resume_decision(Some(&m), "new-code", "img", "ds"),
            ResumeDecision::Start
        );
        assert_eq!(
            resume_decision(Some(&m), "old-code", "other", "ds"),
            ResumeDecision::Start
        );
        assert_eq!(
            resume_decision(Some(&m), "old-code", "img", "other"),
            ResumeDecision::Start
        );
        assert_eq!(resume_decision(None, "c", "i", "d"), ResumeDecision::Start);
    }

    /// A stored checkpoint that fails the contract fails the run; it is never
    /// quietly treated as absent.
    #[test]
    fn a_stored_partial_checkpoint_abandons_the_run() {
        let mut m = manifest("code");
        m.as_object_mut().unwrap().remove("rng_state");
        match resume_decision(Some(&m), "code", "img", "ds") {
            ResumeDecision::Abandon(why) => assert!(why.contains("checkpoint_invalid"), "{why}"),
            other => panic!("{other:?}"),
        }
    }
}
