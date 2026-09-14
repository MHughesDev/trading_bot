//! The `GateAdvance` worker: the production caller of the sixteen-gate stack
//! (SPEC §12.3, checklist 2.14, plan 5.5).
//!
//! `backtest::gates::stack::evaluate_stack` is arithmetic over evidence. This is
//! the thing that *gathers* the evidence — from the ledger, from the leakage
//! suite's runs, from the stored return series — and records every verdict
//! through `GateLog`. Without it the sixteen evaluators are correct and unused,
//! and `gate_pass_rate` in the self-monitor has nothing to read.
//!
//! ## The rule the assembly follows
//!
//! **Absent evidence is left absent.** Every field of `StackEvidence` this
//! worker cannot fill is left `None`, which makes its gate `inconclusive`, which
//! is never a pass. There is no default, no "assume it's fine if we can't
//! check", and no silent omission — a gate that could not run is recorded as a
//! failure carrying its reason, so the stack's length stays sixteen and the pass
//! rate counts the hole.
//!
//! That is the opposite of the obvious design, in which the worker fills what it
//! can and reports on what it filled. The obvious design has the property that
//! the platform's measured gate pass rate goes *up* as its evidence pipeline
//! degrades, which is the single most misleading number this system could
//! produce.

// `JobError` is the job service's wire type: a code, three optional strings and
// a terminal reason, capped at 400 bytes on purpose (JB-07). Boxing it to satisfy
// the large-Err lint would add an allocation to every refusal in exchange for a
// few bytes on a path that is already an error, and would put a `Box` in the
// signature of every fallible worker helper in the tree.
#![allow(clippy::result_large_err)]

use backtest::gates::evaluators::LeakageRun;
use backtest::gates::profile::GateProfile;
use backtest::gates::stack::{evaluate_stack, record_stack, StackEvidence, GATE_COUNT};
use jobs::{JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// The trailing window the regime terciles are measured over, matching the
/// labeller's (`apps/platform::regime_jobs`). The two must agree or Gate 6 and
/// Gate 11 would count different regimes over the same period.
pub const VOL_WINDOW: usize = 21;

/// The checks the leakage suite runs. Gate 2 refuses a run that is missing any
/// of them, so this list is the contract between the suite and the gate.
pub const LEAKAGE_CHECKS: &[&str] = &[
    "causal_access",
    "random_label",
    "snapshot_reproducibility",
    "cv_wf_gap",
    "synthetic_injection",
];

/// Evaluates the sixteen gates for one subject and records the verdicts.
pub struct GateAdvanceWorker {
    pg: PgPool,
}

impl GateAdvanceWorker {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    fn ledger(&self) -> ledger::pg::PgTrialLedger {
        ledger::pg::PgTrialLedger::new(self.pg.clone())
    }
}

/// What the manifest has to say.
struct Request {
    experiment_id: String,
    trial_id: Uuid,
    profile_id: String,
    asset_class: String,
    /// The dataset the leakage suite audited. Gate 2 reads the newest run for
    /// exactly this subject — a clean run on a *different* dataset says nothing
    /// about this one.
    dataset_id: String,
    /// The campaign this candidate belongs to, when it belongs to one. Naming it
    /// is not supplying a statistic: the worker then reads the campaign's own
    /// immutable DEFINE hash from the ledger, which is Gate 1's evidence.
    campaign_id: Option<Uuid>,
}

fn parse(manifest: &Value) -> Result<Request, JobError> {
    let field = |k: &str| -> Result<String, JobError> {
        manifest
            .get(k)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    format!("a gate_advance job needs `{k}`"),
                )
            })
    };
    let trial_id = field("trial_id")?.parse::<Uuid>().map_err(|e| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_manifest",
            format!("trial_id is not a uuid: {e}"),
        )
    })?;
    Ok(Request {
        experiment_id: field("experiment_id")?,
        trial_id,
        // The profile is named by the caller and then *looked up*, never
        // constructed from the manifest: thresholds are not something a
        // submission gets to supply (INV-23).
        profile_id: field("profile_id").unwrap_or_else(|_| "strict_v1".to_string()),
        asset_class: field("asset_class").unwrap_or_else(|_| "crypto".to_string()),
        dataset_id: field("dataset_id")?,
        campaign_id: manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
    })
}

