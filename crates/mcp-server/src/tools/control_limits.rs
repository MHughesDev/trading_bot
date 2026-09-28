//! `check_control_limits` — read the monitor, never touch it (AGENT-006 §5.7,
//! COMP-007 §6; Set S task S-3.18).
//!
//! # Why this is a read and only a read
//!
//! ADR-0034 §2.4's principle is that the kill switch is not a tool the agent can
//! skip. The corollary this module implements is narrower and easier to get
//! wrong: the agent may *ask* what the monitor decided and may never *change*
//! it. So every fact here arrives through a `GET`, the four monitor mutations
//! (`trip`, `reset`, `strategy_action`, `declare_limit`) are behind
//! `require_control` in `crates/api/src/routes/monitor.rs` which no service token
//! carries, and `no_tool_anywhere_posts_to_the_monitor` asserts across the whole
//! `tools/` directory that nothing in this catalogue reaches them. A test against
//! this module alone would pass while another module opened the door, which is
//! the shape `the_holdout_has_exactly_one_door` was written for.
//!
//! # Quiet is not healthy
//!
//! The failure this tool exists to avoid is the one S-3.11's deferred leakage
//! scan names: a check that returns empty and reads as "nothing is wrong". Three
//! distinct kinds of silence are reported as themselves rather than as
//! `within_limits`:
//!
//! * **`unwatched`** — the monitor's lease is not `valid`, so no process has
//!   renewed it. Every `strategy_state` row is then a row somebody left behind,
//!   and "no breach since" is "nothing has looked since". `LeaseStatus` already
//!   spells this out for the order gate; this spells it out for the agent.
//! * **`unmonitored`** — the strategy has no declared control limits, or the
//!   monitor has never seen it at all. With no limit there is nothing to breach,
//!   and a strategy that cannot breach is not a strategy that is behaving.
//! * **a disagreement** — the monitor's current state says `breach` and no breach
//!   event carries this strategy's scope, or the reverse. Two reads of the same
//!   monitor that do not match is a fact about the monitor, so it is reported
//!   instead of one of the two being picked.
//!
//! # Read, never re-derive
//!
//! `ControlLimit::breached_by` is the arithmetic that decides a breach, and the
//! monitor performs it. This tool does not: it reports the state the monitor
//! recorded. That is the rule `tools/audit.rs`'s module docs give — arithmetic a
//! gate also performs must not be duplicated where it can disagree — and here it
//! matters twice over, because `breached_by` treats a `NaN` reading as a breach
//! and a second implementation that used `>` would treat it as a pass.
//!
//! # §5.7's `window`, and how it came to be honoured
//!
//! §5.7 writes the tool as `(strategy_id, window=30d|60d|90d)`. `window` was
//! **refused outright** until 2026-09-28, because it could not be honoured:
//! `GET /api/monitor/events` served `(event_id, scope, family, kind)` and no
//! `occurred_at`, so an event could not be placed inside or outside a window. An
//! argument the handler silently ignores is worse than a missing one — the caller
//! believes it scoped the question and reads the answer as bounded — so the payload
//! said `time_bounded: false` and a test held the label in place.
//!
//! The route now selects `occurred_at` (it had always *ordered by* it) and takes
//! `scope` and `window`, applied in SQL rather than to the page it would otherwise
//! have returned. `window` is accepted here as the same closed set, and the tool's
//! `time_bounded` is read back from the bound the **route echoed** rather than from
//! the argument that was sent. That is the part worth keeping: a tool that reports
//! its own request as the applied bound cannot tell a caller when a server ignored
//! it, which is the failure the old refusal was avoiding in the first place.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;
use crate::tools::market::urlencode;
use crate::ApiClient;

/// How many monitor events one call scans.
///
/// The route's own ceiling is 1000 (`EventsQuery` clamps it). Asking for the
/// ceiling and reporting when the answer came back full is the only way to tell
/// "this strategy has no events" from "this strategy's events are older than the
/// thousand most recent ones", and those two answers are opposite.
const EVENT_SCAN: i64 = 1000;

