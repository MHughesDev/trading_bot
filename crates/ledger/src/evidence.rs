//! The shaped evidence gates 3, 4, 10 and 12 judge on (SPEC §12.3, migration 0119).
//!
//! `mlops.trial_statistic` (ADR-P2-31) carries the *scalars* a gate reads — a
//! PBO, a deflated Sharpe, a breakeven multiple. Four gates do not read a scalar.
//! They read a shape:
//!
//! | Gate | Shape | Why a scalar will not do |
//! |---|---|---|
//! | 3 cost sensitivity | a ladder of `(multiple, net metric)` | the breakeven is *interpolated* between the two rungs bracketing zero |
//! | 4 capacity | a curve of `(AUM, net Sharpe)` + a cross-section | capacity-at-half-Sharpe is where a curve crosses, and participation is per instrument |
//! | 10 factor attribution | one return series per declared factor | a regression needs the series, not its summary |
//! | 12 perturbation | the neighbourhood in parameter order + per-instrument P&L | "no cliff" is a statement about consecutive points |
//!
//! ## The rule every type here keeps
//!
//! **An absent part is absent, never a default.** Every field that can be
//! missing is `Option` or an empty `Vec`, and every reader turns that into
//! `GateOutcome::inconclusive` with a stated reason. There is no neutral value,
//! no zero and no "assume it's fine": it is possible to give a gate an *input*
//! without giving it *evidence*, and that converts an honest `NotComputable`
//! into a dishonest pass.
//!
//! ## Numeraire
//!
//! [`InstrumentPnl`] carries the unit its P&L is denominated in, and
//! [`per_instrument_pnl`] refuses to aggregate across two of them. A USDT P&L
//! summed into a USD one produces a concentration share that is arithmetic over
//! two different units, and nothing downstream can see it happened.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::LedgerError;

/// One rung of the cost ladder: the multiple of modelled costs applied, and the
/// net metric earned at it (Gate 3).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostRung {
    pub multiple: f64,
    pub net_metric: f64,
}

/// The header of a capacity ladder: what the curve below is a curve *of*.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapacityBase {
    /// The AUM the headline result was earned at.
    pub base_aum_usd: f64,
    /// Whether the ADV figures beside it come from a single venue. A
    /// single-venue ADV understates the real one, which makes Gate 4
    /// conservative rather than wrong — but the verdict has to say so.
    pub single_venue: bool,
}

/// One rung of the capacity ladder, and the counted trial that produced it.
///
/// `member_trial_id` is not bookkeeping. INV-16 has no exception for "runs we
/// did to satisfy a gate", and a capacity sweep that dodged the ledger would be
/// exactly the kind of uncounted look the platform exists to make impossible.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapacityPoint {
    pub aum_usd: f64,
    pub net_sharpe: f64,
    pub member_trial_id: Uuid,
}

/// One instrument's contribution, read by Gate 12 (concentration) and Gate 4
/// (participation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentPnl {
    pub instrument: String,
    /// The unit `net_pnl` and the notionals are denominated in. Never assumed.
    pub numeraire: String,
    pub net_pnl: f64,
    pub traded_notional: f64,
    pub peak_daily_notional: f64,
    /// 20-day dollar ADV, measured from the run's **own** bars over the run's
    /// own window. `None` when nobody measured it — reading it at gate time
    /// would be today's liquidity against an old backtest, which is a non-PIT
    /// read, so absence is the honest state and Gate 4 says so.
    pub adv_usd: Option<f64>,
    /// Daily return volatility from the same bars, for the impact model.
    pub daily_vol: Option<f64>,
}

/// One point of the neighbourhood sweep (Gate 12's shape).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeighbourhoodPoint {
    pub param: String,
    pub param_value: f64,
    pub metric: f64,
    /// Marked rather than inferred from position: a neighbourhood truncated by a
    /// budget no longer has its centre in the middle.
    pub is_center: bool,
}

