# Decision engine

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Shared tick

- **`run_step.py`** — **`run_decision_tick`**: single step used by **`app/runtime/live_service`** and **`backtesting/replay.py`**. Records decision latency metric when wired.

## Pipeline

- **`state_engine.py`** — **FB-CAN-004:** builds APEX **`CanonicalStateOutput`** from `ForecastPacket` + features (5-class regime probs, confidence, transition, novelty, heat, reflexivity, degradation); attached to **`RegimeOutput.apex`** and merged into **`RiskState`** for sizing.
- **`trigger_engine.py`** — **FB-CAN-005:** Setup → Pre-Trigger → Confirmed Trigger; **`TriggerOutput`** serialized under **`ForecastPacket.forecast_diagnostics["trigger"]`**; gates proposals in **`run_spec_policy_step`** when trigger invalid.
- **`auction_engine.py`** — **FB-CAN-006:** scores long/short/flat candidates (canonical weighted score + penalties), selects winner under top-N and notional cap; result in **`forecast_diagnostics["auction"]`**.
- **`pipeline.py`** — `DecisionPipeline`: regime + forecast + route + `ActionProposal` from features and risk state
- **`features_live.py`** — Live tick / overlay feature row construction
- **`feature_frame.py`** — Bar enrichment and merges (live/replay)
- **`action_generator.py`** — Route/action wiring (see code for current routes)
- **`audit.py`** — `decision_trace` structures for logging and optional QuestDB persistence

## Parity rule

Any change that affects trading behavior should keep **`run_decision_tick`** behavior consistent for **live** and **replay** unless explicitly documented otherwise.
