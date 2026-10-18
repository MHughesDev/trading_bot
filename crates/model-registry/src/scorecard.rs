//! Normalized 0-100 scorecard with configurable weights.
//!
//! Sub-scores: Quality, Speed, Cost, Safety, Reliability.
//!
//! I-2.10: Quality is now derived from the proper-scoring metrics (CRPS +
//! calibration) produced by the eval sidecar, not the `val_auc` proxy.
//! For training-run metrics (no eval yet), falls back to the confidence proxy
//! as before so the UI always has a non-null scorecard.

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

/// Compute scorecard from training-run metrics (preview; no realized actuals yet).
pub fn compute_scorecard(metrics: &Value) -> Value {
    let w = weights_from_env();

    // Quality: prefer CRPS from the eval sidecar (I-2.10); fall back to the
    // val_auc / avg_confidence proxy for training-run previews.
    let quality = if let Some(crps) = metrics.get("crps").and_then(serde_json::Value::as_f64) {
        quality_from_crps(
            crps,
            metrics
                .get("pit_calibrated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        )
    } else {
        metrics
            .get("val_auc")
            .or_else(|| metrics.get("accuracy"))
            .or_else(|| metrics.get("avg_confidence"))
            .and_then(serde_json::Value::as_f64)
            .map_or(50.0, |v| (v * 100.0).clamp(0.0, 100.0))
    };

    let speed = 80.0_f64;
    let cost = 90.0_f64;
    let safety = 85.0_f64;
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
        },
        "quality_source": if metrics.get("crps").is_some() { "crps+calibration" } else { "proxy" },
    })
}

/// Map CRPS ∈ [0, ∞) → quality ∈ (0, 100], penalize miscalibration (I-2.10).
fn quality_from_crps(crps: f64, calibrated: bool) -> f64 {
    // Typical CRPS for standardized 1-min return distributions: 0.001 – 0.05.
    // We map via 100 / (1 + crps * 1000) so CRPS≈0 → ~100, CRPS≈0.001 → ~50.
    let q = 100.0 / (1.0 + crps * 1000.0);
    let q_adj = if calibrated { q } else { q * 0.85 };
    q_adj.clamp(0.0, 100.0)
}

/// Build a scorecard from a full eval result persisted by the scoring sidecar (I-2.10).
/// Called by `drive_eval` after `dispatch_evaluate` succeeds.
pub fn compute_scorecard_from_eval(eval_result: &Value) -> Value {
    // The sidecar already computed a scorecard — use it directly if present.
    if let Some(sc) =
        eval_result
            .get("scorecard")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
    {
        return sc.clone();
    }
    // Fall back to computing from the metrics block.
    if let Some(metrics) = eval_result.get("metrics") {
        return compute_scorecard(metrics);
    }
    compute_scorecard(&json!({}))
}

/// A scorecard that has been checked against the baseline rule (Set N N-3.5), or the
/// reason it was refused.
#[derive(Clone, Debug)]
pub struct EvalScorecard {
    pub scorecard: Value,
    /// `Some` when an evaluation that claims success carried no baseline: the run is
    /// failed with this reason rather than stored as a scorecard that cannot be read
    /// against the thing it has to beat.
    pub refusal: Option<String>,
}

/// The scorecard for a finished evaluation.
///
/// A *successful* evaluation must carry at least one family baseline
/// ([`crate::verdict::require_baselines`]); without one the scorecard is refused. A stub or
/// failed evaluation falls back to [`compute_scorecard_from_eval`] unchanged: those never
/// claimed a measurement. A checked scorecard also carries the baselines, the usage-rule
/// verdict for `uses` and the complexity flag, so nothing downstream recomputes them.
#[must_use]
pub fn eval_scorecard(
    status: &str,
    eval_json: &Value,
    metrics: &Value,
    uses: &[crate::scoring_v2::ForecastUse],
) -> EvalScorecard {
    let mut scorecard = compute_scorecard_from_eval(eval_json);
    if status != "succeeded" {
        return EvalScorecard {
            scorecard,
            refusal: None,
        };
    }
    match crate::verdict::require_baselines(metrics) {
        Err(e) => {
            if let Some(o) = scorecard.as_object_mut() {
                o.insert("baselines_missing".into(), json!(true));
            }
            EvalScorecard {
                scorecard,
                refusal: Some(e.to_string()),
            }
        }
        Ok(b) => {
            let report = crate::verdict::judge_metrics(b.metrics(), uses);
            if let Some(o) = scorecard.as_object_mut() {
                o.insert("family_baselines".into(), json!(report.baselines));
                o.insert("usage_verdict".into(), report.to_json());
                o.insert(
                    "complexity_unjustified".into(),
                    json!(report
                        .complexity
                        .as_ref()
                        .and_then(|c| c.get("unjustified"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)),
                );
            }
            EvalScorecard {
                scorecard,
                refusal: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring_v2::ForecastUse;

    #[test]
    fn a_successful_evaluation_with_no_baseline_is_refused() {
        let metrics = json!({"crps": 0.01, "pit": {"calibrated": true}});
        let eval = json!({"status": "succeeded", "metrics": metrics});
        let r = eval_scorecard("succeeded", &eval, &metrics, &[ForecastUse::Return]);
        assert!(r.refusal.as_deref().unwrap().contains("baseline"));
        assert_eq!(r.scorecard["baselines_missing"], json!(true));
    }

    #[test]
    fn a_stub_evaluation_is_not_judged_it_never_claimed_a_measurement() {
        let metrics = json!({"note": "sidecar offline"});
        let r = eval_scorecard(
            "stub",
            &json!({"metrics": metrics}),
            &metrics,
            &[ForecastUse::Return],
        );
        assert!(r.refusal.is_none());
    }

    #[test]
    fn a_baselined_scorecard_carries_its_baselines_and_its_verdict() {
        let metrics = json!({
            "crps": 0.01,
            "family_baselines": [{"name": "mean", "family": "baseline", "variant": "mean", "scores": {"crps": 0.02}}],
        });
        let eval = json!({"status": "succeeded", "metrics": metrics});
        let r = eval_scorecard("succeeded", &eval, &metrics, &[ForecastUse::Return]);
        assert!(r.refusal.is_none());
        assert_eq!(r.scorecard["family_baselines"][0]["name"], json!("mean"));
        // No series was recorded, so the verdict exists and is not an allow.
        assert_eq!(r.scorecard["usage_verdict"]["allowed"], json!(false));
        assert_eq!(
            r.scorecard["usage_verdict"]["rules"][0]["status"],
            json!("not_computable")
        );
    }
}
