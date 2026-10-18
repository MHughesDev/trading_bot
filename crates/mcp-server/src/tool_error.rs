//! Typed refusals for the tool handlers (Set S, S-1.19).
//!
//! # Why this module exists
//!
//! `spec::ToolError` is the contract: a closed `code`, a mandatory `fix` that is
//! an edit the caller can make, `retryable`, at most 400 bytes. The handlers did
//! not use it. They returned `json!({"error": "missing_field", "field": f})` -- a
//! bare string the model cannot act on and the harness cannot route on. The
//! bridge renders an error as `ERROR: {error}`, so for a string the model saw the
//! code and never the `fix`, and `retryable` did not exist at all.
//!
//! # The closed set
//!
//! [`TOOL_ERROR_CODES`] is the set the handlers may refuse with. It is separate
//! from `spec::ERROR_CODES` only because that constant lives in a file this
//! change was not allowed to touch; it is closed the same way, by a source scan
//! (`every_refusal_in_the_handlers_uses_a_declared_code`) rather than a comment,
//! and the intent is to fold it into `ERROR_CODES` and delete the duplicate.
//!
//! # Wire shape
//!
//! [`refuse`] returns `ToolError::to_value()`, i.e. `{"error": {code, message,
//! fix, retryable}}`. The `error` key is still there, still non-null, so the
//! bridge, `audit::verdict_of` and the gate's `error.is_none()` checks read it as
//! they always did; what changes is that the model now sees the `fix`.

use serde_json::Value;

use crate::spec::{ToolError, MAX_ERROR_BYTES};

/// Every code a handler in `tools/` may refuse with.
pub const TOOL_ERROR_CODES: &[&str] = &[
    // The shared door codes.
    "missing_field",
    "invalid_request",
    "validation_failed",
    "invalid_definition",
    "serialization_error",
    "draft_not_found",
    "duplicate_node_id",
    "invalid_side",
    "invalid_size_mode",
    "invalid_decimal",
    "invalid_window",
    "invalid_timestamp",
    "invalid_position",
    "invalid_fractions",
    "invalid_shift_bars",
    "invalid_max_delay_bars",
    "invalid_interval_hours",
    "invalid_delta_practical",
    "bad_multipliers",
    "backwards_window",
    "empty_window",
    "window_too_short",
    "unknown_window",
    "unknown_venue",
    "unknown_value",
    "unknown_timeframe",
    "unknown_section",
    "unknown_role",
    "unknown_proposal_kind",
    "unknown_mode",
    "unknown_family",
    "unknown_cost",
    "n_paths_out_of_range",
    "negative_budget",
    "too_many_probes",
    "too_many_positions",
    "too_few_trades",
    "ambiguous_source",
    "empty_source",
    "source_too_large",
    "source_not_offered",
    // A request the platform's answer made impossible to honour.
    "malformed_funding_response",
    "cadence_unknown",
    "funding_unmeasured",
    "costs_not_recorded",
    "no_costs_charged",
    "perturbation_not_applied",
    "params_do_not_materialise",
    "probes_disagree",
    "baseline_never_fires",
    "variant_not_registered",
    "unsupported_metric",
    "full_window_refused",
    "must_not_run",
    "not_falsifiable",
    "falsifier_names_no_metric",
    "falsifier_names_no_comparator",
    "already_registered",
    "budget_already_registered",
    "not_yours_to_set",
    "floor_is_not_yours",
    "not_writable_yet",
    "mode_required",
    "live_automations_disabled",
    // Nothing there to read or run.
    "no_bars",
    "no_samples",
    "no_trades",
    "no_steps",
    "no_plan",
    "no_outputs",
    "no_entry",
    "no_feature_columns",
    "no_selector",
    "no_session",
    "no_workspace",
    "no_time_to_run",
    // The platform's side.
    "unreadable_regimes",
    "unreadable_preview",
    "unreadable_form",
    "unreadable_exit_reasons",
    "unreadable_edit",
    "compile_failed",
    "classification_failed",
    "experiment_failed",
    "study_failed",
    "create_model_failed",
    "start_train_failed",
    "not_expressible_in_slv2",
    "this_is_slv3",
    "workflow_incomplete",
    "workflow_step_owed",
    // Transport. The only retryable ones.
    "network_error",
    "backend_unavailable",
];

