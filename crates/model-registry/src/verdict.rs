//! From an evaluation's recorded series to the usage-rule verdict (Set N N-3.2, N-3.4, N-3.5, N-3.6).
//!
//! [`crate::scoring_v2`] holds the statistics and the rules as pure functions. This module
//! is the caller they were missing: it reads what the trainer's evaluation recorded
//! (`metrics.series`, `metrics.family_baselines`; contract in
//! `apps/model-trainer/app/families/baseline.py`), computes the evidence the rules read,
//! and returns a [`VerdictReport`]. The promotion gate and the verdict route both go through
//! [`judge_metrics`]; nothing else decides whether a forecast may size a position.
//!
//! # What a missing measurement is
//!
//! `NotComputable`, and `NotComputable` blocks (ADR-N-05). An evaluation with no recorded
//! series, a vol model with no HARQ baseline, a return series shorter than the Clark-West
//! minimum: each leaves a field of [`Evidence`] empty, and the rule it feeds says so.
//!
//! # A scorecard always carries its baselines (N-3.5)
//!
//! [`require_baselines`] is the type-level refusal: [`BaselinedMetrics`] can only be built from a
//! metrics blob that has at least one family baseline row, and
//! [`crate::scorecard::compute_scorecard_checked`] takes only that type.

use serde::Serialize;
use serde_json::{json, Value};

use crate::scoring_v2::{
    clark_west, complexity_vs_baseline, es_backtest_gaussian, expanding_mean_benchmark,
    judge_binding, oos_r2, qlike, vol_timed_momentum, BindingVerdict, Complexity, Evidence,
    ForecastUse, RuleVerdict,
};

/// Rows kept out of the benchmark's warm-up: the expanding mean needs this many
/// observations before it forecasts anything.
const MIN_TRAIN: usize = 20;
/// Monte-Carlo draws for the ES backtest p-value.
const ES_SIMS: usize = 2_000;
/// The tail level the platform's `var_95` / `es_95` outputs refer to.
const TAIL_ALPHA: f64 = 0.05;

/// Why a recorded evaluation cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerdictError {
    #[error("the evaluation records no family baseline: a scorecard without the thing it has to beat is refused")]
    NoBaselines,
}

/// A metrics blob proven to carry at least one family baseline row.
#[derive(Clone, Debug)]
pub struct BaselinedMetrics<'a>(&'a Value);

impl<'a> BaselinedMetrics<'a> {
    #[must_use]
    pub fn metrics(&self) -> &'a Value {
        self.0
    }
}

/// N-3.5: the only way to get a [`BaselinedMetrics`].
///
/// # Errors
/// [`VerdictError::NoBaselines`] when `family_baselines` is absent, not a list, or empty.
pub fn require_baselines(metrics: &Value) -> Result<BaselinedMetrics<'_>, VerdictError> {
    // A row the trainer marked `refused` or `unavailable` measured nothing, so it is not
    // a baseline the candidate was scored beside.
    let measured = |r: &Value| {
        r.get("status")
            .and_then(Value::as_str)
            .is_none_or(|s| s == "ok")
    };
    match metrics.get("family_baselines").and_then(Value::as_array) {
        Some(rows) if rows.iter().any(measured) => Ok(BaselinedMetrics(metrics)),
        _ => Err(VerdictError::NoBaselines),
    }
}

fn f64_vec(v: Option<&Value>) -> Option<Vec<f64>> {
    let arr = v?.as_array()?;
    let out: Vec<f64> = arr.iter().filter_map(Value::as_f64).collect();
    (out.len() == arr.len() && !out.is_empty()).then_some(out)
}

/// What the evaluation recorded about its own series.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedSeries {
    pub target_kind: String,
    pub realized: Vec<f64>,
    pub point: Vec<f64>,
    pub sigma: Option<Vec<f64>>,
}

impl RecordedSeries {
    #[must_use]
    pub fn from_metrics(metrics: &Value) -> Option<Self> {
        let s = metrics.get("series")?;
        let realized = f64_vec(s.get("realized"))?;
        let point = f64_vec(s.get("point"))?;
        if realized.len() != point.len() {
            return None;
        }
        let sigma = f64_vec(s.get("sigma")).filter(|v| v.len() == realized.len());
        Some(Self {
            target_kind: s
                .get("target_kind")
                .and_then(Value::as_str)
                .unwrap_or("return")
                .to_string(),
            realized,
            point,
            sigma,
        })
    }
}

