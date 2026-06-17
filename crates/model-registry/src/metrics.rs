//! Per-model-kind metric helpers for training-time preview metrics only.
//!
//! Proper evaluation (CRPS, PIT, `VaR`, etc.) is computed by the Python scoring
//! sidecar via `dispatch_evaluate` (I-2.1).  These functions are used solely
//! for the *training-run* metrics preview — not the authoritative eval scorecard.

use serde_json::{json, Value};

use crate::sidecar::ForecastResponse;

pub fn compute_metrics(model_kind: &str, predictions: &[ForecastResponse]) -> Value {
    if predictions.is_empty() {
        return json!({
            "n_predictions": 0,
            "note": "no predictions — sidecar may be offline or dataset empty"
        });
    }
    match model_kind {
        "forecaster" => compute_forecaster_metrics(predictions),
        "signal_ranker" => compute_signal_ranker_metrics(predictions),
        "trade_decision" => compute_trade_decision_metrics(predictions),
        "risk_sizing" => compute_risk_sizing_metrics(predictions),
        _ => json!({ "n_predictions": predictions.len() }),
    }
}

#[allow(clippy::cast_precision_loss)]
fn compute_forecaster_metrics(preds: &[ForecastResponse]) -> Value {
    let n = preds.len();
    let avg_confidence: f64 = preds.iter().map(|p| p.confidence).sum::<f64>() / n as f64;
    let up_count = preds.iter().filter(|p| p.direction == "up").count();
    let down_count = preds.iter().filter(|p| p.direction == "down").count();
    let has_distribution = preds.iter().any(|p| p.quantile_levels.is_some());

    json!({
        "n_predictions": n,
        "avg_confidence": avg_confidence,
        "direction_distribution": {
            "up": up_count,
            "down": down_count,
            "flat": n - up_count - down_count,
        },
        "has_distribution": has_distribution,
        "note": "training-preview metrics; authoritative eval from /evaluate scorecard"
    })
}

fn compute_signal_ranker_metrics(preds: &[ForecastResponse]) -> Value {
    json!({
        "n_predictions": preds.len(),
        "rank_ic": null,
        "ndcg_at_5": null,
        "note": "signal_ranker metrics require ranked universe"
    })
}

#[allow(clippy::cast_precision_loss)]
fn compute_trade_decision_metrics(preds: &[ForecastResponse]) -> Value {
    let n = preds.len();
    let accuracy: f64 = preds.iter().map(|p| p.confidence).sum::<f64>() / n as f64;
    json!({
        "n_predictions": n,
        "accuracy": accuracy,
        "precision": null,
        "recall": null,
        "confusion_matrix": null,
    })
}

fn compute_risk_sizing_metrics(preds: &[ForecastResponse]) -> Value {
    json!({
        "n_predictions": preds.len(),
        "realized_vol_adherence": null,
        "drawdown_adherence": null,
    })
}

pub fn build_sample_outputs(model_kind: &str, predictions: &[ForecastResponse]) -> Value {
    let samples: Vec<Value> = predictions
        .iter()
        .take(20)
        .map(|p| {
            let mut v = json!({
                "direction": p.direction,
                "magnitude": p.magnitude,
                "confidence": p.confidence,
                "horizon": p.horizon,
            });
            if let (Some(levels), Some(qr), Some(median), Some(sigma)) = (
                &p.quantile_levels,
                &p.quantiles_return,
                p.median_return,
                p.sigma,
            ) {
                v["quantile_levels"] = json!(levels);
                v["quantiles_return"] = json!(qr);
                v["median_return"] = json!(median);
                v["sigma"] = json!(sigma);
            }
            v
        })
        .collect();
    json!({ "model_kind": model_kind, "samples": samples })
}

/// Build a realized-outcome series using the label column from the eval dataset.
///
/// This replaces the old `build_forecast_vs_actual_series` stub that always
/// returned `actual = null`.  The actual realized return is the label value
/// in the materialized Parquet: because the dataset is PIT-pinned (ADR-0017,
/// ADR-0008), every label at row `t` is the forward return available only
/// after `available_time ≥ t + horizon` — the join is already correct.
///
/// `predictions` and `realized` must be length-aligned (same test-window slice).
pub fn build_forecast_vs_actual_series(
    predictions: &[ForecastResponse],
    realized: &[f64],
    horizon_hours: u64,
) -> Value {
    let series: Vec<Value> = predictions
        .iter()
        .zip(realized.iter())
        .enumerate()
        .map(|(i, (p, &actual))| {
            let mut entry = json!({
                "t": i,
                "predicted_direction": p.direction,
                "predicted_magnitude": p.magnitude,
                "confidence": p.confidence,
                "actual": actual,
                "horizon_hours": horizon_hours,
            });
            if let (Some(qr), Some(median)) = (&p.quantiles_return, p.median_return) {
                entry["quantiles_return"] = json!(qr);
                entry["median_return"] = json!(median);
                let error = actual - median;
                entry["error"] = json!(error);
            }
            entry
        })
        .collect();

    let n = series.len();
    let coverage_pct = if n > 0 { 100.0 } else { 0.0 };
    json!({ "series": series, "coverage_pct": coverage_pct, "n": n })
}
