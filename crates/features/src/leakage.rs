//! The leakage suite (SPEC §12.5, §13 M10; checklist 1.7–1.11).
//!
//! Six checks and one apparatus:
//!
//! 1. **Causal access guard** (§12.5·1) — every feature in the pipeline is run
//!    under the windowed proxy, which cannot name a future row at all and raises
//!    on a read past its declared window, and is then probed for the effect: a
//!    feature whose value moves when future rows move is reading them, whatever
//!    its declaration says.
//! 2. **Random-label test** (§12.5·2) — permuted labels must yield no edge. This
//!    validates the *harness* (fold geometry, purge, embargo, frame assembly),
//!    not the features, which is why it runs against the platform itself rather
//!    than per strategy.
//! 3. **Snapshot reproducibility** (§12.5·3) — the same `dataset_id` rebuilt
//!    later must produce the same bytes. Catches retroactive adjusted-price
//!    mutation and calendar revisions.
//! 4. **CV − WF Sharpe gap** — a soft flag, not a block: a cross-validated
//!    Sharpe more than 1.0 above the walk-forward Sharpe is the signature of
//!    overlapping-label leakage.
//! 5. **Causal alignment** ([`causal_alignment`]) — the same behavioural probe
//!    as (1), one layer down, where the ordering key is *knowledge time* rather
//!    than a row index. A grid row's value is recomputed with every observation
//!    that was not yet knowable at that tick replaced by noise; a value that
//!    moves came from the future.
//! 6. **Label-horizon embargo** ([`label_horizon_embargo`]) — the purge gap must
//!    cover the label horizon the frame was actually built with, not a smaller
//!    one a caller happened to declare.
//!
//! **Where a future read is even possible.** Checks (1) and (5) look alike and
//! are not interchangeable, because only one of them can fire. A [`Feature`] is
//! handed a [`WindowedFrame`], whose only accessor counts *backward* from the
//! decision bar and refuses to count past the declared lookback; the frame's
//! rows and decision index are private. A future-reading `Feature` is therefore
//! not merely forbidden, it is **unrepresentable**, which is why [`inject`] has
//! no variant that plants one — a detector that structurally cannot fire, feeding
//! a gate, manufactures evidence of cleanliness. The leak classes that *are*
//! representable live outside the `Feature` trait, where a future read is
//! legitimate by design and so can be got wrong: the master-clock alignment in
//! [`crate::align`], which keys on a `knowledge_ns` nothing independently
//! verifies, and the forward-return label in [`crate::training_frame`], whose
//! honesty rests entirely on the purge gap. (5) and (6) are those two.
//!
//! Plus [`inject`]: deliberate, known leaks planted in a known-clean frame. They
//! are how this suite is tested — a detector nobody has shown a real leak to is
//! a detector nobody has tested — and they are the labelled-data generator M10
//! trains on (§13: "trainable on day one").

use std::fmt;

use dataplane::feature::{FeatureError, FeatureRow, WindowedFrame};

use crate::align::BarObs;
use crate::training_frame::TrainingFrame;

/// A finding either blocks or flags. Nothing in this suite is advisory-only:
/// a blocking finding means the pipeline is not safe to trust, a flag means a
/// human has to look and the trial carries the mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Blocking,
    Flag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    CausalAccess,
    RandomLabel,
    SnapshotReproducibility,
    CvWfGap,
    /// A feature column that is (nearly) the label itself.
    TargetCorrelation,
    /// A feature column standardized over the whole sample rather than within
    /// the training block.
    FullSampleNormalization,
    /// A grid row whose value came from an observation that was not yet
    /// knowable at that grid tick — the alignment layer's future read.
    CausalAlignment,
    /// A purge gap narrower than the label horizon it has to cover.
    LabelHorizonEmbargo,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::CausalAccess => "causal_access",
            Self::RandomLabel => "random_label",
            Self::SnapshotReproducibility => "snapshot_reproducibility",
            Self::CvWfGap => "cv_wf_gap",
            Self::TargetCorrelation => "target_correlation",
            Self::FullSampleNormalization => "full_sample_normalization",
            Self::CausalAlignment => "causal_alignment",
            Self::LabelHorizonEmbargo => "label_horizon_embargo",
        };
        f.write_str(s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub check: Check,
    pub severity: Severity,
    /// What the finding is about: a feature id, a dataset id, a fold.
    pub subject: String,
    /// The number that decided it, so a threshold change is auditable.
    pub statistic: f64,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// Findings that mean the pipeline must not be trusted.
    pub fn blocking(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Blocking)
    }

    #[must_use]
    pub fn has_blocking(&self) -> bool {
        self.blocking().next().is_some()
    }

    fn push(&mut self, f: Finding) {
        self.findings.push(f);
    }

    fn merge(&mut self, other: Self) {
        self.findings.extend(other.findings);
    }
}

// ===========================================================================
// 1.7 — causal access guard
// ===========================================================================

/// Run every feature in `features` over `rows` under the causal proxy.
///
/// Two things are checked, because they fail differently:
///
/// * **Structural** — the windowed view refuses a read past the declared window,
///   and the raising [`dataplane::feature::CausalGuard`] refuses an absolute read
///   past the decision row. This is the proxy §12.5·1 asks for.
/// * **Behavioural** — the value at a decision row is recomputed with every
///   later row replaced by noise, and with every row before the declared window
///   replaced by noise. A value that moves is reading data it declared it would
///   not, whatever mechanism it used to get there.
///
/// The behavioural probe is the one that catches a lying feature, which is why
/// it runs over the whole pipeline's feature set and not only at registration.
///
/// **What this check can and cannot find.** The structural half is a regression
/// test on [`WindowedFrame`] and [`dataplane::feature::CausalGuard`]. The
/// behavioural half cannot fire for any `Feature` reachable through
/// [`crate::runtime`], because the window a feature is handed makes a future read
/// unrepresentable (see the module docs). It is kept because it is what a future
/// `Feature` implementation — or a change to `WindowedFrame` — would have to
/// defeat, and it is deliberately **not** wired to anything as evidence of
/// cleanliness. The check that can find a real future read at this layer is
/// [`causal_alignment`].
#[must_use]
pub fn causal_access(features: &[String], rows: &[FeatureRow]) -> Report {
    let mut report = Report::default();
    if rows.len() < 4 {
        return report;
    }

    for name in features {
        let Ok(f) = crate::runtime::feature(name) else {
            report.push(Finding {
                check: Check::CausalAccess,
                severity: Severity::Blocking,
                subject: name.clone(),
                statistic: 0.0,
                detail: "no registered implementation; an unresolvable feature cannot be audited"
                    .into(),
            });
            continue;
        };
        let def = f.def();

        // The decision row sits two thirds of the way in, so "every later row"
        // in the behavioural probe below is a third of the series rather than
        // the single trailing row a decision of `len - 2` would leave. It is
        // pushed forward far enough for the declared window to exist, and never
        // past `len - 2`, because a probe with no later rows to perturb is not a
        // probe. A feature whose window does not fit is skipped by `value_at`.
        let decision = (rows.len() * 2 / 3)
            .max(def.lookback_bars as usize)
            .min(rows.len() - 2);

        // Structural: the proxy a feature is handed cannot name a future row at
        // all -- its only accessor counts *backward* from the decision bar -- and
        // it refuses to count back past the declared lookback. Unrepresentable is
        // stronger than "raises", but the refusal is still asserted here, because
        // it is what a future `Feature` implementation would have to defeat.
        let frame = WindowedFrame::new(&def.feature_id, rows, decision, def.lookback_bars);
        if !matches!(
            frame.back(def.lookback_bars as usize),
            Err(FeatureError::LookbackExceeded { .. })
        ) {
            report.push(Finding {
                check: Check::CausalAccess,
                severity: Severity::Blocking,
                subject: name.clone(),
                statistic: f64::from(def.lookback_bars),
                detail: "the windowed view did not refuse a read past the declared lookback".into(),
            });
        }
        // And the raising proxy over a bare row index refuses the future, which
        // is the §12.5 contract for any reader that does index absolutely.
        let guard = dataplane::feature::CausalGuard::new(&[0.0; 4], 1);
        if !matches!(guard.at(2), Err(FeatureError::FutureRead { .. })) {
            report.push(Finding {
                check: Check::CausalAccess,
                severity: Severity::Blocking,
                subject: name.clone(),
                statistic: 0.0,
                detail: "the causal guard did not refuse a read past the decision row".into(),
            });
        }

        let Some(base) = crate::runtime::value_at(f.as_ref(), rows, decision) else {
            // No value at this row (window not yet complete) — nothing to probe.
            continue;
        };

        // Behavioural: the future must not matter.
        let future_perturbed = perturb(rows, decision + 1..rows.len());
        if let Some(v) = crate::runtime::value_at(f.as_ref(), &future_perturbed, decision) {
            let delta = (v - base).abs();
            if delta > 1e-12 {
                report.push(Finding {
                    check: Check::CausalAccess,
                    severity: Severity::Blocking,
                    subject: name.clone(),
                    statistic: delta,
                    detail: format!(
                        "value at the decision row changed by {delta:.3e} when only later rows changed: \
                         this feature reads the future"
                    ),
                });
            }
        }

        // Behavioural: data before the declared window must not matter either —
        // an understated lookback is leakage in the embargo computation.
        let lb = def.lookback_bars as usize;
        if decision >= lb {
            let past_perturbed = perturb(rows, 0..decision + 1 - lb);
            if let Some(v) = crate::runtime::value_at(f.as_ref(), &past_perturbed, decision) {
                let delta = (v - base).abs();
                if delta > 1e-12 {
                    report.push(Finding {
                        check: Check::CausalAccess,
                        severity: Severity::Blocking,
                        subject: name.clone(),
                        statistic: delta,
                        detail: format!(
                            "value at the decision row changed by {delta:.3e} when only rows before the \
                             declared lookback of {lb} changed: the declaration understates the window"
                        ),
                    });
                }
            }
        }
    }
    report
}

