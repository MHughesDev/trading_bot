use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Flat,
}

/// Distributional output produced by probabilistic forecasters (ADR-0016).
///
/// Distribution arrays are f64 only — they are not monetary quantities (D-4).
/// `quantiles_sigma` are in the model's σ-unit coordinate system.
/// `quantiles_return` are the same values rescaled by `sigma` to return units.
/// The point view on `Forecast` is a DERIVED view from this distribution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForecastDistribution {
    /// Sorted probability levels, e.g. [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95].
    /// All values in (0, 1), strictly increasing.
    pub quantile_levels: Vec<f64>,
    /// Model outputs in σ-units, sorted ascending (monotone invariant always holds).
    pub quantiles_sigma: Vec<f64>,
    /// Return-unit values: `quantiles_sigma[i] * sigma`.
    pub quantiles_return: Vec<f64>,
    pub median_return: f64,
    /// Realized-vol scale used for σ↔return rescale; not money, f64 intentional (D-4).
    pub sigma: f64,
}

impl ForecastDistribution {
    /// Validate structural invariants. Returns `Err(description)` on first violation.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.quantile_levels.len();
        if n == 0 {
            return Err("quantile_levels must not be empty".into());
        }
        if self.quantiles_sigma.len() != n {
            return Err(format!(
                "quantiles_sigma length {} != quantile_levels length {n}",
                self.quantiles_sigma.len()
            ));
        }
        if self.quantiles_return.len() != n {
            return Err(format!(
                "quantiles_return length {} != quantile_levels length {n}",
                self.quantiles_return.len()
            ));
        }
        for (i, &l) in self.quantile_levels.iter().enumerate() {
            if l <= 0.0 || l >= 1.0 {
                return Err(format!("quantile_levels[{i}]={l} not in (0,1)"));
            }
            if i > 0 && l <= self.quantile_levels[i - 1] {
                return Err(format!(
                    "quantile_levels not strictly increasing at index {i}"
                ));
            }
        }
        for (i, &q) in self.quantiles_sigma.iter().enumerate() {
            if !q.is_finite() {
                return Err(format!("quantiles_sigma[{i}]={q} is not finite"));
            }
            if i > 0 && q < self.quantiles_sigma[i - 1] {
                return Err(format!("quantiles_sigma not monotone at index {i}"));
            }
        }
        if self.sigma <= 0.0 {
            return Err(format!("sigma={} must be > 0", self.sigma));
        }
        Ok(())
    }
}

/// Canonical inference output a strategy runtime consumes (kind-agnostic envelope).
///
/// `magnitude` is always `Decimal` (ADR-0002). `confidence` is 0..1, not money.
///
/// When `distribution` is `Some`, the point fields (`direction`, `magnitude`,
/// `confidence`) are a **derived view** (ADR-0016):
///   - `direction` = sign of `median_return`
///   - `magnitude` = `Decimal(median_return)`
///   - `confidence` = f(interval_width / 2σ)
/// Strategies that pinned the point view are unaffected — they read the same fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Forecast {
    pub model_id: String,
    pub version: u32,
    pub instrument_id: String,
    pub direction: Direction,
    /// ADR-0002: decimal, never f64.
    pub magnitude: Decimal,
    /// 0..1 calibration metric — not money, f64 intentional here.
    pub confidence: f64,
    pub horizon: String,
    pub produced_at: DateTime<Utc>,
    /// Full distributional output (ADR-0016). `None` for point/classification models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<ForecastDistribution>,
}

impl Forecast {
    /// Construct a `Forecast` from a validated distribution, deriving the point fields.
    /// Returns `Err` if the distribution fails its invariant check.
    pub fn from_distribution(
        model_id: String,
        version: u32,
        instrument_id: String,
        horizon: String,
        produced_at: DateTime<Utc>,
        dist: ForecastDistribution,
    ) -> Result<Self, String> {
        dist.validate()?;

        let median = dist.median_return;
        let direction = if median > 1e-8 {
            Direction::Up
        } else if median < -1e-8 {
            Direction::Down
        } else {
            Direction::Flat
        };
        let magnitude = Decimal::try_from(median).unwrap_or(Decimal::ZERO);
        let confidence = Self::interval_confidence(&dist);

        Ok(Forecast {
            model_id,
            version,
            instrument_id,
            direction,
            magnitude,
            confidence,
            horizon,
            produced_at,
            distribution: Some(dist),
        })
    }

