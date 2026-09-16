//! The honesty arithmetic (AGENT-006 §5.4; Set S tasks S-3.6, S-3.7).
//!
//! Four tools, and none of them computes anything this crate owns. Every number
//! here is produced by the code the **gate** runs — `backtest::stats` for the
//! deflated Sharpe, `ledger::neff::NEff` for the trial count, the experiment
//! vault for the holdout — because a tool that agreed with the gate by
//! coincidence is a tool that will disagree with it eventually, and the
//! disagreement would surface as a strategy that passed an audit and failed a
//! promotion for reasons nobody could reconstruct.
//!
//! # Why the trial count cannot come from the caller
//!
//! The deflated Sharpe is the observed Sharpe discounted by how many things were
//! tried. An agent that supplied its own trial count could supply a small one,
//! and the arithmetic that exists to expose selection bias would be reporting
//! whatever the selector preferred. So `trials` is `NEff`, which has no public
//! constructor and can only be minted by a ledger (§12.4, INV-22) — the same
//! value `gate3` uses, obtained the same way.
//!
//! # The holdout has one door
//!
//! `spend_holdout_look` is the only tool that reaches holdout rows, it carries
//! the `SealedHoldout` envelope so it suspends for a human on every tier, and it
//! is capped at one call per run. `check_holdout_budget` is how an agent asks
//! what it has left *without* spending it — which is the whole point, because an
//! agent that can only discover the budget by exhausting it will exhaust it.

use serde_json::{json, Value};

use crate::spec::{Envelope, ToolSpec, TrialKind};
use crate::taxonomy::Risk;
use crate::ApiClient;

fn missing(field: &str) -> Value {
    json!({ "error": "missing_field", "field": field })
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

// ── S-3.6: the discount for having looked ────────────────────────────────────

pub async fn deflated_sharpe(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    post(
        api,
        "/api/agent/audit/deflated-sharpe",
        json!({
            "run_id": run_id,
            "experiment_id": params.get("experiment_id").cloned(),
        }),
    )
    .await
}

pub async fn min_track_record(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    // No `profile` is forwarded, and the schema has no field for one: which gate
    // profile applies is the platform's decision, because a caller who names the
    // profile names the one with no calendar floor. See the route.
    post(
        api,
        "/api/agent/audit/min-track-record",
        json!({ "run_id": run_id }),
    )
    .await
}

// ── S-3.7: the one door ──────────────────────────────────────────────────────

pub async fn check_holdout_budget(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/holdout-budget",
        json!({ "experiment_id": experiment_id }),
    )
    .await
}

pub async fn spend_holdout_look(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    let Some(reason) = str_field(params, "reason") else {
        return missing("reason");
    };
    post(
        api,
        "/api/agent/audit/holdout-look",
        json!({ "experiment_id": experiment_id, "reason": reason }),
    )
    .await
}

// ── S-3.9: what a family of candidates is worth ──────────────────────────────

pub async fn probability_of_overfit(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/probability-of-overfit",
        json!({
            "experiment_id": experiment_id,
            "n_periods": params.get("n_periods").cloned(),
        }),
    )
    .await
}

pub async fn family_wise_test(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/family-wise",
        json!({
            "experiment_id": experiment_id,
            "alpha": params.get("alpha").cloned(),
            "seed": params.get("seed").cloned(),
        }),
    )
    .await
}

// ── S-3.8: does the report match what the platform logged ────────────────────

pub async fn metric_integrity(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    let Some(metric) = str_field(params, "metric") else {
        return missing("metric");
    };
    let Some(claimed) = params.get("claimed_value").and_then(Value::as_f64) else {
        return missing("claimed_value");
    };
    post(
        api,
        "/api/agent/audit/metric-integrity",
        json!({ "run_id": run_id, "metric": metric, "claimed_value": claimed }),
    )
    .await
}

// ── Declarations ─────────────────────────────────────────────────────────────