/// Replace a row range with values that share nothing with the original, so any
/// dependence shows up as a difference rather than a coincidence.
fn perturb(rows: &[FeatureRow], range: std::ops::Range<usize>) -> Vec<FeatureRow> {
    let mut out = rows.to_vec();
    for (i, r) in out.iter_mut().enumerate() {
        if range.contains(&i) {
            let bump = 913.7 + (i % 29) as f64 * 11.3;
            r.open = deviate(r.open, bump);
            r.high = deviate(r.high, bump + 4.0);
            r.low = deviate(r.low, bump - 4.0);
            r.close = deviate(r.close, bump);
            r.volume = deviate(r.volume, bump + 7.0);
        }
    }
    out
}

/// A value that is guaranteed to differ from its input.
///
/// The affine map alone has one fixed point (`v == bump / 3.5`), and a
/// perturbation that silently leaves one value alone is a probe that silently
/// fails to fire on it. Nudging the collision is cheaper than reasoning about
/// whether the data can reach it.
fn deviate(v: f64, bump: f64) -> f64 {
    let out = v.mul_add(-2.5, bump);
    if out == v || !out.is_finite() {
        v + 1.0
    } else {
        out
    }
}

// ===========================================================================
// Causal alignment — the future read that IS representable
// ===========================================================================

/// What a probe takes to be an observation's **true** knowledge time.
///
/// [`crate::align::densify_bars`] keys on the `knowledge_ns` it is handed and has
/// no way to check it. If that stamp is earlier than the moment the bar could
/// actually have been observed, every grid row that carries the bar forward is
/// reading the future, and nothing downstream can tell — the frame looks
/// perfectly causal with respect to a timestamp that is itself the lie.
///
/// So the hypothesis about the stamp is an explicit, typed input rather than
/// something a probe guesses. [`Self::AsDeclared`] is the default and asserts
/// nothing about the data: under it the probe is a regression test on the
/// densifier alone, and an honest densifier yields no finding. The other
/// variants name a specific way a stamp is wrong and ask whether the pipeline
/// would have been fooled by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamping {
    /// Take each observation's declared `knowledge_ns` at face value.
    ///
    /// The null. A finding under this hypothesis is a defect in the densifier,
    /// not in the data, because the data is being believed.
    AsDeclared,
    /// Hypothesis: `ts_ns` is the bar's **open**, so the bar was not knowable
    /// until its own period had elapsed — true knowledge is `ts_ns + step_ns`.
    ///
    /// This is the single most common bitemporal defect in market data: a vendor
    /// or an ingest path stamps a bar with the instant it *began*, and a whole
    /// period of its own future arrives a period early. The hypothesis is
    /// applied as a maximum against the declared stamp, so a series that already
    /// stamps at the close is unchanged by it and stays clean.
    KnowableAtBarClose,
    /// Hypothesis: the source publishes `lag_ns` after the moment it declares —
    /// a filing, a macro release, a revision-bearing series.
    PublishedAfter { lag_ns: i64 },
}

impl Stamping {
    /// The moment this hypothesis says the observation was really knowable.
    #[must_use]
    pub fn truth_ns(self, obs: &BarObs, step_ns: i64) -> i64 {
        match self {
            Self::AsDeclared => obs.knowledge_ns,
            Self::KnowableAtBarClose => obs.knowledge_ns.max(obs.ts_ns.saturating_add(step_ns)),
            Self::PublishedAfter { lag_ns } => obs.knowledge_ns.saturating_add(lag_ns),
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::AsDeclared => "as_declared",
            Self::KnowableAtBarClose => "knowable_at_bar_close",
            Self::PublishedAfter { .. } => "published_after",
        }
    }
}

impl fmt::Display for Stamping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PublishedAfter { lag_ns } => write!(f, "published_after({lag_ns}ns)"),
            other => f.write_str(other.label()),
        }
    }
}

/// Grid ticks probed in one pass, at most.
///
/// Each probed tick costs a full re-densification, so an unbounded probe is
/// quadratic in the series length and would simply never be run. The cap is on
/// the *boundaries* — the ticks at which a new observation first becomes visible
/// — because a carried tick reads the same observation as the boundary before
/// it and cannot fail differently.
pub const MAX_PROBE_TICKS: usize = 64;

/// One causal-alignment pass, with its own coverage.
///
/// The coverage counts are here rather than buried in a log because a probe that
/// examined 64 of 90 000 boundaries and found nothing has not established that
/// the other 89 936 are clean. An absent measurement is not a zero, and a caller
/// that wants to treat this as evidence has to look at `probed_ticks` to know
/// what it is evidence of.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentResult {
    pub stamping: Stamping,
    /// Observations handed in.
    pub observations: usize,
    /// Grid rows the densifier produced.
    pub grid_ticks: usize,
    /// Ticks at which some observation first becomes visible — the ticks a leak
    /// can show at.
    pub boundaries: usize,
    /// Boundaries actually re-densified and compared.
    pub probed_ticks: usize,
    pub report: Report,
}