/// Codes whose cause is the platform or the network, not the call.
const RETRYABLE: &[&str] = &["network_error", "backend_unavailable"];

/// Truncate to at most `max` bytes on a char boundary.
fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

/// A typed refusal, as the wire value.
///
/// `fix` is an edit the caller can make, never "invalid argument". The result is
/// held to [`MAX_ERROR_BYTES`] by shortening the message first, then the fix: the
/// code and the `retryable` flag are what a harness routes on and are never cut.
#[must_use]
pub fn refuse(code: &'static str, message: impl AsRef<str>, fix: impl AsRef<str>) -> Value {
    refuse_ids(code, message, fix, &[])
}

/// As [`refuse`], with citeable ids (`ToolError::ids`).
#[must_use]
pub fn refuse_ids(
    code: &'static str,
    message: impl AsRef<str>,
    fix: impl AsRef<str>,
    ids: &[(&str, &str)],
) -> Value {
    let mut message = clip(message.as_ref(), 260);
    let mut fix = clip(fix.as_ref(), 220);
    loop {
        let mut e = ToolError::new(code, message.clone(), fix.clone());
        if RETRYABLE.contains(&code) {
            e = e.retryable(None);
        }
        for (k, v) in ids {
            e = e.with_id(*k, clip(v, 60));
        }
        let mut v = e.to_value();
        let len = serde_json::to_string(&v).map_or(usize::MAX, |s| s.len());
        if len <= MAX_ERROR_BYTES || (message.len() <= 16 && fix.len() <= 16) {
            // The old ad-hoc shape carried these keys at the top level
            // (`field`, `node_id`, `draft_id`); readers of it still find them.
            // The budget above is on `error`, the evidence is not charged to it.
            if let Some(obj) = v.as_object_mut() {
                for (k, val) in ids {
                    if !matches!(*k, "error" | "summary") {
                        obj.entry((*k).to_string())
                            .or_insert(serde_json::json!(clip(val, 60)));
                    }
                }
            }
            return v;
        }
        // Over budget: shorten whichever is longer and go again.
        if message.len() >= fix.len() {
            message = clip(&message, message.len() * 3 / 4);
        } else {
            fix = clip(&fix, fix.len() * 3 / 4);
        }
    }
}

/// As [`refuse`], keeping evidence the old ad-hoc return carried (`errors`, `got`,
/// `valid_values`...) as **siblings** of `error`, so a reader that wanted them
/// still finds them where they were. `extra` must be a JSON object; anything
/// else is ignored. The 400-byte budget applies to `error`, not to the evidence.
#[must_use]
pub fn refuse_extra(
    code: &'static str,
    message: impl AsRef<str>,
    fix: impl AsRef<str>,
    extra: Value,
) -> Value {
    let mut v = refuse(code, message, fix);
    if let (Some(out), Value::Object(extra)) = (v.as_object_mut(), extra) {
        for (k, val) in extra {
            if k != "error" {
                out.entry(k).or_insert(val);
            }
        }
    }
    v
}

/// The fix for a language diagnostic that carries none of its own.
pub const DIAGNOSTIC_FIX: &str =
    "read the guide section named in `see` (get_slv2_guide), then correct the clause it points at";

/// A refusal the SLv2 language raised. The code is `strategy_lang::Code`, a closed
/// enum owned by that crate, so it is closed by type there rather than by
/// [`TOOL_ERROR_CODES`] here; the diagnostic already carries a message and a fix.
#[must_use]
pub fn from_diagnostic(code: &'static str, message: &str, fix: &str) -> Value {
    refuse(code, message, fix)
}

