//! Fleet-level reconciliation (T-2.2): is one strategy wrong, or is the
//! simulator wrong?
//!
//! Phase 0 answers "does this deployment's realised return track its backtest's"
//! one deployment at a time. That question has a much more valuable sibling that
//! no single deployment can answer: when several deployments miss their backtest
//! **in the same direction**, the thing that is wrong is not the strategies. It
//! is the evaluator every one of them was promoted by — and every promotion
//! decision the platform has ever made was made on its output.
//!
//! So this module's whole job is to keep those two readings apart:
//!
//! | reading | what it means | what it is worth |
//! |---|---|---|
//! | [`FleetVerdict::Idiosyncratic`] | *k* deployments missed, in no agreeing direction, while others tracked fine | *k* strategy findings |
//! | [`FleetVerdict::CommonMode`] | *k* deployments missed the same way | **one** finding, against `backtest::sim`, that outranks a promotion (ADR-P6-10) |
//!
//! ## Why this is not a reconciliation that cannot disagree
//!
//! A reconciliation track once refused to wire a position sweep it was asked
//! for, because the only broker in the process was the paper engine: the sweep
//! would have compared a value with itself and logged "reconciliation OK" for a
//! check that cannot fail. That refusal was correct, and the standard it set
//! applies here.
//!
//! This comparison *can* disagree, and the two sides come from different places:
//! the realised side is built from fills the paper engine actually produced, the
//! expected side is a return series the backtest stored before any of them
//! happened. A systematic cost error in the simulator moves one side and not the
//! other. That is what [`FleetReport::assess`]'s planted-error test exercises.
//!
//! ## What it refuses rather than guesses
//!
//! Three of the states below produce no finding, and only one of them means "the
//! fleet is tracking its backtests":
//!
//! * [`FleetInconclusive::NothingMeasurable`] — no deployment has enough
//!   observed bars to have a mean at all. Not a zero.
//! * [`FleetInconclusive::FleetTooSmall`] — deployments *are* diverging, and
//!   there are fewer of them than the quorum that defines "common", so nothing
//!   here can tell "these strategies are bad" from "the simulator is wrong".
//!   Reporting `Idiosyncratic` from a fleet of two would be reporting the
//!   reassuring answer from evidence that does not reach it.
//! * [`FleetVerdict::NoDivergence`] — measured, and nothing breached. This one
//!   is a real negative, and it states the count it was measured over, because a
//!   clean fleet of two and a clean fleet of forty are different claims.
//!
//! **Only observed-knowledge rows enter** (D-T1). A backfilled row is a repair,
//! not forward evidence, and a common-mode finding assembled out of repairs
//! would indict the simulator on the strength of the reconstruction.

use std::collections::BTreeMap;

use domain::forward::ForwardObservation;
use domain::numeraire::{Numeraire, NumeraireMismatch};
use ledger::falsification::{EvaluatorDefect, Reproduction};
use thiserror::Error;
use uuid::Uuid;

/// The component a common-mode finding is filed against.
///
/// The simulator, not the gate stack and not the strategies: a fleet-wide miss
/// in one direction is a statement about the thing that produced every expected
/// series, and it is the only component all of them share.
pub const COMMON_MODE_COMPONENT: &str = "backtest::sim";

/// What the simulator claims, quoted so the defect is a gap between a claim and
/// a behaviour. Without the claim there is only a behaviour.
pub const COMMON_MODE_GUARANTEE: &str = "SPEC §12.3 (Gate 15): a strategy's realised forward return tracks its stored backtest return within the deployment's declared tolerance, because the simulator's modelled cost and fill model are not systematically optimistic.";

/// Which way the fleet missed.
///
/// The sign is the entire signal. Deployments missing in disagreeing directions
/// are deployments whose strategies disagree with their backtests; deployments
/// missing the same way are one platform fact wearing *k* costumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Realised fell short of expected. The dangerous one: every gate verdict,
    /// every promotion and every ranking the platform has produced was computed
    /// on the flattering side of this.
    SimulatorOptimistic,
    /// Realised beat expected. Still a defect — a simulator wrong in the safe
    /// direction is a simulator that rejected strategies it should have passed.
    SimulatorPessimistic,
}

impl Direction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SimulatorOptimistic => "simulator_optimistic",
            Self::SimulatorPessimistic => "simulator_pessimistic",
        }
    }
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The declared yardstick a fleet is judged against.
///
/// There is no `Default`. A tolerance nobody chose is a detector nobody
/// declared, and §12.6's rule about stopping rules — that one chosen after
/// watching the result is not a stopping rule — applies to a common-mode
/// threshold exactly as it applies to a per-strategy one.
///
/// One yardstick for the whole fleet, deliberately, rather than each
/// deployment's own [`crate::forward::DivergenceThresholds`]: "common" has to
/// mean *k* deployments crossed the same line. *k* deployments each crossing a
/// different line is not one fact, and calling it one would let a fleet of
/// loosely-configured deployments manufacture a defect against the simulator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleetPolicy {
    tolerance: f64,
    min_common: usize,
    min_bars: usize,
}