fn baseline_rows(metrics: &Value) -> Vec<Value> {
    metrics
        .get("family_baselines")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Which forecast uses a model implies: a volatility target or a vol family is a
/// [`ForecastUse::Vol`] forecast; everything else is a return or direction forecast.
/// A tail-risk use is a property of the *consumer* (`var_95` / `es_95`), so it is
/// requested by the caller and never inferred here.
///
/// `definition` is the stored `ModelDefinition` JSON, `family` the registry family the
/// version was trained as (`model_versions.family`), `declaration` the dataset
/// declaration it was trained on (`dataset_versions.dataset_spec`).
#[must_use]
pub fn uses_for_definition(
    definition: &Value,
    family: Option<&str>,
    declaration: Option<&Value>,
) -> Vec<ForecastUse> {
    let framework = definition
        .get("framework")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let target = declaration
        .and_then(|d| {
            d.pointer("/target/kind")
                .or_else(|| d.pointer("/target/type"))
        })
        .and_then(Value::as_str)
        .unwrap_or("");
    if framework == "garch" || family == Some("vol") || target == "realized_vol" {
        vec![ForecastUse::Vol]
    } else {
        vec![ForecastUse::Return]
    }
}

/// Parse a comma-separated `uses` query (`return,vol,tail_risk`).
///
/// # Errors
/// The first token that names no use.
pub fn parse_uses(s: &str) -> Result<Vec<ForecastUse>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| match t {
            "return" | "direction" => Ok(ForecastUse::Return),
            "vol" | "volatility" => Ok(ForecastUse::Vol),
            "tail_risk" | "var_95" | "es_95" => Ok(ForecastUse::TailRisk),
            other => Err(format!(
                "`{other}` is not a forecast use (return, vol, tail_risk)"
            )),
        })
        .collect()
}

/// A number the rules read, and where it came from, for the verdict's `measured` block.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Measured {
    pub oos_r2: Option<f64>,
    pub clark_west_p: Option<f64>,
    pub model_qlike: Option<f64>,
    pub harq_qlike: Option<f64>,
    pub es_z2: Option<f64>,
    pub es_p_z2: Option<f64>,
    pub n: Option<usize>,
    /// Why a measurement is absent, by statistic.
    pub missing: Vec<String>,
}

/// Compute the evidence the rules for `uses` read. Anything that cannot be computed is
/// left `None` and named in `Measured::missing`; it is never zero-filled.
#[must_use]
pub fn evidence_from_metrics(metrics: &Value, uses: &[ForecastUse]) -> (Evidence, Measured) {
    let mut ev = Evidence::default();
    let mut m = Measured::default();
    let Some(series) = RecordedSeries::from_metrics(metrics) else {
        m.missing
            .push("the evaluation recorded no `series` (realized and point predictions)".into());
        return (ev, m);
    };
    m.n = Some(series.realized.len());

    if uses.contains(&ForecastUse::Return) {
        match expanding_mean_benchmark(
            &series.realized,
            MIN_TRAIN
                .min(series.realized.len().saturating_sub(1))
                .max(1),
        ) {
            Ok((start, bench)) => {
                let actual = &series.realized[start..];
                let model = &series.point[start..];
                match oos_r2(actual, model, &bench) {
                    Ok(r2) => {
                        ev.oos_r2 = Some(r2);
                        m.oos_r2 = Some(r2);
                    }
                    Err(e) => m.missing.push(format!("oos_r2: {e}")),
                }
                match clark_west(actual, &bench, model, 0) {
                    Ok(cw) => {
                        ev.clark_west_p = Some(cw.p_value);
                        m.clark_west_p = Some(cw.p_value);
                    }
                    Err(e) => m.missing.push(format!("clark_west: {e}")),
                }
            }
            Err(e) => m.missing.push(format!("expanding-mean benchmark: {e}")),
        }
    }

    if uses.contains(&ForecastUse::Vol) {
        let harq = baseline_rows(metrics).into_iter().find(|b| {
            (b.get("variant").and_then(Value::as_str) == Some("harq")
                || b.get("name").and_then(Value::as_str) == Some("harq"))
                && b.get("status")
                    .and_then(Value::as_str)
                    .is_none_or(|s| s == "ok")
        });
        // A baseline's series is null outside the folds it was scored on; compare on the
        // rows where the model, HARQ and the realised value all exist.
        let harq_sigma: Option<Vec<Option<f64>>> = harq
            .as_ref()
            .and_then(|b| b.pointer("/series/sigma"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Value::as_f64).collect());
        let realized_var: Vec<f64> = series.realized.iter().map(|x| x * x).collect();
        match (series.sigma.as_ref(), harq_sigma) {
            (Some(sigma), Some(h))
                if h.len() == sigma.len() && sigma.len() == realized_var.len() =>
            {
                let mut rv = Vec::new();
                let (mut mv, mut hv) = (Vec::new(), Vec::new());
                for i in 0..sigma.len() {
                    if let Some(hs) = h[i] {
                        rv.push(realized_var[i]);
                        mv.push(sigma[i] * sigma[i]);
                        hv.push(hs * hs);
                    }
                }
                match (qlike(&rv, &mv), qlike(&rv, &hv)) {
                    (Ok(mq), Ok(hq)) => {
                        ev.model_qlike = Some(mq);
                        ev.harq_qlike = Some(hq);
                        m.model_qlike = Some(mq);
                        m.harq_qlike = Some(hq);
                    }
                    (a, b) => {
                        for e in [a.err(), b.err()].into_iter().flatten() {
                            m.missing.push(format!("qlike: {e}"));
                        }
                    }
                }
            }
            (Some(_), Some(_)) => m
                .missing
                .push("the HARQ baseline's series is not aligned with the evaluation's".into()),
            (None, _) => m
                .missing
                .push("the model records no predictive sigma, so QLIKE cannot be computed".into()),
            (_, None) => m
                .missing
                .push("no HARQ baseline was trained on these folds".into()),
        }
    }

    if uses.contains(&ForecastUse::TailRisk) {
        match series.sigma.as_ref() {
            Some(sigma) => match es_backtest_gaussian(
                &series.realized,
                &series.point,
                sigma,
                TAIL_ALPHA,
                ES_SIMS,
                0x00E5_BAC7,
            ) {
                Ok(b) => {
                    ev.es_backtest = Some(b.verdict);
                    m.es_z2 = Some(b.z2);
                    m.es_p_z2 = Some(b.p_z2);
                }
                Err(e) => m.missing.push(format!("es backtest: {e}")),
            },
            None => m
                .missing
                .push("the model records no predictive sigma, so no ES backtest can be run".into()),
        }
    }
    (ev, m)
}