/// Behavioural causal probe on the **alignment layer** (SPEC §2, INV-10, INV-11).
///
/// The claim under test is the platform's core bitemporal one: *a grid row's
/// value is a function only of observations that were knowable at or before that
/// row's tick.* It is checked the way [`causal_access`] checks a feature — by
/// perturbation, not by reading the densifier's source — so it survives the
/// densifier being rewritten:
///
/// 1. Densify `obs` honestly. That is the baseline.
/// 2. At a probe tick `t`, rebuild the frame with the **values** of every
///    observation whose `stamping`-true knowledge time is later than `t`
///    replaced by noise. Knowledge times are left alone, so the grid is
///    identical and the rows line up.
/// 3. If the row at `t` moved, it was carrying an observation that was not yet
///    knowable there.
///
/// Under [`Stamping::AsDeclared`] this fires on nothing an honest densifier
/// produces — that is what makes it a null rather than a ritual. Under
/// [`Stamping::KnowableAtBarClose`] it fires on a series stamped at the bar open,
/// which is the leak it exists to find.
///
/// Nothing perturbed leaves this function. The return value carries counts and
/// findings; the noised series is local, so there is no way for a caller to
/// receive a frame built from it and mistake it for data.
#[must_use]
pub fn causal_alignment(obs: &[BarObs], step_ns: i64, stamping: Stamping) -> AlignmentResult {
    let mut report = Report::default();

    // Convention-independent, and certain: no reading of `ts_ns` makes a bar
    // knowable strictly before it began. This needs no hypothesis and no
    // perturbation, so it runs first and always.
    for (i, o) in obs.iter().enumerate() {
        if o.knowledge_ns < o.ts_ns {
            let gap = o.ts_ns.saturating_sub(o.knowledge_ns);
            report.push(Finding {
                check: Check::CausalAlignment,
                severity: Severity::Blocking,
                subject: format!("observation {i} at ts_ns {}", o.ts_ns),
                statistic: gap as f64,
                detail: format!(
                    "stamped knowable at {} but its own bar starts at {}: knowable {gap} ns before \
                     it began, which no timestamp convention makes honest",
                    o.knowledge_ns, o.ts_ns
                ),
            });
        }
    }

    let baseline = crate::align::densify_bars(obs, step_ns);
    let mut out = AlignmentResult {
        stamping,
        observations: obs.len(),
        grid_ticks: baseline.len(),
        boundaries: 0,
        probed_ticks: 0,
        report,
    };
    if baseline.is_empty() || step_ns <= 0 {
        return out;
    }

    let boundaries = boundary_ticks(&baseline, obs);
    out.boundaries = boundaries.len();
    let probe_at = subsample(&boundaries, MAX_PROBE_TICKS);
    out.probed_ticks = probe_at.len();

    for &i in &probe_at {
        let tick = baseline.rows[i].ts_ns;
        let blinded = blind_after(obs, step_ns, stamping, tick);
        let rebuilt = crate::align::densify_bars(&blinded, step_ns);
        // Blinding touches values only, so the grid is unchanged; if it somehow
        // is not, that is itself a finding rather than something to paper over.
        let Some(row) = rebuilt.rows.get(i).filter(|r| r.ts_ns == tick) else {
            out.report.push(Finding {
                check: Check::CausalAlignment,
                severity: Severity::Blocking,
                subject: format!("tick {tick}"),
                statistic: i as f64,
                detail: "the grid moved when only observation values changed: densification is \
                         not a function of knowledge times alone"
                    .into(),
            });
            continue;
        };
        let base = baseline.rows[i];
        let delta = [
            (row.open - base.open).abs(),
            (row.high - base.high).abs(),
            (row.low - base.low).abs(),
            (row.close - base.close).abs(),
            (row.volume - base.volume).abs(),
        ]
        .into_iter()
        .fold(0.0_f64, f64::max);
        if delta > 0.0 {
            out.report.push(Finding {
                check: Check::CausalAlignment,
                severity: Severity::Blocking,
                subject: format!("tick {tick}"),
                statistic: delta,
                detail: format!(
                    "the grid row at {tick} moved by {delta:.3e} when only observations that were \
                     not yet knowable there changed (stamping {stamping}): this row carries the \
                     future"
                ),
            });
        }
    }

    if out.probed_ticks < out.boundaries {
        out.report.push(Finding {
            check: Check::CausalAlignment,
            severity: Severity::Flag,
            subject: "coverage".into(),
            statistic: (out.boundaries - out.probed_ticks) as f64,
            detail: format!(
                "{} of {} knowledge boundaries were probed; the remaining {} were not examined and \
                 are not evidence of anything",
                out.probed_ticks,
                out.boundaries,
                out.boundaries - out.probed_ticks
            ),
        });
    }
    out
}

/// The grid indices at which some observation first becomes visible.
///
/// A tick between two boundaries carries the same observation forward as the
/// boundary before it, so it cannot fail differently; probing it would cost a
/// full re-densification to re-ask a question already answered.
fn boundary_ticks(frame: &crate::align::MasterClockFrame, obs: &[BarObs]) -> Vec<usize> {
    let mut ticks: Vec<usize> = obs
        .iter()
        .map(|o| frame.rows.partition_point(|r| r.ts_ns < o.knowledge_ns))
        .filter(|&i| i < frame.rows.len())
        .collect();
    ticks.sort_unstable();
    ticks.dedup();
    ticks
}

/// At most `cap` entries, evenly spread, always including the first and last.
fn subsample(xs: &[usize], cap: usize) -> Vec<usize> {
    if xs.len() <= cap || cap == 0 {
        return xs.to_vec();
    }
    let last = xs.len() - 1;
    let mut out: Vec<usize> = (0..cap).map(|k| xs[k * last / (cap - 1).max(1)]).collect();
    out.dedup();
    out
}

/// Every observation not yet knowable at `tick_ns` under `stamping`, with its
/// values replaced by noise and its timestamps untouched.
///
/// Private, and it stays private: this is a deliberately wrong reading of the
/// data, and the only thing any caller may learn from it is whether a value
/// moved.
fn blind_after(obs: &[BarObs], step_ns: i64, stamping: Stamping, tick_ns: i64) -> Vec<BarObs> {
    let mut out = obs.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        if stamping.truth_ns(o, step_ns) > tick_ns {
            let bump = 613.1 + (i % 31) as f64 * 17.9;
            o.open = deviate(o.open, bump);
            o.high = deviate(o.high, bump + 4.0);
            o.low = deviate(o.low, bump - 4.0);
            o.close = deviate(o.close, bump);
            o.volume = deviate(o.volume, bump + 7.0);
        }
    }
    out
}

// ===========================================================================
// Label horizon vs. the purge gap
// ===========================================================================

