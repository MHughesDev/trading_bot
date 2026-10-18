//! Leave-one-rule-out variants of a strategy, and what to make of their results
//! (S-3.13 `ablation_sweep`, AGENT-006 §5.5).
//!
//! # What this is, and the half that is not here
//!
//! An ablation asks of each component *does this rule earn its place?* by
//! running the strategy with that rule neutralised and comparing. This module is
//! the two **pure** halves of that:
//!
//! * [`variants`] -- the neutralised definitions;
//! * [`classify`] -- the verdict from a baseline and a variant's per-period
//!   returns, with a paired block-bootstrap interval so a delta inside the
//!   noise is not called a finding.
//!
//! What it does **not** do is run anything. An ablation varies **the strategy**,
//! and the executor resolves a run's definition from `strategy_definitions` by
//! id, so each variant has to be a stored strategy under its own id. The
//! dispatcher is the `ablation_sweep` tool (`mcp-server`): it registers each
//! variant as a **draft strategy** through the ordinary `POST /api/strategies`
//! door (lineage recorded, the user's strategy table the only thing written, and
//! visibly), runs one `StudyKind::Ablation` study over `[baseline, variants..]`
//! (each member a counted look), and asks
//! `/api/agent/falsification/ablation/*/classify` to read the verdicts, which
//! calls [`classify_equity`] below. ADR-S3-15: draft rows rather than an
//! in-memory variant path -- the rejected alternative, because the executor, the
//! ledger and the run record all key on a stored strategy id and a second
//! resolution path would be a second answer to "which strategy produced this
//! run".
//!
//! # How a rule is neutralised
//!
//! * a `condition` node's expression becomes the tautology `1 > 0`, so the graph
//!   keeps its shape and the signals that watch it still fire -- the rule is
//!   "always satisfied" rather than deleted, which is what leaving it out of a
//!   conjunction means;
//! * an exit rule is removed; removing the last one switches the policy to
//!   `risk_layer_only`, because an empty `rules` list is rejected by the
//!   validator as a disguised `risk_layer_only`.
//!
//! Filter, rank and sizing nodes are not ablated here: neutralising them has no
//! single meaning (a rank cannot be "always satisfied"), and offering a guess
//! would be a result with a false label.

use backtest::rng::DetRng;
use backtest::stats::bootstrap::{politis_white_block, stationary_indices};
use serde::Serialize;
use serde_json::{json, Value};

/// One component, neutralised.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Variant {
    /// `node:<id>` or `exit:<index>:<rule>`.
    pub component: String,
    pub definition: Value,
}

/// The tautology a condition is replaced with.
pub const TAUTOLOGY: &str = "1 > 0";

/// Every leave-one-rule-out variant of a v1 definition, in definition order.
#[must_use]
pub fn variants(def: &Value) -> Vec<Variant> {
    let mut out = Vec::new();
    if let Some(nodes) = def.get("nodes").and_then(Value::as_array) {
        for (i, n) in nodes.iter().enumerate() {
            let is_condition = n.get("type").and_then(Value::as_str) == Some("condition");
            let expr = n.get("expr").and_then(Value::as_str);
            let id = n.get("id").and_then(Value::as_str);
            if let (true, Some(expr), Some(id)) = (is_condition, expr, id) {
                if expr.trim() == TAUTOLOGY {
                    continue; // already neutral; a variant identical to the base
                }
                let mut d = def.clone();
                d["nodes"][i]["expr"] = json!(TAUTOLOGY);
                out.push(Variant { component: format!("node:{id}"), definition: d });
            }
        }
    }
    if let Some(rules) = def
        .get("exits")
        .filter(|e| e.get("mode").and_then(Value::as_str) == Some("rules"))
        .and_then(|e| e.get("rules"))
        .and_then(Value::as_array)
    {
        for (i, r) in rules.iter().enumerate() {
            let name = r.get("rule").and_then(Value::as_str).unwrap_or("rule");
            let mut d = def.clone();
            let kept: Vec<Value> = rules
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, v)| v.clone())
                .collect();
            d["exits"] = if kept.is_empty() {
                json!({"mode": "risk_layer_only"})
            } else {
                json!({"mode": "rules", "rules": kept})
            };
            out.push(Variant { component: format!("exit:{i}:{name}"), definition: d });
        }
    }
    out
}

/// What removing a component did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AblationVerdict {
    /// Removing it makes the strategy worse by more than noise.
    LoadBearing,
    /// Removing it changes nothing distinguishable from noise.
    Redundant,
    /// Removing it makes the strategy better by more than noise.
    Harmful,
    /// Too few aligned periods, or the two runs are not comparable.
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AblationResult {
    pub verdict: AblationVerdict,
    /// Mean per-period return of the baseline minus the variant's: positive when
    /// the component helps.
    pub mean_delta: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub n: usize,
    pub block: f64,
    pub n_boot: usize,
    pub reason: Option<String>,
}

