# Forecaster and policy packages (as implemented)

**Human source of truth:**  
[`docs/Human Provided Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) · [`docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD)

## Current code layout

| Package | Role today |
|---------|------------|
| `forecaster_model/` | Config (`ForecasterConfig`), OHLC-derived features (`features/ohlc.py`), soft regime vector (`regime/soft.py`), **stub** inference producing `ForecastPacket` (`inference/stub.py`) |
| `policy_model/` | Dataclasses for forecast-conditioned policy (`objects.py`), `PolicyObservationBuilder`, heuristic target policy, `PolicyRiskGate`, `ExecutionPlanner`, **`PolicySystem`** end-to-end stub (`system.py`) |
| `app/contracts/forecast_packet.py` | **`ForecastPacket`** — shared contract for forecaster output and policy input |

## Relationship to existing V3 pipeline

- **`decision_engine/pipeline.py`** still uses **`TemporalFusionForecaster`** (Ridge surrogate) and **`ForecastOutput`** for routing — unchanged.
- The new **`ForecastPacket`** + `forecaster_model` / `policy_model` stack is a **parallel** architecture for the human-provided specs; wiring into `run_decision_tick` is backlog (**`FB-PL-P1`** in [`FEATURES_BACKLOG.MD`](../FEATURES_BACKLOG.MD)).

## Dependencies

- Stubs use **NumPy** only. Full PyTorch/xLSTM stacks will require **`[models_torch]`** or a dedicated extra when implemented.
