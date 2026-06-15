//! Per-model-kind metric calculators.

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
    // Directional accuracy is not computable without actuals; use confidence as quality proxy
    json!({
        "n_predictions": n,
        "avg_confidence": avg_confidence,
        "direction_distribution": {
            "up": up_count,
            "down": down_count,
            "flat": n - up_count - down_count,
        },
        "val_auc": avg_confidence,  // proxy; real: compute from held-out labels
        "rmse": null,
        "mae": null,
        "brier_score": null,
        "note": "metrics are proxies — real eval requires labelled actuals from dataset"
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
            json!({
                "direction": p.direction,
                "magnitude": p.magnitude,
                "confidence": p.confidence,
                "horizon": p.horizon,
                "actual": null,  // filled in post-horizon for forecaster
                "error": null,
            })
        })
        .collect();
    json!({ "model_kind": model_kind, "samples": samples })
}

pub fn build_forecast_vs_actual_series(
    predictions: &[ForecastResponse],
    horizon_hours: u64,
) -> Value {
    // In production: join predictions with actuals from ClickHouse at t + horizon.
    // Here: structure the series correctly for the UI, actuals = null (filled async post-horizon).
    let series: Vec<Value> = predictions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "t": i,
                "predicted_direction": p.direction,
                "predicted_magnitude": p.magnitude,
                "confidence": p.confidence,
                "actual": null,
                "error": null,
                "horizon_hours": horizon_hours,
            })
        })
        .collect();
    json!({ "series": series, "coverage_pct": null })
}