/// The purge gap must cover the label horizon the frame was **actually built
/// with** (SPEC §3.5, INV-15).
///
/// [`crate::walk_forward_folds`] already raises the purge to
/// `max(spec.purge_bars, pipeline.horizon_bars)`, so the geometry is safe with
/// respect to the horizon *it is told about*. Nothing checks that this is the
/// horizon the frame's label used, and it is not: [`crate::TrainingFrame`] does
/// not record its own horizon, so a caller that builds a frame at H = 60 and
/// derives its [`dataplane::split::EmbargoInputs`] from a stale H = 1 gets a
/// one-bar purge across fifty-nine bars of overlapping label — and every check
/// downstream passes, because each one is individually correct.
///
/// This is checked by comparison rather than by perturbation on purpose: two
/// integers either agree or they do not, and a question that has a certain
/// answer should not be handed to one that answers with a statistic.
///
/// `frame_horizon_bars` is the horizon passed to [`crate::build_training_frame`]
/// or [`crate::build_aligned_training_frame`]. It is an explicit argument
/// because the frame cannot be asked: see the note above.
#[must_use]
pub fn label_horizon_embargo(
    subject: &str,
    frame_horizon_bars: u64,
    pipeline: &dataplane::split::EmbargoInputs,
) -> Report {
    let mut report = Report::default();
    let declared = u64::from(pipeline.horizon_bars);
    if declared < frame_horizon_bars {
        report.push(Finding {
            check: Check::LabelHorizonEmbargo,
            severity: Severity::Blocking,
            subject: subject.to_string(),
            statistic: (frame_horizon_bars - declared) as f64,
            detail: format!(
                "the frame's label reaches {frame_horizon_bars} bars forward but the fold geometry \
                 was given a horizon of {declared}: {} bars of every train row's label window fall \
                 inside the block it is scored on",
                frame_horizon_bars - declared
            ),
        });
    }
    report
}

// ===========================================================================
// 1.8 — random-label test
// ===========================================================================

/// Threshold above which a permuted-label result is not credible as noise.
///
/// The limit is on the **t-statistic** of the mean out-of-sample return, not on
/// the Sharpe ratio, because only the t-statistic has a calibrated null: a
/// Sharpe of 0.2 is noise on 200 observations and a finding on 200 000, and a
/// fixed Sharpe threshold cannot tell those apart. Under a clean harness each
/// permutation's t is approximately standard normal, so the largest of a handful
/// of draws sits near 2 and essentially never reaches 4. A real leak does not
/// land near the threshold at all — a feature column that *is* the label puts
/// the t in the tens — so there is no regime where this choice is delicate.
pub const RANDOM_LABEL_T_LIMIT: f64 = 4.0;

/// Outcome of one random-label pass.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomLabelResult {
    /// Out-of-sample Sharpe (mean / sd per observation) on the real labels.
    pub real_sharpe: f64,
    /// t-statistic of the mean out-of-sample return on the real labels.
    pub real_t: f64,
    /// Out-of-sample Sharpe per permutation.
    pub permuted_sharpes: Vec<f64>,
    /// t-statistic per permutation — what the limit is applied to.
    pub permuted_t_stats: Vec<f64>,
    /// Mean of `permuted_sharpes`.
    pub permuted_mean: f64,
    /// Largest absolute permuted t-statistic.
    pub permuted_max_abs_t: f64,
    pub report: Report,
}

/// Permuted labels must yield Sharpe ≈ 0 (§12.5·2).
///
/// The probe model is a deterministic ridge fit — deliberately the simplest
/// thing that can express a linear relationship — because the subject of this
/// test is not the model. It is the harness: the fold geometry, the purge and
/// embargo gaps, and the frame assembly. If a permuted label still earns a
/// Sharpe, the harness is leaking the answer and *every* model trained through
/// it inherits that, regardless of which one it is.
///
/// Folds come from [`crate::walk_forward_folds`], so this tests the same
/// geometry production uses rather than a reimplementation of it.
#[must_use]
pub fn random_label(
    frame: &TrainingFrame,
    spec: &domain::model_def::cv::WalkForwardSpec,
    pipeline: &dataplane::split::EmbargoInputs,
    permutations: usize,
    seed: u64,
) -> RandomLabelResult {
    let mut report = Report::default();
    let Ok(folds) = crate::walk_forward_folds(frame.row_count(), spec, pipeline) else {
        report.push(Finding {
            check: Check::RandomLabel,
            severity: Severity::Flag,
            subject: "folds".into(),
            statistic: frame.row_count() as f64,
            detail: "not enough rows to build the production fold geometry; test did not run".into(),
        });
        return RandomLabelResult {
            real_sharpe: f64::NAN,
            real_t: f64::NAN,
            permuted_sharpes: Vec::new(),
            permuted_t_stats: Vec::new(),
            permuted_mean: f64::NAN,
            permuted_max_abs_t: f64::NAN,
            report,
        };
    };

    let (real_sharpe, real_t) = oos_stats(frame, &frame.label, &folds);

    let mut rng = Lcg::new(seed);
    let mut permuted_sharpes = Vec::with_capacity(permutations);
    let mut permuted_t_stats = Vec::with_capacity(permutations);
    for _ in 0..permutations {
        let shuffled = shuffle(&frame.label, &mut rng);
        let (s, t) = oos_stats(frame, &shuffled, &folds);
        permuted_sharpes.push(s);
        permuted_t_stats.push(t);
    }

    let finite: Vec<f64> = permuted_sharpes
        .iter()
        .copied()
        .filter(|s| s.is_finite())
        .collect();
    let permuted_mean = if finite.is_empty() {
        f64::NAN
    } else {
        finite.iter().sum::<f64>() / finite.len() as f64
    };
    let permuted_max_abs_t = permuted_t_stats
        .iter()
        .filter(|t| t.is_finite())
        .fold(0.0_f64, |m, t| m.max(t.abs()));

    if permuted_max_abs_t > RANDOM_LABEL_T_LIMIT {
        report.push(Finding {
            check: Check::RandomLabel,
            severity: Severity::Blocking,
            subject: "harness".into(),
            statistic: permuted_max_abs_t,
            detail: format!(
                "a permuted label earned an out-of-sample mean return with t = {permuted_max_abs_t:.2} \
                 (limit {RANDOM_LABEL_T_LIMIT}); the harness is leaking the answer"
            ),
        });
    }

    RandomLabelResult {
        real_sharpe,
        real_t,
        permuted_sharpes,
        permuted_t_stats,
        permuted_mean,
        permuted_max_abs_t,
        report,
    }
}