/// As [`from_diagnostic`], keeping the diagnostic's `see` pointer.
#[must_use]
pub fn from_diagnostic_see(
    code: &'static str,
    message: &str,
    fix: &str,
    see: &impl serde::Serialize,
) -> Value {
    refuse_extra(
        code,
        message,
        fix,
        serde_json::json!({ "see": serde_json::to_value(see).unwrap_or(Value::Null) }),
    )
}

/// A required argument that was not given.
#[must_use]
pub fn missing_field(field: &str) -> Value {
    let mut v = refuse_ids(
        "missing_field",
        format!("`{field}` is required and was not given."),
        format!("add `{field}` to the arguments and call again"),
        &[("field", field)],
    );
    // The old shape carried `field` at the top level; readers of it still find it.
    v["field"] = serde_json::json!(field);
    v
}

/// As [`missing_field`], with the reason the field matters in the message. The
/// `fix` stays the edit; the reason is what makes the edit make sense.
#[must_use]
pub fn missing_field_because(field: &str, why: &str) -> Value {
    let mut v = refuse_ids(
        "missing_field",
        format!("`{field}` is required and was not given: {why}"),
        format!("add `{field}` to the arguments and call again"),
        &[("field", field)],
    );
    v["field"] = serde_json::json!(field);
    v
}

/// As [`missing_field`], when the caller needs to know what the field is for.
#[must_use]
pub fn missing_field_with(field: &str, fix: &str) -> Value {
    let mut v = refuse_ids(
        "missing_field",
        format!("`{field}` is required and was not given."),
        fix,
        &[("field", field)],
    );
    v["field"] = serde_json::json!(field);
    v
}

