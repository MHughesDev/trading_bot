//! The research lifecycle, as tools (AGENT-013 §5, §7.2; Set S tasks S-10.3,
//! S-10.4).
//!
//! # Why these are tools and `evaluate` is not
//!
//! The lifecycle is intake → ground → preregister → hypothesise → design →
//! execute → **evaluate** → diagnose → refine → report, and one of those steps is
//! deliberately not on this list.
//!
//! A run may *commit* to a falsifier, *bind* a design and *ask* for a
//! refinement. It may not decide whether its own hypothesis was supported. That
//! is §6.2's rule and the reason for it is the same one running through this
//! whole system: a verdict the model writes is a verdict the model can rewrite,
//! and a run that can grade itself will grade itself well. `evaluate` is a
//! platform function that reads the tickets.
//!
//! # These tools spend nothing
//!
//! Sealing a registration is not a look. None of them declares a trial, and
//! `preregister` is the tool the trial gate refuses *on behalf of* — charging for
//! it would put a price on the one step that makes the rest of the run mean
//! anything.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

fn missing(field: &str, why: &str) -> Value {
    json!({ "error": "missing_field", "field": field, "fix": why })
}

fn text(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Pre-flight on a registration, so the agent gets the reason rather than a
/// deserialisation error from the boundary.
///
/// The boundary checks again — `ledger::prereg::Draft` has no serde defaults, so
/// a body missing `delta_practical` cannot deserialise at all. This is a better
/// message, not a second gate, and a check that lived only here would be a check
/// an HTTP client skips.
///
/// # Errors
/// The typed refusal, with the field and the reason it has no default.
pub fn check_registration(params: &Value) -> Result<(), Value> {
    for (field, why) in [
        (
            "falsifier",
            "the result that would refute your hypothesis, written before the first trial. \
             A hypothesis with no falsifier can only ever be confirmed, and a falsifier \
             written after a result is not a falsifier.",
        ),
        (
            "primary_hypothesis",
            "one sentence on what you think is true, stated so it could be wrong.",
        ),
        (
            "null_family",
            "what you are better *than*. Chosen before the result, or the comparison is \
             whichever null happens to flatter it.",
        ),
        (
            "cost_model_version",
            "the costs the result will be judged under. Pinned, because a result that \
             survives one cost model and not another is two different results.",
        ),
        (
            "gate_profile",
            "the gate profile this run is judged under (INV-23).",
        ),
    ] {
        if text(params, field).is_none() {
            return Err(missing(field, why));
        }
    }

    let delta = params.get("delta_practical").and_then(Value::as_f64);
    match delta {
        None => {
            return Err(missing(
                "delta_practical",
                "the smallest improvement that would change a decision. It has no default \
                 on purpose: a default here is a decision nobody made about what is worth \
                 acting on, inherited silently by every run after it.",
            ))
        }
        Some(d) if !(d.is_finite() && d > 0.0) => {
            return Err(json!({
                "error": "invalid_delta_practical",
                "value": d,
                "fix": "it must be finite and positive. Zero means every difference matters, \
                        which is the same as none of them mattering."
            }))
        }
        Some(_) => {}
    }

    let window = params.get("window");
    let (start, end) = (
        window.and_then(|w| w.get("start")).and_then(Value::as_str),
        window.and_then(|w| w.get("end")).and_then(Value::as_str),
    );
    match (start, end) {
        (Some(s), Some(e)) if s <= e => {}
        (Some(s), Some(e)) => {
            return Err(json!({
                "error": "backwards_window",
                "start": s,
                "end": e,
                "fix": "the window runs from start to end"
            }))
        }
        _ => {
            return Err(missing(
                "window",
                "`{start, end}` — the period the claim is about.",
            ))
        }
    }

    if params.get("objective").map_or(true, Value::is_null) {
        return Err(missing(
            "objective",
            "a vector and its constraints — `{\"maximize\": [...], \"subject_to\": {...}}` — \
             never a scalar score (ADR-012).",
        ));
    }
    if params.get("benchmark").map_or(true, Value::is_null) {
        return Err(missing(
            "benchmark",
            "better than what, decided before the result.",
        ));
    }
    Ok(())
}

/// Pre-flight on a hypothesis (§6.2, S-10.4).
///
/// The falsifier must name something the platform can evaluate. A falsifier that
/// names nothing evaluable reads like a commitment and is not one: nothing can
/// ever check it, so it can never refute anything, and the run keeps the
/// *appearance* of pre-registration with none of the content.
///
/// # Errors
/// The typed refusal, naming the missing part.
pub fn check_hypothesis(params: &Value, vocabulary: &[&str]) -> Result<(), Value> {
    let Some(falsifier) = text(params, "falsifier") else {
        return Err(missing(
            "falsifier",
            "the result that would refute this hypothesis.",
        ));
    };
    let metric = vocabulary
        .iter()
        .find(|m| falsifier.contains(*m))
        .map(|m| (*m).to_string());
    let Some(metric) = metric else {
        return Err(json!({
            "error": "falsifier_names_no_metric",
            "falsifier": falsifier,
            "known_metrics": vocabulary,
            "fix": "name a metric the platform computes. A falsifier nothing can evaluate \
                    reads like a commitment and is not one: it can never refute anything, \
                    so the run keeps the appearance of pre-registration with none of it."
        }));
    };
    // A metric with no comparator is a noun, not a claim. `sharpe_after_costs`
    // says nothing that could turn out false; `sharpe_after_costs <= 0` does.
    const COMPARATORS: &[&str] = &["<=", ">=", "<", ">", "==", "!=", " below ", " above "];
    if !COMPARATORS.iter().any(|c| falsifier.contains(c)) {
        return Err(json!({
            "error": "falsifier_names_no_comparator",
            "falsifier": falsifier,
            "metric": metric,
            "comparators": COMPARATORS,
            "fix": "say which way it would have to go. A metric with no comparator is a \
                    noun, not a claim: `sharpe_after_costs` cannot turn out false and \
                    `sharpe_after_costs <= 0` can."
        }));
    }
    match text(params, "role").as_deref() {
        None | Some("primary" | "secondary" | "exploratory") => Ok(()),
        Some(other) => Err(json!({
            "error": "unknown_role",
            "role": other,
            "roles": ["primary", "secondary", "exploratory"],
            "fix": "secondary and exploratory results cap at `suggestive`; only a primary \
                    hypothesis can produce an established finding."
        })),
    }
}

async fn post(api: &crate::ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `preregister` — seal what this run is committing to (§7.2).
pub async fn preregister(api: &crate::ApiClient, params: &Value) -> Value {
    if let Err(e) = check_registration(params) {
        return e;
    }
    post(api, "/api/lifecycle/preregistrations", params.clone()).await
}

/// `design` — bind a hypothesis to what will test it, and check the window can
/// settle it (§5.5, §6.3).
///
/// The power check runs **before any trial**. A run that pre-registers an effect
/// size and then looks for it in a window that could not show it spends every
/// trial it has and learns that at the end.
pub async fn design(api: &crate::ApiClient, params: &Value) -> Value {
    for (field, why) in [
        ("hypothesis_id", "which hypothesis this design tests"),
        (
            "strategy_version",
            "the exact version under test. A design bound to `latest` is a design              whose result cannot be reproduced.",
        ),
        ("null_family", "what the result is compared against"),
    ] {
        if text(params, field).is_none() {
            return missing(field, why);
        }
    }
    post(api, "/api/lifecycle/designs", params.clone()).await
}

/// `hypothesise` — state one thing and the result that would refute it (§6.2).
///
/// The vocabulary comes from the platform, not from a constant here: the metrics
/// the falsifier may name are the ones `mlops.trial_statistic` records, and a
/// second list would be a second answer to "what can this platform evaluate".
pub async fn hypothesise(api: &crate::ApiClient, params: &Value) -> Value {
    let vocabulary = match api.get("/api/lifecycle/metrics").await {
        Ok(v) => v
            .get("metrics")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        Err(e) => return e.to_tool_error(),
    };
    let refs: Vec<&str> = vocabulary.iter().map(String::as_str).collect();
    if let Err(e) = check_hypothesis(params, &refs) {
        return e;
    }
    post(api, "/api/lifecycle/hypotheses", params.clone()).await
}

/// The catalogue entries for this namespace.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "preregister",
            "lifecycle",
            // A write: it seals a row the run is then held to. Not destructive —
            // an amendment supersedes rather than overwrites.
            Risk::Write,
            "COMMIT to what you are testing, before the first trial: objective, benchmark, \
             delta_practical, window, null family, gate profile, and the falsifier for your \
             primary hypothesis. Trial-spending tools refuse until this is sealed.",
            json!({
                "type": "object",
                "properties": {
                    "objective": { "type": "object", "description": "A vector and its constraints, never a scalar." },
                    "benchmark": { "type": "object", "description": "Better than what." },
                    "delta_practical": { "type": "number", "exclusiveMinimum": 0.0 },
                    "window": {
                        "type": "object",
                        "properties": { "start": { "type": "string" }, "end": { "type": "string" } },
                        "required": ["start", "end"]
                    },
                    "cost_model_version": { "type": "string" },
                    "null_family": { "type": "string" },
                    "trial_budget": { "type": "integer", "minimum": 1 },
                    "gate_profile": { "type": "string" },
                    "primary_hypothesis": { "type": "string" },
                    "falsifier": { "type": "string", "description": "The result that would refute it." },
                    "supersedes": { "type": "string", "description": "Amending? The id this replaces." }
                },
                "required": [
                    "objective", "benchmark", "delta_practical", "window",
                    "cost_model_version", "null_family", "trial_budget",
                    "gate_profile", "primary_hypothesis", "falsifier"
                ]
            }),
        )
        .internal()
        .evidence(&["prereg_id", "prereg_hash"])
        .notes("preregister"),
        ToolSpec::new(
            "hypothesise",
            "lifecycle",
            Risk::Write,
            "STATE one thing you think is true and the result that would refute it. The \
             falsifier must name a metric the platform computes and which way it would have \
             to go.",
            json!({
                "type": "object",
                "properties": {
                    "statement": { "type": "string", "description": "What you think is true." },
                    "falsifier": {
                        "type": "string",
                        "description": "A metric and a comparator, e.g. `sharpe_after_costs <= 0`."
                    },
                    "role": {
                        "type": "string",
                        "enum": ["primary", "secondary", "exploratory"],
                        "description": "Secondary and exploratory results cap at `suggestive`."
                    },
                    "family_id": { "type": "string" }
                },
                "required": ["statement", "falsifier"]
            }),
        )
        .internal()
        .evidence(&["hypothesis_id"]),
        ToolSpec::new(
            "design",
            "lifecycle",
            Risk::Write,
            "BIND a hypothesis to the strategy version, split and null that will test it.              Refused if the pre-registered window is too short to distinguish the effect              you registered from zero — before any trial is spent.",
            json!({
                "type": "object",
                "properties": {
                    "hypothesis_id": { "type": "string" },
                    "strategy_version": {
                        "type": "string",
                        "description": "An exact version. `latest` is not reproducible."
                    },
                    "split_spec_id": { "type": "string" },
                    "null_family": { "type": "string" },
                    "timeframe": {
                        "type": "string",
                        "description": "Bar size, e.g. 1h. Decides how many observations the window holds."
                    }
                },
                "required": ["hypothesis_id", "strategy_version", "null_family"]
            }),
        )
        .internal()
        .evidence(&["design_id"])
        .notes("design"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &[&str] = &["sharpe_after_costs", "max_drawdown", "hit_rate"];

    fn registration() -> Value {
        json!({
            "objective": {"maximize": ["sharpe_after_costs"]},
            "benchmark": {"null_family": "block_bootstrap"},
            "delta_practical": 0.15,
            "window": {"start": "2024-01-01", "end": "2024-06-30"},
            "cost_model_version": "costs_v3",
            "null_family": "block_bootstrap",
            "trial_budget": 20,
            "gate_profile": "strict_v1",
            "primary_hypothesis": "1h momentum survives costs on BTC-USD",
            "falsifier": "sharpe_after_costs <= 0 out of sample"
        })
    }

    #[test]
    fn a_complete_registration_passes_pre_flight() {
        check_registration(&registration()).expect("complete");
    }

    /// The field the whole object exists for, and the reason in the error.
    #[test]
    fn a_registration_with_no_falsifier_says_why_that_matters() {
        let mut r = registration();
        r["falsifier"] = json!("");
        let err = check_registration(&r).expect_err("refused");
        assert_eq!(err["field"], "falsifier");
        assert!(
            err["fix"].as_str().unwrap().contains("after a result is not"),
            "{err}"
        );
    }

    /// `delta_practical` has no default, and the error says why not.
    #[test]
    fn delta_practical_is_refused_rather_than_defaulted() {
        let mut r = registration();
        r.as_object_mut().unwrap().remove("delta_practical");
        let err = check_registration(&r).expect_err("refused");
        assert_eq!(err["field"], "delta_practical");
        assert!(
            err["fix"]
                .as_str()
                .unwrap()
                .contains("a decision nobody made"),
            "{err}"
        );

        r["delta_practical"] = json!(0.0);
        let err = check_registration(&r).expect_err("zero is refused too");
        assert_eq!(err["error"], "invalid_delta_practical");
    }

    /// A scalar objective is refused: ADR-012.
    #[test]
    fn an_objective_must_be_present() {
        let mut r = registration();
        r["objective"] = Value::Null;
        let err = check_registration(&r).expect_err("refused");
        assert_eq!(err["field"], "objective");
        assert!(err["fix"].as_str().unwrap().contains("never a scalar score"));
    }

    /// A falsifier that names nothing evaluable is refused, with the vocabulary.
    #[test]
    fn a_falsifier_naming_no_metric_is_refused_with_the_list() {
        let err = check_hypothesis(
            &json!({"falsifier": "it does not work very well"}),
            VOCAB,
        )
        .expect_err("refused");
        assert_eq!(err["error"], "falsifier_names_no_metric");
        assert_eq!(err["known_metrics"], json!(VOCAB));
        assert!(
            err["fix"].as_str().unwrap().contains("can never refute"),
            "the reason is the point: {err}"
        );
    }

    /// A metric with no comparator is a noun, not a claim.
    #[test]
    fn a_falsifier_needs_a_direction() {
        let err = check_hypothesis(&json!({"falsifier": "sharpe_after_costs"}), VOCAB)
            .expect_err("refused");
        assert_eq!(err["error"], "falsifier_names_no_comparator");
        check_hypothesis(&json!({"falsifier": "sharpe_after_costs <= 0"}), VOCAB)
            .expect("a metric and a direction is a claim");
        check_hypothesis(
            &json!({"falsifier": "max_drawdown above 0.35 on the holdout"}),
            VOCAB,
        )
        .expect("prose comparators count");
    }

    /// Only a primary hypothesis can produce an established finding.
    #[test]
    fn the_role_is_from_a_closed_list() {
        for role in ["primary", "secondary", "exploratory"] {
            check_hypothesis(
                &json!({"falsifier": "sharpe_after_costs <= 0", "role": role}),
                VOCAB,
            )
            .unwrap_or_else(|e| panic!("{role}: {e}"));
        }
        let err = check_hypothesis(
            &json!({"falsifier": "sharpe_after_costs <= 0", "role": "definitive"}),
            VOCAB,
        )
        .expect_err("refused");
        assert_eq!(err["error"], "unknown_role");
    }

    /// Neither tool spends a trial.
    ///
    /// `preregister` is the tool the trial gate refuses on behalf of; charging
    /// for it would put a price on the one step that makes the rest of the run
    /// mean anything.
    #[test]
    fn sealing_a_commitment_is_not_a_look() {
        for spec in specs() {
            assert_eq!(
                spec.spends_trial,
                crate::spec::TrialKind::None,
                "`{}` declares a trial",
                spec.name
            );
        }
    }

    /// Both are ring 1: they write platform artifacts and reach no further.
    #[test]
    fn the_lifecycle_tools_reach_no_further_than_the_platform() {
        for spec in specs() {
            assert_eq!(
                spec.ring(),
                crate::spec::RING_PLATFORM_ARTIFACTS,
                "`{}`",
                spec.name
            );
        }
    }
}