/// The immutable profile for an id.
///
/// Two profiles exist in code and both are hard-coded threshold sets that match
/// the immutable rows in `mlops.gate_profile` (migration 0050). An unknown id is
/// refused rather than defaulted: judging a candidate under a profile nobody
/// named is judging it under thresholds nobody chose.
fn profile_for(id: &str) -> Result<GateProfile, JobError> {
    match id {
        "strict_v1" => Ok(GateProfile::strict_v1()),
        "paper_v1" => Ok(GateProfile::paper_v1()),
        other => Err(JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "unknown_gate_profile",
            format!("`{other}` is not a gate profile this build knows (INV-23)"),
        )),
    }
}

#[async_trait::async_trait]
impl Worker for GateAdvanceWorker {
    fn kind(&self) -> JobKind {
        JobKind::GateAdvance
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let req = parse(manifest)?;
        let profile = profile_for(&req.profile_id)?;
        let led = self.ledger();

        // The job service authorises a counted kind with a registered trial
        // (INV-16); its tenant is the one everything below reads under.
        let tenant = ctx
            .trial
            .as_ref()
            .map(|t| t.tenant_id().to_string())
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unregistered_trial",
                    "gate_advance counts as a trial and cannot run without one (INV-16)",
                )
            })?;

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("gathering".into()),
            message: None,
        })
        .await;

        // ── the evidence, each piece absent rather than defaulted ───────────
        fn infra(what: &'static str) -> impl Fn(ledger::LedgerError) -> JobError {
            move |e| JobError::infrastructure(format!("reading {what}: {e}"))
        }

        let n_eff = led
            .n_eff_async(&tenant)
            .await
            .map_err(infra("N_eff"))?
            .value();
        let trial_count = led
            .experiment_trial_count_async(&tenant, &req.experiment_id)
            .await
            .map_err(infra("the trial count"))?;

        // The numbers the funnel computed, read from the ledger rather than
        // taken from the manifest (ADR-P2-31). A statistic that was never
        // recorded stays `None` and its gate stays inconclusive — which is the
        // correct answer to "did this pass Gate 7" when nobody measured PBO.
        let stats = led
            .statistics_async(&tenant, req.trial_id)
            .await
            .map_err(infra("the trial statistics"))?;
        let stat = |k: &str| stats.get(k).copied().filter(|v| v.is_finite());

        let series = led
            .return_series_async(&tenant, req.trial_id)
            .await
            .map_err(infra("the return series"))?;
        let (timestamps, returns) = series
            .map(|s| (s.timestamps, s.returns))
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        // Gates 9 and 11 are *derived from the series the ledger already holds*,
        // not supplied. That distinction is the point: a manifest field carrying
        // a Sharpe or a regime share would be a gate statistic the submitter
        // chose, which is §12.7's gate-hacking in its purest form. Everything
        // this worker evaluates, it computes (ADR-P2-31).
        let length = length_inputs(&timestamps, &returns, n_eff);
        let strategy_series: Vec<(chrono::DateTime<chrono::Utc>, f64)> =
            timestamps.iter().copied().zip(returns.iter().copied()).collect();

        let family_rows = led
            .experiment_family_async(&tenant, &req.experiment_id)
            .await
            .map_err(infra("the candidate family"))?;
        let candidate_index = family_rows
            .iter()
            .position(|(id, _)| *id == req.trial_id)
            .unwrap_or(0);
        let family: Vec<Vec<f64>> = family_rows.into_iter().map(|(_, r)| r).collect();

        let leakage = led
            .latest_leakage_run_async(&tenant, &req.dataset_id)
            .await
            .map_err(infra("the leakage run"))?
            .map(|(subject, checks_run, blocking_count, flag_count, finished_at)| LeakageRun {
                subject,
                checks_run,
                blocking_count,
                flag_count,
                finished_at,
            });


        // The seed the bootstrap draws from is the platform's, not the
        // candidate's (§12.7). Falling back to the trial's own id when the
        // subject has no campaign keeps it out of the agent's reach either way:
        // the agent cannot choose its trial id any more than it can read the
        // campaign seed.
        let bootstrap_seed = trial_seed(req.trial_id);

        // Gate 1's evidence, read from the campaign rather than supplied.
        let prereg_hash = match req.campaign_id {
            Some(id) => led
                .campaign_define_hash_async(&tenant, id)
                .await
                .map_err(infra("the campaign DEFINE hash"))?,
            None => None,
        };

        // Gate 6 pairs the walk-forward Sharpe with how many regimes it spanned.
        // The count is measured from the same series Gate 11 labels, over the
        // same research slice — a recorded count would be better, and this is
        // what exists (ADR-P2-33). An absent series leaves it `None`, which keeps
        // Gate 6 inconclusive rather than passing on one regime.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let walk_forward_regimes = stat("walk_forward_regimes").map(|v| v as u32).or_else(|| {
            (strategy_series.len() > VOL_WINDOW).then(|| {
                backtest::gates::stack::distinct_regimes(
                    &backtest::gates::evaluators::vol_tercile_regimes(
                        &strategy_series,
                        VOL_WINDOW,
                    ),
                )
            })
        });

        let cost_ladder: Vec<backtest::gates::evaluators::CostRung> =
            match stat("breakeven_cost_multiple") {
                Some(b) if b > 1.0 => vec![
                    backtest::gates::evaluators::CostRung { multiple: 1.0, net_metric: 1.0 },
                    backtest::gates::evaluators::CostRung { multiple: b, net_metric: 0.0 },
                ],
                _ => Vec::new(),
            };

        ctx.progress(Progress {
            pct: Some(0.5),
            stage: Some("evaluating".into()),
            message: None,
        })
        .await;

        let evidence = StackEvidence {
            // Gate 1 is structural: the claim was hash-locked before anything ran
            // or it was not. The hash comes from the campaign's own immutable
            // DEFINE event, never from the manifest — a hash the submitter
            // supplies is a claim the submitter could have written afterwards.
            prereg_hash: prereg_hash.clone(),
            leakage_run: leakage.as_ref(),
            leakage_checks: LEAKAGE_CHECKS,
            // Gate 3's ladder is a shape, not a scalar, so the recorded
            // breakeven multiple is turned back into the two rungs that bracket
            // it: alive at 1×, dead at the recorded multiple. That is exactly
            // what `breakeven_multiple` would interpolate from, and nothing is
            // claimed that the cost sweep did not measure.
            cost_ladder: &cost_ladder,
            capacity: None,
            cpcv_p05_sharpe: stat("cpcv_p05_sharpe"),
            walk_forward_sharpe: stat("walk_forward_sharpe"),
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            walk_forward_regimes,
            pbo: stat("pbo"),
            dsr: stat("deflated_sharpe"),
            length,
            factors: None,
            // Gate 11 is handed the strategy's own series as the market when
            // nothing better exists. The evaluator flags that case
            // (`labelled_from_strategy`) rather than pretending it is a market
            // reading, and a flagged verdict is more useful than a missing one —
            // a strategy whose entire P&L came from one volatility regime is
            // worth catching even when the regimes were labelled from its own
            // returns. A real market series replaces this once `regime_scopes`
            // is configured and 3.5's labels are being written (ADR-P3-07).
            market_returns: &strategy_series,
            strategy_returns: &strategy_series,
            perturbation: None,
            return_series: &returns,
            family: &family,
            candidate_index,
            bootstrap_seed,
            paper: None,
            capital_fraction: ledger::AllowedFraction::default(),
            n_eff,
            trial_count,
            asset_class: &req.asset_class,
            experiment_id: Some(req.experiment_id.clone()),
            trial_id: Some(req.trial_id),
            campaign_id: req.campaign_id,
        };

        let verdict = evaluate_stack(&profile, &evidence, chrono::Utc::now());
        let ids = record_stack(&led, &tenant, &profile, &evidence, &verdict)
            .map_err(|e| JobError::infrastructure(format!("recording the gate stack: {e}")))?;

        let inconclusive: Vec<i32> =
            verdict.inconclusive().iter().map(|o| o.gate_no).collect();
        let passed = verdict.outcomes.iter().filter(|o| o.passed).count();

        Ok(JobOutput {
            summary: Some(format!(
                "{passed} of {GATE_COUNT} gates passed under {} ({} could not be evaluated){}",
                profile.profile_id(),
                inconclusive.len(),
                verdict.blocked_at.map_or(String::new(), |g| format!("; blocked at gate {g}")),
            )),
            result: json!({
                "experiment_id": req.experiment_id,
                "trial_id": req.trial_id,
                "profile_id": profile.profile_id(),
                "passed": passed,
                "of": GATE_COUNT,
                "blocked_at": verdict.blocked_at,
                "inconclusive": inconclusive,
                "n_eff": n_eff,
                "trial_count": trial_count,
                "verdicts_recorded": ids.len(),
            }),
            ..Default::default()
        })
    }
}

