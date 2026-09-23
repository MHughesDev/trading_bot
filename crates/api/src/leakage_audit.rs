//! The nightly leakage pass (SPEC §12.5, checklist 1.7–1.11).
//!
//! §12.5·2 is specific about where the random-label test belongs: "it belongs in
//! platform CI and runs nightly against the platform itself, not just per
//! strategy." So this runs against real datasets a tenant has actually
//! materialized, through the real fold geometry, rather than against a fixture.
//!
//! Every pass is recorded whether or not it found anything. A suite that only
//! writes its failures cannot distinguish "no leaks" from "never ran", and
//! "never ran" is the state that matters.
//!
//! # `checks_run` is a claim about work, so the work produces it
//!
//! The `checks_run` array this module persists is read back by the gate worker
//! and handed to Gate 2, whose refusal text is "a check that did not run is not
//! a check that passed". A name in that array is therefore load-bearing: it is
//! the only evidence the gate has that a check happened.
//!
//! This module used to build that array by hand, beside the calls rather than
//! from them, and the two had already drifted — `causal_access` was named by a
//! `push` that sat next to three other calls and no call of its own, and
//! `snapshot_reproducibility` was named before the loop that runs it, so an
//! empty or wholly unrebuildable tenant claimed it too. Neither was visible from
//! the gate, which sees only the array.
//!
//! So the array now lives in [`pass`], behind a private field. A child module's
//! private fields are not reachable from its parent, so nothing in the rest of
//! this file can append to it; the only append site is [`pass::Tally::run`], and
//! the only thing that produces the name it appends is [`pass::Pass::run`],
//! where the name and the call sit in the same expression. A check cannot be
//! claimed without being run, because there is nowhere left to claim it from.
//!
//! One case in that class is still open and is not this module's to close.
//! `target_correlation` returns an empty report on a frame of fewer than 8 rows
//! and `full_sample_normalization` on fewer than 32, without saying so — an
//! empty report is also what a clean frame produces, so there is no signal here
//! to read. `random_label` does say so (it returns zero permutations) and is
//! handled. Closing the other two means giving those two minimums names in
//! `crates/features/src/leakage.rs` and declining out loud; see the report.
//!
//! # The random-label test had never run, and the geometry is why
//!
//! `harness_cv` sized three walk-forward folds as `test = n/10`, `cal = n/20`,
//! `train = n − 3·(test + cal)`. [`features::walk_forward_folds`] lays folds end
//! to end, so three of them reach `3·(train + 2·purge + cal + test) + 2·embargo`
//! rows — about `2.1·n` for those numbers, for **every** `n`. The geometry
//! therefore never built: the fold-geometry screen logged a warning every night
//! and the random-label test returned its "did not run" flag every night, on
//! every tenant, on every frame size. Nothing was wrong with the frames.
//!
//! [`harness_geometry`] sizes the roles out of what is left after the gaps and
//! then *builds the folds to check*, returning them rather than a spec a caller
//! has to re-derive. It also requires each fold to be one
//! [`features::leakage::random_label`] will actually fit and score, because a
//! geometry whose folds are all skipped is the failure below: a full-length
//! vector of `NaN` t-statistics, which from the outside is the shape of a check
//! that ran and found nothing.
//!
//! # The causal-access guard is not run here, and so is not claimed
//!
//! [`features::leakage::causal_access`] needs the dataset's OHLCV rows: its
//! behavioural probe recomputes a feature with rows replaced by noise.
//! `DatasetManager` exposes only the assembled [`features::TrainingFrame`] and
//! keeps its bar store private, so this pass cannot obtain them, and it does not
//! pretend otherwise. Gate 2 consequently reports `causal_access` among the
//! checks that did not run, which is the truth. Wiring it needs a rows accessor
//! on `crates/model-registry/src/datasets.rs`; see the report that accompanied
//! this change.

use chrono::{DateTime, Utc};
use features::leakage::{self, Report, Severity};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub use pass::{Pass, Passed, Tally};

/// The name of a check, and the work that earns it, in one place.
///
/// Nothing here is a convenience wrapper. The privacy is the enforcement: see
/// the module header.
pub mod pass {
    use features::leakage::{self, Check, Finding, Report, Severity};

