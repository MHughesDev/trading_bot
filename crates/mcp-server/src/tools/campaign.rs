//! The campaign door: `define_campaign` and `get_campaign` (AGENT-013 §3, §8.5;
//! Set S task S-10.11).
//!
//! # What a run does *not* send
//!
//! Six of a campaign's DEFINE facts — the objective, the benchmark,
//! `delta_practical`, the gate profile, the hypothesis and the trial budget —
//! are absent from this schema, and their absence is the point. The run has
//! already sealed them in its pre-registration, and §8.5 says the registration
//! *is* the campaign DEFINE: one object. Asking for them again would be asking a
//! model to retype, under no obligation to match, the one commitment it is being
//! held to; the first divergence would produce a campaign judged against a
//! protocol its run never registered, with every statistic computing normally.
//!
//! So `preregister` is a precondition rather than a companion step, and the
//! refusal below says so with the tool name in it.
//!
//! # What it does send
//!
//! What a registration has no place for: a slug, the approval threshold, the
//! three budget dimensions that are not trials, and the space to search. The
//! exploration floor is **not** here and cannot be: INV-21 puts it outside every
//! tool schema, and a campaign that could choose its own floor could choose not
//! to explore.
//!
//! # `optimise` is hidden below `local_high`
//!
//! Driving a campaign means planning a sequence of structural moves against a
//! budget, and AGENT-013 §11 puts that at `local_high` and above. The exposure
//! filter reads `Profile::workflows`; this tool carries no tier logic of its own,
//! because a second place that decided it would be a second answer.

use serde_json::{json, Value};

use crate::spec::Envelope;
use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

fn missing(field: &str, why: &str) -> Value {
    json!({ "error": "missing_field", "field": field, "fix": why })
}

fn number(params: &Value, key: &str) -> Option<f64> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

/// Pre-flight on a campaign definition.
///
/// The boundary checks again — `CampaignDefinition::validate` and then the
/// database — so this is a better message rather than a second gate. Its job is
/// to name the field and say why it has no default, because a refusal a model
/// cannot act on costs a whole step.
///
/// # Errors
/// The typed refusal, with the field and the reason.
pub fn check_definition(params: &Value) -> Result<(), Value> {
    let slug = params
        .get("slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if slug.is_none() {
        return Err(missing(
            "slug",
            "a short name for this campaign, unique within your tenant. It is how the \
             campaign is referred to afterwards, including by you.",
        ));
    }

    for (field, why) in [
        (
            "approval_spend_usd",
            "how much this campaign may spend in one action before a human is asked (§15). \
             It has no default on purpose: a platform-wide one would be a decision nobody \
             made about somebody's money. Zero — ask about everything — is a real answer.",
        ),
        (
            "gpu_hours",
            "the GPU budget. Every budget dimension is REQUIRED at DEFINE: a budget with an \
             unstated dimension is not a budget.",
        ),
        (
            "usd",
            "the money budget. REQUIRED at DEFINE, like every other dimension.",
        ),
        (
            "wall_clock_hours",
            "how long this campaign may run. REQUIRED at DEFINE.",
        ),
    ] {
        match number(params, field) {
            None => return Err(missing(field, why)),
            Some(v) if v < 0.0 => {
                return Err(json!({
                    "error": "negative_budget",
                    "field": field,
                    "value": v,
                    "fix": "a budget dimension is zero or more"
                }))
            }
            Some(_) => {}
        }
    }

    if number(params, "wall_clock_hours").is_some_and(|v| v <= 0.0) {
        return Err(json!({
            "error": "no_time_to_run",
            "field": "wall_clock_hours",
            "fix": "a campaign with no wall-clock budget cannot run a single phase"
        }));
    }

    // The trial budget is the registration's, and there is exactly one of it.
    if params.get("trial_budget").is_some() || params.get("max_trials").is_some() {
        return Err(json!({
            "error": "budget_already_registered",
            "fix": "the trial budget comes from your pre-registration, which is this campaign's \
                    DEFINE. Two numbers would give the trial counter two ceilings and the lower \
                    one would win by accident. Amend the registration if the budget is wrong."
        }));
    }
    // Same argument, for the five other fields the registration already fixes.
    for field in [
        "objective",
        "benchmark",
        "delta_practical",
        "gate_profile",
        "gates_profile",
        "hypothesis",
    ] {
        if params.get(field).is_some() {
            return Err(json!({
                "error": "already_registered",
                "field": field,
                "fix": "this is part of your pre-registration, which IS the campaign's DEFINE \
                        (AGENT-013 §8.5). Restating it here would let the campaign search under \
                        a protocol you did not register. Amend the registration instead."
            }));
        }
    }
    if params.get("exploration_floor").is_some() {
        return Err(json!({
            "error": "floor_is_not_yours",
            "fix": "the exploration floor is the platform's and is not settable from a tool \
                    (INV-21). A campaign that could choose its own floor could choose not to \
                    explore, which is the one thing the floor exists to prevent."
        }));
    }
    Ok(())
}

