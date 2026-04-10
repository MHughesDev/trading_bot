# Decision engine

## Shared tick

- **`run_step.py`** — **`run_decision_tick`**: single step used by **`app/runtime/live_service`** and **`backtesting/replay.py`**. Records decision latency metric when wired.

## Pipeline

- **`pipeline.py`** — `DecisionPipeline`: regime + forecast + route + `ActionProposal` from features and risk state
- **`features_live.py`** — Live tick / overlay feature row construction
- **`feature_frame.py`** — Bar enrichment and merges (live/replay)
- **`action_generator.py`** — Route/action wiring (see code for current routes)
- **`audit.py`** — `decision_trace` structures for logging and optional QuestDB persistence

## Parity rule

Any change that affects trading behavior should keep **`run_decision_tick`** behavior consistent for **live** and **replay** unless explicitly documented otherwise.
