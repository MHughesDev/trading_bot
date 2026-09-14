//! [`Objective`] — what a search optimises, **as data** (FEAT-003 §6).
//!
//! A primary [`MetricKind`] plus hard constraints. A metric set violating any
//! constraint scores `-inf`, so a sampler can never be attracted to a
//! four-trade "100 % win rate" fluke or a strategy that only wins by ignoring
//! drawdown. `aggregate` says how a *distribution* collapses to one number for
//! the sampler's own ranking — never `max` (INV-2).
//!
//! The objective is attached to an Experiment at creation and is immutable for
//! its lifetime; changing it is a new Experiment.

use serde::{Deserialize, Serialize};

use super::metrics::{MetricKind, MetricSet};
use crate::study::Distribution;

/// A hard constraint on a [`MetricSet`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    /// `|max_drawdown| <= value` (value is a positive fraction, e.g. `0.15`).
    MaxDrawdownLte { value: f64 },
    /// `n_trades >= value`. Defaults to 50; never accepted below 1.
    MinTrades { value: u32 },
    /// `turnover <= value` (notional traded, in the run's money unit).
    MaxTurnover { value: f64 },
    /// `exposure_gross >= value` (fraction of time/size in market).
    MinExposure { value: f64 },
    /// `exposure_gross <= value`.
    MaxExposure { value: f64 },
    /// `profit_factor >= value`.
    MinProfitFactor { value: f64 },
}

impl Constraint {
    /// Human-readable violation, or `None` if satisfied.
    #[must_use]
    pub fn violation(&self, m: &MetricSet) -> Option<String> {
        match *self {
            Constraint::MaxDrawdownLte { value } => (m.max_drawdown.abs() > value).then(|| {
                format!(
                    "max drawdown {:.1}% exceeds {:.1}%",
                    m.max_drawdown.abs() * 100.0,
                    value * 100.0
                )
            }),
            Constraint::MinTrades { value } => (m.n_trades < i64::from(value))
                .then(|| format!("{} trades < minimum {value}", m.n_trades)),
            Constraint::MaxTurnover { value } => (m.turnover > value)
                .then(|| format!("turnover {:.0} exceeds {value:.0}", m.turnover)),
            Constraint::MinExposure { value } => (m.exposure_gross < value)
                .then(|| format!("exposure {:.2} below {value:.2}", m.exposure_gross)),
            Constraint::MaxExposure { value } => (m.exposure_gross > value)
                .then(|| format!("exposure {:.2} above {value:.2}", m.exposure_gross)),
            Constraint::MinProfitFactor { value } => (m.profit_factor < value)
                .then(|| format!("profit factor {:.2} below {value:.2}", m.profit_factor)),
        }
    }
}

/// How a sealed distribution collapses to one number **for sampling only**.
/// There is deliberately no `Max` variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    #[default]
    Median,
    Worst5Pct,
}

/// The thing a sweep maximises.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    pub primary: MetricKind,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub aggregate: Aggregate,
}

impl Objective {
    /// A sensible default: maximise `primary` subject to at least 50 trades.
    #[must_use]
    pub fn default_for(primary: MetricKind) -> Self {
        Self {
            primary,
            constraints: vec![Constraint::MinTrades { value: 50 }],
            aggregate: Aggregate::Median,
        }
    }

    /// Reject nonsensical objectives up front (a bad objective fails the
    /// request, not the sweep).
    ///
    /// # Errors
    /// A human-readable reason.
    pub fn validate(&self) -> Result<(), String> {
        for c in &self.constraints {
            match *c {
                Constraint::MinTrades { value: 0 } => {
                    return Err("min_trades must be at least 1".into())
                }
                Constraint::MaxDrawdownLte { value } if !(value > 0.0 && value <= 1.0) => {
                    return Err("max_drawdown_lte must be in (0, 1]".into())
                }
                Constraint::MaxTurnover { value }
                | Constraint::MinExposure { value }
                | Constraint::MaxExposure { value }
                | Constraint::MinProfitFactor { value }
                    if !value.is_finite() || value < 0.0 =>
                {
                    return Err("constraint values must be finite and non-negative".into())
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Every violated constraint, in declaration order.
    #[must_use]
    pub fn violations(&self, m: &MetricSet) -> Vec<String> {
        self.constraints
            .iter()
            .filter_map(|c| c.violation(m))
            .collect()
    }

    /// The scalar a sampler maximises: the primary metric, or `-inf` when any
    /// constraint is violated or the metric is not finite.
    #[must_use]
    pub fn score(&self, m: &MetricSet) -> f64 {
        if self.constraints.iter().any(|c| c.violation(m).is_some()) {
            return f64::NEG_INFINITY;
        }
        let v = m.value(self.primary);
        if v.is_finite() {
            v
        } else {
            f64::NEG_INFINITY
        }
    }

    /// Collapse a sealed distribution for the sampler's own use.
    #[must_use]
    pub fn aggregate_value(&self, d: &Distribution) -> f64 {
        match self.aggregate {
            Aggregate::Median => d.median,
            Aggregate::Worst5Pct => d.worst_5pct,
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn m(n_trades: i64, dd: f64, sortino: f64) -> MetricSet {
        MetricSet {
            n_trades,
            max_drawdown: dd,
            sortino,
            ..MetricSet::empty()
        }
    }

    #[test]
    fn violated_constraint_scores_neg_inf() {
        let o = Objective {
            primary: MetricKind::Sortino,
            constraints: vec![
                Constraint::MinTrades { value: 50 },
                Constraint::MaxDrawdownLte { value: 0.15 },
            ],
            aggregate: Aggregate::Median,
        };
        assert_eq!(o.score(&m(4, -0.05, 9.0)), f64::NEG_INFINITY);
        assert_eq!(o.score(&m(80, -0.30, 9.0)), f64::NEG_INFINITY);
        assert!((o.score(&m(80, -0.10, 1.7)) - 1.7).abs() < 1e-12);
        assert_eq!(o.violations(&m(4, -0.30, 1.0)).len(), 2);
    }

    #[test]
    fn non_finite_primary_is_rejected() {
        let o = Objective::default_for(MetricKind::ProfitFactor);
        let mut ms = m(100, -0.1, 1.0);
        ms.profit_factor = f64::INFINITY;
        assert_eq!(o.score(&ms), f64::NEG_INFINITY);
    }

    #[test]
    fn validate_catches_bad_values() {
        let bad = Objective {
            primary: MetricKind::Calmar,
            constraints: vec![Constraint::MinTrades { value: 0 }],
            aggregate: Aggregate::Median,
        };
        assert!(bad.validate().is_err());
        assert!(Objective::default_for(MetricKind::Calmar)
            .validate()
            .is_ok());
    }

    #[test]
    fn serde_round_trip() {
        let o = Objective::default_for(MetricKind::Expectancy);
        let j = serde_json::to_value(&o).unwrap();
        assert_eq!(j["primary"], "expectancy");
        assert_eq!(j["constraints"][0]["kind"], "min_trades");
        let back: Objective = serde_json::from_value(j).unwrap();
        assert_eq!(back, o);
    }
}