    /// One check this audit can claim, carrying exactly the inputs that check
    /// needs.
    ///
    /// A variant that cannot be supplied with its inputs cannot be constructed,
    /// and a variant that is never constructed is never named. That is the
    /// whole mechanism: there is no path from "I would like to say this ran" to
    /// a name in `checks_run` that does not pass through running it.
    pub enum Pass<'a> {
        /// §12.5·3 — the same `dataset_id` rebuilt later must produce the same
        /// bytes.
        SnapshotReproducibility {
            dataset_id: &'a str,
            recorded: &'a str,
            rebuilt: &'a str,
        },
        /// A feature column that is (nearly) the label itself.
        TargetCorrelation(&'a features::TrainingFrame),
        /// A column standardized over the whole sample.
        FullSampleNormalization(&'a features::TrainingFrame),
        /// §12.5·2 — permuted labels must earn nothing through the production
        /// fold geometry.
        RandomLabel {
            frame: &'a features::TrainingFrame,
            spec: &'a domain::model_def::cv::WalkForwardSpec,
            pipeline: &'a dataplane::split::EmbargoInputs,
            permutations: usize,
            seed: u64,
        },
    }

    /// What one pass produced: the name it earned and the findings it made.
    ///
    /// The fields are private and there is no constructor, so [`Pass::run`] is
    /// the only thing in the program that can pair a [`Check`] with a report. A
    /// caller cannot assemble a `Passed` claiming a name for work it did not do.
    pub struct Passed {
        earned: Option<Check>,
        report: Report,
        random_label: Option<leakage::RandomLabelResult>,
    }

    impl Passed {
        /// The name this pass earned by doing its work, or `None` if it ran and
        /// declined.
        ///
        /// A check can be called and still measure nothing: the random-label
        /// test returns without permuting anything when the frame is too short
        /// to build the production fold geometry, and says so in a flag. Calling
        /// that a check that ran would be the same false claim one layer down,
        /// so it earns no name and Gate 2 reports it missing.
        #[must_use]
        pub fn earned(&self) -> Option<Check> {
            self.earned
        }

        #[must_use]
        pub fn report(&self) -> &Report {
            &self.report
        }

        /// The random-label statistics, which only the random-label pass has.
        #[must_use]
        pub fn random_label(&self) -> Option<&leakage::RandomLabelResult> {
            self.random_label.as_ref()
        }
    }

    impl Pass<'_> {
        /// Run the check and return the name it earned.
        ///
        /// Every arm is one expression: the `Check` and the call that justifies
        /// it are written together, so a mislabelled arm is a mislabelled
        /// finding too and the guard test catches it by comparing the two.
        #[must_use]
        pub fn run(self) -> Passed {
            match self {
                Self::SnapshotReproducibility {
                    dataset_id,
                    recorded,
                    rebuilt,
                } => Passed {
                    earned: Some(Check::SnapshotReproducibility),
                    report: leakage::snapshot_reproducibility(dataset_id, recorded, rebuilt),
                    random_label: None,
                },
                Self::TargetCorrelation(frame) => Passed {
                    earned: Some(Check::TargetCorrelation),
                    report: leakage::target_correlation(frame),
                    random_label: None,
                },
                Self::FullSampleNormalization(frame) => Passed {
                    earned: Some(Check::FullSampleNormalization),
                    report: leakage::full_sample_normalization(frame),
                    random_label: None,
                },
                Self::RandomLabel {
                    frame,
                    spec,
                    pipeline,
                    permutations,
                    seed,
                } => {
                    let mut r = leakage::random_label(frame, spec, pipeline, permutations, seed);

                    // There are two ways this routine returns without measuring
                    // anything, and neither is a check that ran.
                    //
                    // It says so itself when it cannot build the fold geometry:
                    // it permutes nothing and pushes a flag. It does *not* say
                    // so when the geometry builds and every fold is then skipped
                    // for having fewer training rows than coefficients. There it
                    // returns one t-statistic per permutation requested — all
                    // `NaN`, because a fold-less out-of-sample series has no
                    // moments — and a `permuted_max_abs_t` folded to `0.0` out
                    // of an empty set of finite draws. A full-length vector is
                    // what a pass that ran looks like from the outside, and
                    // `0.0` is the cleanest answer the limit can be given, so
                    // that shape reads as a check that ran and found nothing.
                    //
                    // The name is therefore earned by finite statistics, not by
                    // a vector length. The two cases are the same failure and
                    // are treated the same way.
                    let measured = permutations > 0
                        && r.permuted_t_stats.len() == permutations
                        && r.permuted_t_stats.iter().all(|t| t.is_finite());
                    let mut report = Report {
                        findings: r.report.findings.clone(),
                    };
                    if !measured {
                        if report.findings.is_empty() {
                            report.findings.push(Finding {
                                check: Check::RandomLabel,
                                severity: Severity::Flag,
                                subject: "folds".into(),
                                statistic: r.permuted_t_stats.len() as f64,
                                detail: "the fold geometry built but produced no out-of-sample \
                                         observations, so every permutation scored nothing; the \
                                         test did not run"
                                    .into(),
                            });
                        }
                        // An absent measurement is not a zero. `record` persists
                        // these columns whenever they are finite, and the `0.0`
                        // a fold over no finite draws produces is finite.
                        r.real_sharpe = f64::NAN;
                        r.real_t = f64::NAN;
                        r.permuted_mean = f64::NAN;
                        r.permuted_max_abs_t = f64::NAN;
                    }
                    Passed {
                        earned: measured.then_some(Check::RandomLabel),
                        report,
                        random_label: Some(r),
                    }
                }
            }
        }
    }