impl FleetPolicy {
    /// Declare the policy.
    ///
    /// # Errors
    /// [`FleetError::UndeclaredTolerance`] for a tolerance that can never fire;
    /// [`FleetError::QuorumTooSmall`] for a quorum of one, which would make
    /// every single diverging strategy a defect against the simulator;
    /// [`FleetError::MinBarsTooSmall`] for a zero-bar minimum, which would make
    /// a deployment with no observations a measurement.
    pub fn declared(
        tolerance: f64,
        min_common: usize,
        min_bars: usize,
    ) -> Result<Self, FleetError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(FleetError::UndeclaredTolerance);
        }
        if min_common < 2 {
            return Err(FleetError::QuorumTooSmall(min_common));
        }
        if min_bars == 0 {
            return Err(FleetError::MinBarsTooSmall);
        }
        Ok(Self {
            tolerance,
            min_common,
            min_bars,
        })
    }

    /// Largest acceptable absolute mean per-bar divergence for one deployment.
    #[must_use]
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    /// How many deployments must miss the same way before it is the simulator.
    #[must_use]
    pub fn min_common(&self) -> usize {
        self.min_common
    }

    /// Observed bars below which a deployment's mean is not a measurement.
    #[must_use]
    pub fn min_bars(&self) -> usize {
        self.min_bars
    }
}

/// Anything that stops a fleet from being one comparison.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum FleetError {
    #[error(
        "the fleet divergence tolerance was not declared, or is not a tolerance that can fire — a detector that never fires is indistinguishable from one that is not wired up"
    )]
    UndeclaredTolerance,
    #[error(
        "a common-mode quorum of {0} is not a quorum: with one deployment every strategy that misses its backtest would be filed as a defect against the simulator, which is the report that stops being read"
    )]
    QuorumTooSmall(usize),
    #[error(
        "a minimum of zero observed bars would make a deployment that has never traded a measurement of the simulator"
    )]
    MinBarsTooSmall,
    #[error(transparent)]
    MixedNumeraire(#[from] NumeraireMismatch),
}

/// One deployment's forward record reduced to what a fleet comparison needs.
///
/// This is also the per-deployment row a divergence dashboard shows, which is
/// why it keeps `backfilled_excluded`: a deployment whose window is mostly
/// repairs looks identical to a quiet one until the count is on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentDivergence {
    pub deployment_id: Uuid,
    pub strategy_id: String,
    pub numeraire: Numeraire,
    /// Observed-knowledge bars that entered the numbers below.
    pub bars: usize,
    /// Rows in the window that were backfilled and therefore excluded (D-T1).
    pub backfilled_excluded: usize,
    /// Mean per-bar `realised − expected`. Signed: the sign is the signal.
    pub mean_divergence: f64,
    /// Compounded realised growth minus compounded expected growth over the
    /// window. A per-bar mean can sit inside tolerance while a small bias in one
    /// direction accumulates here, which is the failure that matters most.
    pub cumulative_divergence: f64,
    /// Turnover-weighted mean of `realised − modelled` slippage, in bps.
    ///
    /// `None` when the window turned over nothing. A window with no turnover has
    /// no slippage measurement, and reporting `0.0` for it would say the
    /// simulator matched reality exactly on the strength of an absence.
    pub slippage_excess_bps: Option<f64>,
}

impl DeploymentDivergence {
    /// Whether this row is a measurement at all under `policy`.
    #[must_use]
    pub fn is_measurable(&self, policy: &FleetPolicy) -> bool {
        self.bars >= policy.min_bars && self.mean_divergence.is_finite()
    }

    /// Which way this deployment missed, or `None` when it is inside tolerance
    /// or is not a measurement.
    #[must_use]
    pub fn direction_under(&self, policy: &FleetPolicy) -> Option<Direction> {
        if !self.is_measurable(policy) || self.mean_divergence.abs() <= policy.tolerance {
            return None;
        }
        Some(if self.mean_divergence < 0.0 {
            Direction::SimulatorOptimistic
        } else {
            Direction::SimulatorPessimistic
        })
    }
}

/// The fleet in one row: what a divergence dashboard shows above the per
/// deployment table.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetSummary {
    /// Deployments with any rows in the window, measurable or not.
    pub deployments: usize,
    /// Of those, the ones with enough observed bars to have a mean.
    pub measurable: usize,
    /// Observed bars summed across the measurable deployments.
    pub bars: usize,
    pub numeraire: Numeraire,
    /// Bar-weighted mean divergence across the measurable deployments. Weighted
    /// by bars rather than averaged over deployments, so a deployment with four
    /// bars does not carry the same weight as one with four hundred.
    pub mean_divergence: f64,
    /// Measurable deployments outside tolerance.
    pub diverging: usize,
}