/// Event kinds that mean a control limit was crossed.
///
/// `blip` is deliberately absent: COMP-007 §4.1 defines it as a single-cadence
/// excursion that is recorded and acted on by nothing, and folding it in here
/// would report every strategy as breaching.
const BREACH_KINDS: &[&str] = &["warning", "breach", "scaled_down", "halted", "tripped"];

/// `check_control_limits` — the monitor's reading for one strategy.
pub async fn check_control_limits(api: &ApiClient, params: &Value) -> Value {
    let Some(strategy_id) = params
        .get("strategy_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({ "error": "missing_field", "field": "strategy_id" });
    };

    let switch = match api.get("/api/monitor/kill-switch").await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let strategies = match api.get("/api/monitor/strategies").await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let gate = match api
        .get(&format!(
            "/api/monitor/limits/{}/promotion-gate",
            urlencode(strategy_id)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    // `window` is validated here rather than passed through, so an unknown value
    // is a tool error naming the accepted set instead of a 422 from a route the
    // caller cannot see.
    let window = match params.get("window").and_then(Value::as_str) {
        None => None,
        Some(w) if matches!(w, "30d" | "60d" | "90d") => Some(w),
        Some(other) => {
            return json!({
                "error": "unknown_window",
                "field": "window",
                "value": other,
                "known": ["30d", "60d", "90d"],
            })
        }
    };
    // Scoped at the route, not filtered here. Scanning a global page for one
    // strategy's rows makes "the last 30 days" mean "whichever of the last thousand
    // platform-wide events were this strategy's", which shrinks as the platform
    // gets busier with nothing in the answer to say so.
    let mut url = format!(
        "/api/monitor/events?limit={EVENT_SCAN}&scope={}",
        urlencode(&format!("strategy:{strategy_id}"))
    );
    if let Some(w) = window {
        url.push_str("&window=");
        url.push_str(w);
    }
    let events = match api.get(&url).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };

    report(strategy_id, &switch, &strategies, &gate, &events)
}

/// The monitor's own reading for this strategy, or the absence of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// This strategy's orders are refused. A human clears it.
    Halted,
    /// The monitor's lease is not valid, so nothing is measuring.
    Unwatched,
    /// No declared limits, or the monitor has never seen this strategy.
    Unmonitored,
    Breach,
    ScaledDown,
    Warning,
    /// Every read agrees and the monitor is live. Not a promotion.
    WithinLimits,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Halted => "halted",
            Self::Unwatched => "unwatched",
            Self::Unmonitored => "unmonitored",
            Self::Breach => "breach",
            Self::ScaledDown => "scaled_down",
            Self::Warning => "warning",
            Self::WithinLimits => "within_limits",
        }
    }

    /// Every status this tool can report, for the tests that assert a property of
    /// all of them.
    #[must_use]
    pub fn all() -> &'static [Status] {
        &[
            Self::Halted,
            Self::Unwatched,
            Self::Unmonitored,
            Self::Breach,
            Self::ScaledDown,
            Self::Warning,
            Self::WithinLimits,
        ]
    }

    /// Whether this reading concerns halting, scaling or resuming — the three
    /// things only a human session may do.
    #[must_use]
    pub fn touches_the_kill_switch(self) -> bool {
        matches!(self, Self::Halted | Self::ScaledDown | Self::Unwatched)
    }

    /// What to do about it. A closed set of constants, because an action
    /// assembled from the reading is an action that can be assembled wrongly.
    ///
    /// Every variant for which [`Self::touches_the_kill_switch`] holds says a
    /// human does it, and `every_recommendation_that_touches_the_kill_switch_names_a_human`
    /// asserts that rather than trusting the prose below.
    #[must_use]
    pub fn recommended_action(self) -> &'static str {
        match self {
            Self::Halted => {
                "This strategy is halted and its orders are refused. Clearing a halt is a human \
                 action — `POST /api/monitor/strategies/{id}/resume` requires a human session and \
                 no tool in this catalogue carries that scope. Find out why it halted before \
                 asking for it back."
            }
            Self::Unwatched => {
                "Nothing is watching. The monitor's lease is not valid, so every state below is a \
                 row some earlier process left behind and \"no breach\" means \"nothing has \
                 looked\". A human must start or repair the monitor; treat this strategy's live \
                 numbers as unmeasured until then."
            }
            Self::Unmonitored => {
                "No control limits are declared for this strategy, so there is nothing it can \
                 breach and this reading says nothing about its behaviour. Limits are declared by \
                 a human (`PUT /api/monitor/limits/{id}`); until they are, the promotion gate \
                 refuses live and should."
            }
            Self::Breach => {
                "The monitor recorded a breach. Stop citing this strategy's live numbers as \
                 evidence of anything and read the breaching families below; whether it is scaled \
                 or halted is a human's decision, not a tool call."
            }
            Self::ScaledDown => {
                "The monitor has already reduced this strategy's size. Releasing the scale is a \
                 human action (`POST /api/monitor/strategies/{id}/resume`); a backtest run at \
                 full size is no longer a model of what this strategy is doing."
            }
            Self::Warning => {
                "A warning, not a breach: a metric is near its limit and nothing has been scaled \
                 or halted. Worth reading before any promotion argument rests on the live numbers."
            }
            Self::WithinLimits => {
                "Within its declared limits at the monitor's last cadence. That is not a track \
                 record and not a promotion — it is one live reading, and `min_track_record` is \
                 the tool that answers how long one has to be true for."
            }
        }
    }
}