/// One rule's line in a verdict, as the API serves it.
#[derive(Clone, Debug, Serialize)]
pub struct RuleLine {
    pub rule: &'static str,
    pub applies: bool,
    pub status: &'static str,
    pub reason: String,
    pub measured: Value,
}

/// The whole verdict: the binding decision, every rule's line, the baselines the
/// candidate was scored beside, and the complexity check.
#[derive(Clone, Debug, Serialize)]
pub struct VerdictReport {
    pub allowed: bool,
    pub rules: Vec<RuleLine>,
    pub baselines: Vec<Value>,
    pub complexity: Option<Value>,
    #[serde(skip)]
    binding: BindingVerdict,
}

impl VerdictReport {
    #[must_use]
    pub fn binding(&self) -> &BindingVerdict {
        &self.binding
    }

    /// Every reason the binding is refused.
    #[must_use]
    pub fn reasons(&self) -> Vec<String> {
        self.binding.reasons()
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

const ALL_USES: [(ForecastUse, &str); 3] = [
    (ForecastUse::Return, "return_forecast"),
    (ForecastUse::Vol, "vol_forecast"),
    (ForecastUse::TailRisk, "tail_risk"),
];

fn complexity(metrics: &Value) -> Option<Value> {
    let s = RecordedSeries::from_metrics(metrics)?;
    if s.target_kind != "return" || s.realized.len() < 80 {
        return None;
    }
    let model: Vec<f64> = s
        .realized
        .iter()
        .zip(&s.point)
        .map(|(r, p)| r * p.signum())
        .collect();
    let sd = {
        let m = s.realized.iter().sum::<f64>() / s.realized.len() as f64;
        (s.realized.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (s.realized.len() as f64 - 1.0))
            .sqrt()
    };
    let baseline = vol_timed_momentum(&s.realized, 5, 20, sd.max(1e-12), 3.0).ok()?;
    // Align the model's returns to the baseline's (which start later).
    let skip = model.len() - baseline.len();
    let verdict = complexity_vs_baseline(&model[skip..], &baseline).ok()?;
    Some(json!({
        "unjustified": verdict == Complexity::Unjustified,
        "flag": if verdict == Complexity::Unjustified { "complexity_unjustified" } else { "justified" },
        "detail": "per-row Sharpe of sign(point) x label against vol-timed momentum on the same labels (an approximation: overlapping labels are not de-overlapped)",
    }))
}

/// Judge a recorded evaluation for the uses given. The one function the promotion gate,
/// the verdict route and a Gate 3 binding all call.
#[must_use]
pub fn judge_metrics(metrics: &Value, uses: &[ForecastUse]) -> VerdictReport {
    let (ev, measured) = evidence_from_metrics(metrics, uses);
    let binding = judge_binding(uses, &ev);
    let measured_json = serde_json::to_value(&measured).unwrap_or_else(|_| json!({}));
    let rules = ALL_USES
        .iter()
        .map(|(u, name)| {
            let found = binding.rules().iter().find(|(x, _)| x == u);
            match found {
                None => RuleLine {
                    rule: name,
                    applies: false,
                    status: "not_applicable",
                    reason: "this use was not requested".into(),
                    measured: json!({}),
                },
                Some((_, RuleVerdict::Pass)) => RuleLine {
                    rule: name,
                    applies: true,
                    status: "pass",
                    reason: String::new(),
                    measured: measured_json.clone(),
                },
                Some((_, RuleVerdict::Fail { reason })) => RuleLine {
                    rule: name,
                    applies: true,
                    status: "fail",
                    reason: reason.clone(),
                    measured: measured_json.clone(),
                },
                Some((_, RuleVerdict::NotComputable { reason })) => {
                    let mut why = reason.clone();
                    if !measured.missing.is_empty() {
                        why = format!("{why} ({})", measured.missing.join("; "));
                    }
                    RuleLine {
                        rule: name,
                        applies: true,
                        status: "not_computable",
                        reason: why,
                        measured: measured_json.clone(),
                    }
                }
            }
        })
        .collect();
    let baselines = baseline_rows(metrics)
        .into_iter()
        .map(|mut b| {
            // The series are evidence for the rules, not for a table cell.
            if let Some(o) = b.as_object_mut() {
                o.remove("series");
            }
            b
        })
        .collect();
    VerdictReport {
        allowed: binding.allowed(),
        rules,
        baselines,
        complexity: complexity(metrics),
        binding,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backtest::rng::DetRng;

    fn noise(seed: u64, n: usize, sd: f64) -> Vec<f64> {
        let mut r = DetRng::new(seed);
        (0..n)
            .map(|_| {
                let u1 = r.next_f64().max(1e-300);
                let u2 = r.next_f64();
                sd * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
            })
            .collect()
    }

    fn return_metrics(predictable: bool) -> Value {
        let n = 400;
        let signal = noise(1, n, 0.01);
        let eps = noise(2, n, 0.01);
        let realized: Vec<f64> = (0..n).map(|i| 0.001 + signal[i] + eps[i]).collect();
        let point: Vec<f64> = if predictable {
            (0..n).map(|i| 0.001 + signal[i]).collect()
        } else {
            noise(3, n, 0.01)
        };
        json!({
            "series": {"target_kind": "return", "realized": realized, "point": point, "sigma": null},
            "family_baselines": [{"name": "mean", "family": "baseline", "variant": "mean", "folds": [0], "scores": {}}],
        })
    }

    #[test]
    fn a_missing_series_is_not_computable_and_blocks() {
        let r = judge_metrics(
            &json!({"family_baselines": [{"name": "mean"}]}),
            &[ForecastUse::Return],
        );
        assert!(!r.allowed);
        assert_eq!(r.rules[0].status, "not_computable");
        assert!(
            r.rules[0].reason.contains("no `series`"),
            "{}",
            r.rules[0].reason
        );
    }

    #[test]
    fn a_predictable_series_passes_the_return_rule_and_noise_fails_it() {
        let ok = judge_metrics(&return_metrics(true), &[ForecastUse::Return]);
        assert!(ok.allowed, "{:?}", ok.reasons());
        let bad = judge_metrics(&return_metrics(false), &[ForecastUse::Return]);
        assert!(!bad.allowed);
        assert_eq!(bad.rules[0].status, "fail", "{:?}", bad.rules[0]);
    }

    #[test]
    fn a_vol_model_with_no_harq_baseline_is_not_computable() {
        let n = 300;
        let realized: Vec<f64> = noise(4, n, 0.01).iter().map(|x| x.abs() + 0.005).collect();
        let sigma = realized.clone();
        let m = json!({
            "series": {"target_kind": "vol", "realized": realized, "point": sigma, "sigma": sigma},
            "family_baselines": [{"name": "ewma", "family": "baseline", "variant": "ewma"}],
        });
        let r = judge_metrics(&m, &[ForecastUse::Vol]);
        assert!(!r.allowed);
        assert_eq!(r.rules[1].status, "not_computable");
        assert!(r.rules[1].reason.contains("HARQ"), "{}", r.rules[1].reason);
    }

    #[test]
    fn a_vol_model_that_beats_harq_passes_and_one_that_loses_fails() {
        let n = 300;
        let realized: Vec<f64> = noise(5, n, 0.004).iter().map(|x| x.abs() + 0.01).collect();
        // The model forecasts the truth exactly; HARQ forecasts a flat, wrong level.
        let harq: Vec<f64> = vec![0.03; n];
        let mk = |sigma: &Vec<f64>| {
            json!({
                "series": {"target_kind": "vol", "realized": realized, "point": sigma, "sigma": sigma},
                "family_baselines": [{"name": "harq", "family": "baseline", "variant": "harq",
                                      "series": {"sigma": harq}}],
            })
        };
        let good = judge_metrics(&mk(&realized), &[ForecastUse::Vol]);
        assert!(good.allowed, "{:?}", good.reasons());
        let worse: Vec<f64> = vec![0.2; n];
        let bad = judge_metrics(&mk(&worse), &[ForecastUse::Vol]);
        assert!(!bad.allowed);
        assert_eq!(bad.rules[1].status, "fail");
    }

    #[test]
    fn tail_risk_needs_a_passed_es_backtest() {
        // No sigma -> no backtest -> blocked.
        let r = judge_metrics(&return_metrics(true), &[ForecastUse::TailRisk]);
        assert!(!r.allowed);
        assert_eq!(r.rules[2].status, "not_computable");
        // A calibrated gaussian passes.
        let n = 600;
        let sd = 0.01;
        let realized = noise(6, n, sd);
        let m = json!({
            "series": {"target_kind": "return", "realized": realized, "point": vec![0.0; n], "sigma": vec![sd; n]},
            "family_baselines": [{"name": "mean"}],
        });
        let ok = judge_metrics(&m, &[ForecastUse::TailRisk]);
        assert!(ok.allowed, "{:?}", ok.reasons());
        // An understated tail fails.
        let m2 = json!({
            "series": {"target_kind": "return", "realized": realized, "point": vec![0.0; n], "sigma": vec![sd * 0.4; n]},
            "family_baselines": [{"name": "mean"}],
        });
        assert!(!judge_metrics(&m2, &[ForecastUse::TailRisk]).allowed);
    }

    #[test]
    fn a_scorecard_without_a_baseline_is_refused_by_type() {
        assert!(require_baselines(&json!({"crps": 0.01})).is_err());
        assert!(require_baselines(&json!({"family_baselines": []})).is_err());
        assert!(require_baselines(&json!({"family_baselines": [{"name": "mean"}]})).is_ok());
        // A refused or unavailable row measured nothing.
        assert!(require_baselines(
            &json!({"family_baselines": [{"name": "harq", "status": "refused"}]})
        )
        .is_err());
    }

    #[test]
    fn the_definition_decides_the_use_and_tail_risk_is_never_inferred() {
        assert_eq!(
            uses_for_definition(&json!({"framework": "garch"}), None, None),
            vec![ForecastUse::Vol]
        );
        assert_eq!(
            uses_for_definition(
                &json!({}),
                None,
                Some(&json!({"target": {"kind": "realized_vol"}}))
            ),
            vec![ForecastUse::Vol]
        );
        assert_eq!(
            uses_for_definition(&json!({}), Some("vol"), None),
            vec![ForecastUse::Vol]
        );
        assert_eq!(
            uses_for_definition(&json!({"framework": "lightgbm"}), Some("gbm"), None),
            vec![ForecastUse::Return]
        );
        assert_eq!(parse_uses("return, tail_risk").unwrap().len(), 2);
        assert!(parse_uses("sharpe").is_err());
    }

    #[test]
    fn a_return_model_that_loses_to_vol_timed_momentum_is_flagged() {
        // Anti-signal: the model is always on the wrong side.
        let n = 300;
        let realized = noise(7, n, 0.01);
        let point: Vec<f64> = realized.iter().map(|r| -r.signum()).collect();
        let m = json!({
            "series": {"target_kind": "return", "realized": realized, "point": point, "sigma": null},
            "family_baselines": [{"name": "mean"}],
        });
        let r = judge_metrics(&m, &[ForecastUse::Return]);
        let c = r.complexity.expect("long enough to compare");
        assert_eq!(c["unjustified"], json!(true));
    }
}
