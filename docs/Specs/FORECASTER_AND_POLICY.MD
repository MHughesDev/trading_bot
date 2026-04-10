# Forecaster and policy packages (as implemented)

**Human source of truth:**  
[`docs/Human Provided Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) · [`docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD)

## Current code layout

| Package | Role today |
|---------|------------|
| `forecaster_model/` | **Human forecaster spec alignment:** `schemas.py` (B,L,F tensor contracts); `features/ohlc.py`, `features/normalization.py` (causal rolling z-score + soft regime blend), `features/time_future.py` (cyclical `X_known`); `regime/soft.py` + `regime/protocol.py` (`RegimeEstimatorProtocol`, `RuleBasedRegimeEstimator`); `models/numpy_reference.py` (**VSN → causal CNN → multi-res RNN → regime fusion → quantile head**; RNN as xLSTM stand-in per spec §12.7); `calibration/conformal.py` (sliding conformal); `inference/guards.py`; `data/windowing.py` (sliding indices + log-return targets); `inference/build_from_ohlc.py` + `inference/stub.py` → `ForecastPacket`; `training/` walk-forward + pinball loss |
| `policy_model/` | **Human policy spec alignment:** dataclasses (`objects.py`), `PolicyObservationBuilder`, `PolicyRiskGate`, `ExecutionPlanner`, **`PolicySystem`**, `bridge.policy_envelope_from_app_settings`; **`policy/mlp_actor.py`** (multi-branch encoder + continuous action in `[-1,1]`); `policy/heuristic.py` (simple baseline); `env/reward.py` (canonical reward decomposition); `training/protocol.py` (`RLPolicyAlgorithm` protocol) |
| `app/contracts/forecast_packet.py` | **`ForecastPacket`** — shared contract for forecaster output and policy input |

## Relationship to existing V3 pipeline

- **`decision_engine/pipeline.py`** still uses **`TemporalFusionForecaster`** (Ridge surrogate) and **`ForecastOutput`** for routing.
- When **`models.decision_forecast_packet_enabled`** is true, each **`pipeline.step`** also computes a heuristic **`ForecastPacket`** (stub) exposed as **`DecisionPipeline.last_forecast_packet`** for diagnostics / downstream policy experiments.

## Dependencies

- Stubs use **NumPy** only. Full PyTorch/xLSTM stacks will require **`[models_torch]`** or a dedicated extra when implemented.
