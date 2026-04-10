# Forecaster and policy packages (as implemented)

**Human source of truth:**  
[`docs/Human Provided Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) · [`docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD)

## Current code layout

| Package | Role today |
|---------|------------|
| `forecaster_model/` | Config (`ForecasterConfig`), OHLC-derived features (`features/ohlc.py`), soft regime vector (`regime/soft.py`), **stub** inference producing `ForecastPacket` (`inference/stub.py`), **training** helpers: walk-forward indices, pinball loss, JSON checkpoints (`training/`) |
| `policy_model/` | Dataclasses (`objects.py`), `PolicyObservationBuilder`, heuristic policy, `PolicyRiskGate`, `ExecutionPlanner`, **`PolicySystem`**, **`bridge.policy_envelope_from_app_settings`** (`AppSettings` + `RiskState` → `PolicyRiskEnvelope`) |
| `app/contracts/forecast_packet.py` | **`ForecastPacket`** — shared contract for forecaster output and policy input |

## Relationship to existing V3 pipeline

- **`decision_engine/pipeline.py`** still uses **`TemporalFusionForecaster`** (Ridge surrogate) and **`ForecastOutput`** for routing.
- When **`models.decision_forecast_packet_enabled`** is true, each **`pipeline.step`** also computes a heuristic **`ForecastPacket`** (stub) exposed as **`DecisionPipeline.last_forecast_packet`** for diagnostics / downstream policy experiments.

## Dependencies

- Stubs use **NumPy** only. Full PyTorch/xLSTM stacks will require **`[models_torch]`** or a dedicated extra when implemented.