async fn post(api: &crate::ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `define_campaign` — turn this run's registration into a running campaign.
pub async fn define_campaign(api: &crate::ApiClient, params: &Value) -> Value {
    if let Err(e) = check_definition(params) {
        return e;
    }
    post(api, "/api/lifecycle/campaigns", params.clone()).await
}

/// `get_campaign` — where a campaign is, and what it has produced.
pub async fn get_campaign(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(id) = params.get("campaign_id").and_then(Value::as_str) else {
        return missing("campaign_id", "the campaign `define_campaign` returned");
    };
    match api.get(&format!("/api/lifecycle/campaigns/{id}")).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// The catalogue entries for this namespace.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "define_campaign",
            "research",
            // A write: it creates an immutable campaign row and a job that will
            // spend the registration's trial budget. Not `Promotion` — nothing
            // here reaches capital; the gates and the proposal door still stand
            // between a campaign's winner and paper.
            Risk::Write,
            "Turn your sealed pre-registration into a running search campaign. The registration \
             IS the campaign's definition, so send only the slug, the budgets and the space to \
             search. Requires `preregister` first.",
            json!({
                "type": "object",
                "properties": {
                    "slug": {
                        "type": "string",
                        "description": "A short name, unique within your tenant."
                    },
                    "approval_spend_usd": {
                        "type": "number",
                        "minimum": 0.0,
                        "description": "Spending above this in one action asks a human. No default."
                    },
                    "gpu_hours": { "type": "number", "minimum": 0.0 },
                    "usd": { "type": "number", "minimum": 0.0 },
                    "wall_clock_hours": { "type": "number", "exclusiveMinimum": 0.0 },
                    "search_space": {
                        "type": "object",
                        "description": "The parameter space to search. Each generation's worker may propose ranges inside it."
                    }
                },
                "required": [
                    "slug", "approval_spend_usd", "gpu_hours", "usd", "wall_clock_hours"
                ]
            }),
        )
        .internal()
        // §15's approval envelope: a campaign commits a trial budget and a
        // dollar budget in one call, which is the shape of action ADR-P2-20 puts
        // behind a human.
        .envelope(Envelope::Spend),
        ToolSpec::new(
            "get_campaign",
            "research",
            Risk::Read,
            "Where a campaign is: its phase, how many trials it has counted, what it proposed \
             most recently, and whether it has ended.",
            json!({
                "type": "object",
                "properties": {
                    "campaign_id": { "type": "string" }
                },
                "required": ["campaign_id"]
            }),
        )
        .internal()
        .evidence(&["campaign_id"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Value {
        json!({
            "slug": "momentum-4h",
            "approval_spend_usd": 5.0,
            "gpu_hours": 2.0,
            "usd": 25.0,
            "wall_clock_hours": 48.0
        })
    }

    #[test]
    fn a_complete_definition_passes_preflight() {
        check_definition(&good()).expect("complete");
    }

    /// Zero is a real answer to "how much before you ask a human", and refusing
    /// it would push callers toward a number they did not mean.
    #[test]
    fn an_approval_threshold_of_zero_is_accepted() {
        let mut p = good();
        p["approval_spend_usd"] = json!(0.0);
        check_definition(&p).expect("zero means ask about everything");
    }

    #[test]
    fn every_budget_dimension_is_required() {
        for field in ["approval_spend_usd", "gpu_hours", "usd", "wall_clock_hours"] {
            let mut p = good();
            p.as_object_mut().expect("object").remove(field);
            let err = check_definition(&p).expect_err("missing");
            assert_eq!(err["field"], field);
        }
    }

    /// The six fields the registration already fixes are refused with the reason,
    /// not silently ignored. Ignoring them would let a run believe it had set an
    /// objective the campaign then searched under something else.
    #[test]
    fn restating_a_registered_fact_is_refused_by_name() {
        for field in [
            "objective",
            "benchmark",
            "delta_practical",
            "gate_profile",
            "hypothesis",
        ] {
            let mut p = good();
            p[field] = json!("anything");
            let err = check_definition(&p).expect_err("already registered");
            assert_eq!(err["error"], "already_registered", "{field}");
            assert_eq!(err["field"], field);
        }
    }

    #[test]
    fn a_second_trial_budget_is_refused() {
        for field in ["trial_budget", "max_trials"] {
            let mut p = good();
            p[field] = json!(64);
            let err = check_definition(&p).expect_err("one ceiling");
            assert_eq!(err["error"], "budget_already_registered", "{field}");
        }
    }

    /// INV-21: the floor is not writable by an agent and not exposed in any tool
    /// schema. Both halves are asserted — the schema does not offer it, and a
    /// caller that sends it anyway is refused.
    #[test]
    fn the_exploration_floor_is_neither_offered_nor_accepted() {
        let schema = &specs()[0].input_schema;
        assert!(
            schema["properties"].get("exploration_floor").is_none(),
            "the floor is in the schema, which INV-21 forbids"
        );
        let mut p = good();
        p["exploration_floor"] = json!(0.05);
        let err = check_definition(&p).expect_err("not settable");
        assert_eq!(err["error"], "floor_is_not_yours");
    }

    /// The schema asks for what a registration has no room for, and nothing else.
    #[test]
    fn the_schema_asks_only_for_what_the_registration_does_not_carry() {
        let schema = &specs()[0].input_schema;
        let offered: Vec<&str> = schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .map(String::as_str)
            .collect();
        for registered in [
            "objective",
            "benchmark",
            "delta_practical",
            "gate_profile",
            "gates_profile",
            "hypothesis",
            "trial_budget",
            "exploration_floor",
        ] {
            assert!(
                !offered.contains(&registered),
                "`{registered}` is the registration's, and offering it invites a second answer"
            );
        }
    }

    #[test]
    fn both_tools_are_in_the_research_namespace() {
        for s in specs() {
            assert_eq!(s.namespace, "research", "{}", s.name);
        }
    }
}
