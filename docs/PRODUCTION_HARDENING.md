# Production hardening checklist

Use this checklist when hardening NautilusMonster for production. Detailed gaps live in [`features_backlog.md`](features_backlog.md); operational issues in [`issue_log.md`](issue_log.md).

**References:** [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md) · [`QUESTDB_TRACES.md`](QUESTDB_TRACES.md) · [`GRACEFUL_SHUTDOWN.md`](GRACEFUL_SHUTDOWN.md) · [`COINBASE_GRANULARITY.md`](COINBASE_GRANULARITY.md) · [`BACKTESTING_SIMULATOR.md`](BACKTESTING_SIMULATOR.md)

---

## Gates

- [x] Coinbase-only market data (CI: `scripts/ci_spec_compliance.sh`)
- [x] Risk HMAC + intent gate when signing enabled
- [x] No raw text → trades (`OrderIntent` metadata rules)
- [x] No auto MLflow promotion in code (CI: `scripts/ci_mlflow_promotion_policy.sh`)
- [ ] Coinbase **live** real orders (signing, cancel, fills) — see [`features_backlog.md`](features_backlog.md) FB-X*

## Data & features

- [x] WS health + REST retries / fallback
- [ ] Full sentiment + news pipeline wired (not stubs)
- [ ] Qdrant: real embeddings + versioned payloads + integration tests

## Models & orchestration

- [ ] HMM trained + persisted + loaded in inference
- [ ] TFT (PyTorch) or explicit substitute + training pipeline
- [ ] Prefect + MLflow train/evaluate path (manual promotion)

## Observability & ops

- [ ] Stage latency + PnL metrics; Loki shipping + Grafana dashboards as code
- [ ] Runbooks (secrets, incident, flatten, QuestDB)
- [ ] CI: integration tests for Redis, QuestDB, Qdrant

## Definition of done

All items above checked; release notes link this file revision.