/// Fit on each fold's train rows, score on its test rows, and report the Sharpe
/// and the t-statistic of the concatenated out-of-sample position returns.
///
/// **Two probes, and the worse answer wins.** A ridge catches a harness that
/// leaks something a linear model can express -- a global transform, a target
/// that survives the split. It cannot catch memorization: three parameters
/// cannot memorize two thousand labels, so a harness whose test rows are inside
/// its training rows looks perfectly clean to it. A 1-nearest-neighbour probe
/// catches exactly that and nothing else. Neither alone is a leakage test; the
/// maximum of the two is.
fn oos_stats(frame: &TrainingFrame, labels: &[f64], folds: &[crate::Fold]) -> (f64, f64) {
    let ridge = oos_series(frame, labels, folds, Probe::Ridge);
    let nn = oos_series(frame, labels, folds, Probe::NearestNeighbour);
    let (ts, tn) = (t_stat(&ridge), t_stat(&nn));
    let ridge_worse = ts.abs() >= tn.abs() || tn.is_nan();
    if ridge_worse {
        (sharpe(&ridge), ts)
    } else {
        (sharpe(&nn), tn)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    Ridge,
    NearestNeighbour,
}

fn oos_series(
    frame: &TrainingFrame,
    labels: &[f64],
    folds: &[crate::Fold],
    probe: Probe,
) -> Vec<f64> {
    let mut oos: Vec<f64> = Vec::new();
    for fold in folds {
        let train: Vec<usize> = fold.train.clone().collect();
        let test: Vec<usize> = fold.test.clone().collect();
        if train.len() <= frame.feature_names.len() + 1 || test.is_empty() {
            continue;
        }
        let beta = match probe {
            Probe::Ridge => match ridge_fit(frame, labels, &train) {
                Some(b) => Some(b),
                None => continue,
            },
            Probe::NearestNeighbour => None,
        };
        // Demean by the *train* block's mean return, never the test block's.
        // Without this the statistic is dominated by passive drift: a model that
        // is simply always long earns the sample's mean return at a tiny
        // variance, which looks like skill and is not. Train-block, because
        // using the test block's own mean would be a look-ahead inside the
        // diagnostic itself.
        let drift = train.iter().map(|&i| labels[i]).sum::<f64>() / train.len() as f64;
        for &i in &test {
            let pred = match (&beta, probe) {
                (Some(b), _) => predict(frame, b, i),
                (None, _) => nearest_neighbour_label(frame, labels, &train, i) - drift,
            };
            // Position = sign of the prediction; what it earns is the realized
            // label net of drift. A harness with no leak cannot make this
            // positive, because the sign carries no information about the label.
            oos.push(pred.signum() * (labels[i] - drift));
        }
    }
    oos
}

/// The label of the closest training row in feature space. On a sound harness
/// this is noise; on one whose test rows are also training rows it is the row's
/// own label, which is the whole point.
fn nearest_neighbour_label(
    frame: &TrainingFrame,
    labels: &[f64],
    train: &[usize],
    i: usize,
) -> f64 {
    let mut best = (f64::INFINITY, 0.0_f64);
    for &j in train {
        let mut d = 0.0;
        for c in &frame.columns {
            let delta = c[i] - c[j];
            d += delta * delta;
        }
        if d < best.0 {
            best = (d, labels[j]);
        }
    }
    best.1
}

/// Closed-form ridge with an intercept. `None` if the normal equations are
/// singular even after regularization.
fn ridge_fit(frame: &TrainingFrame, labels: &[f64], rows: &[usize]) -> Option<Vec<f64>> {
    const LAMBDA: f64 = 1e-6;
    let k = frame.feature_names.len() + 1;
    let mut xtx = vec![0.0_f64; k * k];
    let mut xty = vec![0.0_f64; k];
    for &i in rows {
        let x = design_row(frame, i);
        for a in 0..k {
            xty[a] += x[a] * labels[i];
            for b in 0..k {
                xtx[a * k + b] += x[a] * x[b];
            }
        }
    }
    // Regularize the slopes, never the intercept.
    for a in 1..k {
        xtx[a * k + a] += LAMBDA * rows.len() as f64;
    }
    solve(&mut xtx, &mut xty, k)
}

fn design_row(frame: &TrainingFrame, i: usize) -> Vec<f64> {
    let mut x = Vec::with_capacity(frame.feature_names.len() + 1);
    x.push(1.0);
    for c in &frame.columns {
        x.push(c[i]);
    }
    x
}

fn predict(frame: &TrainingFrame, beta: &[f64], i: usize) -> f64 {
    design_row(frame, i)
        .iter()
        .zip(beta)
        .map(|(x, b)| x * b)
        .sum()
}

/// Gauss-Jordan with partial pivoting.
fn solve(a: &mut [f64], b: &mut [f64], k: usize) -> Option<Vec<f64>> {
    for col in 0..k {
        let (mut pivot, mut best) = (col, a[col * k + col].abs());
        for r in col + 1..k {
            let v = a[r * k + col].abs();
            if v > best {
                best = v;
                pivot = r;
            }
        }
        if best < 1e-12 {
            return None;
        }
        if pivot != col {
            for c in 0..k {
                a.swap(col * k + c, pivot * k + c);
            }
            b.swap(col, pivot);
        }
        let d = a[col * k + col];
        for c in 0..k {
            a[col * k + c] /= d;
        }
        b[col] /= d;
        for r in 0..k {
            if r == col {
                continue;
            }
            let factor = a[r * k + col];
            if factor == 0.0 {
                continue;
            }
            for c in 0..k {
                a[r * k + c] -= factor * a[col * k + c];
            }
            b[r] -= factor * b[col];
        }
    }
    Some(b.to_vec())
}

/// Per-observation Sharpe (mean / sd). Annualization is deliberately omitted:
/// the frame does not know its own bar frequency, and a constant scale factor
/// would only move the threshold, not change what is being measured.
fn sharpe(xs: &[f64]) -> f64 {
    match moments(xs) {
        Some((mean, sd)) if sd > 0.0 => mean / sd,
        Some(_) => 0.0,
        None => f64::NAN,
    }
}

/// t-statistic of the mean: `mean / (sd / sqrt(n))`. This is the quantity with a
/// calibrated null, which is why the leakage threshold is applied to it.
fn t_stat(xs: &[f64]) -> f64 {
    match moments(xs) {
        Some((mean, sd)) if sd > 0.0 => mean / sd * (xs.len() as f64).sqrt(),
        Some(_) => 0.0,
        None => f64::NAN,
    }
}

fn moments(xs: &[f64]) -> Option<(f64, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some((mean, var.sqrt()))
}

/// A small deterministic PRNG. Deterministic on purpose: a leakage test whose
/// verdict depends on the day it ran is not a test.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }
}

fn shuffle(xs: &[f64], rng: &mut Lcg) -> Vec<f64> {
    let mut out = xs.to_vec();
    for i in (1..out.len()).rev() {
        let j = rng.below(i + 1);
        out.swap(i, j);
    }
    out
}

/// A fold whose roles overlap is not a split at all (§3.5, INV-15).
///
/// This is checked structurally rather than statistically because it can be:
/// two index ranges either intersect or they do not. The random-label test will
/// also catch an overlap through its nearest-neighbour probe, but a check that
/// answers with certainty should not be left to one that answers with a
/// p-value.
#[must_use]
pub fn fold_geometry(folds: &[crate::Fold]) -> Report {
    let mut report = Report::default();
    for fold in folds {
        let overlaps = |a: &std::ops::Range<usize>, b: &std::ops::Range<usize>| {
            a.start < b.end && b.start < a.end
        };
        for (name, a, b) in [
            ("train/test", &fold.train, &fold.test),
            ("train/cal", &fold.train, &fold.cal),
            ("cal/test", &fold.cal, &fold.test),
        ] {
            if overlaps(a, b) {
                report.push(Finding {
                    check: Check::CausalAccess,
                    severity: Severity::Blocking,
                    subject: format!("fold {}", fold.index),
                    statistic: f64::from(fold.index),
                    detail: format!(
                        "{name} overlap: {:?} and {:?} share rows, so the model is scored on data \
                         it was fitted on",
                        a, b
                    ),
                });
            }
        }
    }
    report
}

// ===========================================================================
// Frame-level screens: the two leaks that are visible in the data itself
// ===========================================================================

/// Above this, a feature column is not a predictor of the label — it *is* the
/// label. Set high on purpose: a genuinely strong feature on a short sample can
/// reach 0.9, and this check must not cry wolf at good research.
pub const TARGET_CORRELATION_LIMIT: f64 = 0.98;

/// A feature column that is (nearly) the label itself (§12.5, M10's most common
/// training example).
///
/// This is the leak that ships most often and survives review most easily,
/// because the offending column usually has an innocent name. It is not caught
/// by the random-label test — permuting the labels breaks the very relation this
/// leak consists of — so it needs its own screen.
#[must_use]
pub fn target_correlation(frame: &TrainingFrame) -> Report {
    let mut report = Report::default();
    if frame.row_count() < 8 {
        return report;
    }
    for (name, col) in frame.feature_names.iter().zip(&frame.columns) {
        let r = pearson(col, &frame.label).abs();
        if r > TARGET_CORRELATION_LIMIT {
            report.push(Finding {
                check: Check::TargetCorrelation,
                severity: Severity::Blocking,
                subject: name.clone(),
                statistic: r,
                detail: format!(
                    "|corr(feature, label)| = {r:.4} at the same row (limit {TARGET_CORRELATION_LIMIT}): \
                     this column is the answer, not a predictor of it"
                ),
            });
        }
    }
    report
}

