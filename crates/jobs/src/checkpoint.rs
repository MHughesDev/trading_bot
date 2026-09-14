//! The checkpoint manifest contract (SPEC §9, checklist 2.4/2.5, ADR-P2-07,
//! AT-61).
//!
//! A checkpoint is a promise that a run can be continued and reach the same
//! answer it would have reached uninterrupted. Almost every field below is a
//! piece of state that, left out, makes that promise quietly false:
//!
//! * **RNG state** — omit it and the resumed run draws a different dropout mask,
//!   a different shuffle, a different augmentation. The loss curve still looks
//!   plausible. Nothing announces the divergence.
//! * **Optimizer state** — Adam's moments are most of what the optimizer *is*.
//!   Resuming without them restarts the adaptation from scratch at whatever
//!   learning rate the schedule had reached, which is usually worse than
//!   restarting from step 0.
//! * **Dataloader position** — resume from the top of the epoch and the model
//!   sees some rows twice and some not at all in that epoch.
//! * **AMP scaler state** — the loss scale is a running measurement. Reset it
//!   and the first resumed steps either overflow or waste precision.
//! * **`code_hash` / `image_digest` / `dataset_id`** — a checkpoint is only
//!   resumable *into the same computation*. Resuming into different code or a
//!   different dataset produces a model whose provenance is a lie.
//!
//! So the registry **refuses** a checkpoint manifest missing any of them. A
//! partial checkpoint is the classic silent-divergence bug: the run resumes, the
//! numbers look fine, and the result is not the one the uninterrupted run would
//! have produced. Refusing to store one is the only place that can be caught,
//! because by the time anyone sees the output there is nothing left to compare
//! it against.
//!
//! ## Frameworks that cannot do this
//!
//! [`ResumeSupport::Unsupported`] is a real answer. A framework whose resume is
//! not bit-identical is marked as such in its adapter, and 2.5 restarts the run
//! from step 0 **under the same trial** rather than pretending. The trial is the
//! look, not the process — a restart is not a second look at the data.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether a framework's resume reproduces the uninterrupted run exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeSupport {
    /// Resume is bit-identical, demonstrated by AT-61 rather than asserted.
    BitIdentical,
    /// Resume is not bit-identical. The run restarts from step 0 under the same
    /// trial (ADR-P2-07). Stated, not silently tolerated.
    Unsupported,
}

/// RNG state, per source. Every field is required because every one of them is
/// consumed by a real training loop, and the one that is missing is the one that
/// diverges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RngState {
    pub python: String,
    pub numpy: String,
    pub torch_cpu: String,
    /// One per visible CUDA device, in device order. Empty is valid — a CPU run
    /// has no CUDA generators — but the field is not optional, because "absent"
    /// and "empty" are different claims and only one of them is checkable.
    pub torch_cuda: Vec<String>,
}

/// The GBDT half of the contract.
///
/// A gradient-boosted model has no optimizer moments or dataloader position, but
/// it has a boosting round and a set of determinism flags that decide whether
/// two runs of the same configuration agree at all. LightGBM's `force_row_wise`
/// is in here because without it the histogram construction order depends on
/// thread scheduling, and the model differs run to run on the same machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoostingState {
    pub boosting_round: u32,
    pub seed: u64,
    /// The determinism flags actually in effect, as `name -> value`. Recorded
    /// rather than assumed: a checkpoint written by a run that had them off is
    /// not reproducible, and the manifest should say so instead of implying
    /// otherwise.
    pub determinism_flags: std::collections::BTreeMap<String, String>,
}