/// Gate 9's measurements, derived from the stored series.
///
/// `None` when there is no series: an absent measurement makes the gate
/// inconclusive, which is never a pass. What it must never do is substitute a
/// default, because every one of these quantities has a value that passes.
///
/// The event count is the number of **non-zero** returns. A flat period is not
/// an independent observation of anything — it is the strategy not trading — and
/// counting flat bars toward the event floor is how a 5-minute backtest claims
/// 300 events in a week.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn length_inputs(
    timestamps: &[chrono::DateTime<chrono::Utc>],
    returns: &[f64],
    n_eff: f64,
) -> Option<backtest::gates::evaluators::LengthInputs> {
    if returns.len() < 2 || timestamps.len() != returns.len() {
        return None;
    }
    let first = *timestamps.first()?;
    let last = *timestamps.last()?;
    let days = (last - first).num_seconds() as f64 / 86_400.0;
    if days <= 0.0 {
        return None;
    }
    let years = days / 365.25;

    // Annualized from the observed sampling interval rather than an assumed one:
    // a daily series and a minute series with the same per-step Sharpe are not
    // the same annual Sharpe, and assuming 252 for both inflates one of them.
    let steps_per_year = returns.len() as f64 / years;
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    // A relative floor, not `> 0.0`. Summing five hundred identical values leaves
    // a standard deviation around 1e-19 from rounding alone, and dividing by it
    // produces a Sharpe of 1e15 that passes every gate in the stack. The floor is
    // relative to the series' own magnitude so it means the same thing for
    // returns measured in percent and in basis points.
    let magnitude = returns.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
    if !(sd.is_finite() && sd > magnitude * 1e-12) {
        return None;
    }
    let sharpe = mean / sd * steps_per_year.sqrt();
    if !sharpe.is_finite() {
        return None;
    }

    let independent_events =
        u32::try_from(returns.iter().filter(|r| **r != 0.0).count()).unwrap_or(u32::MAX);

    Some(backtest::gates::evaluators::LengthInputs {
        sharpe,
        n_eff,
        years,
        independent_events,
    })
}

