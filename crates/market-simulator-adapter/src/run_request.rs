//! Build market_simulator `RunRequest` from a strategy definition and exported
//! Arrow IPC data.

use domain::strategy_def::StrategyDefinition;
use thiserror::Error;

use crate::contract::{RunRequest, OHLCV_SCHEMA_VERSION};

/// Errors produced while building a run request.
#[derive(Debug, Error)]
pub enum RunRequestError {
    #[error("failed to serialise strategy definition: {0}")]
    SerialiseDefinition(#[from] serde_json::Error),

    #[error("IPC bytes are empty")]
    EmptyIpcBytes,
}

/// Build a `RunRequest` from a strategy definition and pre-exported IPC bytes.
pub fn build_run_request(
    definition: &StrategyDefinition,
    instrument_id: &str,
    ipc_bytes: Vec<u8>,
    start_capital: f64,
) -> Result<RunRequest, RunRequestError> {
    if ipc_bytes.is_empty() {
        return Err(RunRequestError::EmptyIpcBytes);
    }

    let definition_json = serde_json::to_value(definition)?;

    Ok(RunRequest {
        strategy_id: definition.strategy_id.clone(),
        definition: definition_json,
        instrument_id: instrument_id.to_owned(),
        ohlcv_ipc_bytes: ipc_bytes,
        start_capital,
        data_schema_version: OHLCV_SCHEMA_VERSION.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::Side;
    use domain::strategy_def::{
        actions::{Action, ActionKind, OrderSpec, SizeMode},
        inputs::InputDeclaration,
        nodes::{Node, NodeKind},
        risk_overrides::RiskOverrides,
        StrategyDefinition,
    };

    fn minimal_def() -> StrategyDefinition {
        StrategyDefinition {
            strategy_id: "test_s".into(),
            definition_version: "1.0".into(),
            asset_class: "crypto_spot_cex".into(),
            min_trust_tier: domain::TrustTier::CentralizedExchange,
            inputs: vec![InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            }],
            nodes: vec![Node {
                id: "n1".into(),
                kind: NodeKind::Condition {
                    expr: "1.0 > 0.0".into(),
                },
            }],
            actions: vec![Action {
                on_signal: "sig".into(),
                kind: ActionKind::PlaceOrder {
                    order: OrderSpec {
                        side: Side::Buy,
                        size_mode: SizeMode::Fixed,
                        size: "0.01".into(),
                    },
                },
            }],
            risk_overrides: RiskOverrides::default(),
        }
    }

    #[test]
    fn empty_bytes_returns_error() {
        let def = minimal_def();
        let err = build_run_request(&def, "BTC-USDT", vec![], 1000.0);
        assert!(matches!(err, Err(RunRequestError::EmptyIpcBytes)));
    }

    #[test]
    fn valid_request_has_correct_fields() {
        let def = minimal_def();
        let fake_ipc = b"fake_arrow_bytes".to_vec();
        let req = build_run_request(&def, "BTC-USDT", fake_ipc.clone(), 1000.0).unwrap();
        assert_eq!(req.strategy_id, "test_s");
        assert_eq!(req.instrument_id, "BTC-USDT");
        assert_eq!(req.start_capital, 1000.0);
        assert_eq!(req.data_schema_version, OHLCV_SCHEMA_VERSION);
        assert_eq!(req.ohlcv_ipc_bytes, fake_ipc);
    }
}
