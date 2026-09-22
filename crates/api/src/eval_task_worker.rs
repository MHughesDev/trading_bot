//! The `EvalTask` worker: the COMPARE phase's child job, and the production
//! caller of `ComparisonPlan::judge` (SPEC §10, §11.5; checklist 2.1, 2.12;
//! ADR-P2-04, ADR-P2-12, ADR-P2-35).
//!
//! Until this existed, `stats::compare` was correct and unused: the sealed
//! [`Comparison`](backtest::stats::compare::Comparison) could be built but
//! nothing in the platform built one, so "search-loop validation scores are
//! inadmissible" was a property of a type nobody held. This is the thing that
//! holds one.
//!
//! ## It judges; it does not measure
//!
//! Every number this worker reads was written by the EXPERIMENT phase's study,
//! each row naming the counted trial that produced it. Nothing here runs a
//! backtest, and nothing here accepts a metric from its manifest — for the same
//! reason the gate worker does not (ADR-P2-31): a statistic the submitter
//! supplies is a statistic the submitter chose, and a comparison protocol fed
//! chosen numbers is a ceremony.
//!
//! The consequence worth stating plainly: this worker does not count a trial,
//! and that is correct rather than a gap. It dispatches no compute — INV-16 is
//! about compute, and the looks this arithmetic is computed from were counted
//! where they were taken.
//!
//! ## Everything it cannot compute stays absent
//!
//! Two of the protocol's qualifiers are computed here and the rest are left at
//! their "nothing known to be wrong" default:
//!
//! * `cost_nuisance_unrandomized` is raised whenever the evidence contains
//!   fewer than two distinct cost assumptions, which in this build is always
//!   (ADR-P2-36, and the study worker's header says why);
//! * the pairing itself is checked — an unpaired or short set is refused by
//!   `judge`, never zipped.
//!
//! ## The scale is derived, so it is not a knob
//!
//! The confidence sequence needs a declared bound on how large a paired
//! difference could be, and a caller who could choose it could choose it after
//! seeing the data. So it is not chosen: it is
//! [`DIFFERENCE_SCALE_MULTIPLE`] times the campaign's `delta_practical`, which
//! was fixed at DEFINE and is immutable. A difference outside that bound is an
//! error the job reports, never a clipped observation — clipping would narrow
//! the estimand without saying so.

// `JobError` is the job service's wire type, capped at 400 bytes on purpose
// (JB-07); boxing it would put a `Box` in every fallible worker helper here.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use backtest::stats::compare::{
    ComparisonFlags, ComparisonPlan, Measurement, Replicate, RopeScale, DEFAULT_ALPHA,
};
use jobs::store::JobStore;
use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use ledger::evidence::{ReplicateMeasurement, INCUMBENT_ARM};
use ledger::phase::CampaignPhase;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// How much wider than the declared effect size a paired difference may be
/// before the protocol refuses to judge it.
///
/// `delta_practical` is the smallest difference that would change a decision. A
/// paired difference more than four times that is either a transformative
/// result or a defect in the measurement, and in both cases the right behaviour
/// is to stop and say so rather than to quietly fold it into an interval. Four
/// is a chosen number, recorded in ADR-P2-35; what matters is that it is
/// **derived from a DEFINE fact**, so no caller — agent, human or job manifest
/// — has a parameter to move.
pub const DIFFERENCE_SCALE_MULTIPLE: f64 = 4.0;

/// Judges one campaign's COMPARE phase.
pub struct EvalTaskWorker {
    store: Arc<JobStore>,
    pg: PgPool,
}

impl EvalTaskWorker {
    #[must_use]
    pub fn new(store: Arc<JobStore>, pg: PgPool) -> Self {
        Self { store, pg }
    }

    fn ledger(&self) -> ledger::pg::PgTrialLedger {
        ledger::pg::PgTrialLedger::new(self.pg.clone())
    }
}

/// What the campaign driver hands its COMPARE child: which campaign, and where
/// in the fold. Nothing else — see the module header.
struct Request {
    campaign_id: Uuid,
    seq: i64,
}

