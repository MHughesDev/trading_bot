//! `explain_trade` (Set M M-2.7): why one trade of a stored run happened.
//!
//! A thin client of `GET /api/backtest/runs/{run_id}/trades/{idx}/explain`. A read over the rich
//! outputs (`clickhouse/11_backtest_outputs_v2.sql`); it spends no trial. An index out of range
//! is a typed refusal that states how many trades the run has.

use serde_json::{json, Value};

use crate::ApiClient;

pub async fn explain_trade(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = params.get("run_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return crate::tool_error::missing_field("run_id");
    };
    let Some(idx) = params.get("trade_idx").and_then(Value::as_u64) else {
        return crate::tool_error::missing_field("trade_idx");
    };
    match api
        .get(&format!(
            "/api/backtest/runs/{}/trades/{idx}/explain",
            crate::tools::market::urlencode(run_id)
        ))
        .await
    {
        Ok(v) => json!({
            "summary_fields": {
                "side": v["side"], "exit_reason": v["exit_reason"],
                "path": v.pointer("/entry_decision/path").cloned().unwrap_or(Value::Null),
            },
            "detail": v,
            "ids": { "run_id": run_id },
        }),
        Err(e) => e.to_tool_error(),
    }
}

#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![ToolSpec::new(
        "explain_trade",
        "backtest",
        Risk::Read,
        "EXPLAIN ONE TRADE of a stored run: the slot values and signals on the decision bar that \
         led to its entry, the regime and path, MAE and MFE, the exit reason and the cost legs. \
         An index past the run's trade count is refused with the count. Spends no trial.",
        json!({
            "type": "object",
            "required": ["run_id", "trade_idx"],
            "properties": {
                "run_id": { "type": "string" },
                "trade_idx": { "type": "integer", "minimum": 0 }
            }
        }),
    )
    .internal()
    .evidence(&["run_id"])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_read_that_spends_no_trial_and_names_both_required_fields() {
        let s = &specs()[0];
        assert_eq!(s.spends_trial, crate::spec::TrialKind::None);
        assert_eq!(s.input_schema["required"], json!(["run_id", "trade_idx"]));
    }
}
