//! Forward-test evidence: what a deployed strategy actually did, bar by bar,
//! beside what the backtest said it would do.
//!
//! This is Gate 15's only input (SPEC §12.3, §12.6). Every other gate asks
//! whether the evidence is internally sound; Gate 15 asks whether the simulator
//! is telling the truth, and it is the one gate that cannot be satisfied by
//! computing harder. Its evidence is a calendar.
//!
//! Two rules are structural here rather than conventional, because both failure
//! modes are silent:
//!
//! 1. **Only [`ObservationSource::Observed`] rows are forward evidence** (D-T1,
//!    ADR-P2-24). A row reconstructed after the fact is not a forward
//!    observation however recent its timestamp — the whole content of the claim
//!    is that nobody knew the answer when the order was sent.
//! 2. **A series carries its [`Numeraire`]**. Two return series are only
//!    element-wise comparable when they are counted in the same unit, and a
//!    divergence figure computed across units is worse than no figure at all.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::numeraire::Numeraire;

/// Where an observation's knowledge came from.
///
/// Stored as a column rather than inferred from timestamps, because the
/// distinction is about *when it was knowable*, and a row's `observed_at` can
/// be recent for either reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    /// Recorded from a fill as it happened, forward, in observed time. The only
    /// kind that counts as Gate 15 evidence.
    Observed,
    /// Reconstructed after the fact — a gap repaired, a replay, an import. Kept
    /// because it is useful for operations, and excluded from the gate because
    /// it is not evidence about the future.
    Backfill,
}

impl ObservationSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Backfill => "backfill",
        }
    }

    /// Whether a row from this source may enter a Gate 15 count.
    ///
    /// The single place the rule is expressed. Callers ask this rather than
    /// matching on the variant, so a third source added later cannot default
    /// into the count by omission.
    #[must_use]
    pub fn is_forward_evidence(self) -> bool {
        matches!(self, Self::Observed)
    }
}

impl std::str::FromStr for ObservationSource {
    type Err = UnknownSource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "observed" => Ok(Self::Observed),
            "backfill" => Ok(Self::Backfill),
            other => Err(UnknownSource(other.to_owned())),
        }
    }
}

/// An unrecognised observation source.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown observation source {0:?}")]
pub struct UnknownSource(pub String);

/// One bar of forward record for one deployed strategy.
///
/// `bar` is the alignment key: the realised series and the backtest series are
/// joined on it, and a realised row with no backtest counterpart is a finding
/// (the live system traded when the tested one would not have), not a row to
/// drop quietly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardObservation {
    pub deployment_id: Uuid,
    /// Denormalised so an observation is legible without a join.
    pub strategy_id: String,
    /// Bar close this observation belongs to — the join key against the
    /// backtest series.
    pub bar: DateTime<Utc>,
    /// What the deployed strategy actually returned over this bar.
    pub realised_return: f64,
    /// What the stored backtest series says it should have returned.
    pub expected_return: f64,
    /// The unit both of the above are counted in.
    pub numeraire: Numeraire,
    pub source: ObservationSource,
    /// Whether the live signal reproduced the backtest's signal for this bar
    /// (§12.6's first and most valuable process check).
    pub signal_matched: bool,
    /// Realised execution slippage, in basis points against the decision price.
    pub realised_slippage_bps: f64,
    /// What the simulator modelled for the same trade, in basis points.
    pub modelled_slippage_bps: f64,
    /// Notional turned over this bar, realised and expected, in `numeraire`.
    pub realised_turnover: f64,
    pub expected_turnover: f64,
    /// Broker rejects over this bar attributable to the model's own output —
    /// not to connectivity, not to venue maintenance.
    pub model_attributable_rejects: u32,
    /// Wall-clock time the row was written.
    pub observed_at: DateTime<Utc>,
}

impl ForwardObservation {
    /// The realised-minus-expected return for this bar.
    #[must_use]
    pub fn return_divergence(&self) -> f64 {
        self.realised_return - self.expected_return
    }

    /// Whether this row may enter a Gate 15 count.
    #[must_use]
    pub fn is_forward_evidence(&self) -> bool {
        self.source.is_forward_evidence()
    }
}

/// The span of time a Gate 15 verdict was reached over.
///
/// Part of the verdict, never a footnote. A gate passed on four observations
/// and a gate passed on four hundred are different claims, and a stack that
/// renders them identically is lying by omission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationWindow {
    /// First observed bar in the window.
    pub first_bar: DateTime<Utc>,
    /// Last observed bar in the window.
    pub last_bar: DateTime<Utc>,
}

impl ObservationWindow {
    /// Calendar days spanned, inclusive of both ends. A single day of
    /// observations is one day, not zero.
    #[must_use]
    pub fn days(&self) -> u32 {
        let span = (self.last_bar.date_naive() - self.first_bar.date_naive()).num_days();
        u32::try_from(span.max(0)).unwrap_or(u32::MAX).saturating_add(1)
    }
}