/// One declared factor's return series (Gate 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactorSeries {
    pub factor: String,
    pub ts: Vec<DateTime<Utc>>,
    pub returns: Vec<f64>,
}

/// Everything migration 0119 stores for one trial, written in one call and read
/// in one call.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrialEvidence {
    pub cost_ladder: Vec<CostRung>,
    pub capacity_base: Option<CapacityBase>,
    pub capacity_curve: Vec<CapacityPoint>,
    pub instrument_pnl: Vec<InstrumentPnl>,
    pub neighbourhood: Vec<NeighbourhoodPoint>,
    pub factors: Vec<FactorSeries>,
    /// Which job, study or artifact produced this. A number with no provenance
    /// is a number nobody can re-derive, and an un-re-derivable number is not
    /// evidence.
    pub produced_by: String,
}

impl TrialEvidence {
    /// A bundle attributed to one producer.
    #[must_use]
    pub fn produced_by(produced_by: impl Into<String>) -> Self {
        Self {
            produced_by: produced_by.into(),
            ..Self::default()
        }
    }

    /// Whether this bundle carries nothing at all. Writing an empty bundle is a
    /// no-op rather than an error: a funnel that could measure none of these
    /// has not failed, it has nothing to say, and every gate stays inconclusive.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cost_ladder.is_empty()
            && self.capacity_base.is_none()
            && self.capacity_curve.is_empty()
            && self.instrument_pnl.is_empty()
            && self.neighbourhood.is_empty()
            && self.factors.is_empty()
    }

    /// Everything the database's CHECKs would refuse, refused here first so the
    /// in-memory ledger and the Postgres one agree about what is admissible.
    ///
    /// # Errors
    /// A non-finite number, an empty name, a mis-shaped factor series, a
    /// negative notional, more than one declared centre, or a duplicate
    /// instrument.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), LedgerError> {
        let bad = |m: String| Err(LedgerError::Invalid(m));
        if self.produced_by.trim().is_empty() && !self.is_empty() {
            return bad("gate evidence with no `produced_by` cannot be re-derived".into());
        }

        for r in &self.cost_ladder {
            if !(r.multiple.is_finite() && r.multiple > 0.0) {
                return bad(format!(
                    "cost rung multiple {} is not a cost assumption",
                    r.multiple
                ));
            }
            if !r.net_metric.is_finite() {
                return bad(format!(
                    "cost rung net metric {} is not an observation",
                    r.net_metric
                ));
            }
        }
        // One rung is a cost assumption, not a sensitivity. Refusing it here
        // rather than letting Gate 3 call it inconclusive keeps the stored
        // ladder meaning what its name says.
        if self.cost_ladder.len() == 1 {
            return bad(
                "a cost ladder of one rung is a single cost assumption, not a sensitivity".into(),
            );
        }

        if let Some(b) = self.capacity_base {
            if !(b.base_aum_usd.is_finite() && b.base_aum_usd > 0.0) {
                return bad(format!("base AUM {} is not an amount", b.base_aum_usd));
            }
        }
        for p in &self.capacity_curve {
            if !(p.aum_usd.is_finite() && p.aum_usd > 0.0) {
                return bad(format!("capacity rung AUM {} is not an amount", p.aum_usd));
            }
            if !p.net_sharpe.is_finite() {
                return bad(format!(
                    "capacity rung Sharpe {} is not an observation",
                    p.net_sharpe
                ));
            }
        }
        // A curve without the base it is a curve *of* cannot be read: "20% of
        // capacity" needs both halves, and supplying one is how a caller would
        // get a fraction computed against a number nobody stored.
        if !self.capacity_curve.is_empty() && self.capacity_base.is_none() {
            return bad("a capacity curve with no base AUM has nothing to be a fraction of".into());
        }

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for i in &self.instrument_pnl {
            if i.instrument.trim().is_empty() || i.numeraire.trim().is_empty() {
                return bad("per-instrument P&L needs an instrument and a numeraire".into());
            }
            if !seen.insert(i.instrument.as_str()) {
                return bad(format!(
                    "{} appears twice in the per-instrument cross-section; a share computed over \
                     duplicated rows is not a share",
                    i.instrument
                ));
            }
            if !i.net_pnl.is_finite() {
                return bad(format!("{}'s P&L is not an observation", i.instrument));
            }
            for (label, v) in [
                ("traded notional", i.traded_notional),
                ("peak daily notional", i.peak_daily_notional),
            ] {
                if !(v.is_finite() && v >= 0.0) {
                    return bad(format!("{}'s {label} {v} is not a notional", i.instrument));
                }
            }
            if let Some(adv) = i.adv_usd {
                if !(adv.is_finite() && adv > 0.0) {
                    return bad(format!("{}'s ADV {adv} is not a volume", i.instrument));
                }
            }
            if let Some(v) = i.daily_vol {
                if !(v.is_finite() && v >= 0.0) {
                    return bad(format!(
                        "{}'s daily vol {v} is not a volatility",
                        i.instrument
                    ));
                }
            }
        }

        let centres = self.neighbourhood.iter().filter(|p| p.is_center).count();
        if centres > 1 {
            return bad(format!(
                "{centres} neighbourhood points claim to be the centre; a neighbourhood has one"
            ));
        }
        for p in &self.neighbourhood {
            if p.param.trim().is_empty() {
                return bad("a neighbourhood point must name the parameter it varied".into());
            }
            if !(p.param_value.is_finite() && p.metric.is_finite()) {
                return bad(format!(
                    "neighbourhood point ({}, {}) is not an observation",
                    p.param_value, p.metric
                ));
            }
        }

        let mut factors: BTreeSet<&str> = BTreeSet::new();
        for f in &self.factors {
            if f.factor.trim().is_empty() {
                return bad("a factor series must name its factor".into());
            }
            if !factors.insert(f.factor.as_str()) {
                return bad(format!("factor `{}` has two series", f.factor));
            }
            if f.ts.len() != f.returns.len() || f.ts.is_empty() {
                return bad(format!(
                    "factor `{}` has {} timestamps and {} returns",
                    f.factor,
                    f.ts.len(),
                    f.returns.len()
                ));
            }
            if f.returns.iter().any(|r| !r.is_finite()) {
                return bad(format!("factor `{}` carries a non-finite return", f.factor));
            }
            if f.ts.windows(2).any(|w| w[1] <= w[0]) {
                return bad(format!(
                    "factor `{}` is not strictly increasing in time; an unordered series cannot be \
                     aligned to the strategy's",
                    f.factor
                ));
            }
        }
        Ok(())
    }
}

