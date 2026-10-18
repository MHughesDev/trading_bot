//! Proposing a paper deployment for a person to decide (Set U U-7.7; Set T
//! T-1.x; ADR-0022).
//!
//! An agent may *ask* for a deployment; it may not start one. These tools are
//! thin clients of the routes the deployment desk calls:
//!
//! * `propose_deployment` → `POST /api/deployment-proposals` writes a pending
//!   proposal — the strategy revision, its instrument, the sixteen gates'
//!   verdict for that revision, the capital and the review period. Asking
//!   costs a row and nothing else.
//! * `list_deployment_proposals` → `GET /api/deployment-proposals` reads them
//!   back with each one's decision.
//!
//! Approval does not start anything either: a person decides
//! (`POST /api/deployment-proposals/{id}/decision`, a human scope), and then a
//! person or the UI starts it with `POST /api/deployments` carrying the
//! `proposal_id`. Neither of those is a tool here, and the test below keeps it
//! that way.
//!
//! The divergence limits the deployment will be halted at are governance, and
//! no agent tool may set governance (INV-23). The tool takes none: every
//! proposal it writes carries [`STANDARD_DIVERGENCE`], shown to the person who
//! decides, who rejects a proposal whose limits are wrong for it.

use serde_json::{json, Value};

use crate::ApiClient;

/// The fields a proposal carries, forwarded as given; the route validates
/// them (a stale revision, a failed gate and a missing field are its refusals).
const PROPOSAL_FIELDS: [&str; 8] = [
    "strategy_name",
    "revision_hash",
    "instrument_id",
    "profile_id",
    "verdict_revision_hash",
    "verdict",
    "starting_capital",
    "review_period_days",
];

/// The divergence limits every agent-made proposal carries: the deployment
/// path's standard paper limits (2% per bar, 5% cumulative, positions equal,
/// 90% of signals matching). Not an input: see the module docs.
const STANDARD_DIVERGENCE: (f64, f64, f64, f64) = (0.02, 0.05, 0.0, 0.9);

/// `propose_deployment` → `POST /api/deployment-proposals`
pub async fn propose(api: &ApiClient, params: &Value) -> Value {
    let mut body = serde_json::Map::new();
    for f in PROPOSAL_FIELDS {
        match params.get(f) {
            Some(v) => {
                body.insert(f.into(), v.clone());
            }
            None => return json!({"error": "missing_field", "field": f}),
        }
    }
    let (per_bar, cumulative, position, signals) = STANDARD_DIVERGENCE;
    body.insert(
        "thresholds".into(),
        json!({
            "max_abs_return_divergence": per_bar,
            "max_cum_return_divergence": cumulative,
            "max_position_divergence": position,
            "min_signal_match": signals,
        }),
    );
    match api
        .post("/api/deployment-proposals", Value::Object(body))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_deployment_proposals` → `GET /api/deployment-proposals`
pub async fn list(api: &ApiClient, _params: &Value) -> Value {
    match api.get("/api/deployment-proposals").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Declarations for the proposal tools.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "propose_deployment",
            "lifecycle",
            Risk::Write,
            "Ask a person to deploy a strategy revision to paper. Writes a pending proposal \
             only: a person decides, and approval starts nothing.",
            json!({
                "type": "object",
                "required": PROPOSAL_FIELDS,
                "properties": {
                    "strategy_name": {"type": "string"},
                    "revision_hash": {"type": "string"},
                    "instrument_id": {"type": "string", "description": "or selection:<revision>"},
                    "profile_id": {"type": "string", "description": "e.g. paper_v1"},
                    "verdict_revision_hash": {"type": "string"},
                    "verdict": {"type": "array", "description": "16 x {gate_no, passed}"},
                    "starting_capital": {"type": "string"},
                    "review_period_days": {"type": "integer"}
                }
            }),
        )
        .internal()
        .evidence(&["proposal_id"]),
        ToolSpec::new(
            "list_deployment_proposals",
            "lifecycle",
            Risk::Read,
            "Your deployment proposals with each one's decision, and the deployment an approved \
             one became, if any.",
            json!({"type": "object", "properties": {}}),
        )
        .internal(),
    ]
}

#[cfg(test)]
mod tests {
    /// U-7.7's boundary, by source scan: the tools reach the platform only
    /// through the proposal routes, which are mounted, and never through the
    /// routes that decide a proposal or start a deployment.
    #[test]
    fn a_proposal_tool_can_ask_but_never_decide_or_deploy() {
        let src = include_str!("deployment_proposals.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        let code: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(code.contains("\"/api/deployment-proposals\""));
        for forbidden in [
            "/decision",
            "/api/deployments",
            "sqlx",
            "ledger::",
            "domain::",
        ] {
            assert!(
                !code.contains(forbidden),
                "the proposal tools must not use `{forbidden}`"
            );
        }
        let mounted = include_str!("../../../api/src/routes/mod.rs");
        assert!(
            mounted.contains("\"/api/deployment-proposals\""),
            "the proposal route is mounted"
        );
    }
}