/// Whether a failed platform call is worth repeating, and what to do about it.
///
/// A transport failure or a 5xx is the platform's, so it is retryable; a 4xx is
/// the call's own fault and repeating it unchanged will fail the same way. The
/// advice is an edit the caller can make, not a restatement of the failure.
#[must_use]
pub fn api_advice(status: Option<u16>) -> (bool, &'static str) {
    match status {
        None => (
            true,
            "the platform did not answer; retry once, and if it repeats say the platform is down",
        ),
        Some(s) if s >= 500 => (
            true,
            "the platform failed on its side; retry once, then report it rather than working around it",
        ),
        Some(_) => (
            false,
            "the platform refused this call; read `detail`, change the arguments, and call again",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{is_code_shaped, ERROR_CODES};
    use serde_json::json;

    #[test]
    fn every_declared_code_is_shaped_like_a_code_and_declared_once() {
        let mut seen = std::collections::BTreeSet::new();
        for c in TOOL_ERROR_CODES {
            assert!(is_code_shaped(c), "{c} is not a routable code");
            assert!(seen.insert(*c), "{c} is declared twice");
        }
    }

    #[test]
    fn a_refusal_carries_code_message_fix_and_retryable_under_the_error_key() {
        let v = missing_field("strategy_id");
        let e = &v["error"];
        assert_eq!(e["code"], "missing_field");
        assert!(e["message"]
            .as_str()
            .is_some_and(|m| m.contains("strategy_id")));
        assert!(e["fix"].as_str().is_some_and(|f| f.contains("strategy_id")));
        assert_eq!(e["retryable"], false);
        assert_eq!(e["ids"]["field"], "strategy_id");
    }

    /// The fix is not optional, and not a restatement of the failure.
    #[test]
    fn a_refusal_without_a_fix_is_impossible_to_build_through_this_door() {
        for v in [
            missing_field("x"),
            refuse("no_bars", "empty", "widen the window"),
        ] {
            let fix = v["error"]["fix"].as_str().unwrap_or("");
            assert!(!fix.trim().is_empty(), "{v}");
        }
    }

    #[test]
    fn only_a_platform_or_network_failure_is_retryable() {
        assert!(api_advice(None).0);
        assert!(api_advice(Some(502)).0);
        assert!(!api_advice(Some(404)).0);
        assert!(!api_advice(Some(422)).0);
        assert_eq!(missing_field("f")["error"]["retryable"], false);
        for st in [None, Some(500), Some(404)] {
            assert!(!api_advice(st).1.trim().is_empty());
        }
    }

    /// The pass-through error keeps the platform's own code under `error` (the
    /// audit verdict keys on it) and gains the two things the harness and the
    /// model need.
    #[test]
    fn a_platform_error_keeps_its_code_and_gains_retryable_and_fix() {
        let e = crate::ApiError {
            status: Some(403),
            body: json!({ "error": "forbidden", "message": "no" }),
        }
        .to_tool_error();
        assert_eq!(e["error"], "forbidden");
        assert_eq!(e["retryable"], false);
        assert!(e["fix"].as_str().is_some_and(|f| !f.is_empty()));
        let down = crate::ApiError {
            status: Some(503),
            body: json!({}),
        }
        .to_tool_error();
        assert_eq!(down["retryable"], true);
    }

    #[test]
    fn a_refusal_never_exceeds_the_error_budget_however_long_its_inputs() {
        let long = "x".repeat(5_000);
        let v = refuse_ids(
            "no_bars",
            &long,
            &long,
            &[("a", long.as_str()), ("b", long.as_str())],
        );
        let len = serde_json::to_string(&v["error"]).unwrap().len();
        assert!(len <= MAX_ERROR_BYTES, "{len} bytes");
        // The routing fields survive the cut.
        assert_eq!(v["error"]["code"], "no_bars");
        assert_eq!(v["error"]["retryable"], false);
    }

    #[test]
    fn the_error_key_is_present_and_not_null_so_existing_readers_still_see_a_failure() {
        let v = missing_field("f");
        assert!(v.get("error").is_some_and(|e| !e.is_null()));
    }

    // ── The scan: the closed set is enforced, not described ──────────────────

    /// Every handler file, named. A new file that can refuse has to be added
    /// here, which is the moment to notice it is doing so.
    const HANDLERS: &[(&str, &str)] = &[
        ("assets.rs", include_str!("tools/assets.rs")),
        ("audit.rs", include_str!("tools/audit.rs")),
        ("authoring.rs", include_str!("tools/authoring.rs")),
        ("automations.rs", include_str!("tools/automations.rs")),
        ("backtests.rs", include_str!("tools/backtests.rs")),
        ("builder.rs", include_str!("tools/builder.rs")),
        ("campaign.rs", include_str!("tools/campaign.rs")),
        ("code.rs", include_str!("tools/code.rs")),
        ("control_limits.rs", include_str!("tools/control_limits.rs")),
        ("core.rs", include_str!("tools/core.rs")),
        ("discovery.rs", include_str!("tools/discovery.rs")),
        ("falsification.rs", include_str!("tools/falsification.rs")),
        ("lifecycle.rs", include_str!("tools/lifecycle.rs")),
        (
            "lifecycle_sessions.rs",
            include_str!("tools/lifecycle_sessions.rs"),
        ),
        ("market.rs", include_str!("tools/market.rs")),
        ("models.rs", include_str!("tools/models.rs")),
        ("path_risk.rs", include_str!("tools/path_risk.rs")),
        ("portfolio.rs", include_str!("tools/portfolio.rs")),
        ("research.rs", include_str!("tools/research.rs")),
        ("research_tools.rs", include_str!("tools/research_tools.rs")),
        ("strategy.rs", include_str!("tools/strategy.rs")),
        ("strategy_lang.rs", include_str!("tools/strategy_lang.rs")),
        ("venues.rs", include_str!("tools/venues.rs")),
        ("workflows.rs", include_str!("tools/workflows.rs")),
    ];

    /// Files not scanned because they were not this change's to edit (Set U's
    /// territory). Listed so the omission is a recorded fact, and
    /// `the_unscanned_handlers_are_exactly_the_ones_still_ad_hoc` fails the day
    /// one is converted so it gets moved into [`HANDLERS`].
    const UNSCANNED: &[(&str, &str)] = &[
        ("selections.rs", include_str!("tools/selections.rs")),
        (
            "deployment_proposals.rs",
            include_str!("tools/deployment_proposals.rs"),
        ),
        ("strategy_v3.rs", include_str!("tools/strategy_v3.rs")),
    ];

    fn production(src: &str) -> String {
        let src = src.replace("\r\n", "\n");
        match src.find("\n#[cfg(test)]") {
            Some(i) => src[..i].to_string(),
            None => src,
        }
    }

    /// Lines in production code that build an ad-hoc `{"error": "<string>"}`.
    ///
    /// A diagnostic from `strategy-lang` (`d.code.as_str()`) is excluded: its
    /// `Code` is a closed enum owned by that crate and its diagnostic already
    /// carries `message` and `fix`.
    fn ad_hoc_error_sites(src: &str) -> Vec<String> {
        let src = production(src);
        let mut out = Vec::new();
        for (i, _) in src.match_indices("\"error\":") {
            let rest = src[i + "\"error\":".len()..].trim_start();
            if rest.starts_with('"') || rest.starts_with("format!(") {
                let line = src[..i].matches('\n').count() + 1;
                out.push(format!(
                    "line {line}: {}",
                    clip(rest.lines().next().unwrap_or(""), 60)
                ));
            }
        }
        out
    }

    #[test]
    fn no_handler_returns_a_bare_string_error() {
        let mut offenders = Vec::new();
        for (file, src) in HANDLERS {
            for s in ad_hoc_error_sites(src) {
                offenders.push(format!("{file} {s}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "ad-hoc `{{\"error\": \"...\"}}` returns -- use tool_error::refuse / missing_field so the \
             model sees a `fix` and the harness sees `retryable`: {offenders:#?}"
        );
    }

    #[test]
    fn the_unscanned_handlers_are_exactly_the_ones_still_ad_hoc() {
        for (file, src) in UNSCANNED {
            assert!(
                !ad_hoc_error_sites(src).is_empty(),
                "{file} no longer builds an ad-hoc error: move it into HANDLERS so it stays scanned"
            );
        }
    }

    /// The first string literal after `marker(`, for every occurrence.
    fn first_literals(src: &str, marker: &str) -> Vec<String> {
        let src = production(src);
        let mut out = Vec::new();
        for (i, _) in src.match_indices(marker) {
            // Not a definition, and not a longer name (`refuse_ids(` vs `refuse(`).
            let before = src[..i].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &src[i + marker.len()..];
            let Some(open) = rest.find('"') else { continue };
            if !rest[..open].trim().is_empty() {
                out.push(format!("<computed after `{marker}`>"));
                continue;
            }
            let after = &rest[open + 1..];
            if let Some(close) = after.find('"') {
                out.push(after[..close].to_string());
            }
        }
        out
    }

    #[test]
    fn every_refusal_in_the_handlers_uses_a_declared_code() {
        let mut bad = Vec::new();
        let mut n = 0;
        for (file, src) in HANDLERS {
            for marker in ["refuse(", "refuse_ids(", "ToolError::new("] {
                for code in first_literals(src, marker) {
                    n += 1;
                    if !TOOL_ERROR_CODES.contains(&code.as_str())
                        && !ERROR_CODES.contains(&code.as_str())
                    {
                        bad.push(format!("{file}: `{code}`"));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "undeclared or computed codes: {bad:#?}");
        assert!(
            n >= 20,
            "the scan found only {n} refusals; it has stopped reading the handlers"
        );
    }

    /// Adversarial: the scan itself must reject what it exists to reject.
    #[test]
    fn the_scan_rejects_a_free_text_code_and_a_bare_string_error() {
        let bad = "fn f() -> Value { refuse(\"it went wrong\", \"m\", \"f\") }\nfn g() -> Value { json!({ \"error\": \"nope\" }) }";
        assert_eq!(
            first_literals(bad, "refuse("),
            vec!["it went wrong".to_string()]
        );
        assert!(!TOOL_ERROR_CODES.contains(&"it went wrong"));
        assert_eq!(ad_hoc_error_sites(bad).len(), 1);
        let ok = "fn g() -> Value { json!({ \"error\": snap.get(\"error\") }) }";
        assert!(ad_hoc_error_sites(ok).is_empty());
        let _ = json!({});
    }
}