/// Why a checkpoint manifest was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    #[error("the checkpoint manifest has no `{0}`; a checkpoint missing it resumes into a different computation and says nothing about it")]
    Missing(&'static str),
    #[error("`{field}` is empty; an empty reference is not a reference")]
    Empty { field: &'static str },
    #[error("the manifest is not an object")]
    NotAnObject,
    #[error("a GBDT checkpoint must declare the determinism flags in effect; a run without them is not reproducible and the manifest must not imply it is")]
    NoDeterminismFlags,
}

/// A checkpoint manifest that passed the contract.
///
/// Sealed: [`CheckpointManifest::parse`] is the only way to make one, and the
/// artifact registry takes one of these rather than a `Value`. So "stored a
/// checkpoint whose manifest was never checked" is not a thing that can happen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub step: u64,
    pub rng_state: RngState,
    pub optimizer_state_ref: String,
    pub lr_scheduler_state_ref: String,
    pub dataloader_position: u64,
    pub amp_scaler_state: String,
    /// The computation this checkpoint belongs to. Resuming into anything else
    /// produces a model whose provenance is a lie.
    pub code_hash: String,
    pub image_digest: String,
    pub dataset_id: String,
    pub resume_support: ResumeSupport,
    /// Present for gradient-boosted frameworks, absent for gradient-descent ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boosting: Option<BoostingState>,
}

/// Reads one of the manifest's string references.
type RefAccessor = fn(&CheckpointManifest) -> &String;

/// The gradient-descent fields, by the name they carry on the wire.
const REQUIRED_REFS: [(&str, RefAccessor); 5] = [
    ("optimizer_state_ref", |m| &m.optimizer_state_ref),
    ("lr_scheduler_state_ref", |m| &m.lr_scheduler_state_ref),
    ("amp_scaler_state", |m| &m.amp_scaler_state),
    ("code_hash", |m| &m.code_hash),
    ("image_digest", |m| &m.image_digest),
];

impl CheckpointManifest {
    /// Parse and check a manifest.
    ///
    /// # Errors
    /// Any missing or empty contract field. There is no partial success: a
    /// manifest that fails here is not stored, because a checkpoint nobody can
    /// resume from is worse than no checkpoint — it looks like insurance.
    pub fn parse(value: &Value) -> Result<Self, CheckpointError> {
        if !value.is_object() {
            return Err(CheckpointError::NotAnObject);
        }
        // Field-by-field, so the error names the field rather than saying the
        // shape was wrong somewhere.
        for field in [
            "step",
            "rng_state",
            "optimizer_state_ref",
            "lr_scheduler_state_ref",
            "dataloader_position",
            "amp_scaler_state",
            "code_hash",
            "image_digest",
            "dataset_id",
            "resume_support",
        ] {
            if value.get(field).is_none_or(Value::is_null) {
                return Err(CheckpointError::Missing(leak(field)));
            }
        }
        for field in ["python", "numpy", "torch_cpu", "torch_cuda"] {
            if value
                .get("rng_state")
                .and_then(|r| r.get(field))
                .is_none_or(Value::is_null)
            {
                return Err(CheckpointError::Missing(leak(field)));
            }
        }

        let manifest: Self = serde_json::from_value(value.clone())
            .map_err(|_| CheckpointError::NotAnObject)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The checks that are about content rather than presence.
    ///
    /// # Errors
    /// An empty reference, or a GBDT checkpoint with no determinism flags.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        for (name, get) in REQUIRED_REFS {
            if get(self).trim().is_empty() {
                return Err(CheckpointError::Empty { field: leak(name) });
            }
        }
        if self.dataset_id.trim().is_empty() {
            return Err(CheckpointError::Empty { field: "dataset_id" });
        }
        for (name, value) in [
            ("python", &self.rng_state.python),
            ("numpy", &self.rng_state.numpy),
            ("torch_cpu", &self.rng_state.torch_cpu),
        ] {
            if value.trim().is_empty() {
                return Err(CheckpointError::Empty { field: leak(name) });
            }
        }
        if let Some(b) = &self.boosting {
            if b.determinism_flags.is_empty() {
                return Err(CheckpointError::NoDeterminismFlags);
            }
        }
        Ok(())
    }

    /// Whether resuming from this checkpoint is expected to reproduce the
    /// uninterrupted run.
    ///
    /// A caller that gets `false` restarts from step 0 under the same trial. It
    /// does **not** resume anyway and hope.
    #[must_use]
    pub fn resumable(&self) -> bool {
        self.resume_support == ResumeSupport::BitIdentical
    }

    /// Whether this checkpoint belongs to the computation about to resume it.
    ///
    /// All three have to match. Resuming into different code is a different
    /// model; into a different image, a different numeric library; into a
    /// different dataset, a different question.
    #[must_use]
    pub fn matches(&self, code_hash: &str, image_digest: &str, dataset_id: &str) -> bool {
        self.code_hash == code_hash
            && self.image_digest == image_digest
            && self.dataset_id == dataset_id
    }
}