/// One declaration per tool (AGENT-006 §2.3).
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "deflated_sharpe",
            "audit",
            Risk::Read,
            "Discounts a run's Sharpe ratio by how many strategies were tried to find it. \
             Refuses when the run is not sealed into a hypothesis family, because an unsealed \
             family has no honest denominator.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": {"type": "string"},
                    "experiment_id": {"type": "string", "description": "the family, if the run belongs to one"}
                }
            }),
        )
        .internal()
        .evidence(&["run_id", "trial_id"])
        .summary(
            "deflated Sharpe {deflated_sharpe} against a raw {sharpe} over {n_obs} observations, \
             discounted for {trials} effective trials; {verdict}",
            &[
                ("deflated_sharpe", ""),
                ("sharpe", ""),
                ("n_obs", "obs"),
                ("trials", "trials"),
                ("verdict", ""),
            ],
        )
        .notes("deflated_sharpe"),
        ToolSpec::new(
            "min_track_record",
            "audit",
            Risk::Read,
            "How long a track record has to be before this Sharpe could be distinguished from \
             luck. A blocking precondition for promotion, not an advisory number.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": { "run_id": {"type": "string"} }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .summary(
            "needs {required_years} years to distinguish this Sharpe from luck; the run covers \
             {observed_years}; {verdict}",
            &[
                ("required_years", "years"),
                ("observed_years", "years"),
                ("verdict", ""),
            ],
        ),
        ToolSpec::new(
            "metric_integrity",
            "audit",
            Risk::Read,
            "Re-derives a metric from the platform's own record of a run and diffs it against              the value being reported. Run it on any number before putting it in a finding.",
            json!({
                "type": "object",
                "required": ["run_id", "metric", "claimed_value"],
                "properties": {
                    "run_id": {"type": "string"},
                    "metric": {
                        "type": "string",
                        "enum": ["cagr", "total_return", "sharpe", "sortino", "calmar",
                                 "detrended_sharpe", "max_drawdown", "profit_factor", "expectancy"]
                    },
                    "claimed_value": {"type": "number", "description": "the number about to be reported"}
                }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .summary(
            "reported {claimed_value} against a recomputed {platform_value} ({metric});              {verdict}",
            &[
                ("claimed_value", ""),
                ("platform_value", ""),
                ("metric", ""),
                ("verdict", ""),
            ],
        )
        .notes("metric_integrity"),
        ToolSpec::new(
            "probability_of_overfit",
            "audit",
            Risk::Read,
            "How often this experiment's in-sample-best configuration underperformed out of              sample, by combinatorially-symmetric cross-validation. Refuses below ten distinct              configurations, where the statistic has too few rankings to take a value.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "n_periods": {"type": "integer", "minimum": 2, "maximum": 16, "default": 8}
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .summary(
            "PBO {probability_of_overfit} over {n_configs} configurations and {n_periods}              periods; {verdict}",
            &[
                ("probability_of_overfit", ""),
                ("n_configs", "configs"),
                ("n_periods", "periods"),
                ("verdict", ""),
            ],
        )
        .notes("probability_of_overfit"),
        ToolSpec::new(
            "family_wise_test",
            "audit",
            Risk::Read,
            "Which candidates in this experiment survive once you account for having generated              the whole family, by Romano-Wolf stepdown over their own return series. Use it              before reporting the best result of a sweep.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "alpha": {"type": "number", "minimum": 0.001, "maximum": 0.2, "default": 0.05},
                    "seed": {"type": "integer", "description": "fixed for reproducibility; defaults to 0"}
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .summary(
            "{n_survivors} of {n_candidates} candidates survive at alpha={alpha}; {verdict}",
            &[
                ("n_survivors", ""),
                ("n_candidates", "candidates"),
                ("alpha", ""),
                ("verdict", ""),
            ],
        )
        .notes("family_wise_test"),
        ToolSpec::new(
            "check_holdout_budget",
            "audit",
            Risk::Read,
            "Whether this experiment's sealed holdout has been spent, and what was recorded \
             when. Ask this before planning work that depends on a holdout look.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": { "experiment_id": {"type": "string"} }
            }),
        )
        .internal()
        .summary(
            "holdout {state}; {looks_taken} of {looks_allowed} looks used; {can_run_note}",
            &[
                ("state", ""),
                ("looks_taken", "looks"),
                ("looks_allowed", "looks"),
                ("can_run_note", ""),
            ],
        ),
        ToolSpec::new(
            "spend_holdout_look",
            "audit",
            Risk::Read,
            "Evaluates the candidate against the sealed holdout. This is the only tool that \
             can read holdout data, it can be done once per experiment ever, and it needs a \
             human's approval.",
            json!({
                "type": "object",
                "required": ["experiment_id", "reason"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "reason": {
                        "type": "string",
                        "description": "why this candidate is ready — recorded permanently in the access log"
                    }
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .envelope(Envelope::SealedHoldout)
        // Bounded independently of the envelope. Approval is a human saying yes
        // once; this is the arithmetic refusing a second attempt inside one run
        // even if a human would have said yes again.
        .max_calls_per_run(1)
        .spends_trial(TrialKind::None)
        .summary(
            "holdout evaluated: {verdict}; the experiment is now {state} and this door is \
             closed permanently",
            &[("verdict", ""), ("state", "")],
        )
        .notes("spend_holdout_look"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tier's whole claim in one assertion.
    #[test]
    fn spending_the_holdout_needs_approval_and_cannot_be_repeated() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "spend_holdout_look")
            .expect("declared");
        assert_eq!(spec.envelope, Envelope::SealedHoldout);
        assert!(spec.envelope.requires_approval());
        assert_eq!(spec.max_calls_per_run, Some(1));
    }

    /// Asking what is left must never be the thing that spends it.
    #[test]
    fn checking_the_budget_is_free() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "check_holdout_budget")
            .expect("declared");
        assert_eq!(spec.envelope, Envelope::None);
        assert_eq!(spec.spends_trial, TrialKind::None);
        assert_eq!(spec.risk, Risk::Read);
    }

    /// `spend_holdout_look` is the only tool in the whole catalogue that may
    /// reach holdout rows, and that is asserted against the catalogue rather
    /// than against this module — a second door added elsewhere is exactly the
    /// failure this is for.
    #[test]
    fn the_holdout_has_exactly_one_door() {
        let doors: Vec<&str> = crate::spec::catalog()
            .iter()
            .filter(|s| s.envelope == Envelope::SealedHoldout)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            doors,
            vec!["spend_holdout_look"],
            "another tool carries the SealedHoldout envelope. If that is deliberate it needs \
             an ADR; a holdout with two doors has been looked at twice."
        );
    }
}