fn parse(manifest: &Value) -> Result<Request, JobError> {
    let bad = |m: &str| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_manifest",
            m.to_string(),
        )
    };
    let campaign_id: Uuid = manifest
        .get("campaign_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("an eval_task job needs a `campaign_id`"))?;
    let seq = manifest
        .get("seq")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("an eval_task job needs the fold position `seq` it was dispatched at"))?;
    if seq < 0 {
        return Err(bad("`seq` is a fold position and cannot be negative"));
    }
    Ok(Request { campaign_id, seq })
}

/// Pair two arms' measurements on the replicate key they share.
///
/// An inner join, deliberately. §11.5's difference is paired, and a replicate
/// one arm has and the other does not is not a pair — including it by carrying
/// the missing side forward, or by zipping the two lists positionally, would
/// estimate a different and much noisier quantity while looking identical in
/// the output.
fn pair(
    a: &[ReplicateMeasurement],
    b: &[ReplicateMeasurement],
) -> (Vec<Measurement>, Vec<Measurement>) {
    let index: std::collections::BTreeMap<_, _> = b.iter().map(|m| (m.key, m.value)).collect();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for m in a {
        if let Some(other) = index.get(&m.key) {
            let replicate = Replicate {
                window: u32::try_from(m.key.window_index).unwrap_or(u32::MAX),
                cost_bp: m.key.cost_bp,
                seed: m.key.seed.unsigned_abs(),
            };
            left.push(Measurement { replicate, value: m.value });
            right.push(Measurement { replicate, value: *other });
        }
    }
    (left, right)
}

/// How many distinct counted trials back one arm's evidence.
fn trials_behind(ms: &[ReplicateMeasurement]) -> i64 {
    let distinct: std::collections::BTreeSet<Uuid> = ms.iter().map(|m| m.trial_id).collect();
    i64::try_from(distinct.len()).unwrap_or(i64::MAX)
}

/// The qualifiers this build can actually compute.
///
/// Everything else keeps `ComparisonFlags`' default, which is "nothing known to
/// be wrong" — and that default is only safe because each field is a *warning*
/// that something must set explicitly.
fn flags_for(a: &[ReplicateMeasurement], b: &[ReplicateMeasurement]) -> ComparisonFlags {
    let costs: std::collections::BTreeSet<i32> =
        a.iter().chain(b.iter()).map(|m| m.key.cost_bp).collect();
    ComparisonFlags {
        cost_nuisance_unrandomized: costs.len() < 2,
        ..ComparisonFlags::default()
    }
}