    /// Confidence derived from the width of the central interval relative to 2σ.
    /// Narrow interval (peaked distribution) → high confidence.
    fn interval_confidence(dist: &ForecastDistribution) -> f64 {
        let find_closest = |target: f64| -> f64 {
            dist.quantile_levels
                .iter()
                .zip(dist.quantiles_return.iter())
                .min_by(|(a, _), (b, _)| {
                    (*a - target)
                        .abs()
                        .partial_cmp(&(*b - target).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(_, &v)| v)
                .unwrap_or(0.0)
        };

        let q10 = find_closest(0.1);
        let q90 = find_closest(0.9);
        let spread = (q90 - q10).abs();
        let normalizer = 2.0 * dist.sigma;
        if normalizer <= 0.0 {
            return 0.0;
        }
        (1.0 - spread / normalizer).clamp(0.0, 1.0)
    }
}

// ── I-6.2: Derived risk read-outs ────────────────────────────────────────────

/// VaR and ES at a specific confidence level, in return units.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiskAtLevel {
    /// Value-at-Risk (left-tail quantile) at this confidence level.
    pub var: f64,
    /// Expected Shortfall (mean of exceedances below VaR).
    pub es: f64,
}

/// Risk read-outs derived directly from the published quantile distribution.
/// All values are f64 return-units — not monetary quantities (D-4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForecastRisk {
    pub var_95: RiskAtLevel,
    pub var_99: RiskAtLevel,
    /// Distribution skew: (q75 - q50) - (q50 - q25) normalised by σ.
    pub skew: f64,
    /// 90% interval width: q95 - q05 in return units.
    pub spread_90: f64,
}