const MIN_PERIODS: usize = 60;
const N_BOOT: usize = 2000;

/// Classify one variant against the baseline.
///
/// `base` and `variant` are per-period returns over the **same** periods; a
/// length mismatch is `Inconclusive`, never truncated to fit. The interval is a
/// 95% percentile interval from a stationary bootstrap of the paired
/// differences, block length Politis-White -- derived, not chosen.
#[must_use]
pub fn classify(base: &[f64], variant: &[f64], seed: u64) -> AblationResult {
    let inconclusive = |reason: String, n: usize| AblationResult {
        verdict: AblationVerdict::Inconclusive,
        mean_delta: f64::NAN,
        ci_low: f64::NAN,
        ci_high: f64::NAN,
        n,
        block: f64::NAN,
        n_boot: 0,
        reason: Some(reason),
    };
    if base.len() != variant.len() {
        return inconclusive(
            format!(
                "the baseline has {} periods and the variant {}; a paired comparison needs the same periods",
                base.len(),
                variant.len()
            ),
            base.len().min(variant.len()),
        );
    }
    let d: Vec<f64> = base.iter().zip(variant).map(|(a, b)| a - b).collect();
    if d.len() < MIN_PERIODS || d.iter().any(|x| !x.is_finite()) {
        return inconclusive(
            format!("fewer than {MIN_PERIODS} finite aligned periods"),
            d.len(),
        );
    }
    let n = d.len();
    let mean = d.iter().sum::<f64>() / n as f64;
    let block = politis_white_block(&d);
    let mut rng = DetRng::new(seed);
    let mut means: Vec<f64> = Vec::with_capacity(N_BOOT);
    for _ in 0..N_BOOT {
        let idx = stationary_indices(n, block, &mut rng);
        means.push(idx.iter().map(|&i| d[i]).sum::<f64>() / n as f64);
    }
    means.sort_by(f64::total_cmp);
    let lo = means[(0.025 * N_BOOT as f64) as usize];
    let hi = means[((0.975 * N_BOOT as f64) as usize).min(N_BOOT - 1)];
    let verdict = if lo > 0.0 {
        AblationVerdict::LoadBearing
    } else if hi < 0.0 {
        AblationVerdict::Harmful
    } else {
        AblationVerdict::Redundant
    };
    AblationResult {
        verdict,
        mean_delta: mean,
        ci_low: lo,
        ci_high: hi,
        n,
        block,
        n_boot: N_BOOT,
        reason: None,
    }
}

/// Per-period simple returns of an equity curve, keyed by the timestamp each
/// return was realised at. Non-finite or non-positive previous points are
/// skipped, as in `regime_search`.
#[must_use]
pub fn equity_returns(
    equity: &[(chrono::DateTime<chrono::Utc>, f64)],
) -> Vec<(chrono::DateTime<chrono::Utc>, f64)> {
    let mut out = Vec::with_capacity(equity.len().saturating_sub(1));
    for w in equity.windows(2) {
        let (prev, (t, e)) = (w[0].1, w[1]);
        if prev > 0.0 && prev.is_finite() && e.is_finite() {
            out.push((t, e / prev - 1.0));
        }
    }
    out
}