/// The field names are compile-time constants; this keeps the error type's
/// lifetime `'static` without allocating per failure.
fn leak(field: &str) -> &'static str {
    const NAMES: [&str; 15] = [
        "step",
        "rng_state",
        "optimizer_state_ref",
        "lr_scheduler_state_ref",
        "dataloader_position",
        "amp_scaler_state",
        "code_hash",
        "image_digest",
        "dataset_id",
        "resume_support",
        "python",
        "numpy",
        "torch_cpu",
        "torch_cuda",
        "boosting",
    ];
    NAMES.iter().copied().find(|n| *n == field).unwrap_or("unknown_field")
}

// ───────────────────────────────────────────────────────────────────────────────
// 2.4 — checkpoint cadence
// ───────────────────────────────────────────────────────────────────────────────

/// The optimal checkpoint interval, `√(2·cost·MTBF)` (Young/Daly).
///
/// The shape is the same as the economic order quantity, and for the same
/// reason: checkpointing too often pays the write cost repeatedly, too rarely
/// pays the lost-work cost when something dies. The minimum is where the two
/// curves cross.
///
/// Returns seconds. `None` when either input is not positive — an unmeasured
/// checkpoint cost or an unknown failure rate does not get a made-up interval,
/// because a made-up interval is indistinguishable from a measured one once it
/// is in a config file.
#[must_use]
pub fn checkpoint_interval_seconds(checkpoint_cost_s: f64, mtbf_s: f64) -> Option<f64> {
    (checkpoint_cost_s.is_finite()
        && mtbf_s.is_finite()
        && checkpoint_cost_s > 0.0
        && mtbf_s > 0.0)
        .then(|| (2.0 * checkpoint_cost_s * mtbf_s).sqrt())
}

/// §11.3's threshold for turning freeze-thaw on: resume must cost less than this
/// fraction of a rung's duration, **measured**, not assumed.
pub const FREEZE_THAW_MAX_RESUME_FRACTION: f64 = 0.15;