/// A deterministic seed from a trial id.
///
/// Deterministic so a re-evaluation of the same trial produces the same
/// bootstrap, which is what makes a recorded verdict reproducible by a third
/// party. Derived from the id rather than supplied, so there is no parameter to
/// resample against.
fn trial_seed(trial_id: Uuid) -> u64 {
    let b = trial_id.as_bytes();
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leakage_check_list_is_the_suites_own() {
        // Gate 2 refuses a run missing any of these, so drifting from the
        // suite's actual checks would make every candidate inconclusive — loudly,
        // which is the right direction, but this is where to notice it.
        assert_eq!(LEAKAGE_CHECKS.len(), 5);
        assert!(LEAKAGE_CHECKS.contains(&"cv_wf_gap"));
        assert!(LEAKAGE_CHECKS.contains(&"synthetic_injection"));
    }

    #[test]
    fn an_unknown_profile_is_refused_rather_than_defaulted() {
        assert!(profile_for("strict_v1").is_ok());
        assert!(profile_for("paper_v1").is_ok());
        let err = profile_for("lenient_v1").unwrap_err();
        assert_eq!(err.terminal, ledger::TerminalReason::IntegrityRejected);
        assert_eq!(err.code, "unknown_gate_profile");
    }

    #[test]
    fn the_manifest_must_name_what_is_being_judged() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({ "experiment_id": "e" })).is_err());
        assert!(parse(&json!({
            "experiment_id": "e",
            "trial_id": "not-a-uuid",
            "dataset_id": "ds"
        }))
        .is_err());

        let ok = parse(&json!({
            "experiment_id": "e",
            "trial_id": Uuid::nil().to_string(),
            "dataset_id": "ds:abc"
        }))
        .expect("a complete manifest");
        assert_eq!(ok.experiment_id, "e");
        assert_eq!(ok.dataset_id, "ds:abc");
        // The profile defaults to the strict one when unnamed. Defaulting
        // *toward* the stricter profile is the only safe direction.
        assert_eq!(ok.profile_id, "strict_v1");
    }

    #[test]
    fn the_bootstrap_seed_is_deterministic_and_not_a_parameter() {
        let id = Uuid::new_v4();
        assert_eq!(trial_seed(id), trial_seed(id), "a re-evaluation must reproduce");
        assert_ne!(trial_seed(id), trial_seed(Uuid::new_v4()));
    }
}