/// Why a cross-section could not be aggregated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AggregationRefused {
    #[error("the per-instrument cross-section is empty")]
    Empty,
    #[error(
        "per-instrument P&L spans {0} numeraires ({1}); summing one into another would make the \
         concentration share arithmetic over two different units"
    )]
    MixedNumeraire(usize, String),
}

/// The `(instrument, P&L)` pairs Gate 12 reads — **only** when every row shares
/// one numeraire.
///
/// This is the whole of the numeraire rule. The alternative, converting at some
/// rate, would need a point-in-time FX series nobody stored and would make the
/// gate's verdict depend on it silently.
///
/// # Errors
/// An empty cross-section, or one spanning more than one numeraire.
pub fn per_instrument_pnl(
    rows: &[InstrumentPnl],
) -> Result<(String, Vec<(String, f64)>), AggregationRefused> {
    if rows.is_empty() {
        return Err(AggregationRefused::Empty);
    }
    let units: BTreeSet<&str> = rows.iter().map(|r| r.numeraire.as_str()).collect();
    if units.len() > 1 {
        let n = units.len();
        let listed = units.into_iter().collect::<Vec<_>>().join(", ");
        return Err(AggregationRefused::MixedNumeraire(n, listed));
    }
    let numeraire = rows[0].numeraire.clone();
    Ok((
        numeraire,
        rows.iter()
            .map(|r| (r.instrument.clone(), r.net_pnl))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pnl(instrument: &str, numeraire: &str, v: f64) -> InstrumentPnl {
        InstrumentPnl {
            instrument: instrument.into(),
            numeraire: numeraire.into(),
            net_pnl: v,
            traded_notional: v.abs() * 10.0,
            peak_daily_notional: v.abs(),
            adv_usd: None,
            daily_vol: None,
        }
    }

    #[test]
    fn one_numeraire_aggregates() {
        let rows = [pnl("BTC-USD", "USD", 100.0), pnl("ETH-USD", "USD", 40.0)];
        let (unit, pairs) = per_instrument_pnl(&rows).expect("one unit");
        assert_eq!(unit, "USD");
        assert_eq!(pairs.len(), 2);
    }

    /// The trap this exists to stop: a USDT P&L and a USD P&L summed into one
    /// denominator, producing a share that is arithmetic over two units.
    #[test]
    fn two_numeraires_are_refused_rather_than_summed() {
        let rows = [pnl("BTC-USD", "USD", 100.0), pnl("BTC-USDT", "USDT", 100.0)];
        assert!(matches!(
            per_instrument_pnl(&rows),
            Err(AggregationRefused::MixedNumeraire(2, _))
        ));
    }

    #[test]
    fn an_empty_cross_section_is_not_a_zero() {
        assert_eq!(per_instrument_pnl(&[]), Err(AggregationRefused::Empty));
    }

    #[test]
    fn a_single_rung_is_not_a_sensitivity() {
        let mut e = TrialEvidence::produced_by("test");
        e.cost_ladder = vec![CostRung {
            multiple: 1.0,
            net_metric: 0.1,
        }];
        assert!(e.validate().is_err());
        e.cost_ladder.push(CostRung {
            multiple: 3.0,
            net_metric: -0.1,
        });
        assert!(e.validate().is_ok());
    }

    #[test]
    fn a_capacity_curve_needs_the_base_it_is_a_fraction_of() {
        let mut e = TrialEvidence::produced_by("test");
        e.capacity_curve = vec![CapacityPoint {
            aum_usd: 1_000_000.0,
            net_sharpe: 1.0,
            member_trial_id: Uuid::nil(),
        }];
        assert!(e.validate().is_err());
        e.capacity_base = Some(CapacityBase {
            base_aum_usd: 1_000_000.0,
            single_venue: true,
        });
        assert!(e.validate().is_ok());
    }

    #[test]
    fn a_factor_series_must_be_aligned_and_ordered() {
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        let mut e = TrialEvidence::produced_by("test");
        e.factors = vec![FactorSeries {
            factor: "cmkt".into(),
            ts: vec![t0, t1],
            returns: vec![0.01],
        }];
        assert!(e.validate().is_err(), "mismatched lengths");
        e.factors[0].returns.push(0.02);
        assert!(e.validate().is_ok());
        e.factors[0].ts = vec![t1, t0];
        assert!(e.validate().is_err(), "out of order");
    }

    #[test]
    fn a_duplicated_instrument_is_refused() {
        let mut e = TrialEvidence::produced_by("test");
        e.instrument_pnl = vec![pnl("BTC-USD", "USD", 1.0), pnl("BTC-USD", "USD", 2.0)];
        assert!(e.validate().is_err());
    }

    #[test]
    fn a_neighbourhood_has_at_most_one_centre() {
        let mut e = TrialEvidence::produced_by("test");
        e.neighbourhood = (0..3)
            .map(|i| NeighbourhoodPoint {
                param: "fast".into(),
                param_value: f64::from(i),
                metric: 0.1,
                is_center: true,
            })
            .collect();
        assert!(e.validate().is_err());
    }
}
