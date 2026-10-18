//! `alpha_chain_audit` (Set Q Q-1.7): five stages, each read from gate verdicts on record.
//!
//! A thin client of `GET /api/ledger/experiments/{id}/alpha_chain` (ADR-0010). A read: it
//! dispatches nothing and spends no trial. The arithmetic is
//! `backtest::gates::alpha_chain::audit`, and the rule that makes it an audit is there: a stage
//! is `pass` only when every gate row it reads exists and passed.

use serde_json::{json, Value};

use crate::ApiClient;

/// `alpha_chain_audit`.
pub async fn alpha_chain_audit(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = params
        .get("experiment_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return crate::tool_error::missing_field("experiment_id");
    };
    match api
        .get(&format!(
            "/api/ledger/experiments/{}/alpha_chain",
            crate::tools::market::urlencode(experiment_id)
        ))
        .await
    {
        Ok(v) => {
            let stages = v.get("stages").cloned().unwrap_or(Value::Null);
            let count = |verdict: &str| {
                stages
                    .as_array()
                    .map_or(0, |a| a.iter().filter(|s| s["verdict"] == verdict).count())
            };
            json!({
                "summary_fields": {
                    "n_pass": count("pass"),
                    "n_fail": count("fail"),
                    "n_unknown": count("unknown"),
                    "first_unresolved": v["first_unresolved"],
                },
                "detail": v,
                "n": v["gate_rows_read"],
                "ids": { "experiment_id": experiment_id },
            })
        }
        Err(e) => e.to_tool_error(),
    }
}

#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![ToolSpec::new(
        "alpha_chain_audit",
        "audit",
        Risk::Read,
        "AUDIT WHETHER AN EDGE IS REAL AND COLLECTABLE: five stages (point-in-time information, \
         stable signal, feasible positions, executable orders, net alpha after costs and \
         capacity), each `pass`, `fail` or `unknown` from the gate verdicts already recorded for \
         the experiment. A stage with no evidence is `unknown`, never `pass`; each names the gate \
         rows it read. Spends no trial.",
        json!({
            "type": "object",
            "required": ["experiment_id"],
            "properties": {
                "experiment_id": { "type": "string", "description": "the experiment whose recorded gate verdicts are read" }
            }
        }),
    )
    .internal()
    .evidence(&["experiment_id"])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_is_a_read_that_spends_no_trial_and_names_its_one_required_field() {
        let s = &specs()[0];
        assert_eq!(s.name, "alpha_chain_audit");
        assert_eq!(s.spends_trial, crate::spec::TrialKind::None);
        assert_eq!(s.input_schema["required"], json!(["experiment_id"]));
        assert!(s.description.contains("never `pass`"));
    }
}