    /// The names a pass has earned, and the findings it made.
    ///
    /// `checks_run` is private and this module is a child of `leakage_audit`, so
    /// the audit itself cannot reach it. [`Tally::run`] is the only append site
    /// in the program.
    #[derive(Default)]
    pub struct Tally {
        checks_run: Vec<String>,
        report: Report,
        random_label: Option<leakage::RandomLabelResult>,
    }

    impl Tally {
        /// Run one check and record the name it earned.
        ///
        /// Repeats are recorded once: the snapshot screen runs per dataset, and
        /// "snapshot_reproducibility" five times over would say nothing a gate
        /// reads that it does not already say once.
        pub fn run(&mut self, pass: Pass<'_>) {
            let passed = pass.run();
            if let Some(check) = passed.earned {
                let name = check.to_string();
                if !self.checks_run.contains(&name) {
                    self.checks_run.push(name);
                }
            }
            self.report.findings.extend(passed.report.findings);
            if passed.random_label.is_some() {
                self.random_label = passed.random_label;
            }
        }

        /// Record findings from work that is not one of the suite's named
        /// checks.
        ///
        /// The fold-geometry screen is the one such: it is real work, its
        /// findings belong in the report, and it has no name of its own in
        /// [`Check`] to claim. Findings without a name err in the safe
        /// direction — a finding is evidence that work happened, whereas a name
        /// is only an assertion that it did.
        pub fn add_findings(&mut self, report: Report) {
            self.report.findings.extend(report.findings);
        }

        /// The checks that ran, in the order they ran.
        #[must_use]
        pub fn checks_run(&self) -> &[String] {
            &self.checks_run
        }

        #[must_use]
        pub fn report(&self) -> &Report {
            &self.report
        }

        #[must_use]
        pub fn random_label(&self) -> Option<&leakage::RandomLabelResult> {
            self.random_label.as_ref()
        }
    }
}

/// Datasets audited per tenant per pass. Each one is a full rebuild, so this is
/// a real cost; the most recent are the ones a rebuild can still be compared
/// against.
const MAX_DATASETS_PER_PASS: i64 = 5;

/// Permutations per random-label pass. Eight is enough for the maximum |t| to
/// have a usable null and cheap enough to run nightly.
const PERMUTATIONS: usize = 8;

/// Walk-forward folds the harness pass runs over. Three is the smallest number
/// that can show a trend across folds rather than one sample.
const FOLDS: u64 = 3;

#[derive(Debug, Default, Serialize)]
pub struct AuditReport {
    pub tenant: String,
    pub datasets_audited: usize,
    pub blocking: usize,
    pub flags: usize,
    /// Datasets whose spec could not be rebuilt (instruments retired, bars
    /// aged out). Counted, never silently skipped.
    pub unrebuildable: usize,
}

/// Tenants with any materialized dataset.
///
/// # Errors
/// Backend failures.
pub async fn dataset_tenants(pg: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT t FROM dataplane.dataset_tenants() AS t ORDER BY t")
            .fetch_all(pg)
            .await?,
    )
}