impl ForecastRisk {
    /// Compute risk read-outs from a validated `ForecastDistribution`.
    pub fn from_distribution(dist: &ForecastDistribution) -> Self {
        let q = |target: f64| -> f64 {
            dist.quantile_levels
                .iter()
                .zip(dist.quantiles_return.iter())
                .min_by(|(a, _), (b, _)| {
                    (*a - target)
                        .abs()
                        .partial_cmp(&(*b - target).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(_, &v)| v)
                .unwrap_or(0.0)
        };

        let q05 = q(0.05);
        let q25 = q(0.25);
        let q50 = q(0.50);
        let q75 = q(0.75);
        let q95 = q(0.95);

        // VaR at 95%: the 5th-percentile return (left tail).
        let var_95 = q05;
        // ES at 95%: average of returns below VaR_95; approximate from quantiles.
        let es_95 = q05 * 0.5; // conservative linear interpolation to the tail

        // VaR at 99%: linear extrapolation from q05 toward the extreme.
        let var_99 = q05 - (q25 - q05) * 0.5;
        let es_99 = var_99 * 1.1;

        let skew = if dist.sigma > 0.0 {
            ((q75 - q50) - (q50 - q25)) / dist.sigma
        } else {
            0.0
        };

        ForecastRisk {
            var_95: RiskAtLevel { var: var_95, es: es_95 },
            var_99: RiskAtLevel { var: var_99, es: es_99 },
            skew,
            spread_90: q95 - q05,
        }
    }
}

// ── I-6.1: Calibrated publish contract ────────────────────────────────────────

/// The immutable, point-in-time distributional **publish contract** (I-6.1).
///
/// Exposes `predict(asset, timeframe, as_of) -> CalibratedForecast`.
/// Downstream surfaces (Strategies/Backtest/Live) consume this seam.
/// The suite never goes further — no order, size, or P&L here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalibratedForecast {
    pub model_id: String,
    /// Pinned version — behavior is stable; new promotions don't change this.
    pub version: u32,
    pub instrument_id: String,
    pub timeframe: String,
    /// The `as_of` ceiling used; the contract never read a bar beyond this.
    pub as_of: DateTime<Utc>,
    pub produced_at: DateTime<Utc>,
    pub quantile_levels: Vec<f64>,
    /// Combined + conformal-calibrated quantiles in return units (sorted ascending).
    pub quantiles_return: Vec<f64>,
    pub median_return: f64,
    pub sigma: f64,
    /// Derived risk read-outs from the quantile distribution.
    pub risk: ForecastRisk,
    /// Direction derived from median (for backward compat with point consumers).
    pub direction: Direction,
    /// Confidence derived from interval width.
    pub confidence: f64,
    /// True = this was served from a pinned artifact (immutable, reproducible).
    pub point_in_time: bool,
}

impl CalibratedForecast {
    /// Build from a validated `ForecastDistribution` and an `as_of` ceiling.
    pub fn from_distribution(
        model_id: String,
        version: u32,
        instrument_id: String,
        timeframe: String,
        as_of: DateTime<Utc>,
        dist: ForecastDistribution,
    ) -> Result<Self, String> {
        dist.validate()?;

        let risk = ForecastRisk::from_distribution(&dist);
        let direction = if dist.median_return > 1e-8 {
            Direction::Up
        } else if dist.median_return < -1e-8 {
            Direction::Down
        } else {
            Direction::Flat
        };
        let confidence = Forecast::interval_confidence(&dist);
        let produced_at = Utc::now();

        Ok(CalibratedForecast {
            model_id,
            version,
            instrument_id,
            timeframe,
            as_of,
            produced_at,
            quantile_levels: dist.quantile_levels.clone(),
            quantiles_return: dist.quantiles_return.clone(),
            median_return: dist.median_return,
            sigma: dist.sigma,
            risk,
            direction,
            confidence,
            point_in_time: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn standard_normal_dist(sigma: f64) -> ForecastDistribution {
        let quantile_levels = vec![0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95];
        let quantiles_sigma = vec![-1.645, -1.282, -0.674, 0.0, 0.674, 1.282, 1.645];
        let quantiles_return = quantiles_sigma.iter().map(|&q| q * sigma).collect();
        ForecastDistribution {
            quantile_levels,
            quantiles_sigma,
            quantiles_return,
            median_return: 0.0,
            sigma,
        }
    }

    #[test]
    fn distribution_roundtrip() {
        let dist = standard_normal_dist(0.01);
        assert!(dist.validate().is_ok());
        let json = serde_json::to_string(&dist).unwrap();
        let dist2: ForecastDistribution = serde_json::from_str(&json).unwrap();
        assert_eq!(dist.quantile_levels, dist2.quantile_levels);
        assert_eq!(dist.sigma, dist2.sigma);
        assert_eq!(dist.quantiles_sigma, dist2.quantiles_sigma);
    }

    #[test]
    fn derived_point_view_from_positive_median() {
        let sigma = 0.01;
        let mut dist = standard_normal_dist(sigma);
        dist.median_return = 0.005;

        let forecast = Forecast::from_distribution(
            "mdl_test".into(),
            1,
            "BTC-USD".into(),
            "1h".into(),
            Utc::now(),
            dist.clone(),
        )
        .unwrap();

        assert_eq!(forecast.direction, Direction::Up);
        let magnitude_f64: f64 = forecast.magnitude.try_into().unwrap();
        assert!((magnitude_f64 - 0.005).abs() < 1e-9);
        assert!(forecast.distribution.is_some());
    }

    #[test]
    fn derived_point_view_from_negative_median() {
        let mut dist = standard_normal_dist(0.01);
        dist.median_return = -0.003;
        let f = Forecast::from_distribution(
            "m".into(),
            1,
            "ETH-USD".into(),
            "1h".into(),
            Utc::now(),
            dist,
        )
        .unwrap();
        assert_eq!(f.direction, Direction::Down);
    }

    #[test]
    fn non_monotone_quantiles_rejected_by_validate() {
        let mut dist = standard_normal_dist(0.01);
        dist.quantiles_sigma[3] = -1.0; // break monotonicity
        assert!(dist.validate().is_err());
    }

    #[test]
    fn sigma_zero_rejected() {
        let mut dist = standard_normal_dist(0.01);
        dist.sigma = 0.0;
        assert!(dist.validate().is_err());
    }

    #[test]
    fn level_out_of_range_rejected() {
        let mut dist = standard_normal_dist(0.01);
        dist.quantile_levels[0] = 0.0; // must be strictly > 0
        assert!(dist.validate().is_err());
    }

    #[test]
    fn length_mismatch_rejected() {
        let mut dist = standard_normal_dist(0.01);
        dist.quantiles_sigma.pop();
        assert!(dist.validate().is_err());
    }
}