/// Why a fleet produced no divergence reading.
///
/// Neither variant means "the fleet is tracking its backtests" — see
/// [`FleetVerdict::NoDivergence`] for the one that does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FleetInconclusive {
    /// No deployment has `min_bars` observed bars. An absent measurement, not a
    /// clean one.
    NothingMeasurable { deployments: usize, min_bars: usize },
    /// Deployments are diverging and the fleet is smaller than the quorum, so
    /// nothing here separates "these strategies are bad" from "the simulator is
    /// wrong".
    FleetTooSmall {
        measurable: usize,
        diverging: usize,
        min_common: usize,
    },
}

impl std::fmt::Display for FleetInconclusive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingMeasurable {
                deployments,
                min_bars,
            } => write!(
                f,
                "no measurement: {deployments} deployment(s) in the window, none with the {min_bars} observed bar(s) a mean needs — this is an absent reading, not a fleet that tracked its backtests"
            ),
            Self::FleetTooSmall {
                measurable,
                diverging,
                min_common,
            } => write!(
                f,
                "inconclusive: {diverging} of {measurable} measurable deployment(s) are diverging, under the quorum of {min_common} — a fleet this small cannot tell diverging strategies from a wrong simulator, and calling it idiosyncratic would be the reassuring answer"
            ),
        }
    }
}

/// What the fleet says.
#[derive(Clone, Debug, PartialEq)]
pub enum FleetVerdict {
    /// Nothing could be concluded, with the reason stated.
    NotComputable(FleetInconclusive),
    /// Measured, nothing outside tolerance. States its own count, because a
    /// clean fleet of two and a clean fleet of forty are different claims.
    NoDivergence { measurable: usize, bars: usize },
    /// Deployments are diverging, and not together. *k* strategy findings.
    Idiosyncratic {
        diverging: Vec<Uuid>,
        measurable: usize,
    },
    /// Deployments are diverging the same way. **One** finding, against the
    /// simulator.
    CommonMode(CommonModeFinding),
}

impl FleetVerdict {
    /// The common-mode finding, if this is one.
    #[must_use]
    pub fn common_mode(&self) -> Option<&CommonModeFinding> {
        match self {
            Self::CommonMode(f) => Some(f),
            _ => None,
        }
    }

    /// Whether a reader may treat this as "the fleet tracked its backtests".
    ///
    /// False for both [`Self::NotComputable`] variants, which is the whole point
    /// of them being a separate variant rather than an empty
    /// [`Self::Idiosyncratic`].
    #[must_use]
    pub fn is_measured_negative(&self) -> bool {
        matches!(self, Self::NoDivergence { .. })
    }
}

/// Deployments missing their backtests the same way.
///
/// One value, naming every deployment involved — not *k* values. That is the
/// shape of the claim: the strategies are evidence, the simulator is the
/// finding.
#[derive(Clone, Debug, PartialEq)]
pub struct CommonModeFinding {
    pub direction: Direction,
    /// The aligned deployments, sorted by strategy then deployment id.
    pub deployments: Vec<DeploymentDivergence>,
    /// Measurable deployments in the whole fleet, so the finding carries its own
    /// denominator: 5 of 5 and 5 of 500 are different claims.
    pub fleet_measurable: usize,
    pub tolerance: f64,
    /// Bar-weighted mean divergence across the aligned deployments.
    pub mean_divergence: f64,
    pub numeraire: Numeraire,
}

