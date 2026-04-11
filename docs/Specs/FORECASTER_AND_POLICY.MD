# Forecaster and policy packages (as implemented)

**Human source of truth (verbatim filenames):**  
[`docs/Human Provided Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) · [`docs/Human Provided Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD`](../Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD)

This document tracks **only** how the repository maps to those two files. Where the human spec gives an illustrative package layout (§26 forecaster, §27 policy), the code follows that layout **by path and module name**. Training stacks (full PyTorch trainer, replay buffers, nightly jobs) are separate backlog items (`FB-FR-P0`, `FB-PL-P0`).

## Forecaster (`forecaster_model/`) — alignment with human §26

| Spec path (illustrative) | Implementation |
|--------------------------|----------------|
| `config/schema.py`, `config/defaults.yaml` | `forecaster_model/config/schema.py`, `ForecasterConfig` in `forecaster_model/config/__init__.py`, `forecaster_model/config/defaults.yaml` |
| `features/*` | `forecaster_model/features/` — `ohlc.py`, `normalization.py`, `time_future.py` |
| `regime/*` | `forecaster_model/regime/` — `protocol.py` (`RegimeEstimatorProtocol`), `soft.py` |
| `models/vsn.py` … `forecaster_model.py` | `forecaster_model/models/vsn.py`, `latent_encoder.py`, `xlstm_cell.py` (`forward_xlstm_sequence` uses **LSTM** per spec §12.7 drop-in rule until full xLSTM), `multi_resolution_xlstm.py`, `fusion.py`, `decoder.py`, `forecaster_model.py` (`ForecasterModel`) |
| `losses/quantile.py` | `forecaster_model/losses/quantile.py` (pinball); `forecaster_model/training/metrics.py` re-exports usage |
| `calibration/conformal.py` | `forecaster_model/calibration/conformal.py` — `SlidingConformalCalibrator`, `MultiHorizonConformal`, JSON `save_conformal_state` / `load_conformal_state`; optional apply in `build_forecast_packet_methodology` when `ForecasterConfig.calibration_enabled` |
| `training/*` | `forecaster_model/training/` — walk-forward, metrics, checkpoint |
| `inference/*` | `forecaster_model/inference/` — `build_from_ohlc.py`, `guards.py` (incl. drift), `robust.py`, `stub.py` |
| `data/dataset_manifest.py` | Versioned manifests + SHA256 — `forecaster_model/data/dataset_manifest.py` |
| `models/ensemble.py` | `build_ensemble_forecast_packet`, `forward_ensemble_numpy` |
| `training/torch_trainer.py` | `train_forecaster_stub`, optional `train_forecaster_torch` (`[models_torch]`) |

**Canonical pipeline (human §30 short form):**  
VSN → Latent CNN → Multi-Resolution xLSTM → Regime-Conditioned Fusion → Quantile Decoder — implemented in `ForecasterModel.forward` and `forward_numpy_reference`.

**Tensor contract:** `app/contracts/forecast_packet.py` + `forecaster_model/schemas.py` (`ForecasterTensors`).

## Policy (`policy_model/`) — alignment with human §27

| Spec path | Implementation |
|-----------|------------------|
| `config/schema.py`, `defaults.yaml` | `policy_model/config/schema.py` (`PolicyConfig`), `policy_model/config/defaults.yaml` |
| `objects/*` | `policy_model/objects/` — one module per spec §8 (`forecast_packet.py` re-exports `ForecastPacket`, `risk_state.py` is **`RiskState`** per §8.4; `PolicyRiskEnvelope` remains an alias) |
| `observation/builder.py` | `policy_model/observation/builder.py` — `PolicyObservationBuilder` |
| `policy/encoders.py`, `actor.py`, `critic.py`, `action_projection.py`, `policy_network.py` | Present; actor is `MultiBranchMLPPolicy` (`mlp_actor.py`); `ValueCritic` stub in `critic.py`; `PolicyNetwork` wraps actor + `update` stub |
| `risk/gate.py` | `policy_model/risk/gate.py` — class **`RiskGate`** (alias `PolicyRiskGate`) |
| `env/rewards.py` | `policy_model/env/rewards.py` → `one_step_reward` in `env/reward.py` |
| `env/environment.py` | `policy_model/env/environment.py` — `TradingPolicyEnvironment` Protocol; **`replay_env.ReplayPolicyEnvironment`** concrete replay loop |
| `training/protocol.py` | `RLPolicyAlgorithm` Protocol |
| `training/buffer.py`, `actor_critic.py`, `behavior_cloning.py` | Replay buffer, walk-forward slices, BC loss |
| `shadow.py` | `shadow_compare_actions` (canary vs primary) |
| `integration/full_policy_loop.py` | `run_stub_episode` — end-to-end stub |
| `env/replay_env.py` | `ReplayPolicyEnvironment` |
| Top-level `PolicySystem` §28 | `policy_model/system.py` — `policy_algorithm`, `action_projector`, `risk_gate`, `execution_planner` |

**Bridge:** `policy_model/bridge.py` maps `AppSettings` + app `RiskState` → policy-layer `RiskState` (spec §8.4).

## Relationship to V3 runtime

- **Spec policy mode (default):** **`models.decision_pipeline_mode: spec_policy`** — **`ForecastPacket`** from **`forecaster_model`** drives **`PolicySystem`**, and **`ExecutionPlan`** maps to **`ActionProposal`** for **`RiskEngine`**. See [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD).
- **Legacy mode:** Set **`models.decision_pipeline_mode: legacy`** (or **`NM_MODELS_DECISION_PIPELINE_MODE=legacy`**) — **`TemporalFusionForecaster`** (Ridge surrogate) + router + **`propose_action`**.
- **Packet-only routing (legacy mode):** **`decision_forecast_routing_source: packet`** derives **`ForecastOutput`** from **`ForecastPacket`** via **`decision_engine/forecast_packet_adapter.py`** without switching the full policy path.

## Dependencies

- Reference forward paths use **NumPy**. Optional **`[models_torch]`** for future PyTorch/xLSTM training.
