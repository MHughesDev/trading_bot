# NautilusMonster V3 Architecture

## Overview

NautilusMonster V3 is a contract-first trading system where:

- Coinbase is the **only** market data source.
- Execution is routed by mode:
  - `paper` -> Alpaca paper adapter
  - `live` -> Coinbase adapter
- Decision logic is shared between live runtime and backtesting paths.
- Risk checks are mandatory and sit between action generation and order submission.

## Core Runtime Flow

1. `data_plane.ingest.coinbase_ws.CoinbaseWebSocketIngest` receives market events.
2. Events are normalized into typed contracts (`app/contracts/events.py`).
3. Data is persisted to QuestDB and current state is cached in Redis.
4. Trade events are aggregated into bars (`data_plane/bars/aggregator.py`).
5. Feature pipeline computes technical/microstructure-derived features (`FeatureBuilder`).
6. Regime + forecast + memory retrieval produce structured model outputs.
7. Route selector emits a ranked route decision.
8. Action generator emits an action intent.
9. Risk engine approves/adjusts/blocks.
10. Execution router dispatches to Alpaca (paper) or Coinbase (live).
11. Structured decision traces and execution reports are published for audit.

## Subsystems

- `app/`: Runtime state, modes, scheduler, contracts, and config.
- `data_plane/`: Coinbase ingest, normalization, storage, bars, features, memory retrieval.
- `models/`: Regime model, TFT proxy, deterministic route selector, MLflow registry wrapper.
- `decision_engine/`: Route-to-action generation and order intent conversion.
- `risk_engine/`: Hard constraints and system mode gates.
- `execution/`: Base adapter contract and concrete adapters with routing.
- `backtesting/`: Replay and fill simulation using shared intent contracts.
- `control_plane/`: FastAPI endpoints and Streamlit dashboard.
- `orchestration/`: Prefect nightly retrain flow (manual promotion gate).
- `observability/`: JSON logging and Prometheus metrics.

## Contracts and Auditability

Contracts are centralized in `app/contracts/`:

- `events.py`: canonical market/event schemas.
- `models.py`: model output schemas.
- `decisions.py`: route/action/order/risk/execution schemas.
- `audit.py`: decision trace schema.

Every decision cycle can be reconstructed from serialized traces and execution reports.
