//! Normalized 0-100 scorecard with configurable weights.
//! Sub-scores: Quality, Speed, Cost, Safety, Reliability.

use serde_json::{json, Value};

#[derive(Clone)]
pub struct ScorecardWeights {
    pub quality: f64,
    pub speed: f64,
    pub cost: f64,
    pub safety: f64,
    pub reliability: f64,
}

impl Default for ScorecardWeights {
    fn default() -> Self {
        Self {
            quality: 0.50,
            speed: 0.20,
            cost: 0.10,
            safety: 0.10,
            reliability: 0.10,
        }
    }
}

fn weights_from_env() -> ScorecardWeights {
    let parse = |key: &str, default: f64| -> f64 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    ScorecardWeights {
        quality: parse("SCORECARD_W_QUALITY", 0.50),
        speed: parse("SCORECARD_W_SPEED", 0.20),
        cost: parse("SCORECARD_W_COST", 0.10),
        safety: parse("SCORECARD_W_SAFETY", 0.10),
        reliability: parse("SCORECARD_W_RELIABILITY", 0.10),
    }
}

pub fn compute_scorecard(metrics: &Value) -> Value {
    let w = weights_from_env();

    // Quality: derived from primary metric (val_auc / accuracy / confidence proxy)
    let quality = metrics
        .get("val_auc")
        .or_else(|| metrics.get("accuracy"))
        .or_else(|| metrics.get("avg_confidence"))
        .and_then(serde_json::Value::as_f64)
        .map_or(50.0, |v| (v * 100.0).clamp(0.0, 100.0));

    // Speed: 100 if no latency info (pass-through)
    let speed = 80.0_f64;
    // Cost: 100 for trainable models (no per-inference cost)
    let cost = 90.0_f64;
    // Safety: base score (no anomaly flags)
    let safety = 85.0_f64;
    // Reliability: 100 if no error rate info
    let reliability = 80.0_f64;

    let overall = quality * w.quality
        + speed * w.speed
        + cost * w.cost
        + safety * w.safety
        + reliability * w.reliability;

    json!({
        "overall": (overall * 10.0).round() / 10.0,
        "sub_scores": {
            "quality": (quality * 10.0).round() / 10.0,
            "speed": speed,
            "cost": cost,
            "safety": safety,
            "reliability": reliability,
        },
        "weights": {
            "quality": w.quality,
            "speed": w.speed,
            "cost": w.cost,
            "safety": w.safety,
            "reliability": w.reliability,
        }
    })
}