/// Classify a variant's equity curve against the baseline's.
///
/// The two curves are paired **by timestamp**, not by position: a variant that
/// trades differently can stop earlier or start later, and pairing by index would
/// compare different days and call the difference a finding. Only periods both
/// curves realised are compared; [`classify`] then refuses fewer than its minimum
/// and says so.
#[must_use]
pub fn classify_equity(
    baseline: &[(chrono::DateTime<chrono::Utc>, f64)],
    variant: &[(chrono::DateTime<chrono::Utc>, f64)],
    seed: u64,
) -> AblationResult {
    let b = equity_returns(baseline);
    let v: std::collections::BTreeMap<_, _> = equity_returns(variant).into_iter().collect();
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for (t, r) in b {
        if let Some(rv) = v.get(&t) {
            xs.push(r);
            ys.push(*rv);
        }
    }
    classify(&xs, &ys, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> Value {
        json!({
            "strategy_id": "s",
            "nodes": [
                {"id": "n1", "type": "condition", "expr": "bar('close') > bar('open')"},
                {"id": "n2", "type": "condition", "expr": "feature('rsi_14') < 30"},
                {"id": "n3", "type": "signal", "when": "n1", "emit": "long"}
            ],
            "exits": {"mode": "rules", "rules": [
                {"rule": "stop_loss", "fraction": "0.02"},
                {"rule": "take_profit", "fraction": "0.05"}
            ]}
        })
    }

    #[test]
    fn each_condition_and_exit_rule_gets_one_variant_and_signals_are_left_alone() {
        let v = variants(&def());
        let names: Vec<&str> = v.iter().map(|x| x.component.as_str()).collect();
        assert_eq!(names, ["node:n1", "node:n2", "exit:0:stop_loss", "exit:1:take_profit"]);
        assert_eq!(v[0].definition["nodes"][0]["expr"], TAUTOLOGY);
        // The other condition and the signal are untouched.
        assert_eq!(v[0].definition["nodes"][1], def()["nodes"][1]);
        assert_eq!(v[0].definition["nodes"][2], def()["nodes"][2]);
    }

    /// Removing the last exit rule must not leave an empty `rules` list, which
    /// the validator rejects as a disguised `risk_layer_only`.
    #[test]
    fn removing_the_last_exit_rule_switches_to_risk_layer_only() {
        let d = json!({"nodes": [], "exits": {"mode": "rules", "rules": [{"rule": "stop_loss", "fraction": "0.02"}]}});
        let v = variants(&d);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].definition["exits"], json!({"mode": "risk_layer_only"}));
    }

    #[test]
    fn an_already_neutral_condition_is_not_ablated_against_itself() {
        let d = json!({"nodes": [{"id": "n1", "type": "condition", "expr": "1 > 0"}]});
        assert!(variants(&d).is_empty());
    }

    fn noise(i: usize) -> f64 {
        let x = (i as f64 * 12.9898).sin() * 43758.5453;
        (x - x.floor() - 0.5) * 0.01
    }

    /// A component that carries real edge is load-bearing; one that does nothing
    /// is redundant, not "slightly helpful"; one that costs money is harmful.
    #[test]
    fn the_three_verdicts_each_need_a_delta_outside_the_noise() {
        let base: Vec<f64> = (0..600).map(|i| 0.0008 + noise(i)).collect();
        let helped: Vec<f64> = (0..600).map(noise).collect(); // base earns 0.0008 more
        assert_eq!(classify(&base, &helped, 1).verdict, AblationVerdict::LoadBearing);
        assert_eq!(classify(&base, &base, 1).verdict, AblationVerdict::Redundant);
        assert_eq!(classify(&helped, &base, 1).verdict, AblationVerdict::Harmful);
    }

    /// A small point delta inside the interval is not a finding.
    #[test]
    fn a_point_delta_inside_the_noise_is_redundant() {
        let a: Vec<f64> = (0..300).map(noise).collect();
        let b: Vec<f64> = (0..300).map(|i| noise(i + 1000)).collect();
        let r = classify(&a, &b, 7);
        assert_eq!(r.verdict, AblationVerdict::Redundant, "{r:?}");
        assert!(r.ci_low < 0.0 && r.ci_high > 0.0);
    }

    #[test]
    fn mismatched_or_short_series_are_inconclusive_not_truncated() {
        let a = vec![0.0; 100];
        assert_eq!(classify(&a, &a[..90], 1).verdict, AblationVerdict::Inconclusive);
        let short = vec![0.0; 10];
        assert_eq!(classify(&short, &short, 1).verdict, AblationVerdict::Inconclusive);
    }

    #[test]
    fn the_same_seed_gives_the_same_interval() {
        let base: Vec<f64> = (0..400).map(|i| 0.0003 + noise(i)).collect();
        let var: Vec<f64> = (0..400).map(|i| noise(i + 5)).collect();
        assert_eq!(classify(&base, &var, 9), classify(&base, &var, 9));
    }

    fn curve(r: &[f64]) -> Vec<(chrono::DateTime<chrono::Utc>, f64)> {
        use chrono::TimeZone;
        let t0 = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let mut e = 100.0;
        let mut out = vec![(t0, e)];
        for (i, x) in r.iter().enumerate() {
            e *= 1.0 + x;
            out.push((t0 + chrono::Duration::days(i as i64 + 1), e));
        }
        out
    }

    /// Pairing is by timestamp: a variant that starts later is compared on the
    /// days both realised, not index against index.
    #[test]
    fn equity_curves_are_paired_by_timestamp() {
        let base_r: Vec<f64> = (0..400).map(|i| 0.0008 + noise(i)).collect();
        let var_r: Vec<f64> = (0..400).map(noise).collect();
        let base = curve(&base_r);
        let var = curve(&var_r);
        let r = classify_equity(&base, &var, 1);
        assert_eq!(r.verdict, AblationVerdict::LoadBearing, "{r:?}");
        assert_eq!(r.n, 400);
        // Drop the variant's first 100 periods: the comparison shrinks to the
        // shared days instead of shifting by a hundred.
        let late: Vec<_> = var[100..].to_vec();
        let r2 = classify_equity(&base, &late, 1);
        assert_eq!(r2.n, 300, "{r2:?}");
        // No overlap at all is inconclusive, not an empty-mean verdict.
        let none = classify_equity(&base[..5], &var[300..], 1);
        assert_eq!(none.verdict, AblationVerdict::Inconclusive);
    }
}