impl CommonModeFinding {
    /// What the platform claims, for the defect's `claimed_guarantee`.
    #[must_use]
    pub fn claimed_guarantee(&self) -> &'static str {
        COMMON_MODE_GUARANTEE
    }

    /// The reporter's expectation, in words, for a [`Reproduction`].
    #[must_use]
    pub fn expected_statement(&self) -> String {
        format!(
            "each of {} deployments realises within {:.6} per bar of its stored backtest series, and any misses are in no agreeing direction",
            self.deployments.len(),
            self.tolerance
        )
    }

    /// What happened instead.
    #[must_use]
    pub fn observed_statement(&self) -> String {
        format!(
            "{} of {} measurable deployments missed their stored backtest series in the same direction ({}), mean {:.6} per bar against a tolerance of {:.6}: {}",
            self.deployments.len(),
            self.fleet_measurable,
            self.direction,
            self.mean_divergence,
            self.tolerance,
            self.evidence_line()
        )
    }

    /// Per-deployment evidence as one line, so the observation is checkable
    /// without opening the dashboard.
    #[must_use]
    pub fn evidence_line(&self) -> String {
        self.deployments
            .iter()
            .map(|d| {
                format!(
                    "{} ({} bars, mean {:.6}, cumulative {:.6})",
                    d.strategy_id, d.bars, d.mean_divergence, d.cumulative_divergence
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The replay a human runs to confirm this in ten minutes.
    #[must_use]
    pub fn replay_steps(&self) -> Vec<String> {
        let mut steps = vec![
            "Load mlops.forward_observation for each deployment below, source = 'observed' only."
                .to_owned(),
        ];
        for d in &self.deployments {
            steps.push(format!(
                "Re-run the stored backtest for deployment {} (strategy {}) on the same bars and align it against the realised series with RealisedSeries::align.",
                d.deployment_id, d.strategy_id
            ));
        }
        steps.push(format!(
            "Confirm every aligned series misses in the {} direction by more than {:.6} per bar.",
            self.direction, self.tolerance
        ));
        steps
    }

    /// File this as a defect against the simulator.
    ///
    /// `replay` is supplied by the caller and not minted here, and that is not
    /// an ergonomic accident. A [`Reproduction`] asserts *a deterministic replay
    /// in which a stated expectation failed*; a forward divergence is an
    /// observation of live trading, which is the one thing that cannot be
    /// replayed. Minting one here from [`Self::replay_steps`] would put the
    /// platform's most expensive claim behind a reproduction nobody ran, and
    /// `Reproduction` was sealed precisely so that could not happen quietly.
    ///
    /// So the division is: this type supplies the wording and the evidence
    /// ([`Self::expected_statement`], [`Self::observed_statement`],
    /// [`Self::replay_steps`]); the caller, who re-ran the backtests, supplies
    /// the artifact and the seed.
    #[must_use]
    pub fn defect(&self, replay: Reproduction) -> EvaluatorDefect {
        EvaluatorDefect::reported(COMMON_MODE_COMPONENT, self.claimed_guarantee(), replay)
    }
}

/// The whole fleet reading: per deployment, in aggregate, and judged.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetReport {
    /// Every deployment with rows in the window, sorted by strategy then
    /// deployment id. Present whatever the verdict, because the per-deployment
    /// realised-vs-expected table is shown even when nothing can be concluded
    /// from it collectively.
    pub deployments: Vec<DeploymentDivergence>,
    /// `None` when nothing was measurable — an aggregate over zero measurements
    /// is a row of zeroes that reads like a healthy fleet.
    pub aggregate: Option<FleetSummary>,
    pub verdict: FleetVerdict,
}

impl FleetReport {
    /// Judge a window of forward observations across every deployment in it.
    ///
    /// `observations` is the flat result of one query, ungrouped. Grouping
    /// happens here rather than at the call site so there is no way to hand this
    /// a mis-grouped fleet — a deployment's rows split across two groups would
    /// halve its bar count and could push it under `min_bars` without anything
    /// saying so.
    ///
    /// # Errors
    /// [`FleetError::MixedNumeraire`] when the window spans units. Two return
    /// series counted in different numeraires are not one aggregate, and a
    /// fleet-wide mean across units is worse than no mean at all.
    pub fn assess(
        observations: &[ForwardObservation],
        policy: &FleetPolicy,
    ) -> Result<Self, FleetError> {
        let mut grouped: BTreeMap<(String, Uuid), Vec<&ForwardObservation>> = BTreeMap::new();
        let mut unit: Option<&Numeraire> = None;
        for o in observations {
            match unit {
                None => unit = Some(&o.numeraire),
                Some(first) => first.assert_comparable(&o.numeraire)?,
            }
            grouped
                .entry((o.strategy_id.clone(), o.deployment_id))
                .or_default()
                .push(o);
        }

        let deployments: Vec<DeploymentDivergence> =
            grouped.values().map(|rows| measure(rows)).collect();

        let measurable: Vec<&DeploymentDivergence> = deployments
            .iter()
            .filter(|d| d.is_measurable(policy))
            .collect();

        if measurable.is_empty() {
            return Ok(Self {
                verdict: FleetVerdict::NotComputable(FleetInconclusive::NothingMeasurable {
                    deployments: deployments.len(),
                    min_bars: policy.min_bars,
                }),
                aggregate: None,
                deployments,
            });
        }

        let bars: usize = measurable.iter().map(|d| d.bars).sum();
        let numeraire = measurable[0].numeraire.clone();
        let diverging: Vec<&DeploymentDivergence> = measurable
            .iter()
            .copied()
            .filter(|d| d.direction_under(policy).is_some())
            .collect();

        let aggregate = FleetSummary {
            deployments: deployments.len(),
            measurable: measurable.len(),
            bars,
            numeraire: numeraire.clone(),
            mean_divergence: bar_weighted_mean(&measurable),
            diverging: diverging.len(),
        };

        let verdict = if diverging.is_empty() {
            FleetVerdict::NoDivergence {
                measurable: measurable.len(),
                bars,
            }
        } else {
            let aligned = largest_aligned_group(&diverging, policy);
            if aligned.1.len() >= policy.min_common {
                FleetVerdict::CommonMode(CommonModeFinding {
                    direction: aligned.0,
                    mean_divergence: bar_weighted_mean(&aligned.1),
                    deployments: aligned.1.into_iter().cloned().collect(),
                    fleet_measurable: measurable.len(),
                    tolerance: policy.tolerance,
                    numeraire,
                })
            } else if measurable.len() < policy.min_common {
                // Breaches exist and the fleet cannot host a quorum. Saying
                // "idiosyncratic" here would be a conclusion the evidence does
                // not reach.
                FleetVerdict::NotComputable(FleetInconclusive::FleetTooSmall {
                    measurable: measurable.len(),
                    diverging: diverging.len(),
                    min_common: policy.min_common,
                })
            } else {
                FleetVerdict::Idiosyncratic {
                    diverging: diverging.iter().map(|d| d.deployment_id).collect(),
                    measurable: measurable.len(),
                }
            }
        };

        Ok(Self {
            deployments,
            aggregate: Some(aggregate),
            verdict,
        })
    }
}

/// Reduce one deployment's rows, dropping backfilled ones first (D-T1).
fn measure(rows: &[&ForwardObservation]) -> DeploymentDivergence {
    let first = rows[0];
    let observed: Vec<&&ForwardObservation> =
        rows.iter().filter(|o| o.is_forward_evidence()).collect();
    let backfilled_excluded = rows.len() - observed.len();

    let bars = observed.len();
    let mut divergence_sum = 0.0_f64;
    let mut realised_growth = 1.0_f64;
    let mut expected_growth = 1.0_f64;
    let mut slippage_weighted = 0.0_f64;
    let mut weight = 0.0_f64;
    for o in &observed {
        divergence_sum += o.return_divergence();
        realised_growth *= 1.0 + o.realised_return;
        expected_growth *= 1.0 + o.expected_return;
        let w = o.realised_turnover;
        if w > 0.0 {
            slippage_weighted += (o.realised_slippage_bps - o.modelled_slippage_bps) * w;
            weight += w;
        }
    }

    let mean_divergence = if bars == 0 {
        f64::NAN
    } else {
        divergence_sum / usize_as_float(bars)
    };

    DeploymentDivergence {
        deployment_id: first.deployment_id,
        strategy_id: first.strategy_id.clone(),
        numeraire: first.numeraire.clone(),
        bars,
        backfilled_excluded,
        mean_divergence,
        cumulative_divergence: if bars == 0 {
            f64::NAN
        } else {
            realised_growth - expected_growth
        },
        slippage_excess_bps: if weight > 0.0 {
            Some(slippage_weighted / weight)
        } else {
            None
        },
    }
}

/// The diverging deployments that agree on direction, as the larger of the two
/// groups.
///
/// "Larger", not "first": a fleet with three optimistic misses and one
/// pessimistic one is a common-mode finding about the three, and the fourth is
/// its own strategy finding. Picking whichever direction appeared first would
/// make the verdict depend on the query's ordering.
fn largest_aligned_group<'a>(
    diverging: &[&'a DeploymentDivergence],
    policy: &FleetPolicy,
) -> (Direction, Vec<&'a DeploymentDivergence>) {
    let mut optimistic = Vec::new();
    let mut pessimistic = Vec::new();
    for d in diverging {
        match d.direction_under(policy) {
            Some(Direction::SimulatorOptimistic) => optimistic.push(*d),
            Some(Direction::SimulatorPessimistic) => pessimistic.push(*d),
            None => {}
        }
    }
    if pessimistic.len() > optimistic.len() {
        (Direction::SimulatorPessimistic, pessimistic)
    } else {
        (Direction::SimulatorOptimistic, optimistic)
    }
}

fn bar_weighted_mean(rows: &[&DeploymentDivergence]) -> f64 {
    let bars: usize = rows.iter().map(|d| d.bars).sum();
    if bars == 0 {
        return f64::NAN;
    }
    let total: f64 = rows
        .iter()
        .map(|d| d.mean_divergence * usize_as_float(d.bars))
        .sum();
    total / usize_as_float(bars)
}

/// A bar count as a divisor. Isolated so the one lossy cast in this module is in
/// a named place; a fleet would need 2^53 bars for it to lose anything.
#[allow(clippy::cast_precision_loss)]
fn usize_as_float(n: usize) -> f64 {
    n as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use domain::forward::ObservationSource;

    const BAR: i64 = 60;

    fn policy() -> FleetPolicy {
        FleetPolicy::declared(0.001, 3, 4).expect("a declared policy")
    }

    fn at(i: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(i * BAR, 0).unwrap()
    }

    struct Deployment {
        deployment_id: Uuid,
        strategy_id: String,
    }

    fn deployment(name: &str) -> Deployment {
        Deployment {
            deployment_id: Uuid::new_v4(),
            strategy_id: name.to_owned(),
        }
    }

    /// `bars` pairs of (realised, expected) returns, all observed knowledge.
    fn rows(d: &Deployment, pairs: &[(f64, f64)]) -> Vec<ForwardObservation> {
        rows_from(d, pairs, ObservationSource::Observed, Numeraire::Usd)
    }

    fn rows_from(
        d: &Deployment,
        pairs: &[(f64, f64)],
        source: ObservationSource,
        numeraire: Numeraire,
    ) -> Vec<ForwardObservation> {
        pairs
            .iter()
            .enumerate()
            .map(|(i, (realised, expected))| ForwardObservation {
                deployment_id: d.deployment_id,
                strategy_id: d.strategy_id.clone(),
                bar: at(i as i64 + 1),
                realised_return: *realised,
                expected_return: *expected,
                numeraire: numeraire.clone(),
                source,
                signal_matched: true,
                realised_slippage_bps: 12.0,
                modelled_slippage_bps: 4.0,
                realised_turnover: 1_000.0,
                expected_turnover: 1_000.0,
                model_attributable_rejects: 0,
                observed_at: at(i as i64 + 1),
            })
            .collect()
    }

    /// The same expected series for every strategy, each realising it short by
    /// `shortfall` on every bar — the shape of a systematic cost error in the
    /// simulator.
    fn fleet_short_by(shortfall: f64, n: usize) -> Vec<ForwardObservation> {
        let expected = [0.004_f64, -0.002, 0.006, 0.001, -0.003, 0.002];
        let mut out = Vec::new();
        for i in 0..n {
            let d = deployment(&format!("strategy-{i}"));
            let pairs: Vec<(f64, f64)> = expected.iter().map(|e| (e - shortfall, *e)).collect();
            out.extend(rows(&d, &pairs));
        }
        out
    }

    // ── the planted systematic cost error ────────────────────────────────────

    /// T-2.2's acceptance clause, literally: a planted systematic cost error is
    /// detected as common-mode and **not** as k separate strategy failures.
    #[test]
    fn a_planted_systematic_cost_error_is_one_common_mode_finding_not_five_strategy_failures() {
        let observations = fleet_short_by(0.0015, 5);
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");

        let finding = report
            .verdict
            .common_mode()
            .expect("five deployments short of their backtests the same way is the simulator");
        assert_eq!(finding.deployments.len(), 5);
        assert_eq!(finding.fleet_measurable, 5);
        assert_eq!(finding.direction, Direction::SimulatorOptimistic);
        assert!(
            (finding.mean_divergence + 0.0015).abs() < 1e-9,
            "the finding must carry the size of the miss, got {}",
            finding.mean_divergence
        );
        assert!(
            !matches!(report.verdict, FleetVerdict::Idiosyncratic { .. }),
            "five strategy findings is the reading this task exists to prevent"
        );
        // And the observation is checkable without the dashboard.
        assert!(finding.observed_statement().contains("strategy-0"));
        assert_eq!(
            finding.replay_steps().len(),
            7,
            "one per deployment, plus the load and the confirm"
        );
    }

    /// The contrast case. Same tolerance, same quorum, misses in disagreeing
    /// directions: that is the strategies, not the simulator.
    #[test]
    fn misses_in_disagreeing_directions_are_not_a_finding_against_the_simulator() {
        let expected = [0.004_f64, -0.002, 0.006, 0.001, -0.003, 0.002];
        let offsets = [0.0015_f64, -0.0015, 0.0018, -0.0016, 0.0];
        let mut observations = Vec::new();
        for (i, offset) in offsets.iter().enumerate() {
            let d = deployment(&format!("mixed-{i}"));
            let pairs: Vec<(f64, f64)> = expected.iter().map(|e| (e + offset, *e)).collect();
            observations.extend(rows(&d, &pairs));
        }

        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        match &report.verdict {
            FleetVerdict::Idiosyncratic {
                diverging,
                measurable,
            } => {
                assert_eq!(diverging.len(), 4, "four missed, one tracked");
                assert_eq!(*measurable, 5);
            }
            other => panic!("disagreeing directions are not common mode: {other:?}"),
        }
        assert!(report.verdict.common_mode().is_none());
    }

    /// A fleet whose largest agreeing group is under the quorum, inside a fleet
    /// big enough to host one, is idiosyncratic — the other deployments tracked
    /// fine, so the simulator is not the thing that is wrong.
    #[test]
    fn two_agreeing_misses_in_a_fleet_of_five_are_not_the_simulator() {
        let expected = [0.004_f64, -0.002, 0.006, 0.001, -0.003, 0.002];
        let offsets = [-0.0015_f64, -0.0016, 0.0, 0.0, 0.0];
        let mut observations = Vec::new();
        for (i, offset) in offsets.iter().enumerate() {
            let d = deployment(&format!("pair-{i}"));
            let pairs: Vec<(f64, f64)> = expected.iter().map(|e| (e + offset, *e)).collect();
            observations.extend(rows(&d, &pairs));
        }
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        assert!(matches!(
            report.verdict,
            FleetVerdict::Idiosyncratic { measurable: 5, .. }
        ));
    }

    // ── what it refuses ─────────────────────────────────────────────────────

    #[test]
    fn an_empty_fleet_is_not_a_fleet_that_tracked_its_backtests() {
        let report = FleetReport::assess(&[], &policy()).expect("no rows is not a unit clash");
        assert_eq!(
            report.verdict,
            FleetVerdict::NotComputable(FleetInconclusive::NothingMeasurable {
                deployments: 0,
                min_bars: 4,
            })
        );
        assert!(
            !report.verdict.is_measured_negative(),
            "an absent reading must not be readable as a clean one"
        );
        assert!(
            report.aggregate.is_none(),
            "an aggregate over zero measurements is a row of zeroes that reads healthy"
        );
    }

    #[test]
    fn a_fleet_too_small_to_host_a_quorum_cannot_call_its_misses_idiosyncratic() {
        let observations = fleet_short_by(0.0015, 2);
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        assert_eq!(
            report.verdict,
            FleetVerdict::NotComputable(FleetInconclusive::FleetTooSmall {
                measurable: 2,
                diverging: 2,
                min_common: 3,
            })
        );
        assert!(!report.verdict.is_measured_negative());
        // The per-deployment table is still shown: the reading is inconclusive
        // about the fleet, not about the deployments.
        assert_eq!(report.deployments.len(), 2);
        assert!(report.aggregate.is_some());
    }

    /// D-T1: a common-mode finding assembled out of repairs would indict the
    /// simulator on the strength of the reconstruction.
    #[test]
    fn a_fleet_of_backfilled_rows_produces_no_finding() {
        let expected = [0.004_f64, -0.002, 0.006, 0.001, -0.003, 0.002];
        let mut observations = Vec::new();
        for i in 0..5 {
            let d = deployment(&format!("backfilled-{i}"));
            let pairs: Vec<(f64, f64)> = expected.iter().map(|e| (e - 0.0015, *e)).collect();
            observations.extend(rows_from(
                &d,
                &pairs,
                ObservationSource::Backfill,
                Numeraire::Usd,
            ));
        }
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        assert!(matches!(
            report.verdict,
            FleetVerdict::NotComputable(FleetInconclusive::NothingMeasurable {
                deployments: 5,
                ..
            })
        ));
        assert!(
            report.deployments.iter().all(|d| d.backfilled_excluded == 6 && d.bars == 0),
            "the excluded rows must be counted, so a window of repairs is not mistaken for a quiet one"
        );
    }

    #[test]
    fn a_fleet_spanning_two_numeraires_is_not_one_aggregate() {
        let usd = deployment("usd-1");
        let usdt = deployment("usdt-1");
        let pairs = [(0.004_f64, 0.004_f64); 6];
        let mut observations = rows(&usd, &pairs);
        observations.extend(rows_from(
            &usdt,
            &pairs,
            ObservationSource::Observed,
            Numeraire::of("USDT"),
        ));
        assert!(matches!(
            FleetReport::assess(&observations, &policy()),
            Err(FleetError::MixedNumeraire(_))
        ));
    }

    #[test]
    fn a_deployment_with_too_few_bars_is_not_a_measurement_of_the_simulator() {
        let strict = FleetPolicy::declared(0.001, 3, 20).unwrap();
        let observations = fleet_short_by(0.0015, 5);
        let report = FleetReport::assess(&observations, &strict).expect("one numeraire");
        assert!(
            matches!(
                report.verdict,
                FleetVerdict::NotComputable(FleetInconclusive::NothingMeasurable {
                    deployments: 5,
                    min_bars: 20
                })
            ),
            "six bars each is not enough to indict the simulator, however aligned"
        );
    }

    #[test]
    fn a_tracking_fleet_reports_its_count_rather_than_a_bare_ok() {
        let expected = [0.004_f64, -0.002, 0.006, 0.001, -0.003, 0.002];
        let mut observations = Vec::new();
        for i in 0..4 {
            let d = deployment(&format!("clean-{i}"));
            let pairs: Vec<(f64, f64)> = expected.iter().map(|e| (*e, *e)).collect();
            observations.extend(rows(&d, &pairs));
        }
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        assert_eq!(
            report.verdict,
            FleetVerdict::NoDivergence {
                measurable: 4,
                bars: 24
            }
        );
        assert!(report.verdict.is_measured_negative());
        let aggregate = report.aggregate.expect("a measured fleet has an aggregate");
        assert_eq!(aggregate.measurable, 4);
        assert_eq!(aggregate.bars, 24);
        assert_eq!(aggregate.diverging, 0);
    }

    // ── the policy is declared, not defaulted ───────────────────────────────

    #[test]
    fn a_quorum_of_one_is_refused() {
        assert_eq!(
            FleetPolicy::declared(0.001, 1, 4),
            Err(FleetError::QuorumTooSmall(1))
        );
        assert_eq!(
            FleetPolicy::declared(0.001, 0, 4),
            Err(FleetError::QuorumTooSmall(0))
        );
    }

    #[test]
    fn a_tolerance_that_can_never_fire_is_refused() {
        assert_eq!(
            FleetPolicy::declared(0.0, 3, 4),
            Err(FleetError::UndeclaredTolerance)
        );
        assert_eq!(
            FleetPolicy::declared(f64::INFINITY, 3, 4),
            Err(FleetError::UndeclaredTolerance)
        );
        assert_eq!(
            FleetPolicy::declared(f64::NAN, 3, 4),
            Err(FleetError::UndeclaredTolerance)
        );
    }

    #[test]
    fn a_zero_bar_minimum_is_refused() {
        assert_eq!(
            FleetPolicy::declared(0.001, 3, 0),
            Err(FleetError::MinBarsTooSmall)
        );
    }

    // ── the slippage reading does not manufacture a match ───────────────────

    #[test]
    fn an_untraded_window_reports_no_slippage_excess_rather_than_a_perfect_match() {
        let d = deployment("untraded-1");
        let pairs = [(0.0_f64, 0.0_f64); 6];
        let mut observations = rows(&d, &pairs);
        for o in &mut observations {
            o.realised_turnover = 0.0;
            o.realised_slippage_bps = 0.0;
            o.modelled_slippage_bps = 0.0;
        }
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        assert_eq!(
            report.deployments[0].slippage_excess_bps, None,
            "a window with no turnover has no slippage measurement; 0.0 would say the simulator matched reality exactly"
        );
    }

    #[test]
    fn a_traded_window_reports_the_excess_over_the_modelled_figure() {
        let d = deployment("traded-1");
        let pairs = [(0.0_f64, 0.0_f64); 6];
        let report = FleetReport::assess(&rows(&d, &pairs), &policy()).expect("one numeraire");
        let excess = report.deployments[0]
            .slippage_excess_bps
            .expect("turnover was non-zero");
        assert!(
            (excess - 8.0).abs() < 1e-9,
            "12 bps realised against 4 bps modelled is 8 bps of unmodelled cost, got {excess}"
        );
    }

    // ── the finding's worth, and the seal on how it is filed ────────────────

    /// ADR-P6-10: the report has to be worth more than the exploit, or an agent
    /// that found the simulator generous would keep it.
    #[test]
    fn a_verified_common_mode_defect_outranks_a_promotion() {
        let observations = fleet_short_by(0.0015, 5);
        let report = FleetReport::assess(&observations, &policy()).expect("one numeraire");
        let finding = report.verdict.common_mode().expect("common mode");

        let replay = Reproduction::replayed(
            "trial-abc",
            42,
            finding.expected_statement(),
            finding.observed_statement(),
            finding.replay_steps(),
        )
        .expect("expectation and observation differ");
        let defect = finding
            .defect(replay)
            .verified(ledger::falsification::Verification::Verified);

        assert_eq!(defect.component(), COMMON_MODE_COMPONENT);
        assert!(
            defect.score().outranks_promotion(),
            "a simulator defect must be worth more than the promotion it invalidates"
        );
    }

    /// The finding supplies the wording and refuses to supply the replay. A
    /// forward divergence is an observation of live trading, which is the one
    /// thing that cannot be replayed, and `Reproduction` is sealed so a defect
    /// cannot be filed behind a replay nobody ran.
    #[test]
    fn nothing_here_mints_a_reproduction_for_a_replay_nobody_ran() {
        let source = include_str!("fleet.rs");
        let code = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("the production half");
        assert!(
            !code.contains("Reproduction::replayed"),
            "fleet.rs must not construct a Reproduction: the only honest constructor is at the call site that actually re-ran the backtests"
        );
        assert!(
            code.contains("pub fn defect(&self, replay: Reproduction)"),
            "the defect must take the replay rather than inventing one"
        );
    }
}
