//! Regression detection: compare candidate metrics to production baseline.

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