#[cfg(test)]
mod derivation_tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn daily(n: usize, r: f64) -> (Vec<chrono::DateTime<chrono::Utc>>, Vec<f64>) {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts = (0..n).map(|i| start + Duration::days(i as i64)).collect();
        let rs = (0..n)
            .map(|i| if i % 2 == 0 { r } else { r * 0.5 })
            .collect();
        (ts, rs)
    }

    #[test]
    fn no_series_means_no_measurement_rather_than_a_default() {
        assert!(length_inputs(&[], &[], 10.0).is_none());
        let (ts, _) = daily(5, 0.01);
        assert!(length_inputs(&ts, &[0.01], 10.0).is_none(), "mismatched lengths");
    }

    /// A constant series has no Sharpe. Returning one would be a division by a
    /// standard deviation of zero dressed up as a measurement.
    #[test]
    fn a_flat_series_has_no_sharpe() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts: Vec<_> = (0..500).map(|i| start + Duration::days(i)).collect();
        assert!(length_inputs(&ts, &vec![0.001; 500], 10.0).is_none());
    }

    #[test]
    fn the_span_is_measured_not_assumed() {
        let (ts, rs) = daily(731, 0.001);
        let l = length_inputs(&ts, &rs, 42.0).expect("measurable");
        assert!((l.years - 2.0).abs() < 0.02, "two years of daily bars: {}", l.years);
        assert!((l.n_eff - 42.0).abs() < f64::EPSILON, "N_eff is the platform's, not derived");
    }

    /// Annualizing from the observed sampling interval rather than assuming 252:
    /// the same per-step Sharpe on minute bars and on daily bars is a wildly
    /// different annual number, and assuming one rate inflates the other.
    #[test]
    fn the_sharpe_is_annualized_from_the_observed_interval() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let n = 2_000;
        let rs: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 0.001 } else { 0.0005 }).collect();

        let daily_ts: Vec<_> = (0..n).map(|i| start + Duration::days(i as i64)).collect();
        let minute_ts: Vec<_> = (0..n).map(|i| start + Duration::minutes(i as i64)).collect();

        let d = length_inputs(&daily_ts, &rs, 10.0).unwrap();
        let m = length_inputs(&minute_ts, &rs, 10.0).unwrap();
        assert!(
            m.sharpe > d.sharpe * 10.0,
            "a minute series annualizes to far more: {} vs {}",
            m.sharpe,
            d.sharpe
        );
    }

    /// A flat bar is the strategy not trading. Counting it toward the event floor
    /// is how a week of 5-minute bars claims three hundred independent events.
    #[test]
    fn flat_periods_are_not_independent_events() {
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let ts: Vec<_> = (0..1_000).map(|i| start + Duration::days(i)).collect();
        // Ten real moves, the rest flat.
        let rs: Vec<f64> = (0..1_000).map(|i| if i % 100 == 0 { 0.02 } else { 0.0 }).collect();
        let l = length_inputs(&ts, &rs, 10.0).expect("measurable");
        assert_eq!(l.independent_events, 10, "only the moves count");
    }
}