/// Gate 15's four §12.6 measurements, plus the count and window they were
/// measured over.
///
/// Built only from observed-knowledge rows. The filter lives here, in one
/// function, rather than at each call site — a rule enforced at three call
/// sites is a rule with three chances to be forgotten.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gate15Evidence {
    /// Fraction of observed bars whose live signal reproduced the backtest's.
    pub signal_match: f64,
    /// Realised slippage over modelled slippage, turnover-weighted.
    pub slippage_ratio: f64,
    /// Realised turnover over expected turnover.
    pub turnover_ratio: f64,
    pub model_attributable_rejects: u32,
    /// Observed-knowledge rows that entered the measurement. Backfilled rows
    /// are not counted here or anywhere else in this struct.
    pub observations: u32,
    /// Distinct calendar days those rows span.
    pub observed_days: u32,
    pub window: ObservationWindow,
    pub numeraire: Numeraire,
}

/// Why no Gate 15 evidence could be formed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("no observed-knowledge forward observations; {backfilled} backfilled rows were excluded (D-T1)")]
    NoObservedRows { backfilled: usize },
    #[error("observations span more than one numeraire ({left} and {right}); they are not one series")]
    MixedNumeraire { left: Numeraire, right: Numeraire },
}

impl Gate15Evidence {
    /// Aggregate forward observations into Gate 15's inputs.
    ///
    /// **Only [`ObservationSource::Observed`] rows enter** (D-T1, ADR-P2-24).
    /// Backfilled rows in the same window are discarded before anything is
    /// computed, including the count — a backfilled row is not evidence about
    /// the future however recent its timestamp.
    ///
    /// # Errors
    /// [`EvidenceError::NoObservedRows`] when the window holds no observed row,
    /// and [`EvidenceError::MixedNumeraire`] when the rows are not all counted
    /// in the same unit.
    pub fn from_observations(rows: &[ForwardObservation]) -> Result<Self, EvidenceError> {
        let backfilled = rows.iter().filter(|r| !r.is_forward_evidence()).count();
        let observed: Vec<&ForwardObservation> =
            rows.iter().filter(|r| r.is_forward_evidence()).collect();

        let Some(first) = observed.first() else {
            return Err(EvidenceError::NoObservedRows { backfilled });
        };
        let numeraire = first.numeraire;
        if let Some(other) = observed.iter().find(|r| r.numeraire != numeraire) {
            return Err(EvidenceError::MixedNumeraire {
                left: numeraire,
                right: other.numeraire,
            });
        }

        let n = observed.len();
        let matched = observed.iter().filter(|r| r.signal_matched).count();
        #[allow(clippy::cast_precision_loss)]
        let signal_match = matched as f64 / n as f64;

        // Turnover-weighted, so one large trade is not averaged away by many
        // tiny ones that happened to fill at the mid.
        let mut realised_slip = 0.0_f64;
        let mut modelled_slip = 0.0_f64;
        let mut realised_turnover = 0.0_f64;
        let mut expected_turnover = 0.0_f64;
        let mut rejects: u32 = 0;
        for r in &observed {
            realised_slip += r.realised_slippage_bps * r.realised_turnover;
            modelled_slip += r.modelled_slippage_bps * r.realised_turnover;
            realised_turnover += r.realised_turnover;
            expected_turnover += r.expected_turnover;
            rejects = rejects.saturating_add(r.model_attributable_rejects);
        }

        // A deployment that has not traded yet has no slippage and no turnover
        // to compare. Reporting 1.0 — "matched the model exactly" — would be a
        // pass manufactured out of an absence, so report 0 turnover honestly
        // and let the gate's tolerance decide.
        let slippage_ratio = if modelled_slip.abs() > f64::EPSILON {
            realised_slip / modelled_slip
        } else {
            0.0
        };
        let turnover_ratio = if expected_turnover.abs() > f64::EPSILON {
            realised_turnover / expected_turnover
        } else {
            0.0
        };

        let mut bars: Vec<DateTime<Utc>> = observed.iter().map(|r| r.bar).collect();
        bars.sort_unstable();
        let window = ObservationWindow {
            first_bar: bars[0],
            last_bar: bars[n - 1],
        };

        let mut days: Vec<_> = bars.iter().map(chrono::DateTime::date_naive).collect();
        days.dedup();

        Ok(Self {
            signal_match,
            slippage_ratio,
            turnover_ratio,
            model_attributable_rejects: rejects,
            observations: u32::try_from(n).unwrap_or(u32::MAX),
            observed_days: u32::try_from(days.len()).unwrap_or(u32::MAX),
            window,
            numeraire,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn row(bar: DateTime<Utc>, source: ObservationSource) -> ForwardObservation {
        ForwardObservation {
            deployment_id: Uuid::nil(),
            strategy_id: "sma-cross-1".into(),
            bar,
            realised_return: 0.001,
            expected_return: 0.001,
            numeraire: Numeraire::Usd,
            source,
            signal_matched: true,
            realised_slippage_bps: 2.0,
            modelled_slippage_bps: 2.0,
            realised_turnover: 1_000.0,
            expected_turnover: 1_000.0,
            model_attributable_rejects: 0,
            observed_at: bar,
        }
    }

    const DAY: i64 = 86_400;

    #[test]
    fn evidence_states_its_count_and_window() {
        let rows: Vec<_> = (0..4)
            .map(|d| row(at(d * DAY), ObservationSource::Observed))
            .collect();
        let e = Gate15Evidence::from_observations(&rows).unwrap();
        assert_eq!(e.observations, 4);
        assert_eq!(e.observed_days, 4);
        assert_eq!(e.window.first_bar, at(0));
        assert_eq!(e.window.last_bar, at(3 * DAY));
        assert_eq!(e.window.days(), 4);
    }

    /// The adversarial case D-T1 exists for: a backfilled row planted inside the
    /// window must not enter the count, however recent it is.
    #[test]
    fn a_backfilled_row_in_the_window_does_not_enter_the_count() {
        let mut rows: Vec<_> = (0..4)
            .map(|d| row(at(d * DAY), ObservationSource::Observed))
            .collect();
        // Planted: same window, same day even, and it disagrees with the
        // others, so if it entered it would move every number.
        let mut planted = row(at(2 * DAY + 3_600), ObservationSource::Backfill);
        planted.signal_matched = false;
        planted.realised_slippage_bps = 500.0;
        planted.realised_turnover = 1_000_000.0;
        planted.model_attributable_rejects = 99;
        rows.push(planted);

        let e = Gate15Evidence::from_observations(&rows).unwrap();
        assert_eq!(e.observations, 4, "the backfilled row must not be counted");
        assert_eq!(e.observed_days, 4);
        assert!(
            (e.signal_match - 1.0).abs() < 1e-12,
            "the backfilled row must not move signal_match"
        );
        assert_eq!(
            e.model_attributable_rejects, 0,
            "the backfilled row must not contribute rejects"
        );
        assert!(
            (e.slippage_ratio - 1.0).abs() < 1e-9,
            "the backfilled row must not move the slippage ratio"
        );
    }

    #[test]
    fn a_window_of_only_backfilled_rows_produces_no_evidence() {
        let rows: Vec<_> = (0..40)
            .map(|d| row(at(d * DAY), ObservationSource::Backfill))
            .collect();
        assert_eq!(
            Gate15Evidence::from_observations(&rows),
            Err(EvidenceError::NoObservedRows { backfilled: 40 })
        );
    }

    #[test]
    fn mixed_numeraires_are_not_one_series() {
        let mut rows = vec![row(at(0), ObservationSource::Observed)];
        let mut usdt = row(at(DAY), ObservationSource::Observed);
        usdt.numeraire = Numeraire::Usdt;
        rows.push(usdt);
        assert!(matches!(
            Gate15Evidence::from_observations(&rows),
            Err(EvidenceError::MixedNumeraire { .. })
        ));
    }

    #[test]
    fn an_untraded_window_does_not_report_a_perfect_slippage_match() {
        // Zero modelled slippage and zero turnover. Reporting 1.0 would be a
        // pass manufactured out of an absence.
        let mut r = row(at(0), ObservationSource::Observed);
        r.realised_slippage_bps = 0.0;
        r.modelled_slippage_bps = 0.0;
        r.realised_turnover = 0.0;
        r.expected_turnover = 0.0;
        let e = Gate15Evidence::from_observations(&[r]).unwrap();
        assert_eq!(e.slippage_ratio, 0.0);
        assert_eq!(e.turnover_ratio, 0.0);
    }

    #[test]
    fn a_single_day_of_observations_is_one_day_not_zero() {
        let rows = vec![
            row(at(3_600), ObservationSource::Observed),
            row(at(7_200), ObservationSource::Observed),
        ];
        let e = Gate15Evidence::from_observations(&rows).unwrap();
        assert_eq!(e.observations, 2);
        assert_eq!(e.observed_days, 1);
        assert_eq!(e.window.days(), 1);
    }

    #[test]
    fn only_observed_rows_are_forward_evidence() {
        assert!(ObservationSource::Observed.is_forward_evidence());
        assert!(!ObservationSource::Backfill.is_forward_evidence());
    }

    #[test]
    fn source_round_trips_through_its_token() {
        for s in [ObservationSource::Observed, ObservationSource::Backfill] {
            assert_eq!(s.as_str().parse::<ObservationSource>().unwrap(), s);
        }
    }

    #[test]
    fn unknown_source_is_rejected_rather_than_defaulted_to_observed() {
        // Defaulting an unparseable source to `Observed` is how a backfilled
        // row would become forward evidence through a typo.
        assert!("Observed".parse::<ObservationSource>().is_err());
        assert!("".parse::<ObservationSource>().is_err());
    }
}