/// A column standardized over the whole sample leaks the future distribution
/// into every row, including the rows a model is supposed to be tested on.
///
/// The signature is exact rather than statistical: full-sample standardization
/// leaves mean 0 and sd 1 *to machine precision* over the entire frame, which a
/// rolling or train-block normalization never does.
#[must_use]
pub fn full_sample_normalization(frame: &TrainingFrame) -> Report {
    const EPS: f64 = 1e-9;
    let mut report = Report::default();
    if frame.row_count() < 32 {
        return report;
    }
    for (name, col) in frame.feature_names.iter().zip(&frame.columns) {
        let n = col.len() as f64;
        let mean = col.iter().sum::<f64>() / n;
        let sd = (col.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
        if mean.abs() < EPS && (sd - 1.0).abs() < EPS {
            report.push(Finding {
                check: Check::FullSampleNormalization,
                severity: Severity::Blocking,
                subject: name.clone(),
                statistic: sd,
                detail: format!(
                    "column has mean {mean:.2e} and sd {sd:.12} over the whole frame: it was \
                     standardized before the split, so every training row knows the test set's \
                     distribution"
                ),
            });
        }
    }
    report
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }
    let (x, y) = (&x[..n], &y[..n]);
    let nf = n as f64;
    let mx = x.iter().sum::<f64>() / nf;
    let my = y.iter().sum::<f64>() / nf;
    let cov: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum::<f64>().sqrt();
    let sy: f64 = y.iter().map(|b| (b - my).powi(2)).sum::<f64>().sqrt();
    if sx * sy < 1e-12 { 0.0 } else { cov / (sx * sy) }
}

// ===========================================================================
// 1.9 — snapshot reproducibility
// ===========================================================================

/// Re-running on an older snapshot must reproduce it byte for byte (§12.5·3).
///
/// The comparison is between the digest recorded when a `dataset_id` was first
/// materialized and the digest of rebuilding it now. A difference means the
/// underlying data changed *under a fixed spec* — a retroactive price
/// adjustment, a revised calendar, a vendor restatement — which is precisely the
/// failure `dataset_id` claims cannot happen (INV-12).
#[must_use]
pub fn snapshot_reproducibility(dataset_id: &str, recorded: &str, rebuilt: &str) -> Report {
    let mut report = Report::default();
    if recorded != rebuilt {
        report.push(Finding {
            check: Check::SnapshotReproducibility,
            severity: Severity::Blocking,
            subject: dataset_id.to_string(),
            statistic: 1.0,
            detail: format!(
                "rebuilding {dataset_id} produced {rebuilt} but it was recorded as {recorded}: \
                 the data under a fixed spec changed"
            ),
        });
    }
    report
}

// ===========================================================================
// 1.10 — CV − WF Sharpe gap
// ===========================================================================

/// The gap above which cross-validated performance stops being credible.
pub const CV_WF_GAP_LIMIT: f64 = 1.0;

/// `CV_Sharpe − WF_Sharpe > 1.0` ⇒ suspect overlapping-label leakage (§12.5).
///
/// A flag, not a block, exactly as the spec has it: the gap has innocent causes
/// (a regime change late in the sample) and guilty ones (labels whose horizons
/// overlap across the CV split), and only a human can tell them apart. What is
/// not optional is that the trial carries the mark.
#[must_use]
pub fn cv_wf_gap(subject: &str, cv_sharpe: f64, wf_sharpe: f64) -> Report {
    let mut report = Report::default();
    let gap = cv_sharpe - wf_sharpe;
    if gap.is_finite() && gap > CV_WF_GAP_LIMIT {
        report.push(Finding {
            check: Check::CvWfGap,
            severity: Severity::Flag,
            subject: subject.to_string(),
            statistic: gap,
            detail: format!(
                "cross-validated Sharpe exceeds walk-forward by {gap:.2} (limit {CV_WF_GAP_LIMIT}): \
                 suspect overlapping-label leakage"
            ),
        });
    }
    report
}

// ===========================================================================
// 1.11 — synthetic leak injection (M10's labelled-data generator)
// ===========================================================================

/// Deliberate leaks, planted in a known-clean frame.
///
/// Each one is a leak that has actually shipped in real research code, which is
/// why it is here rather than an abstract perturbation. They serve two purposes:
/// they are the only way to show the detector above catches anything, and they
/// are the unlimited labelled-data source M10 trains on (§13).
pub mod inject {
    use super::TrainingFrame;

    /// Every leak this module can plant, and the check that is supposed to catch
    /// it. The name is the label M10 learns.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Leak {
        /// A feature column *is* the label. Caught by
        /// [`super::target_correlation`].
        TargetInFeature,
        /// A feature column is standardized over the whole frame, so every
        /// training row knows the test set's distribution. Caught by
        /// [`super::full_sample_normalization`].
        FullSampleNormalization,
        /// The label repeats across blocks, so rows sharing an outcome land on
        /// both sides of a split unless the split purges. **No static check here
        /// catches this** -- it is only visible as a gap between cross-validated
        /// and walk-forward performance, which is exactly why
        /// [`super::cv_wf_gap`] exists as a flag. It is planted anyway, because
        /// M10 needs labelled examples of it.
        OverlappingLabelBlocks,
        /// The harness itself is broken: the test rows are inside the training
        /// rows. Caught by [`super::random_label`], which is the only check that
        /// can see a harness defect at all.
        TrainTestOverlap,
    }

    impl Leak {
        #[must_use]
        pub fn label(self) -> &'static str {
            match self {
                Self::TargetInFeature => "target_in_feature",
                Self::FullSampleNormalization => "full_sample_normalization",
                Self::OverlappingLabelBlocks => "overlapping_label_blocks",
                Self::TrainTestOverlap => "train_test_overlap",
            }
        }

        #[must_use]
        pub fn all() -> [Self; 4] {
            [
                Self::TargetInFeature,
                Self::FullSampleNormalization,
                Self::OverlappingLabelBlocks,
                Self::TrainTestOverlap,
            ]
        }

        /// Whether planting this leak changes the frame. `TrainTestOverlap` is a
        /// property of the fold geometry, not of the data, so it does not.
        #[must_use]
        pub fn is_in_the_frame(self) -> bool {
            self != Self::TrainTestOverlap
        }
    }

    /// Plant `leak` in a clean frame, returning the leaky copy. A leak that
    /// lives in the fold geometry rather than the data returns the frame
    /// unchanged -- plant it with [`overlapping_folds`] instead.
    #[must_use]
    pub fn plant(frame: &TrainingFrame, leak: Leak) -> TrainingFrame {
        let mut out = frame.clone();
        let n = out.row_count();
        if n == 0 || out.columns.is_empty() {
            return out;
        }
        match leak {
            Leak::TargetInFeature => {
                out.columns[0] = out.label.clone();
                out.feature_names[0] = "leaked_target".into();
            }
            Leak::FullSampleNormalization => {
                let col = &mut out.columns[0];
                let nf = n as f64;
                let mean = col.iter().sum::<f64>() / nf;
                let sd = (col.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nf).sqrt();
                if sd > 0.0 {
                    for x in col.iter_mut() {
                        *x = (*x - mean) / sd;
                    }
                }
            }
            Leak::OverlappingLabelBlocks => {
                // Every block of 20 rows shares one outcome, so a split that
                // does not purge puts the same answer on both sides.
                let block = 20;
                for start in (0..n).step_by(block) {
                    let end = (start + block).min(n);
                    let v = out.label[start];
                    for l in &mut out.label[start..end] {
                        *l = v;
                    }
                }
            }
            Leak::TrainTestOverlap => {}
        }
        out
    }

    /// The `TrainTestOverlap` leak: folds whose test rows are also training
    /// rows. This is what a harness defect looks like, and the random-label test
    /// is the only check in the suite that can see one.
    #[must_use]
    pub fn overlapping_folds(n_rows: usize, folds: usize) -> Vec<crate::Fold> {
        (0..folds)
            .map(|index| {
                let test_start = n_rows * (folds - 1) / folds;
                crate::Fold {
                    index: u32::try_from(index).unwrap_or(0),
                    train: 0..n_rows,
                    cal: 0..1,
                    test: test_start..n_rows,
                }
            })
            .collect()
    }
}

