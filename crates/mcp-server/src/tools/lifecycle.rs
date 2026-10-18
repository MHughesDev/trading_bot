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

use crate::spec::{ToolSpec, TrialKind};
use crate::taxonomy::Risk;

fn missing(field: &str, why: &str) -> Value {
    crate::tool_error::missing_field_because(field, why)
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
            return Err(crate::tool_error::refuse_extra(
                "invalid_delta_practical",
                format!("delta_practical {d} is not a finite positive number."),
                "set delta_practical to a finite number above zero; zero means every difference matters",
                json!({ "value": d }),
            ))
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
            return Err(crate::tool_error::refuse_extra(
                "backwards_window",
                format!("window.start {s} is after window.end {e}."),
                "swap them so window.start is the earlier instant",
                json!({ "start": s, "end": e }),
            ))
        }
        _ => {
            return Err(missing(
                "window",
                "`{start, end}` — the period the claim is about.",
            ))
        }
    }

    if params.get("objective").is_none_or(Value::is_null) {
        return Err(missing(
            "objective",
            "a vector and its constraints — `{\"maximize\": [...], \"subject_to\": {...}}` — \
             never a scalar score (ADR-012).",
        ));
    }
    if params.get("benchmark").is_none_or(Value::is_null) {
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
        return Err(crate::tool_error::refuse_extra(
            "falsifier_names_no_metric",
            "the falsifier names no metric the platform computes, so nothing could ever refute it.",
            "rewrite `falsifier` around one of `known_metrics`, e.g. `<metric> <= 0`",
            json!({ "falsifier": falsifier, "known_metrics": vocabulary }),
        ));
    };
    // A metric with no comparator is a noun, not a claim. `sharpe_after_costs`
    // says nothing that could turn out false; `sharpe_after_costs <= 0` does.
    const COMPARATORS: &[&str] = &["<=", ">=", "<", ">", "==", "!=", " below ", " above "];
    if !COMPARATORS.iter().any(|c| falsifier.contains(c)) {
        return Err(crate::tool_error::refuse_extra(
            "falsifier_names_no_comparator",
            "the falsifier names a metric but not which way it would have to go; a metric alone cannot turn out false.",
            format!("add a comparator, e.g. `{metric} <= 0` (one of {})", COMPARATORS.join(" ").trim()),
            json!({ "falsifier": falsifier, "metric": metric, "comparators": COMPARATORS }),
        ));
    }
    match text(params, "role").as_deref() {
        None | Some("primary" | "secondary" | "exploratory") => Ok(()),
        Some(other) => Err(crate::tool_error::refuse_extra(
            "unknown_role",
            format!("role `{other}` is not a hypothesis role."),
            "set role to \"primary\", \"secondary\" or \"exploratory\"; only a primary hypothesis can produce an established finding",
            json!({ "role": other, "roles": ["primary", "secondary", "exploratory"] }),
        )),
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

/// `propose` — ask a human for a decision that reaches ring 1 or 2 (§6.8, §7.5).
///
/// **Creating one changes nothing**, which is why it is not gated. What carries
/// the approval envelope is the *decision*, because that is where something
/// happens — and gating the asking would teach a run that asking is expensive,
/// which is how a run starts looking for another way to get what it wants.
pub async fn propose(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(kind) = text(params, "kind") else {
        return missing("kind", "promote_strategy, change_limits or deploy_paper");
    };
    if !["promote_strategy", "change_limits", "deploy_paper"].contains(&kind.as_str()) {
        return crate::tool_error::refuse_extra(
            "unknown_proposal_kind",
            format!("proposal kind `{kind}` is not one this tool files."),
            "set kind to \"promote_strategy\", \"change_limits\" or \"deploy_paper\"",
            json!({ "kind": kind, "kinds": ["promote_strategy", "change_limits", "deploy_paper"] }),
        );
    }
    if text(params, "dossier_ref").is_none() {
        return missing(
            "dossier_ref",
            "the dossier the decision will be made from. A proposal with no dossier asks a              human to decide from your own summary of your own work.",
        );
    }
    if text(params, "subject").is_none() {
        return missing("subject", "what is being promoted, armed or changed");
    }
    // A limit is eight fields and a name describes none of them, so the
    // proposal has to carry the limit itself (migration `0137`). Refused here
    // rather than at the approval: a proposal that reaches a human, gets
    // approved and only then turns out to be unwritable is the worst possible
    // ordering, because somebody has already decided.
    if kind == "change_limits"
        && params
            .get("payload")
            .and_then(Value::as_object)
            .is_none_or(serde_json::Map::is_empty)
    {
        return missing(
            "payload",
            "the limit the approval would write: `metric`, `family`, `version`, `upper` \
             and/or `lower`, `method`, and `declared_from` naming the backtest or paper run \
             the baseline came from. You do not send `declared_by` or `declared_at` — the \
             human who approves it is the one who declared it, and that is the point of \
             asking.",
        );
    }
    // The row the approval acts on. Separate from `subject`, which is what the
    // human reads: resolving a name to a row at approval time would put a guess
    // at the one moment the platform acts on somebody's authority, and the
    // person approving would never see it happen.
    if text(params, "subject_id").is_none() {
        return missing(
            "subject_id",
            "the id of the thing being promoted, armed or changed. `subject` is the name a              human reads on the desk; this is the row the approval writes to, and a proposal              without one is approved and then cannot be performed.",
        );
    }
    post(api, "/api/lifecycle/proposals", params.clone()).await
}

/// Pre-flight on an open question (AGENT-013 §9; S-10.12).
///
/// `why_it_matters` is the field this tool exists for. The loop already writes a
/// question down at run end from whatever the reflection said, and those entries
/// carry "not stated" because prose does not contain stakes however confidently
/// it is read. A question with no stated stakes sits in the store forever being
/// retrieved and skipped, so the deliberate door asks for them and refuses
/// without.
///
/// # Errors
/// The typed refusal, with the field and the reason.
pub fn check_question(params: &Value) -> Result<(), Value> {
    for (field, why) in [
        (
            "subject",
            "what the question is about, so the next run working on that subject is told. \
             An instrument, a family, a component.",
        ),
        ("question", "the question itself, in one sentence."),
        (
            "why_it_matters",
            "what changes depending on the answer. Required, and it is the whole point of \
             opening one deliberately: the run-end write already records questions with no \
             stated stakes, and those are the ones nobody ever prioritises.",
        ),
    ] {
        if text(params, field).is_none() {
            return Err(missing(field, why));
        }
    }
    const VALUES: &[&str] = &["curious", "useful", "decisive"];
    match text(params, "value").as_deref() {
        None | Some("curious" | "useful" | "decisive") => {}
        Some(other) => {
            return Err(crate::tool_error::refuse_extra(
                "unknown_value",
                format!("value `{other}` is not a question value."),
                "set value to \"curious\", \"useful\" or \"decisive\"; `decisive` means something is being decided on an unchecked assumption",
                json!({ "value": other, "values": VALUES }),
            ))
        }
    }
    // `status` is not a field on this request, and neither is `closed_by`. A run
    // does not close its own questions — the platform closes one when a later
    // finding cites it (§9), for the same reason a run does not write its own
    // verdict.
    for field in ["status", "closed_by", "how"] {
        if params.get(field).is_some() {
            return Err(crate::tool_error::refuse_ids(
                "not_yours_to_set",
                format!("`{field}` is not a field a run may set; the platform closes a question when a later finding cites it."),
                format!("remove `{field}` from this call and cite the question in a finding instead"),
                &[("field", field)],
            ));
        }
    }
    Ok(())
}

/// `open_question` — write down something this run could not answer (§9).
pub async fn open_question(api: &crate::ApiClient, params: &Value) -> Value {
    if let Err(e) = check_question(params) {
        return e;
    }
    post(api, "/api/lifecycle/questions", params.clone()).await
}

/// `find_prior_hypothesis` — has this question been registered before?
///
/// `hypothesise` writes the registry and, until this existed, nothing read it by
/// content. So the same hypothesis could be registered twice under two ids and
/// counted as two independent looks — which is a multiple comparison the
/// correction does not know about, in the direction that flatters.
pub async fn find_prior_hypothesis(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(statement) = text(params, "statement") else {
        return missing(
            "statement",
            "the hypothesis you are about to register, as you would register it",
        );
    };
    match api
        .post(
            "/api/lifecycle/hypotheses/similar",
            json!({ "statement": statement }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `prior_negative_results` — what has already been answered no, and under what
/// condition it may be asked again.
///
/// The retry condition is **not** the caller's to state. It is derived from the
/// failure class at the moment the platform wrote the negative result, and
/// `None` is the important half: a weak effect has no retry condition, so an
/// idea that died of one does not get to come back when a venue changes.
pub async fn prior_negative_results(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(subject) = text(params, "subject") else {
        return missing(
            "subject",
            "the instrument, hypothesis family, strategy or component you are about to work on",
        );
    };
    match api
        .get(&format!(
            "/api/memory/negative?subject={}",
            crate::tools::market::urlencode(&subject)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Resolve the session this call is about.
///
/// With no `session_id`, the platform answers from the calling run's own
/// attribution (`/api/research/sessions/current`) — the run does not have to
/// have been told an id, and cannot name another session by mistake. With one,
/// it is used as given and ownership is checked at the boundary.
pub(crate) async fn session_of(api: &crate::ApiClient, params: &Value) -> Result<String, Value> {
    if let Some(id) = text(params, "session_id") {
        return Ok(id);
    }
    match api.get("/api/research/sessions/current").await {
        Ok(v) => v
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                crate::tool_error::refuse(
                    "no_session",
                    "this run is not part of a research session, and no session_id was given",
                    "pass `session_id`; a bounded Research run is its own session, and if you are neither there is nothing to read",
                )
            }),
        Err(e) => Err(e.to_tool_error()),
    }
}

/// `get_session` — the session this run is part of, and what it is doing.
///
/// Read-only, and read-only in a way that matters: a run cannot start, approve,
/// pause or amend its own session through any tool, because the routes behind
/// those actions refuse anything but a human token (AGENT-014 P6).
pub async fn get_session(api: &crate::ApiClient, params: &Value) -> Value {
    let id = match session_of(api, params).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    match api
        .get(&format!(
            "/api/research/sessions/{}",
            crate::tools::market::urlencode(&id)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_agenda` — what the session has asked, is asking and has answered.
///
/// The reason a run is given this at all: its siblings' questions are the
/// cheapest way to find out that what it is about to test has already been
/// tested. The brief carries the last five verdicts; this is for when five is
/// not enough.
pub async fn list_agenda(api: &crate::ApiClient, params: &Value) -> Value {
    let id = match session_of(api, params).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    let status = text(params, "status")
        .map(|s| format!("?status={}", crate::tools::market::urlencode(&s)))
        .unwrap_or_default();
    match api
        .get(&format!(
            "/api/research/sessions/{}/agenda{status}",
            crate::tools::market::urlencode(&id)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_digest` — what the platform told the human about this session.
///
/// Written by the platform from yield and verdicts, not by any run's prose
/// (AGENT-014 P7), which is exactly why a run reading it learns something it did
/// not write.
pub async fn get_digest(api: &crate::ApiClient, params: &Value) -> Value {
    let id = match session_of(api, params).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    let want = params
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 10) as usize;
    match api
        .get(&format!(
            "/api/research/sessions/{}/digests",
            crate::tools::market::urlencode(&id)
        ))
        .await
    {
        Ok(v) => {
            let all = v
                .get("digests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            json!({ "digests": all.into_iter().take(want).collect::<Vec<_>>() })
        }
        Err(e) => e.to_tool_error(),
    }
}

/// `compare_candidates` — SPEC §11.5 as one door (AGENT-013 §8.3).
///
/// N candidates are measured on the same paired replicate grid (windows crossed
/// with a -20 % / modelled / +20 % cost perturbation) and judged pairwise with Holm
/// across the pairs. **There is no seed, no `delta_practical`, no ROPE bound and no
/// replicate count in the schema**, and the route refuses one sent anyway: the seed
/// is the platform's (ADR-P2-02) and the effect size is the Experiment's, declared
/// before any trial ran.
pub async fn compare_candidates(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(experiment) = text(params, "experiment_id") else {
        return missing(
            "experiment_id",
            "the Experiment whose research slice the candidates are measured on; its declared              delta_practical is the effect size the comparison asks about",
        );
    };
    let Some(candidates) = params.get("candidates").filter(|c| c.is_array()) else {
        return missing(
            "candidates",
            "two or more `{name, params}` objects: what to compare, each under its own parameters",
        );
    };
    post(
        api,
        "/api/lifecycle/compare",
        json!({ "experiment": experiment, "candidates": candidates }),
    )
    .await
}

/// `reproduce` — re-run a stored result under its own config and report whether
/// the platform still computes the same thing (AGENT-013 §6.9, §8.4).
///
/// Spends no trial: a reproduction is not a look (ADR-P2-17, ADR-S10-18). There is
/// no tolerance argument, deliberately — it asks whether the platform computed the
/// same number, not whether it computed a close one.
pub async fn reproduce(api: &crate::ApiClient, params: &Value) -> Value {
    let Some(run_id) = text(params, "run_id") else {
        return missing(
            "run_id",
            "the stored Run to re-run, as a Study's member list names it",
        );
    };
    post(api, "/api/lifecycle/reproduce", json!({ "run_id": run_id })).await
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
        ToolSpec::new(
            "propose",
            "lifecycle",
            // A write: it creates a row. **Not** a promotion — the proposal
            // changes nothing, and the envelope belongs on the human's decision.
            // Declaring it `Promotion` here would gate the asking, and a run that
            // learns asking is expensive looks for another way.
            Risk::Write,
            "ASK a human to approve something that reaches beyond research: registering a              strategy, changing a limit, or arming paper trading. It changes nothing by              itself — a human decides, and the change happens then.",
            json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["promote_strategy", "change_limits", "deploy_paper"]
                    },
                    "subject": {
                        "type": "string",
                        "description": "What is being promoted, armed or changed, as a person reads it."
                    },
                    "subject_id": {
                        "type": "string",
                        "description": "The id of that thing. The approval writes to this row, so a proposal without one is approved and then cannot be performed."
                    },
                    "payload": {
                        "type": "object",
                        "description": "For `change_limits`, the limit to write: metric, \
                                        family, version, lower and/or upper, method, and \
                                        declared_from naming the run the baseline came from. \
                                        Omit for other kinds — a payload nothing applies is \
                                        refused rather than ignored."
                    },
                    "dossier_ref": {
                        "type": "string",
                        "description": "The dossier the decision is made from."
                    },
                    "rationale": { "type": "string", "description": "One line on why." }
                },
                "required": ["kind", "subject", "subject_id", "dossier_ref"]
            }),
        )
        .internal()
        .evidence(&["proposal_id"])
        .notes("propose"),
        ToolSpec::new(
            "find_prior_hypothesis",
            "lifecycle",
            Risk::Read,
            "Has this hypothesis already been registered? Call it before `hypothesise`: the \
             same question registered twice is two looks the multiple-comparison correction \
             does not know about. Exact match on the normalised statement, so an empty result \
             is not proof the question is new.",
            json!({
                "type": "object",
                "properties": {
                    "statement": {
                        "type": "string",
                        "description": "The hypothesis, as you would register it."
                    }
                },
                "required": ["statement"]
            }),
        )
        .internal(),
        ToolSpec::new(
            "prior_negative_results",
            "memory",
            Risk::Read,
            "What this subject has already been answered no about, and the condition under \
             which each may be asked again. The condition is the platform's, derived from the \
             failure class — and no condition is the common answer: an idea that died of a \
             weak effect does not come back because a venue changed.",
            json!({
                "type": "object",
                "properties": {
                    "subject": {
                        "type": "string",
                        "description": "An instrument, a hypothesis family, a strategy or a component."
                    }
                },
                "required": ["subject"]
            }),
        )
        .internal(),
        ToolSpec::new(
            "get_session",
            "lifecycle",
            Risk::Read,
            "What research session you are part of: its charter or question, its state, its              budget and how many runs it has started. Omit `session_id` and the platform              answers about the session you are in.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Omit this. The platform knows which session you are in."
                    }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "list_agenda",
            "lifecycle",
            Risk::Read,
            "What this session has asked, is asking and has already answered. Read it before              you commit to a question: a sibling run may have answered yours yesterday, and              re-testing it spends trials on a result the session already holds.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Omit this. The platform knows which session you are in."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["proposed", "approved", "queued", "claimed", "running",
                                 "done", "vetoed", "superseded", "expired"],
                        "description": "Narrow to one column of the board."
                    }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "get_digest",
            "lifecycle",
            Risk::Read,
            "What the platform last told the human about this session: verdicts, findings,              what is waiting for them. Written by the platform from the ledger, not by any              run's prose, so it is the one summary you did not write.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Omit this. The platform knows which session you are in."
                    },
                    "n": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "description": "How many of the most recent digests. Default 1."
                    }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "open_question",
            "lifecycle",
            // A write: it puts a row in semantic memory that later runs read.
            Risk::Write,
            "Write down something you could not answer, so the next run on this subject is \
             told rather than rediscovering it. Say what changes depending on the answer — a \
             question with no stated stakes is one nobody will ever prioritise.",
            json!({
                "type": "object",
                "properties": {
                    "subject": { "type": "string", "description": "What it is about: an instrument, a family, a component." },
                    "question": { "type": "string" },
                    "why_it_matters": { "type": "string", "description": "What changes depending on the answer." },
                    "blocking": {
                        "type": "boolean",
                        "description": "True when the answer changes what a run should do next. Blocking questions lead the next run's brief."
                    },
                    "value": { "type": "string", "enum": ["curious", "useful", "decisive"] }
                },
                "required": ["subject", "question", "why_it_matters"]
            }),
        )
        .internal(),
        ToolSpec::new(
            "compare_candidates",
            "lifecycle",
            Risk::Write,
            "Rank two or more candidates by SPEC §11.5: every one measured on the same paired              replicates (windows x -20% / modelled / +20% cost), every pair judged with an              anytime-valid confidence sequence against the Experiment's own delta_practical,              Holm across the pairs. `indistinguishable` is a real and common answer. You              cannot choose the seed, the effect size or the replicate count; they are the              platform's and the Experiment's. Counted trials.",
            json!({
                "type": "object",
                "required": ["experiment_id", "candidates"],
                "properties": {
                    "experiment_id": { "type": "string", "description": "The Experiment (its research slice and declared delta_practical are used)." },
                    "candidates": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "required": ["name"],
                            "properties": {
                                "name": { "type": "string", "description": "A label; must be distinct." },
                                "params": { "type": "object", "description": "The parameters this candidate runs under." }
                            }
                        }
                    }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Search)
        .evidence(&["matrix"])
        .max_calls_per_run(3),
        ToolSpec::new(
            "reproduce",
            "lifecycle",
            // It appends a finding row; it changes nothing a run reads as a result.
            Risk::Write,
            "Re-run a stored result under its own config and report whether the platform still              computes the same thing: bit-identical, a named receipt difference, or drift.              Spends no trial - a reproduction is not a look. There is no tolerance: close              enough is not an outcome.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string", "description": "A stored Run id, as a Study's member list names it." }
                }
            }),
        )
        .internal(),
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
            err["error"]["message"]
                .as_str()
                .unwrap()
                .contains("after a result is not"),
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
            err["error"]["message"]
                .as_str()
                .unwrap()
                .contains("a decision nobody made"),
            "{err}"
        );

        r["delta_practical"] = json!(0.0);
        let err = check_registration(&r).expect_err("zero is refused too");
        assert_eq!(err["error"]["code"], "invalid_delta_practical");
    }

    /// A scalar objective is refused: ADR-012.
    #[test]
    fn an_objective_must_be_present() {
        let mut r = registration();
        r["objective"] = Value::Null;
        let err = check_registration(&r).expect_err("refused");
        assert_eq!(err["field"], "objective");
        assert!(err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("never a scalar score"));
    }

    /// A falsifier that names nothing evaluable is refused, with the vocabulary.
    #[test]
    fn a_falsifier_naming_no_metric_is_refused_with_the_list() {
        let err = check_hypothesis(&json!({"falsifier": "it does not work very well"}), VOCAB)
            .expect_err("refused");
        assert_eq!(err["error"]["code"], "falsifier_names_no_metric");
        assert_eq!(err["known_metrics"], json!(VOCAB));
        assert!(
            err["error"]["message"].as_str().unwrap().contains("refute"),
            "the reason is the point: {err}"
        );
    }

    /// A metric with no comparator is a noun, not a claim.
    #[test]
    fn a_falsifier_needs_a_direction() {
        let err = check_hypothesis(&json!({"falsifier": "sharpe_after_costs"}), VOCAB)
            .expect_err("refused");
        assert_eq!(err["error"]["code"], "falsifier_names_no_comparator");
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
        assert_eq!(err["error"]["code"], "unknown_role");
    }

    /// Neither tool spends a trial.
    ///
    /// `preregister` is the tool the trial gate refuses on behalf of; charging
    /// for it would put a price on the one step that makes the rest of the run
    /// mean anything.
    #[test]
    fn sealing_a_commitment_is_not_a_look() {
        // `compare_candidates` is the one tool here that *is* a look: it measures N
        // candidates on real Runs (counted trials, S-10.10), so it declares one and
        // the gate refuses it without a sealed registration. It is named, not
        // inferred, so a second spending tool arriving in this namespace fails here.
        const SPENDS: &[&str] = &["compare_candidates"];
        for spec in specs() {
            if SPENDS.contains(&spec.name) {
                assert_eq!(spec.spends_trial, crate::spec::TrialKind::Search);
                continue;
            }
            assert_eq!(
                spec.spends_trial,
                crate::spec::TrialKind::None,
                "`{}` declares a trial",
                spec.name
            );
        }
    }

    /// None of them reaches past ring 1.
    ///
    /// An **upper bound**, not an equality. It was an equality when every tool
    /// here wrote something; the precheck reads (S-8.2) are `Risk::Read` and
    /// therefore ring 0, which is not a weakening — a read reaches nothing, and
    /// asserting it reaches ring 1 would have forced a read to be declared as a
    /// write to satisfy a test. The property the name states is the ceiling.
    #[test]
    fn the_lifecycle_tools_reach_no_further_than_the_platform() {
        for spec in specs() {
            assert!(
                matches!(
                    spec.ring(),
                    crate::spec::RING_RESEARCH | crate::spec::RING_PLATFORM_ARTIFACTS
                ),
                "`{}` reaches {}",
                spec.name,
                spec.ring()
            );
            // And a read is ring 0 rather than being quietly promoted.
            if spec.risk == crate::taxonomy::Risk::Read {
                assert_eq!(spec.ring(), crate::spec::RING_RESEARCH, "`{}`", spec.name);
            }
        }
    }

    /// The field the deliberate door exists for.
    #[test]
    fn a_question_without_stated_stakes_is_refused() {
        let p = json!({ "subject": "BTC-USD", "question": "does the funding leg survive costs" });
        let err = check_question(&p).expect_err("no stakes");
        assert_eq!(err["field"], "why_it_matters");
    }

    #[test]
    fn a_complete_question_passes_preflight() {
        check_question(&json!({
            "subject": "BTC-USD",
            "question": "does the funding leg survive costs",
            "why_it_matters": "the whole carry thesis rests on it",
            "blocking": true,
            "value": "decisive"
        }))
        .expect("complete");
    }

    /// §9's rule, as a refusal rather than a silently ignored field: a run does
    /// not close its own questions. Ignoring the field would let a run believe
    /// it had closed one and report accordingly.
    #[test]
    fn a_run_cannot_close_its_own_question() {
        for field in ["status", "closed_by", "how"] {
            let mut p = json!({
                "subject": "BTC-USD",
                "question": "q",
                "why_it_matters": "w"
            });
            p[field] = json!("closed");
            let err = check_question(&p).expect_err("not yours");
            assert_eq!(err["error"]["code"], "not_yours_to_set", "{field}");
        }
    }

    /// The schema offers no way to close one either, so the refusal above is a
    /// second line rather than the only one.
    #[test]
    fn the_question_schema_offers_no_closing_fields() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "open_question")
            .expect("open_question is in the catalogue");
        let props = spec.input_schema["properties"]
            .as_object()
            .expect("properties");
        for field in ["status", "closed_by", "how", "closed"] {
            assert!(!props.contains_key(field), "the schema offers `{field}`");
        }
    }

    #[test]
    fn the_question_values_are_the_harness_enum() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "open_question")
            .expect("in the catalogue");
        let offered: Vec<&str> = spec.input_schema["properties"]["value"]["enum"]
            .as_array()
            .expect("an enum")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        // Spelled here and in `harness::memory::questions`; a mismatch would
        // offer a value the store then refuses.
        assert_eq!(offered, vec!["curious", "useful", "decisive"]);
    }
}
