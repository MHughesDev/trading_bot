# Migration: human-spec forecaster + RL policy as the default decision spine

**Goal:** Make **`ForecastPacket`** from the **forecaster_model** stack (VSN → latent encoder → multi-resolution **xLSTM** / LSTM stand-in per §12.7 → fusion → quantiles) and **`PolicySystem`** (**`policy_model`**) the **primary** path for decision handling, instead of the legacy **Ridge `TemporalFusionForecaster`** + deterministic router + **`propose_action`**.

## Current vs target

| Aspect | Legacy (default today) | Spec-aligned (target) |
|--------|-------------------------|-------------------------|
| Forecast brain | Ridge surrogate in `models/forecast/tft_forecast.py` (class name “TFT”) | **`ForecastPacket`** from `forecaster_model` (methodology stub / future torch weights) |
| Routing / action | `DeterministicRouteSelector` + `propose_action` | **`PolicySystem.decide`** → **`ExecutionPlan`** → mapped to **`ActionProposal`** |
| Risk | `RiskEngine` on `ActionProposal` | Unchanged — **RiskEngine remains final** |

## Configuration

| Setting | Values | Meaning |
|---------|--------|---------|
| **`models.decision_pipeline_mode`** | **`legacy`** (default) \| **`spec_policy`** | **`legacy`**: Ridge + router + `propose_action`. **`spec_policy`**: packet-driven forecast + **`PolicySystem`** + plan → proposal. |
| `models.decision_forecast_routing_source` | `ridge` \| `packet` | In **`spec_policy`** mode, **`ForecastOutput`** is **always** derived from **`ForecastPacket`** (packet is always built). In **`legacy`**, same as before. |
| `models.decision_forecast_packet_enabled` | bool | In **`legacy`**, optional diagnostics packet. In **`spec_policy`**, implied on. |

**Environment:** `NM_MODELS_DECISION_PIPELINE_MODE=legacy|spec_policy`

## Code touchpoints

- **`decision_engine/pipeline.py`** — `DecisionPipeline.step` builds **`ForecastPacket`**, runs **`PolicySystem`** when mode is **`spec_policy`**, maps **`ExecutionPlan`** → **`ActionProposal`**.
- **`decision_engine/run_step.py`** — **`run_decision_tick`** passes **`portfolio_equity_usd`** (and uses existing **`position_signed_qty`**, **`mid_price`**) into the pipeline for portfolio state in spec mode.
- **`app/runtime/live_service.py`** / **`backtesting/replay.py`** — pass **equity** when available so **`PortfolioState`** is meaningful.

## Migration checklist (operator)

1. Enable shadow / replay comparison with **`spec_policy`** in a **paper** environment.
2. Set **`NM_MODELS_DECISION_PIPELINE_MODE=spec_policy`** when ready.
3. Keep **`NM_MODELS_FORECAST_PATH`** Ridge artifact only as **fallback** if you reintroduce a dual path; default spec path does not require Ridge for routing.
4. Train and load **torch** forecaster weights when replacing NumPy reference (separate backlog / training jobs).

## Related docs

- [`docs/Specs/FORECASTER_AND_POLICY.MD`](Specs/FORECASTER_AND_POLICY.MD) — package map  
- Human specs: [`docs/Human Provided Specs/`](Human%20Provided%20Specs/README.MD)