/// Run every check that can be answered from a frame alone, and merge the
/// reports. The callers that also hold bars or digests add those checks.
#[must_use]
pub fn suite(
    features: &[String],
    rows: &[FeatureRow],
    frame: &TrainingFrame,
    spec: &domain::model_def::cv::WalkForwardSpec,
    pipeline: &dataplane::split::EmbargoInputs,
    seed: u64,
) -> Report {
    let mut report = causal_access(features, rows);
    report.merge(target_correlation(frame));
    report.merge(full_sample_normalization(frame));
    report.merge(random_label(frame, spec, pipeline, 8, seed).report);
    report
}

/// `random_label` against an explicit fold set, for callers that need to test the
/// geometry itself rather than take it from a spec.
#[must_use]
pub fn random_label_over_folds(
    frame: &TrainingFrame,
    folds: &[crate::Fold],
    permutations: usize,
    seed: u64,
) -> RandomLabelResult {
    let mut rng = Lcg::new(seed);
    let (real_sharpe, real_t) = oos_stats(frame, &frame.label, folds);
    let mut permuted_sharpes = Vec::with_capacity(permutations);
    let mut permuted_t_stats = Vec::with_capacity(permutations);
    for _ in 0..permutations {
        let shuffled = shuffle(&frame.label, &mut rng);
        let (s, t) = oos_stats(frame, &shuffled, folds);
        permuted_sharpes.push(s);
        permuted_t_stats.push(t);
    }
    let finite: Vec<f64> = permuted_sharpes.iter().copied().filter(|s| s.is_finite()).collect();
    let permuted_mean = if finite.is_empty() {
        f64::NAN
    } else {
        finite.iter().sum::<f64>() / finite.len() as f64
    };
    let permuted_max_abs_t = permuted_t_stats
        .iter()
        .filter(|t| t.is_finite())
        .fold(0.0_f64, |m, t| m.max(t.abs()));
    let mut report = Report::default();
    if permuted_max_abs_t > RANDOM_LABEL_T_LIMIT {
        report.push(Finding {
            check: Check::RandomLabel,
            severity: Severity::Blocking,
            subject: "harness".into(),
            statistic: permuted_max_abs_t,
            detail: format!(
                "a permuted label earned an out-of-sample mean return with t = {permuted_max_abs_t:.2} \
                 (limit {RANDOM_LABEL_T_LIMIT}); the harness is leaking the answer"
            ),
        });
    }
    RandomLabelResult {
        real_sharpe,
        real_t,
        permuted_sharpes,
        permuted_t_stats,
        permuted_mean,
        permuted_max_abs_t,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::inject::Leak;
    use super::*;
    use dataplane::split::EmbargoInputs;
    use domain::model_def::cv::{WalkForwardSpec, WindowMode};

    fn rows(n: usize) -> Vec<FeatureRow> {
        (0..n)
            .map(|i| {
                let c = 100.0 + ((i * 7919) % 97) as f64 * 0.37;
                FeatureRow {
                    ts_ns: i as i64 * 60_000_000_000,
                    open: c * 0.999,
                    high: c * 1.004,
                    low: c * 0.995,
                    close: c,
                    volume: 10.0 + ((i * 31) % 17) as f64,
                }
            })
            .collect()
    }

    fn spec() -> WalkForwardSpec {
        WalkForwardSpec {
            mode: WindowMode::Expanding,
            folds: 3,
            train_bars: 300,
            cal_bars: 40,
            test_bars: 80,
            purge_bars: 2,
            embargo_bars: 2,
        }
    }

    fn pipeline() -> EmbargoInputs {
        EmbargoInputs {
            horizon_bars: 1,
            max_lookback_bars: 0,
            max_knowledge_lag_ms: 0,
            settlement_lag_bars: 0,
        }
    }

    fn clean_frame() -> TrainingFrame {
        let bars: Vec<crate::align::BarObs> = (0..2_000)
            .map(|i| {
                // Deterministic pseudo-noise with no exploitable structure: a
                // full-period LCG, not `i mod k`, which would be a sawtooth a
                // linear model can read straight off.
                let mut r = Lcg::new(i as u64 + 1);
                let x = (r.next_u64() % 10_007) as f64 / 10_007.0 - 0.5;
                let c = 100.0 * (1.0 + x * 0.01);
                let ts = i as i64 * 60_000_000_000;
                crate::align::BarObs {
                    ts_ns: ts,
                    knowledge_ns: ts,
                    open: c,
                    high: c * 1.001,
                    low: c * 0.999,
                    close: c,
                    volume: 1.0,
                    quality: dataplane::quality::QualityFlags::NONE,
                }
            })
            .collect();
        crate::build_aligned_training_frame(
            &bars,
            &["close".to_string(), "ema_7".to_string()],
            1,
            60_000_000_000,
        )
    }

    // ------------------------------------------------------------------ //
    // 1.7 causal access
    // ------------------------------------------------------------------ //

    /// Every catalogued feature passes the guard. If this ever fails, one of
    /// them started reading outside its declaration.
    #[test]
    fn the_shipped_feature_set_passes_the_causal_guard() {
        let names: Vec<String> = crate::feature_sets::resolve("fs_core_ohlcv_v3")
            .expect("feature set")
            .features
            .clone();
        let r = causal_access(&names, &rows(400));
        assert!(r.is_clean(), "{:?}", r.findings);
    }

    #[test]
    fn an_unresolvable_feature_is_a_blocking_finding() {
        let r = causal_access(&["not_a_feature".to_string()], &rows(100));
        assert!(r.has_blocking());
    }

    #[test]
    fn causal_access_on_a_short_series_reports_nothing_rather_than_panicking() {
        assert!(causal_access(&["close".to_string()], &rows(2)).is_clean());
    }

    // ------------------------------------------------------------------ //
    // 1.8 random label
    // ------------------------------------------------------------------ //

    /// The real test: on a clean harness, permuted labels earn nothing.
    #[test]
    fn permuted_labels_earn_no_edge_on_a_clean_harness() {
        let frame = clean_frame();
        let r = random_label(&frame, &spec(), &pipeline(), 8, 7);
        assert!(!r.permuted_sharpes.is_empty(), "the test actually ran");
        assert!(
            !r.report.has_blocking(),
            "clean harness flagged: max |t| {} — {:?}",
            r.permuted_max_abs_t,
            r.report.findings
        );
    }

    /// And the converse, which is what makes the test above mean anything: a
    /// harness whose test rows are inside its training rows is caught. This is
    /// the defect class the random-label test exists for -- a broken split makes
    /// even a meaningless label look predictable.
    #[test]
    fn a_harness_that_trains_on_its_test_rows_is_caught() {
        let frame = clean_frame();
        let folds = inject::overlapping_folds(frame.row_count(), 2);
        let r = random_label_over_folds(&frame, &folds, 8, 7);
        assert!(
            r.report.has_blocking(),
            "train/test overlap must be caught; max |t| {}",
            r.permuted_max_abs_t
        );
    }

    #[test]
    fn the_random_label_test_says_so_when_it_could_not_run() {
        let frame = clean_frame();
        let too_big = WalkForwardSpec {
            train_bars: 1_000_000,
            ..spec()
        };
        let r = random_label(&frame, &too_big, &pipeline(), 4, 7);
        assert!(!r.report.is_clean());
        assert!(!r.report.has_blocking(), "a test that did not run is not a failure");
    }

    /// Determinism: a leakage verdict that depends on the day it ran is not a
    /// verdict.
    #[test]
    fn the_random_label_test_is_deterministic() {
        let frame = clean_frame();
        let a = random_label(&frame, &spec(), &pipeline(), 4, 11);
        let b = random_label(&frame, &spec(), &pipeline(), 4, 11);
        assert_eq!(a.permuted_sharpes, b.permuted_sharpes);
        assert_eq!(a.permuted_t_stats, b.permuted_t_stats);
    }

    // ------------------------------------------------------------------ //
    // frame-level screens
    // ------------------------------------------------------------------ //

    #[test]
    fn overlapping_folds_are_caught_structurally() {
        let clean = crate::walk_forward_folds(2_000, &spec(), &pipeline()).expect("folds");
        assert!(fold_geometry(&clean).is_clean(), "production geometry is sound");
        let broken = inject::overlapping_folds(2_000, 2);
        let r = fold_geometry(&broken);
        assert!(r.has_blocking());
    }

    #[test]
    fn a_clean_frame_passes_both_frame_screens() {
        let f = clean_frame();
        assert!(target_correlation(&f).is_clean(), "{:?}", target_correlation(&f).findings);
        assert!(full_sample_normalization(&f).is_clean());
    }

    #[test]
    fn a_column_that_is_the_label_is_caught() {
        let f = inject::plant(&clean_frame(), Leak::TargetInFeature);
        let r = target_correlation(&f);
        assert!(r.has_blocking());
        assert!(r.findings[0].statistic > 0.99);
    }

    #[test]
    fn a_column_standardized_before_the_split_is_caught() {
        let f = inject::plant(&clean_frame(), Leak::FullSampleNormalization);
        let r = full_sample_normalization(&f);
        assert!(r.has_blocking(), "{:?}", r.findings);
        assert_eq!(r.findings[0].check, Check::FullSampleNormalization);
    }

    /// The screens are cheap to fool into crying wolf, so both refuse to run on
    /// a sample too small to mean anything.
    #[test]
    fn the_frame_screens_stay_quiet_on_a_sample_too_small_to_judge() {
        let mut tiny = clean_frame();
        tiny.ts_ns.truncate(4);
        tiny.label.truncate(4);
        for c in &mut tiny.columns {
            c.truncate(4);
        }
        assert!(target_correlation(&tiny).is_clean());
        assert!(full_sample_normalization(&tiny).is_clean());
    }

    // ------------------------------------------------------------------ //
    // 1.9 / 1.10
    // ------------------------------------------------------------------ //

    #[test]
    fn an_identical_rebuild_is_reproducible_and_a_changed_one_is_not() {
        assert!(snapshot_reproducibility("ds", "sha256:a", "sha256:a").is_clean());
        let r = snapshot_reproducibility("ds", "sha256:a", "sha256:b");
        assert!(r.has_blocking());
        assert_eq!(r.findings[0].check, Check::SnapshotReproducibility);
    }

    #[test]
    fn the_cv_wf_gap_flags_but_does_not_block() {
        assert!(cv_wf_gap("run", 1.4, 1.0).is_clean(), "0.4 is under the limit");
        let r = cv_wf_gap("run", 2.5, 1.0);
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].severity, Severity::Flag);
        assert!(!r.has_blocking());
        assert!((r.findings[0].statistic - 1.5).abs() < 1e-12);
    }

    #[test]
    fn a_non_finite_gap_is_not_a_flag() {
        assert!(cv_wf_gap("run", f64::NAN, 1.0).is_clean());
        assert!(cv_wf_gap("run", 2.0, f64::NAN).is_clean());
    }

    // ------------------------------------------------------------------ //
    // 1.11 injection apparatus
    // ------------------------------------------------------------------ //

    #[test]
    fn every_in_frame_leak_kind_actually_changes_the_frame() {
        let clean = clean_frame();
        for leak in Leak::all().into_iter().filter(|l| l.is_in_the_frame()) {
            let leaky = inject::plant(&clean, leak);
            assert_ne!(
                (leaky.columns.clone(), leaky.label.clone()),
                (clean.columns.clone(), clean.label.clone()),
                "{} planted nothing",
                leak.label()
            );
        }
    }

    /// A geometry leak is not a data leak: planting it must leave the frame
    /// alone, or the two kinds would be indistinguishable to M10.
    #[test]
    fn a_geometry_leak_leaves_the_frame_untouched() {
        let clean = clean_frame();
        assert_eq!(inject::plant(&clean, Leak::TrainTestOverlap), clean);
        let folds = inject::overlapping_folds(clean.row_count(), 2);
        assert!(folds.iter().all(|f| f.train.contains(&f.test.start)));
    }

    /// Every leak has a named detector, and each one catches its own. The
    /// overlapping-label leak deliberately has none: it is only visible as a
    /// CV/WF gap, which is why that flag exists.
    #[test]
    fn each_leak_is_caught_by_the_check_that_claims_it() {
        let clean = clean_frame();
        assert!(target_correlation(&inject::plant(&clean, Leak::TargetInFeature)).has_blocking());
        assert!(
            full_sample_normalization(&inject::plant(&clean, Leak::FullSampleNormalization))
                .has_blocking()
        );
        let folds = inject::overlapping_folds(clean.row_count(), 2);
        assert!(random_label_over_folds(&clean, &folds, 8, 7).report.has_blocking());
        // And the one nothing here catches, stated rather than hidden:
        let overlapped = inject::plant(&clean, Leak::OverlappingLabelBlocks);
        assert!(
            suite(
                &["close".to_string(), "ema_7".to_string()],
                &rows(400),
                &overlapped,
                &spec(),
                &pipeline(),
                3
            )
            .is_clean(),
            "if this starts failing, a static check now catches overlapping labels \
             and the CV/WF flag is no longer the only signal"
        );
    }

    #[test]
    fn planting_into_an_empty_frame_is_a_no_op() {
        let empty = TrainingFrame::default();
        for leak in Leak::all() {
            assert_eq!(inject::plant(&empty, leak), empty);
        }
    }

    #[test]
    fn the_suite_runs_every_frame_level_check() {
        let names = vec!["close".to_string(), "ema_7".to_string()];
        let r = suite(&names, &rows(400), &clean_frame(), &spec(), &pipeline(), 3);
        assert!(!r.has_blocking(), "{:?}", r.findings);
    }
}