/// Whether freeze-thaw (pausing at a rung boundary) is worth it for this
/// framework.
///
/// Off by default and turned on only by a measurement (ADR-P2-07). An unmeasured
/// resume cost returns `false`: until somebody has timed it, stopping is
/// terminal and right-censored, which loses work but never silently costs more
/// than it saves.
#[must_use]
pub fn freeze_thaw_worthwhile(resume_cost_s: f64, rung_duration_s: f64) -> bool {
    resume_cost_s.is_finite()
        && rung_duration_s.is_finite()
        && resume_cost_s > 0.0
        && rung_duration_s > 0.0
        && resume_cost_s < rung_duration_s * FREEZE_THAW_MAX_RESUME_FRACTION
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn complete() -> Value {
        json!({
            "step": 1200,
            "rng_state": {
                "python": "abc",
                "numpy": "def",
                "torch_cpu": "ghi",
                "torch_cuda": ["jkl"]
            },
            "optimizer_state_ref": "art_opt",
            "lr_scheduler_state_ref": "art_sched",
            "dataloader_position": 4096,
            "amp_scaler_state": "art_scaler",
            "code_hash": "sha256:code",
            "image_digest": "sha256:image",
            "dataset_id": "ds:abc",
            "resume_support": "bit_identical"
        })
    }

    #[test]
    fn a_complete_manifest_parses() {
        let m = CheckpointManifest::parse(&complete()).expect("complete");
        assert_eq!(m.step, 1200);
        assert_eq!(m.dataloader_position, 4096);
        assert!(m.resumable());
        assert!(m.matches("sha256:code", "sha256:image", "ds:abc"));
    }

    /// Every contract field, one at a time. This is the test that matters: a
    /// partial checkpoint is the classic silent-divergence bug, and the only
    /// place to catch it is before it is stored.
    #[test]
    fn every_missing_contract_field_is_refused_by_name() {
        for field in [
            "step",
            "rng_state",
            "optimizer_state_ref",
            "lr_scheduler_state_ref",
            "dataloader_position",
            "amp_scaler_state",
            "code_hash",
            "image_digest",
            "dataset_id",
            "resume_support",
        ] {
            let mut v = complete();
            v.as_object_mut().unwrap().remove(field);
            match CheckpointManifest::parse(&v) {
                Err(CheckpointError::Missing(named)) => assert_eq!(named, field),
                other => panic!("removing {field} gave {other:?}"),
            }
        }
    }

    #[test]
    fn every_missing_rng_source_is_refused_by_name() {
        for field in ["python", "numpy", "torch_cpu", "torch_cuda"] {
            let mut v = complete();
            v["rng_state"].as_object_mut().unwrap().remove(field);
            match CheckpointManifest::parse(&v) {
                Err(CheckpointError::Missing(named)) => assert_eq!(named, field),
                other => panic!("removing rng_state.{field} gave {other:?}"),
            }
        }
    }

    /// A CPU run has no CUDA generators. Empty is a valid answer; absent is not,
    /// because only one of them is a claim somebody made.
    #[test]
    fn no_cuda_devices_is_a_valid_answer_and_absent_is_not() {
        let mut v = complete();
        v["rng_state"]["torch_cuda"] = json!([]);
        assert!(CheckpointManifest::parse(&v).is_ok());

        v["rng_state"].as_object_mut().unwrap().remove("torch_cuda");
        assert!(CheckpointManifest::parse(&v).is_err());
    }

    #[test]
    fn an_empty_reference_is_not_a_reference() {
        for field in [
            "optimizer_state_ref",
            "lr_scheduler_state_ref",
            "amp_scaler_state",
            "code_hash",
            "image_digest",
            "dataset_id",
        ] {
            let mut v = complete();
            v[field] = json!("   ");
            match CheckpointManifest::parse(&v) {
                Err(CheckpointError::Empty { field: named }) => assert_eq!(named, field),
                other => panic!("blank {field} gave {other:?}"),
            }
        }
    }

    /// LightGBM without `force_row_wise` builds histograms in thread-scheduling
    /// order and differs run to run on one machine. A GBDT checkpoint that does
    /// not say which flags were on is not reproducible, and the manifest must
    /// not imply it is.
    #[test]
    fn a_gbdt_checkpoint_must_state_its_determinism_flags() {
        let mut v = complete();
        v["boosting"] = json!({
            "boosting_round": 400,
            "seed": 7,
            "determinism_flags": {}
        });
        assert_eq!(
            CheckpointManifest::parse(&v),
            Err(CheckpointError::NoDeterminismFlags)
        );

        v["boosting"]["determinism_flags"] =
            json!({ "deterministic": "true", "force_row_wise": "true" });
        let m = CheckpointManifest::parse(&v).expect("flags declared");
        assert_eq!(m.boosting.unwrap().boosting_round, 400);
    }

    /// A framework that cannot resume bit-identically says so, and the caller
    /// restarts rather than resuming and hoping.
    #[test]
    fn an_unsupported_resume_is_a_stated_answer() {
        let mut v = complete();
        v["resume_support"] = json!("unsupported");
        let m = CheckpointManifest::parse(&v).expect("still a valid manifest");
        assert!(!m.resumable());
    }

    /// Resuming into different code, a different image or a different dataset is
    /// resuming into a different computation.
    #[test]
    fn a_checkpoint_only_matches_its_own_computation() {
        let m = CheckpointManifest::parse(&complete()).unwrap();
        assert!(!m.matches("sha256:other", "sha256:image", "ds:abc"));
        assert!(!m.matches("sha256:code", "sha256:other", "ds:abc"));
        assert!(!m.matches("sha256:code", "sha256:image", "ds:other"));
    }

    #[test]
    fn the_checkpoint_interval_is_the_young_daly_optimum() {
        // 10 s to write, an hour between failures → about 268 s.
        let i = checkpoint_interval_seconds(10.0, 3600.0).unwrap();
        assert!((i - (2.0 * 10.0 * 3600.0_f64).sqrt()).abs() < 1e-9);
        assert!(i > 250.0 && i < 290.0, "{i}");

        // A cheaper checkpoint is taken more often; a more reliable box, less.
        assert!(checkpoint_interval_seconds(1.0, 3600.0).unwrap() < i);
        assert!(checkpoint_interval_seconds(10.0, 36_000.0).unwrap() > i);
    }

    /// An unmeasured cost gets no interval. A made-up interval is
    /// indistinguishable from a measured one once it is in a config file.
    #[test]
    fn an_unmeasured_cost_gets_no_interval() {
        assert_eq!(checkpoint_interval_seconds(0.0, 3600.0), None);
        assert_eq!(checkpoint_interval_seconds(10.0, 0.0), None);
        assert_eq!(checkpoint_interval_seconds(f64::NAN, 3600.0), None);
    }

    /// Freeze-thaw is off until somebody measures it. Not "off unless
    /// configured" — off unless *measured*, which is a different and stronger
    /// default.
    #[test]
    fn freeze_thaw_stays_off_until_it_is_measured_to_be_cheap() {
        assert!(!freeze_thaw_worthwhile(0.0, 600.0), "unmeasured is not cheap");
        assert!(!freeze_thaw_worthwhile(120.0, 600.0), "20 % of the rung is too much");
        assert!(freeze_thaw_worthwhile(60.0, 600.0), "10 % is worth it");
        assert!(!freeze_thaw_worthwhile(60.0, 0.0));
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// 2.5 — preempt → recover → resume, under the same trial
// ───────────────────────────────────────────────────────────────────────────────

/// What a worker should do when it picks up a job that has run before.
///
/// The trial is the look, not the process (ADR-P2-07). All three outcomes here
/// happen **under the same `trial_id`**: a re-queued job is the same look at the
/// same data, and minting a second trial for it would inflate the counter that
/// every significance correction in the platform divides by.
#[derive(Clone, Debug, PartialEq)]
pub enum ResumePlan {
    /// Resume from this checkpoint's step.
    Resume { step: u64, from: String },
    /// Start again from step 0 — under the same trial.
    ///
    /// Carries why, because "we restarted" and "we restarted because the
    /// checkpoint was written by different code" are different facts and only
    /// one of them is worth investigating.
    Restart { reason: String },
    /// The checkpoint exists, claims to be resumable, and is not. The job fails
    /// `checkpoint_invalid` and the trial settles `preempted_abandoned`.
    ///
    /// Never "resume from something else wearing the same id": a checkpoint that
    /// fails validation is a checkpoint nobody can vouch for, and continuing
    /// from it produces a model whose lineage says something untrue.
    Abandon { reason: String },
}

/// Decide how to pick up a job.
///
/// `candidate` is the newest checkpoint artifact's manifest, if one exists.
/// `code_hash`, `image_digest` and `dataset_id` describe the computation about
/// to run.
#[must_use]
pub fn plan_resume(
    candidate: Option<&Value>,
    code_hash: &str,
    image_digest: &str,
    dataset_id: &str,
) -> ResumePlan {
    let Some(raw) = candidate else {
        return ResumePlan::Restart {
            reason: "no checkpoint exists for this job".into(),
        };
    };

    let manifest = match CheckpointManifest::parse(raw) {
        Ok(m) => m,
        // A stored checkpoint that does not satisfy the contract is the one case
        // that must not be quietly restarted: something wrote it, the registry
        // should have refused it, and continuing as though it were merely absent
        // would hide that.
        Err(e) => {
            return ResumePlan::Abandon {
                reason: format!("checkpoint_invalid: {e}"),
            }
        }
    };

    if !manifest.matches(code_hash, image_digest, dataset_id) {
        return ResumePlan::Restart {
            reason: format!(
                "the checkpoint belongs to a different computation (code {}, image {}, dataset {}); \
                 restarting under the same trial",
                manifest.code_hash, manifest.image_digest, manifest.dataset_id
            ),
        };
    }

    if !manifest.resumable() {
        return ResumePlan::Restart {
            reason: "this framework's resume is not bit-identical (checkpoint_resume: unsupported); \
                     restarting from step 0 under the same trial"
                .into(),
        };
    }

    ResumePlan::Resume {
        step: manifest.step,
        from: manifest.optimizer_state_ref.clone(),
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> Value {
        json!({
            "step": 900,
            "rng_state": { "python": "a", "numpy": "b", "torch_cpu": "c", "torch_cuda": [] },
            "optimizer_state_ref": "art_opt",
            "lr_scheduler_state_ref": "art_sched",
            "dataloader_position": 128,
            "amp_scaler_state": "art_scaler",
            "code_hash": "sha256:code",
            "image_digest": "sha256:image",
            "dataset_id": "ds:abc",
            "resume_support": "bit_identical"
        })
    }

    #[test]
    fn a_matching_resumable_checkpoint_is_resumed_from() {
        assert_eq!(
            plan_resume(Some(&manifest()), "sha256:code", "sha256:image", "ds:abc"),
            ResumePlan::Resume { step: 900, from: "art_opt".into() }
        );
    }

    #[test]
    fn no_checkpoint_restarts_rather_than_failing() {
        let plan = plan_resume(None, "sha256:code", "sha256:image", "ds:abc");
        assert!(matches!(plan, ResumePlan::Restart { .. }), "{plan:?}");
    }

    /// A checkpoint from different code is not this run's checkpoint. Restarting
    /// under the same trial is correct: the look is the same, only the process
    /// is new.
    #[test]
    fn a_checkpoint_from_a_different_computation_restarts_with_a_reason() {
        for (code, image, dataset) in [
            ("sha256:other", "sha256:image", "ds:abc"),
            ("sha256:code", "sha256:other", "ds:abc"),
            ("sha256:code", "sha256:image", "ds:other"),
        ] {
            match plan_resume(Some(&manifest()), code, image, dataset) {
                ResumePlan::Restart { reason } => {
                    assert!(reason.contains("different computation"), "{reason}");
                    assert!(reason.contains("same trial"), "{reason}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_framework_that_cannot_resume_restarts_rather_than_pretending() {
        let mut m = manifest();
        m["resume_support"] = json!("unsupported");
        match plan_resume(Some(&m), "sha256:code", "sha256:image", "ds:abc") {
            ResumePlan::Restart { reason } => assert!(reason.contains("not bit-identical"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    /// The one case that is not a restart. A checkpoint that fails the contract
    /// should never have been stored; continuing as though it were merely absent
    /// would hide that something wrote it.
    #[test]
    fn an_invalid_checkpoint_is_abandoned_never_resumed_from() {
        let mut m = manifest();
        m.as_object_mut().unwrap().remove("optimizer_state_ref");
        match plan_resume(Some(&m), "sha256:code", "sha256:image", "ds:abc") {
            ResumePlan::Abandon { reason } => {
                assert!(reason.starts_with("checkpoint_invalid"), "{reason}");
                assert!(reason.contains("optimizer_state_ref"), "{reason}");
            }
            other => panic!("an invalid checkpoint must not be silently restarted: {other:?}"),
        }
    }

    /// Every plan keeps the same trial. There is no variant that mints a new one,
    /// and that absence is the point: a re-queued job is the same look at the
    /// same data, and a second trial would inflate the count every significance
    /// correction divides by.
    #[test]
    fn no_plan_creates_a_second_trial() {
        let plans = [
            plan_resume(Some(&manifest()), "sha256:code", "sha256:image", "ds:abc"),
            plan_resume(None, "a", "b", "c"),
            plan_resume(Some(&json!({})), "a", "b", "c"),
        ];
        for p in plans {
            assert!(
                matches!(
                    p,
                    ResumePlan::Resume { .. } | ResumePlan::Restart { .. } | ResumePlan::Abandon { .. }
                ),
                "the three outcomes are the whole vocabulary"
            );
        }
    }
}