/// Decide and render, over payloads rather than over HTTP, so the decision is
/// testable without a platform.
#[must_use]
pub fn report(
    strategy_id: &str,
    switch: &Value,
    strategies: &Value,
    gate: &Value,
    events: &Value,
) -> Value {
    let scope = format!("strategy:{strategy_id}");

    // The bound the *route* says it applied. `None` when the route echoed none,
    // which includes a route version that predates the field: an older server
    // answers `time_bounded: false`, truthfully, rather than inheriting this
    // tool's request.
    let applied_since = events
        .get("window")
        .and_then(|w| w.get("since"))
        .filter(|v| !v.is_null())
        .cloned();

    let lease = switch
        .get("lease")
        .and_then(Value::as_str)
        .unwrap_or("unreadable")
        .to_string();
    // Absent reads as "not permitted". The one direction a missing field must not
    // resolve in is the permissive one.
    let orders_permitted = switch
        .get("orders_permitted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let halted_scopes: Vec<String> = switch
        .get("halted_scopes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let scope_halted = halted_scopes.iter().any(|s| s == &scope);
    let platform_halted = halted_scopes.iter().any(|s| s == "global");

    let row = strategies
        .get("strategies")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.get("strategy_id").and_then(Value::as_str) == Some(strategy_id))
        });
    let state = row
        .and_then(|r| r.get("state"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let scale_pct = row.and_then(|r| r.get("scale_pct")).and_then(Value::as_i64);
    let environment = row
        .and_then(|r| r.get("environment"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    // `may_promote == false` covers two different refusals, and only one of them
    // means "unmonitored". `PromotionRefusal::NoLimitsDeclared` is the one that
    // does; `FamilyUnmeasured` means some limits exist. The route serves the
    // refusal as prose, so the discriminator is the phrase it renders.
    let may_promote = gate.get("may_promote").and_then(Value::as_bool);
    let gate_detail = gate
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let no_limits_declared = may_promote == Some(false) && gate_detail.contains("no declared");

    let event_rows: Vec<&Value> = events
        .get("events")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let scanned = event_rows.len();
    let breaching: Vec<Value> = event_rows
        .iter()
        .filter(|e| e.get("scope").and_then(Value::as_str) == Some(scope.as_str()))
        .filter(|e| {
            e.get("kind")
                .and_then(Value::as_str)
                .is_some_and(|k| BREACH_KINDS.contains(&k))
        })
        .map(|e| {
            json!({
                "event_id": e.get("event_id").cloned().unwrap_or(Value::Null),
                "family": e.get("family").cloned().unwrap_or(Value::Null),
                "kind": e.get("kind").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    let state_says_breach = matches!(
        state.as_deref(),
        Some("breach" | "warning" | "scaled_down" | "halted")
    );
    // Two reads of one monitor that do not agree. Reported, not resolved: picking
    // one would be this tool deciding a breach, which is the monitor's job.
    let disagree = state_says_breach == breaching.is_empty();

    let status = if state.as_deref() == Some("halted") || scope_halted {
        Status::Halted
    } else if lease != "valid" {
        Status::Unwatched
    } else if no_limits_declared || row.is_none() {
        Status::Unmonitored
    } else {
        match state.as_deref() {
            Some("breach") => Status::Breach,
            Some("scaled_down") => Status::ScaledDown,
            Some("warning") => Status::Warning,
            _ => Status::WithinLimits,
        }
    };

    json!({
        "summary_fields": {
            "strategy_id": strategy_id,
            "status": status.as_str(),
            "n_breaching": breaching.len(),
            "lease": lease,
            "orders_permitted": orders_permitted,
            "recommended_action": status.recommended_action(),
        },
        "detail": {
            "strategy_id": strategy_id,
            "status": status.as_str(),
            "recommended_action": status.recommended_action(),
            "state": state,
            "scale_pct": scale_pct,
            "environment": environment,
            "monitor": {
                "lease": lease,
                "orders_permitted": orders_permitted,
                "platform_halted": platform_halted,
                "this_strategy_halted": scope_halted,
                "halted_scopes": halted_scopes,
                "maintenance_windows": switch
                    .get("maintenance_windows")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            },
            "limits": {
                "may_promote_to_live": may_promote,
                "no_limits_declared": no_limits_declared,
                "gate_detail": gate_detail,
            },
            "breaching": breaching,
            "n_breaching": breaching.len(),
            "events_scanned": scanned,
            // Read back from what the route echoed, never from what was requested.
            // A tool that reports its own argument as the applied bound cannot tell
            // a caller that the server ignored it, which is exactly the lie the old
            // outright refusal of `window` existed to avoid.
            "time_bounded": applied_since.is_some(),
            "time_bound": applied_since,
            "time_bound_note": if applied_since.is_some() {
                "`time_bound` is the lower bound the route reports having applied, not the window \
                 that was asked for. Events are this strategy's own, filtered server-side."
            } else {
                "the route applied no lower bound, so these events are the most recent the monitor \
                 recorded and not the ones inside any window. `events_truncated` true means older \
                 events for this strategy exist and were not scanned."
            },
            "events_truncated": events
                .get("window")
                .and_then(|w| w.get("truncated"))
                .and_then(Value::as_bool)
                .unwrap_or(scanned as i64 >= EVENT_SCAN),
            "state_and_events_disagree": disagree,
            "disagreement_note": if disagree {
                "the monitor's current state for this strategy and its recorded events do not \
                 agree about whether a limit was crossed. Both were read; neither was corrected. \
                 Do not treat either as the answer until a human reconciles them."
            } else {
                ""
            },
            "reads_only": "every fact above arrived through a GET. Tripping, resetting, halting, \
                           scaling and declaring a limit all require a human session, and no \
                           tool in this catalogue carries that scope.",
        },
        // `strategy_id` is the one citeable handle, because it is the one another
        // tool accepts as an argument. A `monitor_events` uuid is nothing's input,
        // so citing it would be citing a row nobody can look up; the ids stay in
        // `detail.breaching[]`, where they are provenance rather than a handle.
        "ids": { "strategy_id": strategy_id },
    })
}

/// Fields the summary template may name. Declared once so the template and the
/// payload cannot disagree.
const SUMMARY_FIELDS: &[(&str, &str)] = &[
    ("strategy_id", ""),
    ("status", ""),
    ("n_breaching", "events"),
    ("lease", ""),
    ("orders_permitted", ""),
    ("recommended_action", ""),
];

const SUMMARY: &str = "{strategy_id}: {status}; {n_breaching} recorded breach event(s); monitor \
                       lease {lease}, orders_permitted {orders_permitted}. {recommended_action}";

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        "check_control_limits",
        "portfolio",
        // Read, so ring 0 — a Chat turn may ask what the monitor decided. Write
        // would be a lie about a tool that only issues GETs, and would also put
        // it in ring 1 where a Chat turn could not reach it.
        Risk::Read,
        "What the live monitor recorded about one strategy: its state, whether its limits are \
         declared at all, and which families breached. Read-only — it cannot halt, scale or \
         clear anything.",
        json!({
            "type": "object",
            "required": ["strategy_id"],
            // Closed, so no action can be smuggled in as an extra field.
            "additionalProperties": false,
            "properties": {
                "strategy_id": {
                    "type": "string",
                    "description": "the strategy instance id the monitor keys on"
                },
                "window": {
                    "type": "string",
                    // A closed set rather than a duration, because these are the
                    // three §5.7 names and an argument a schema does not describe
                    // is one nobody can reason about from the outside.
                    "enum": ["30d", "60d", "90d"],
                    "description": "how far back to read this strategy's monitor events. Omit for \
                                    the most recent regardless of age. The result reports the bound \
                                    the server says it applied, in `time_bound`."
                }
            }
        }),
    )
    .internal()
    .evidence(&["strategy_id"])
    .summary(SUMMARY, SUMMARY_FIELDS)
    .notes("check_control_limits")]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(lease: &str, halted: &[&str], orders: bool) -> Value {
        json!({
            "lease": lease,
            "orders_permitted": orders,
            "halted_scopes": halted,
            "maintenance_windows": [],
        })
    }

    fn strategies(id: &str, state: &str) -> Value {
        json!({ "strategies": [
            { "strategy_id": id, "state": state, "scale_pct": 100, "environment": "paper" }
        ]})
    }

    fn gate_ok(id: &str) -> Value {
        json!({ "strategy_id": id, "may_promote": true })
    }

    fn gate_no_limits(id: &str) -> Value {
        json!({
            "strategy_id": id,
            "may_promote": false,
            "detail": format!("{id} has no declared control limits"),
        })
    }

    fn events(rows: Value) -> Value {
        json!({ "events": rows })
    }

    /// A route response that echoes the bound it says it applied.
    fn events_since(rows: Value, since: &str, truncated: bool) -> Value {
        json!({
            "events": rows,
            "window": { "since": since, "scope": "strategy:s1", "limit": 1000, "truncated": truncated },
        })
    }

    /// The healthy path, so the refusals below are refusals of something.
    #[test]
    fn a_live_monitor_with_declared_limits_and_a_nominal_state_is_within_limits() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "within_limits");
        assert_eq!(out["detail"]["state_and_events_disagree"], false);
    }

    /// The failure this tool exists to avoid, in its first form: a monitor that
    /// stopped running leaves rows behind, and reading them as "no breach" turns
    /// "nothing looked" into "nothing happened".
    #[test]
    fn a_monitor_whose_lease_has_lapsed_is_unwatched_and_never_within_limits() {
        for lease in ["expired", "missing", "unreadable"] {
            let out = report(
                "s1",
                &switch(lease, &[], false),
                // The row still says nominal. That is the trap.
                &strategies("s1", "nominal"),
                &gate_ok("s1"),
                &events(json!([])),
            );
            assert_eq!(
                out["detail"]["status"], "unwatched",
                "lease {lease} read as a reading"
            );
        }
    }

    /// The second form: nothing to breach is not the same as behaving.
    #[test]
    fn a_strategy_with_no_declared_limits_is_unmonitored_not_within_limits() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_no_limits("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "unmonitored");
        assert_eq!(out["detail"]["limits"]["no_limits_declared"], true);
    }

    /// The third form: a strategy the monitor has never seen has no state, and an
    /// absent state must not render as a good one.
    #[test]
    fn a_strategy_the_monitor_has_never_seen_is_unmonitored_not_within_limits() {
        let out = report(
            "never-seen",
            &switch("valid", &[], true),
            &strategies("someone-else", "nominal"),
            &gate_ok("never-seen"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "unmonitored");
        assert!(out["detail"]["state"].is_null(), "an absent state is null");
    }

    /// A halt recorded against this strategy's scope outranks a stale row that
    /// says nominal, and outranks a lapsed lease: the halted-scope table is
    /// durable and a halt is a positive fact, not an absence.
    #[test]
    fn a_scope_halt_outranks_a_nominal_row_and_a_lapsed_lease() {
        let out = report(
            "s1",
            &switch("expired", &["strategy:s1"], false),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "halted");
        assert_eq!(out["detail"]["monitor"]["this_strategy_halted"], true);
    }

    /// One strategy's scope is not another's. A prefix match would halt `s1` when
    /// `s10` tripped.
    #[test]
    fn another_strategys_halt_does_not_halt_this_one() {
        let out = report(
            "s1",
            &switch("valid", &["strategy:s10"], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "within_limits");
        assert_eq!(out["detail"]["monitor"]["this_strategy_halted"], false);
    }

    /// Only this strategy's events. A global trip is in the same list and is not
    /// this strategy breaching.
    #[test]
    fn an_event_in_another_scope_is_not_this_strategys_breach() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([
                { "event_id": "e1", "scope": "global", "family": "platform", "kind": "tripped" },
                { "event_id": "e2", "scope": "strategy:s2", "family": "pnl", "kind": "breach" },
            ])),
        );
        assert_eq!(out["detail"]["n_breaching"], 0);
        assert_eq!(out["detail"]["breaching"].as_array().unwrap().len(), 0);
    }

    /// `blip` is defined as acted on by nothing. Counting it would report every
    /// strategy as breaching and make the field worthless.
    #[test]
    fn a_blip_is_not_counted_as_a_breach() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([
                { "event_id": "e1", "scope": "strategy:s1", "family": "pnl", "kind": "blip" },
            ])),
        );
        assert_eq!(out["detail"]["n_breaching"], 0);
        assert_eq!(out["detail"]["status"], "within_limits");
    }

    /// Two reads of one monitor that do not match is a fact about the monitor.
    /// Neither is silently preferred.
    #[test]
    fn a_breach_state_with_no_breach_event_is_reported_as_a_disagreement() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "breach"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["status"], "breach");
        assert_eq!(out["detail"]["state_and_events_disagree"], true);
        assert!(
            out["detail"]["disagreement_note"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "the disagreement is explained, not only flagged"
        );
    }

    /// And the reverse direction, which is the more dangerous one: a recorded
    /// breach event with a current state that says nominal.
    #[test]
    fn a_breach_event_with_a_nominal_state_is_reported_as_a_disagreement() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([
                { "event_id": "e1", "scope": "strategy:s1", "family": "pnl", "kind": "breach" },
            ])),
        );
        assert_eq!(out["detail"]["state_and_events_disagree"], true);
        assert_eq!(out["detail"]["n_breaching"], 1);
    }

    /// A missing field must not resolve permissively. An answer with no
    /// `orders_permitted` is not an answer that orders are permitted.
    #[test]
    fn an_absent_orders_permitted_is_not_permission() {
        let out = report(
            "s1",
            &json!({}),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["summary_fields"]["orders_permitted"], false);
        assert_eq!(out["detail"]["status"], "unwatched");
    }

    /// A route that echoed no bound leaves the payload saying so, whatever was
    /// asked for. This was the only behaviour until 2026-09-28, and it stays the
    /// answer for a server that predates the field — an older monitor answers
    /// truthfully rather than inheriting this tool's request.
    #[test]
    fn a_route_that_echoes_no_bound_is_reported_as_unbounded() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        assert_eq!(out["detail"]["time_bounded"], false);
        assert!(out["detail"]["time_bound"].is_null());
        let note = out["detail"]["time_bound_note"].as_str().expect("a note");
        assert!(
            note.contains("no lower bound"),
            "the note has to say the list is unbounded, because a caller that asked \
             for 30d and reads only `events` will otherwise assume it got them: {note}"
        );
    }

    /// `[was the whole reason `window` was refused]` The bound reported is the one
    /// the route echoed.
    #[test]
    fn the_reported_bound_is_the_one_the_route_says_it_applied() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events_since(json!([]), "2026-08-29T00:00:00Z", false),
        );
        assert_eq!(out["detail"]["time_bounded"], true);
        assert_eq!(out["detail"]["time_bound"], "2026-08-29T00:00:00Z");
        assert_eq!(out["detail"]["events_truncated"], false);
    }

    /// A `since` the route echoes as `null` is not a bound. Distinguished from the
    /// key being absent because a route is free to echo the shape with nothing in
    /// it, and `"window": {"since": null}` read as bounded would be the original
    /// lie with an extra step.
    #[test]
    fn an_echoed_null_bound_is_not_a_bound() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &json!({ "events": [], "window": { "since": null, "truncated": false } }),
        );
        assert_eq!(out["detail"]["time_bounded"], false);
    }

    /// Truncation comes from the route too, because the tool cannot see it: a page
    /// that came back shorter than the limit might be all there is, or might be a
    /// server with a lower ceiling than `EVENT_SCAN`.
    #[test]
    fn truncation_is_the_routes_answer_and_not_a_count_comparison() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events_since(json!([]), "2026-08-29T00:00:00Z", true),
        );
        assert_eq!(
            out["detail"]["events_truncated"], true,
            "zero rows scanned, and the route still says there are older ones"
        );
    }

    /// The schema accepts exactly §5.7's three windows and nothing else. An open
    /// enum here would let a duration through that the route refuses, turning a
    /// tool error into a 422 the caller cannot see.
    #[test]
    fn the_schema_accepts_the_three_windows_and_no_other_argument() {
        let spec = &specs()[0];
        let props = spec.input_schema["properties"].as_object().unwrap();
        assert_eq!(props.len(), 2, "strategy_id and window: {props:?}");
        assert!(props.contains_key("strategy_id"));
        assert_eq!(
            props["window"]["enum"],
            json!(["30d", "60d", "90d"]),
            "the closed set §5.7 names"
        );
        assert_eq!(
            spec.input_schema["additionalProperties"], false,
            "an open schema is a field an action could arrive through"
        );
        assert_eq!(
            spec.input_schema["required"],
            json!(["strategy_id"]),
            "`window` is optional: omitting it must keep meaning the most recent \
             events regardless of age, which is what every existing caller expects"
        );
    }

    /// The acceptance criterion, first half, read literally: no agent-callable
    /// halt. Every reading about halting, scaling or resuming says a human does
    /// it, so a model reading the recommendation cannot conclude it may.
    #[test]
    fn every_recommendation_that_touches_the_kill_switch_names_a_human() {
        for status in Status::all() {
            if !status.touches_the_kill_switch() {
                continue;
            }
            let action = status.recommended_action();
            assert!(
                action.contains("human"),
                "{} recommends {action:?} without naming a human",
                status.as_str()
            );
        }
    }

    /// Every status has an action and no two share one, so a reading cannot be
    /// rendered under the wrong advice.
    #[test]
    fn every_status_has_its_own_recommended_action() {
        let mut seen: Vec<&str> = Status::all()
            .iter()
            .map(|s| s.recommended_action())
            .collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(before, seen.len(), "two statuses share one recommendation");
        assert!(seen.iter().all(|a| a.len() > 40), "an empty recommendation");
    }

    /// The acceptance criterion, second half: no write path. Asserted over the
    /// whole `tools/` directory rather than this file, because a test against one
    /// module passes while another module opens the door — the same reason
    /// `the_holdout_has_exactly_one_door` scans the whole catalogue.
    #[test]
    fn no_tool_anywhere_posts_to_the_monitor() {
        let tools = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("tools");
        let mut offenders: Vec<String> = Vec::new();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        let mut stack = vec![tools];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("tools/ is readable") {
                let path = entry.expect("a readable entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 10, "the file scan found only {}", files.len());

        // Scanned forwards from the *verb*, not backwards from the path: a path
        // in prose ("a human resets it with POST /api/monitor/...") is exactly
        // the sentence this tool needs to be able to write, and a scan keyed on
        // the path would forbid saying so. `.post(` is not prose.
        //
        // The receiver is not part of the pattern, because ten of these calls
        // wrap the line between the receiver and the method, and a pattern with
        // `api` in it would miss every one of them — which is how a scan comes
        // to report clean about the calls it cannot see.
        const WRITE_VERBS: &[&str] = &[".post(", ".delete(", ".put(", ".patch("];
        // How far after the verb the path can still be part of the same call.
        const CALL_SPAN: usize = 400;
        let mut verb_sites = 0usize;
        for path in files {
            let src = std::fs::read_to_string(&path).expect("a readable module");
            for verb in WRITE_VERBS {
                let mut from = 0;
                while let Some(rel) = src[from..].find(verb) {
                    let at = from + rel;
                    verb_sites += 1;
                    let end = (at + CALL_SPAN).min(src.len());
                    // `is_char_boundary` because these files carry §, — and ×.
                    let mut end = end;
                    while end > at && !src.is_char_boundary(end) {
                        end -= 1;
                    }
                    if src[at..end].contains("/api/monitor") {
                        let line = src[..at].matches('\n').count() + 1;
                        offenders.push(format!(
                            "{}:{line}: {verb}",
                            path.file_name().unwrap().to_string_lossy()
                        ));
                    }
                    from = at + verb.len();
                }
            }
        }
        assert!(
            verb_sites > 20,
            "the scan found only {verb_sites} write calls in the whole tools directory; \
             `ApiClient`'s methods have been renamed and this test has stopped looking"
        );
        assert!(
            offenders.is_empty(),
            "a tool reaches a monitor route other than by GET. The kill switch is not a tool \
             the agent can skip, and it is not one the agent can throw either: {offenders:#?}"
        );

        // And the list above is complete: a new verb on `ApiClient` would be a
        // write this scan does not look for, so the client's own method set is
        // pinned here rather than trusted to a comment.
        let impl_block = include_str!("../lib.rs")
            .split_once("impl ApiClient {")
            .expect("ApiClient has an impl block")
            .1
            .split_once("\n}\n")
            .expect("the impl block ends")
            .0;
        let found: Vec<&str> = ["get", "post", "delete", "put", "patch", "head"]
            .into_iter()
            .filter(|v| impl_block.contains(&format!("pub async fn {v}(")))
            .collect();
        assert_eq!(
            found,
            vec!["get", "post", "delete"],
            "ApiClient's HTTP verbs changed; add the new one to WRITE_VERBS above"
        );
    }

    /// The declaration and the payload agree, so the rendered summary cannot name
    /// a field the handler does not emit.
    #[test]
    fn the_summary_template_renders_over_the_payload_it_will_be_given() {
        let out = report(
            "s1",
            &switch("valid", &[], true),
            &strategies("s1", "nominal"),
            &gate_ok("s1"),
            &events(json!([])),
        );
        let fields = out["summary_fields"].as_object().unwrap();
        for (name, _) in SUMMARY_FIELDS {
            assert!(
                fields.contains_key(*name),
                "{name} is declared and not emitted"
            );
        }
        let rendered =
            crate::spec::render_summary(SUMMARY, SUMMARY_FIELDS, &out["summary_fields"]).unwrap();
        assert!(rendered.contains("within_limits"));
        assert!(!rendered.contains("(n unknown)"), "{rendered}");
    }

    /// A read, so ring 0 — reachable from a Chat turn, which is the point of a
    /// tool that answers "is this thing safe to look at".
    #[test]
    fn the_tool_is_a_read_under_no_envelope() {
        let spec = &specs()[0];
        assert_eq!(spec.risk, Risk::Read);
        assert_eq!(spec.envelope, crate::spec::Envelope::None);
        assert_eq!(spec.spends_trial, crate::spec::TrialKind::None);
        assert_eq!(spec.ring(), crate::spec::RING_RESEARCH);
    }

    /// The wrapper refuses **before** the first request, so a call with no
    /// strategy names the missing field rather than a network error from a
    /// platform it should never have been asked about.
    #[tokio::test]
    async fn a_missing_strategy_id_is_named_before_any_request() {
        // Port 1 accepts nothing: if this reached the network the assertion below
        // would see a transport error instead.
        let api = ApiClient::new("http://127.0.0.1:1", "t");
        for args in [json!({}), json!({ "strategy_id": "   " })] {
            let out = check_control_limits(&api, &args).await;
            assert_eq!(out["error"], "missing_field", "{args}");
            assert_eq!(out["field"], "strategy_id");
        }
    }
}