/// Audit one tenant's recent datasets.
///
/// For each: rebuild it and compare the digest against the one recorded when it
/// was first materialized (§12.5·3). A dataset that cannot be rebuilt is counted
/// as unrebuildable rather than passed, and — since the comparison never
/// happened — does not put `snapshot_reproducibility` in `checks_run` either.
///
/// # Errors
/// Backend failures. A single dataset's failure is recorded, not propagated.
pub async fn audit_tenant(
    pg: &PgPool,
    datasets: &model_registry::datasets::DatasetManager,
    tenant: &str,
) -> anyhow::Result<AuditReport> {
    let started_at = Utc::now();
    let mut out = AuditReport {
        tenant: tenant.to_string(),
        ..AuditReport::default()
    };
    let mut tally = Tally::default();

    let specs = recent_specs(pg, tenant).await?;
    let mut newest_plan = None;
    for (dataset_id, spec_json, request_json, recorded) in specs {
        out.datasets_audited += 1;
        let plan = match datasets.replay(tenant, &dataset_id, &spec_json, &request_json) {
            Ok(p) => p,
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, %dataset_id, error = %e, "leakage audit: dataset cannot be replayed by this process");
                continue;
            }
        };
        match datasets.rebuild_digest(&plan).await {
            Ok((rebuilt, _)) => {
                tally.run(Pass::SnapshotReproducibility {
                    dataset_id: &dataset_id,
                    recorded: &recorded,
                    rebuilt: &rebuilt,
                });
                if newest_plan.is_none() {
                    newest_plan = Some(plan);
                }
            }
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, %dataset_id, error = %e, "leakage audit: rebuild failed");
            }
        }
    }

    // The harness-level pass (§12.5·2): the two frame screens and the
    // random-label test through the *production* fold geometry. It runs over a
    // real dataset rather than a fixture, because a fixture only ever tests the
    // fixture.
    //
    // The causal guard (§12.5·1) is absent here and is absent from `checks_run`
    // with it: it needs the dataset's rows, which this process cannot obtain.
    // See the module header.
    if let Some(plan) = newest_plan {
        match datasets.training_frame(&plan).await {
            Ok(frame) => {
                tally.run(Pass::TargetCorrelation(&frame));
                tally.run(Pass::FullSampleNormalization(&frame));

                let pipeline =
                    features::walk_forward::embargo_inputs(&plan.features, plan.horizon_bars);
                match harness_geometry(&frame, &pipeline) {
                    Some((spec, folds)) => {
                        // Real work with no name of its own to claim: the
                        // fold-geometry screen reports under `causal_access`,
                        // which is a different check. Its findings are recorded;
                        // no check is claimed for them.
                        tally.add_findings(leakage::fold_geometry(&folds));

                        tally.run(Pass::RandomLabel {
                            frame: &frame,
                            spec: &spec,
                            pipeline: &pipeline,
                            permutations: PERMUTATIONS,
                            seed: seed_for(tenant),
                        });
                    }
                    None => {
                        tracing::warn!(
                            %tenant,
                            rows = frame.row_count(),
                            "leakage audit: no fold geometry fits this frame; the random-label test did not run"
                        );
                        tally.add_findings(geometry_unavailable(&frame));
                    }
                }
            }
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, error = %e, "leakage audit: could not build the harness frame");
            }
        }
    }

    out.blocking = tally
        .report()
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Blocking)
        .count();
    out.flags = tally.report().findings.len() - out.blocking;

    record(
        pg,
        tenant,
        "platform",
        tally.checks_run(),
        tally.report(),
        tally.random_label(),
        started_at,
    )
    .await?;
    Ok(out)
}

/// The most recently created dataset specs that have a recorded digest to
/// compare a rebuild against.
async fn recent_specs(
    pg: &PgPool,
    tenant: &str,
) -> anyhow::Result<Vec<(String, serde_json::Value, serde_json::Value, String)>> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
    let rows = sqlx::query(
        "SELECT s.dataset_id, s.spec, d.request, d.frame_digest \
           FROM dataplane.dataset_spec s \
           JOIN dataplane.dataset_frame_digest d ON d.dataset_id = s.dataset_id \
          ORDER BY s.created_at DESC \
          LIMIT $1",
    )
    .bind(MAX_DATASETS_PER_PASS)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("dataset_id"),
                r.get::<serde_json::Value, _>("spec"),
                r.get::<serde_json::Value, _>("request"),
                r.get::<String, _>("frame_digest"),
            )
        })
        .collect())
}