#[async_trait::async_trait]
impl Worker for EvalTaskWorker {
    fn kind(&self) -> JobKind {
        JobKind::EvalTask
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let req = parse(manifest)?;
        let led = self.ledger();

        // This kind counts no trial, so there is no ticket to read the tenant
        // from. It comes from the job row the service wrote at submission —
        // never from the manifest, which a submitter controls.
        let job = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let tenant = job.user_id.to_string();

        let campaign = led
            .load_campaign_async(&tenant, req.campaign_id)
            .await
            .map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unknown_campaign",
                    format!("campaign {}: {e}", req.campaign_id),
                )
            })?;
        let def = campaign.definition();
        let (metric, _) = crate::study_worker::objective_metric(def)?;

        // Which challenger — from the fold, not from the clock. The COMPARE
        // phase judges the candidate the EXPERIMENT phase immediately before it
        // measured, and the log is what says which one that was.
        let state = led
            .campaign_state_async(&tenant, req.campaign_id)
            .await
            .map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "campaign_log_unreadable",
                    format!("folding campaign {}: {e}", req.campaign_id),
                )
            })?;
        let experiment_seq = state
            .children
            .iter()
            .rev()
            .find(|(phase, _, _)| *phase == CampaignPhase::Experiment)
            .map(|(_, seq, _)| *seq)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::DependencyFailure,
                    "no_experiment_to_compare",
                    "this campaign has no EXPERIMENT child in its log; COMPARE judges what \
                     EXPERIMENT measured and will not invent an arm",
                )
            })?;
        let challenger = crate::study_worker::challenger_arm(experiment_seq);

        ctx.progress(Progress {
            pct: Some(0.2),
            stage: Some("gathering".into()),
            message: Some(format!("{challenger} vs {INCUMBENT_ARM} on {metric}")),
        })
        .await;

        let infra = |what: &'static str| {
            move |e: ledger::LedgerError| JobError::infrastructure(format!("reading {what}: {e}"))
        };
        let a = led
            .replicate_measurements_async(&tenant, req.campaign_id, &challenger, &metric)
            .await
            .map_err(infra("the challenger's measurements"))?;
        let b = led
            .replicate_measurements_async(&tenant, req.campaign_id, INCUMBENT_ARM, &metric)
            .await
            .map_err(infra("the incumbent's measurements"))?;
        let (left, right) = pair(&a, &b);

        let n_eff = led
            .n_eff_async(&tenant)
            .await
            .map_err(infra("N_eff"))?
            .value();

        // `k_max` is the campaign's pre-registered replicate count: the most
        // this comparison may spend before the protocol calls the pair
        // practically equivalent rather than keeping on sampling.
        let k_max = def
            .search_space
            .get("replicates")
            .and_then(Value::as_u64)
            .map_or(crate::study_worker::DEFAULT_REPLICATES, |k| {
                usize::try_from(k).unwrap_or(crate::study_worker::DEFAULT_REPLICATES)
            })
            .max(backtest::stats::compare::MIN_REPLICATES);

        let scale = RopeScale::new(DIFFERENCE_SCALE_MULTIPLE * def.delta_practical).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "bad_difference_scale",
                format!("the campaign's delta_practical gives no usable scale: {e}"),
            )
        })?;

        let plan = ComparisonPlan {
            profile_id: def.gates_profile.clone(),
            n_eff,
            delta_practical: def.delta_practical,
            scale,
            trials_a: trials_behind(&a),
            trials_b: trials_behind(&b),
            k_max,
            flags: flags_for(&a, &b),
            alpha: DEFAULT_ALPHA,
        };

        ctx.progress(Progress {
            pct: Some(0.7),
            stage: Some("judging".into()),
            message: None,
        })
        .await;

        // The one door (§11.5). There is no second path to a verdict here, and
        // a refusal is reported rather than worked around: too few pairs, an
        // unpaired set or an out-of-scale difference are all statements about
        // the evidence, and the campaign's log stays where it is.
        let comparison = plan.judge(&left, &right).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::DependencyFailure,
                "comparison_refused",
                format!(
                    "{challenger} vs {INCUMBENT_ARM} on {metric}: {e} (paired {} of {}/{} \
                     measurements)",
                    left.len(),
                    a.len(),
                    b.len()
                ),
            )
        })?;

        let record = comparison.to_record(req.seq, &challenger, INCUMBENT_ARM, &metric);
        let fresh = led
            .record_comparison_async(&tenant, req.campaign_id, &record)
            .await
            .map_err(|e| JobError::infrastructure(format!("recording the comparison: {e}")))?;

        Ok(JobOutput {
            summary: Some(comparison.headline()),
            result: json!({
                "campaign_id": req.campaign_id,
                "seq": req.seq,
                "arm_a": challenger,
                "arm_b": INCUMBENT_ARM,
                "metric": metric,
                "verdict": comparison.verdict().as_str(),
                "mean_difference": comparison.mean_difference(),
                "cs_lower": comparison.interval().lower,
                "cs_upper": comparison.interval().upper,
                "p_a_better": comparison.p_a_better(),
                "achieved_k": comparison.achieved_k(),
                "k_max": k_max,
                "n_eff": n_eff,
                "delta_practical": def.delta_practical,
                "profile_id": comparison.profile_id(),
                "flags": comparison.flags().raised(),
                // False means this position had already been judged and the
                // stored verdict stands — the expected outcome of a re-queue.
                "recorded": fresh,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::evidence::ReplicateKey;

    fn m(arm: &str, window: i32, cost_bp: i32, value: f64, trial: Uuid) -> ReplicateMeasurement {
        ReplicateMeasurement {
            arm: arm.into(),
            key: ReplicateKey { window_index: window, cost_bp, seed: i64::from(window) + 100 },
            metric: "sharpe_net".into(),
            value,
            trial_id: trial,
        }
    }

    #[test]
    fn the_manifest_names_a_campaign_and_a_position() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({ "campaign_id": Uuid::nil().to_string() })).is_err());
        assert!(parse(&json!({ "campaign_id": Uuid::nil().to_string(), "seq": -2 })).is_err());
        assert_eq!(
            parse(&json!({ "campaign_id": Uuid::nil().to_string(), "seq": 9 }))
                .unwrap()
                .seq,
            9
        );
    }

    /// A replicate one arm has and the other does not is dropped, not carried.
    /// Zipping positionally would produce the same count of "pairs" out of two
    /// misaligned lists, which is the failure this join exists to make
    /// impossible.
    #[test]
    fn pairing_is_a_join_on_the_replicate_and_not_a_zip() {
        let t = Uuid::new_v4();
        let a = vec![m("c", 0, 0, 1.0, t), m("c", 1, 0, 2.0, t), m("c", 2, 0, 3.0, t)];
        // The incumbent is missing window 1 and has an extra window 9.
        let b = vec![m("i", 0, 0, 0.5, t), m("i", 2, 0, 1.5, t), m("i", 9, 0, 9.9, t)];
        let (left, right) = pair(&a, &b);
        assert_eq!(left.len(), 2, "only the shared replicates pair");
        assert_eq!(right.len(), 2);
        assert_eq!(left[0].replicate, right[0].replicate);
        assert_eq!(left[1].replicate, right[1].replicate);
        assert!((left[1].value - 3.0).abs() < f64::EPSILON);
        assert!((right[1].value - 1.5).abs() < f64::EPSILON);
    }

    /// A pair whose evidence was all measured at one cost assumption is flagged.
    /// §11.5 asks for every nuisance source to be randomized, and the direction
    /// of this omission is the flattering one, so it has to be visible.
    #[test]
    fn evidence_measured_at_one_cost_assumption_raises_its_flag() {
        let t = Uuid::new_v4();
        let flat = vec![m("c", 0, 0, 1.0, t), m("c", 1, 0, 2.0, t)];
        let f = flags_for(&flat, &flat);
        assert!(f.cost_nuisance_unrandomized);
        assert!(f.any(), "an unrandomized nuisance qualifies the verdict");
        assert!(f.raised().contains(&"cost_nuisance_unrandomized"));

        let varied = vec![m("c", 0, -2000, 1.0, t), m("c", 0, 2000, 2.0, t)];
        assert!(!flags_for(&varied, &varied).cost_nuisance_unrandomized);
    }

    /// The trial count behind an arm is the number of distinct counted looks,
    /// not the number of rows: two measurements served from one cached run are
    /// one trial's worth of evidence twice over.
    #[test]
    fn an_arms_trial_count_is_its_distinct_looks() {
        let one = Uuid::new_v4();
        let two = Uuid::new_v4();
        assert_eq!(trials_behind(&[m("c", 0, 0, 1.0, one), m("c", 1, 0, 2.0, one)]), 1);
        assert_eq!(trials_behind(&[m("c", 0, 0, 1.0, one), m("c", 1, 0, 2.0, two)]), 2);
        assert_eq!(trials_behind(&[]), 0);
    }

    /// The scale is derived from a DEFINE fact, so there is no argument, field
    /// or manifest key that moves it.
    #[test]
    fn the_difference_scale_is_derived_from_the_declared_effect_size() {
        let s = RopeScale::new(DIFFERENCE_SCALE_MULTIPLE * 0.25).unwrap();
        assert!((s.half_width() - 1.0).abs() < 1e-12);
        // And a campaign cannot have a non-positive delta_practical — the
        // definition validator and the database CHECK both refuse it — so this
        // construction cannot fail for a campaign that exists.
        assert!(RopeScale::new(DIFFERENCE_SCALE_MULTIPLE * 0.0).is_err());
    }
}
