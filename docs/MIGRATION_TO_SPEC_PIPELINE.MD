# Migration note: single master pipeline

The repository previously supported a **dual path**: a Ridge surrogate labeled “TFT” under `models/forecast/` plus a deterministic router, versus the **forecast packet + policy** stack.

**Current state:** that legacy path has been **removed**. The only runtime decision path is the **master system pipeline** (`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`):

**Market data → features → forecaster (VSN → latent CNN → multi-resolution xLSTM/LSTM → fusion → quantiles) → `ForecastPacket` → `PolicySystem` → risk gate → execution plan → `RiskEngine` / venues.**

Configuration for serving lineage:

- `NM_MODELS_FORECASTER_CHECKPOINT_ID` — optional string stored on `ForecastPacket.source_checkpoint_id`
- `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` — optional JSON for conformal calibration state

There is **no** `NM_MODELS_FORECAST_PATH`, `NM_MODELS_REGIME_PATH`, `decision_pipeline_mode`, or Ridge routing switch.