/// The fold geometry the harness pass runs under, together with the folds it
/// builds — or `None` when this frame cannot carry one that measures anything.
///
/// Three expanding folds, sized from the frame so the test works on whatever the
/// tenant actually has rather than only on a convenient sample.
///
/// [`features::walk_forward_folds`] lays the folds end to end, so `FOLDS` of
/// them reach `FOLDS·(train + 2·purge + cal + test) + (FOLDS−1)·embargo` rows.
/// The roles get what is left once the gaps are paid for, which is the step the
/// previous sizing skipped; see the module header.
///
/// The folds are returned rather than the spec alone because the caller needs
/// them anyway, and a caller that rebuilds them from the spec can get a
/// different answer than the one verified here.
///
/// # Verified, not asserted
///
/// The candidate is checked by building it, and each fold is checked against the
/// rule [`features::leakage::random_label`] skips folds by: a fold whose
/// training rows cannot outnumber the coefficients is never fitted. A geometry
/// whose folds are all skipped does not fail loudly — it returns one `NaN`
/// t-statistic per permutation, which is the shape of a pass that ran. So it is
/// refused here instead.
#[must_use]
pub fn harness_geometry(
    frame: &features::TrainingFrame,
    pipeline: &dataplane::split::EmbargoInputs,
) -> Option<(domain::model_def::cv::WalkForwardSpec, Vec<features::Fold>)> {
    let rows = frame.row_count();
    // The spec asks for neither, so both are the pipeline's own minimum: purge
    // is forced up to the label horizon and embargo to §12.2's computed value.
    let purge = u64::from(pipeline.horizon_bars);
    let embargo = u64::from(dataplane::split::compute_embargo(pipeline));

    let gaps = 2 * FOLDS * purge + (FOLDS - 1) * embargo;
    let unit = (rows as u64).checked_sub(gaps)? / FOLDS;
    let test = (unit / 5).max(1);
    let cal = (unit / 10).max(1);
    let train = unit.checked_sub(test + cal)?;

    let spec = domain::model_def::cv::WalkForwardSpec {
        mode: domain::model_def::cv::WindowMode::Expanding,
        folds: u32::try_from(FOLDS).ok()?,
        train_bars: train,
        cal_bars: cal,
        test_bars: test,
        purge_bars: 0,
        embargo_bars: 0,
    };

    let folds = features::walk_forward_folds(rows, &spec, pipeline).ok()?;
    let min_train = frame.feature_names.len() + 2;
    folds
        .iter()
        .all(|f| f.train.len() >= min_train && !f.test.is_empty())
        .then_some((spec, folds))
}

/// A flag saying the harness pass had nowhere to run, so that a frame too short
/// to carry the geometry is a recorded fact rather than a silent absence.
fn geometry_unavailable(frame: &features::TrainingFrame) -> Report {
    Report {
        findings: vec![features::leakage::Finding {
            check: features::leakage::Check::RandomLabel,
            severity: Severity::Flag,
            subject: "folds".into(),
            statistic: frame.row_count() as f64,
            detail: format!(
                "{} rows cannot carry {FOLDS} walk-forward folds that this pipeline's purge and \
                 embargo leave room to fit and score; the random-label test did not run and \
                 claims nothing",
                frame.row_count()
            ),
        }],
    }
}

/// A per-tenant seed, fixed. A leakage verdict that changes with the calendar is
/// not a verdict, so the permutation draw must be the same every night for the
/// same tenant and different across tenants.
fn seed_for(tenant: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in tenant.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

async fn record(
    pg: &PgPool,
    tenant: &str,
    subject: &str,
    checks_run: &[String],
    report: &Report,
    random_label: Option<&leakage::RandomLabelResult>,
    started_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let run_id = Uuid::new_v4();
    let blocking = i32::try_from(report.blocking().count()).unwrap_or(i32::MAX);
    let flags = i32::try_from(report.findings.len()).unwrap_or(i32::MAX) - blocking;

    let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
    sqlx::query(
        "INSERT INTO mlops.leakage_run \
         (leakage_run_id, tenant_id, subject, checks_run, blocking_count, flag_count, \
          permuted_max_abs_t, real_sharpe, started_at, finished_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,now())",
    )
    .bind(run_id)
    .bind(tenant)
    .bind(subject)
    .bind(checks_run)
    .bind(blocking)
    .bind(flags.max(0))
    .bind(
        random_label
            .map(|r| r.permuted_max_abs_t)
            .filter(|t| t.is_finite()),
    )
    .bind(
        random_label
            .map(|r| r.real_sharpe)
            .filter(|s| s.is_finite()),
    )
    .bind(started_at)
    .execute(&mut *tx)
    .await?;

    for (i, f) in report.findings.iter().enumerate() {
        sqlx::query(
            "INSERT INTO mlops.leakage_finding \
             (leakage_run_id, ordinal, tenant_id, check_name, severity, subject, statistic, detail) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(run_id)
        .bind(i32::try_from(i).unwrap_or(i32::MAX))
        .bind(tenant)
        .bind(f.check.to_string())
        .bind(match f.severity {
            Severity::Blocking => "blocking",
            Severity::Flag => "flag",
        })
        .bind(&f.subject)
        .bind(f.statistic)
        .bind(&f.detail)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Permutations the random-label pass uses. Exposed so the job and its test
/// agree on the number rather than each picking one.
#[must_use]
pub const fn permutations() -> usize {
    PERMUTATIONS
}
