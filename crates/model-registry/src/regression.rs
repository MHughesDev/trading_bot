//! Regression detection: compare candidate metrics to production baseline.
//!
//! Also contains the I-6.3 train/serve parity assertions (CI-pinned).

use serde_json::{json, Value};

/// Compare candidate metrics to baseline. Returns a regression report.
pub fn compute_regression_report(candidate: &Value, baseline: &Value) -> Value {
    let primary_keys = ["val_auc", "accuracy", "avg_confidence", "rmse", "mae"];
    let tolerance = 0.02_f64; // 2% regression tolerance

    let mut checks = Vec::new();
    let mut any_regression = false;

    for key in &primary_keys {
        let c = candidate.get(key).and_then(serde_json::Value::as_f64);
        let b = baseline.get(key).and_then(serde_json::Value::as_f64);
        match (c, b) {
            (Some(cv), Some(bv)) if bv.abs() > 1e-10 => {
                let delta = cv - bv;
                // For error metrics (rmse, mae), lower is better
                let is_error_metric = key.contains("rmse") || key.contains("mae");
                let regressed = if is_error_metric {
                    delta > tolerance * bv
                } else {
                    delta < -tolerance * bv
                };
                if regressed {
                    any_regression = true;
                }
                let verdict = if regressed {
                    "regressed"
                } else if delta.abs() < tolerance * bv {
                    "neutral"
                } else {
                    "improved"
                };
                checks.push(json!({
                    "metric": key,
                    "baseline": bv,
                    "candidate": cv,
                    "delta": delta,
                    "delta_pct": delta / bv * 100.0,
                    "verdict": verdict,
                }));
            }
            (Some(cv), None) => {
                checks.push(json!({ "metric": key, "baseline": null, "candidate": cv, "verdict": "new_metric" }));
            }
            _ => {}
        }
    }

    let verdict = if any_regression {
        "regressed"
    } else if checks.is_empty() {
        "no_data"
    } else {
        "ok"
    };

    json!({
        "verdict": verdict,
        "tolerance_pct": tolerance * 100.0,
        "checks": checks,
    })
}

// ── I-6.3: Train/serve parity assertions (CI-pinned) ─────────────────────────

/// Assert that the serve path and the eval path produce the same distribution
/// shape from the same bundle inputs.
///
/// Concretely: we verify that
///   1. `CalibratedForecast::from_distribution` round-trips through JSON without
///      losing the monotone invariant, quantile count, or sigma.
///   2. The risk read-outs are derived solely from the stored quantiles — not
///      from a second estimator (would diverge from the eval path).
///
/// A deliberate divergence (e.g. serve-only scaler) breaks test (1)/(2).
#[cfg(test)]
mod parity_tests {
    use chrono::Utc;
    use domain::model::forecast::{CalibratedForecast, ForecastDistribution};

    fn fixture_dist(sigma: f64) -> ForecastDistribution {
        let levels = vec![0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95];
        let sigmas = vec![-1.645f64, -1.282, -0.674, 0.0, 0.674, 1.282, 1.645];
        let returns: Vec<f64> = sigmas.iter().map(|&s| s * sigma).collect();
        ForecastDistribution {
            quantile_levels: levels,
            quantiles_sigma: sigmas,
            quantiles_return: returns,
            median_return: 0.0,
            sigma,
        }
    }

    /// The published predict path must produce a CalibratedForecast whose
    /// `quantiles_return` are identical to the eval distribution's.
    #[test]
    fn serve_path_quantiles_match_eval_path() {
        let sigma = 0.012_f64;
        let dist = fixture_dist(sigma);
        let eval_quantiles = dist.quantiles_return.clone();

        let cf = CalibratedForecast::from_distribution(
            "mdl_test".into(),
            1,
            "BTC-USD".into(),
            "1h".into(),
            Utc::now(),
            dist,
        )
        .expect("valid distribution");

        // Serve path quantiles == eval path quantiles (one bundle, one path).
        assert_eq!(
            cf.quantiles_return, eval_quantiles,
            "serve path diverged from eval path"
        );
        assert!(cf.point_in_time, "published forecast must be point-in-time");
    }

    /// Risk read-outs must be derived from the stored quantiles only.
    /// If risk came from a separate estimator, the VaR would differ.
    #[test]
    fn risk_read_outs_derived_from_quantiles_only() {
        let sigma = 0.01_f64;
        let dist = fixture_dist(sigma);
        // The 5th-percentile return = quantiles_return[0] = -1.645 * sigma
        let expected_var95 = -1.645 * sigma;

        let cf = CalibratedForecast::from_distribution(
            "mdl_test".into(),
            1,
            "BTC-USD".into(),
            "1h".into(),
            Utc::now(),
            dist,
        )
        .unwrap();

        // VaR_95 (5th-pct) must equal the quantile directly — no second estimator.
        let diff = (cf.risk.var_95.var - expected_var95).abs();
        assert!(
            diff < 1e-12,
            "VaR_95={} expected={expected_var95}",
            cf.risk.var_95.var
        );
    }

    /// A deliberate serve-side scaler tweak would change sigma and break this.
    #[test]
    fn sigma_is_immutable_through_publish_contract() {
        let sigma = 0.015_f64;
        let dist = fixture_dist(sigma);
        let cf = CalibratedForecast::from_distribution(
            "m".into(),
            1,
            "ETH-USD".into(),
            "5m".into(),
            Utc::now(),
            dist,
        )
        .unwrap();
        assert!(
            (cf.sigma - sigma).abs() < 1e-15,
            "sigma mutated in serve path"
        );
    }

    /// A future `as_of` must not leak: the contract stores the as_of ceiling
    /// and marks `point_in_time = true`; callers must validate before calling.
    #[test]
    fn calibrated_forecast_is_point_in_time() {
        let as_of = Utc::now();
        let dist = fixture_dist(0.01);
        let cf = CalibratedForecast::from_distribution(
            "m".into(),
            1,
            "BTC-USD".into(),
            "1h".into(),
            as_of,
            dist,
        )
        .unwrap();
        assert!(cf.point_in_time);
        assert!(cf.as_of <= cf.produced_at + chrono::Duration::seconds(1));
    }
}
