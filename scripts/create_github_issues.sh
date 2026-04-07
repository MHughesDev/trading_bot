#!/usr/bin/env bash
# Create GitHub issues for Master Spec V3 from embedded definitions.
# Prerequisites: gh CLI, auth with repo scope: gh auth login
# Usage: bash scripts/create_github_issues.sh
#        REPO=owner/repo bash scripts/create_github_issues.sh

set -euo pipefail
REPO="${REPO:-MHughesDev/trading_bot}"

if ! gh auth status &>/dev/null; then
  echo "Run: gh auth login"
  exit 1
fi

create() {
  local title="$1"
  shift
  local body="$1"
  gh issue create -R "$REPO" --title "$title" --body "$body" --label enhancement 2>/dev/null || \
    gh issue create -R "$REPO" --title "$title" --body "$body"
}

create "Epic: Master Spec V3 — remaining work" "Parent tracker for NautilusMonster V3 spec compliance. Close when child issues are done and \`docs/PRODUCTION_HARDENING.md\` is fully checked.

See: \`docs/PRODUCTION_HARDENING.md\` and \`docs/ISSUE_LOG.md\`"

create "Data: Wire Coinbase WS feed health to risk (stale data)" "## Goal
Use \`CoinbaseWebSocketClient.last_message_at\` and feed gaps with \`NM_RISK_STALE_DATA_SECONDS\`.

## Acceptance
- [ ] Risk reflects stale/disconnected WS before orders
- [ ] Metric when feed exceeds threshold

## Refs
\`data_plane/ingest/coinbase_ws.py\`, \`risk_engine/engine.py\`, \`app/runtime/live_service.py\`"

create "Data: Harden Coinbase REST (rate limits, errors, V1 symbol candles)" "## Goal
Retries, rate limits, pagination, errors; BTC/ETH/SOL candles.

## Refs
\`data_plane/ingest/coinbase_rest.py\`"

create "Data: Normalizer tests from recorded Coinbase WS fixtures" "## Goal
Fixture-based tests; metrics for unknown messages.

## Refs
\`data_plane/ingest/normalizers.py\`, \`tests/fixtures/\`"

create "Data: Product metadata cache (tick size, min size, status)" "## Goal
Cache for sizing and pre-trade validation.

## Refs
\`data_plane/ingest/coinbase_rest.py\`"

create "Storage: QuestDB batch writes, retention, persist decision traces" "## Goal
Production writes + decision_traces from live path.

## Refs
\`data_plane/storage/questdb.py\`"

create "Storage: Redis TTL and bounded pub/sub" "## Goal
TTL on keys; reconnect policy.

## Refs
\`data_plane/storage/redis_state.py\`"

create "Storage: Qdrant versioned payloads + integration tests" "## Goal
Embedding version in payload; top-K query tests.

## Refs
\`data_plane/memory/qdrant_memory.py\`"

create "Features: Full FeaturePipeline in live path + backtest parity test" "## Goal
Same feature code live vs replay; drift test.

## Refs
\`data_plane/features/pipeline.py\`, \`app/runtime/live_service.py\`, \`backtesting/replay.py\`"

create "Features: Microstructure + sentiment (FinBERT, frequency, shocks)" "## Goal
Spec §5 features; no raw text in orders.

## Refs
\`data_plane/features/\`, \`data_plane/ingest/news_ingest.py\`"

create "Memory: 60s Qdrant retrieval loop into features" "## Goal
\`NM_MEMORY_RETRIEVAL_INTERVAL_SECONDS\` asyncio loop in live runner.

## Refs
\`data_plane/memory/\`"

create "Models: Train + persist HMM with validated semantic mapping" "## Goal
Persist artifacts; inference-only in prod.

## Refs
\`models/regime/hmm_regime.py\`"

create "Models: TFT — replace Ridge surrogate (or document deviation)" "## Goal
PyTorch TFT per spec.

## Refs
\`models/forecast/tft_forecast.py\`"

create "Models: Route selector thresholds from config + tests" "## Goal
No magic numbers; YAML-driven.

## Refs
\`models/routing/route_selector.py\`"

create "MLflow: Manual promotion only — document + enforce" "## Goal
No auto-promote in automation.

## Refs
\`models/registry/mlflow_registry.py\`"

create "Decision: Tests for RouteDecision + ActionProposal vs spec" "## Goal
route_id, confidence, ranking; per-route actions vs risk.

## Refs
\`decision_engine/\`, \`app/contracts/decisions.py\`"

create "Risk: Limit precedence doc + FLATTEN/REDUCE_ONLY with positions" "## Goal
Position-aware closes; not stub-only.

## Refs
\`risk_engine/engine.py\`"

create "Execution: Single adapter factory from config" "## Goal
paper/live adapters only via config + ExecutionService.

## Refs
\`execution/router.py\`, \`execution/service.py\`"

create "Execution: Alpaca paper — resilience + symbol map + reconcile" "## Goal
Retries, mapping tests, position reconciliation.

## Refs
\`execution/adapters/alpaca_paper.py\`"

create "Execution: Coinbase live signed orders + cancel + fills" "## Goal
CDP/JWT; remove synthetic ack for prod.

## Refs
\`execution/adapters/coinbase_live.py\`"

create "Runtime: Full live pipeline + QuestDB audit rows" "## Goal
WS→bars→features→models→risk→submit→storage.

## Refs
\`app/runtime/live_service.py\`"

create "Runtime: Graceful shutdown documented + implemented" "## Goal
SIGTERM, task cancellation.

## Refs
\`app/runtime/live_service.py\`"

create "Backtest: CI import test — shared decision/risk with live" "## Goal
No drift between replay and live.

## Refs
\`backtesting/replay.py\`"

create "Backtest: Simulator fees + reproducible seeds" "## Goal
Expand portfolio + slippage.

## Refs
\`backtesting/simulator.py\`"

create "Control plane: Streamlit pages per spec §14" "## Goal
Live, Regimes, Routes, Models, Logs, Emergency.

## Refs
\`control_plane/dashboard.py\`"

create "Observability: Stage metrics + Loki + Grafana alerts" "## Goal
Latency per stage; log shipping; alert rules.

## Refs
\`observability/\`, \`infra/\`"

create "Orchestration: Prefect nightly retrain with manual gate" "## Goal
Replace stub; walk-forward eval.

## Refs
\`orchestration/nightly_retrain.py\`"

create "Ops: Runbooks + secret rotation docs" "## Goal
Incident, flatten, restore.

## Refs
\`docs/\`"

create "CI: Integration tests with Redis, QuestDB, Qdrant" "## Goal
Testcontainers or compose in GHA.

## Refs
\`tests/\`"

create "CI: Optional E2E paper + release checklist" "## Goal
Secrets-based workflow optional; model version in deploy.

## Refs
\`.github/\`"

create "Infra: Compose alignment (MLflow, Prefect, Streamlit)" "## Goal
README matches stack.

## Refs
\`infra/docker-compose.yml\`"

echo "Done. Created issues in $REPO (if API allowed). If some failed, run from a machine with issues:write scope."
