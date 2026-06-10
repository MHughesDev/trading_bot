# docs/archive/

Files here are superseded. They belong to one of two categories:

1. **Old Python/APEX system** — written before the Rust rewrite (June 2026). References the three-phase ML pipeline, APEX canonical specs, queue system, QuestDB, Qdrant, and the old Python module structure. All superseded by the Rust rewrite documented in `docs/plans/`, `docs/adr/`, and `docs/specs/`.

2. **Root-level duplicates** — files that were migrated to a proper subdirectory (`docs/architecture/`, `docs/operations/`, `docs/governance/`, etc.) and left as stale copies at the root.

Do not reference these files from active docs. If a file here turns out to still be relevant, move it to the correct subdirectory using `docs/procedures/add-doc.md`.

---

## Index

### Old Python/APEX system (superseded by Rust rewrite)

| File | Was |
|------|-----|
| [ADR_CANONICAL_BAR_STORAGE.MD](./ADR_CANONICAL_BAR_STORAGE.MD) | ADR for Python bar storage in QuestDB |
| [ADR_MANAGED_DATA_SERVICES.MD](./ADR_MANAGED_DATA_SERVICES.MD) | ADR for managed vs self-hosted Redis/QuestDB/Qdrant |
| [AUTOMATION_QUEUE_SLICE_PROMPT.MD](./AUTOMATION_QUEUE_SLICE_PROMPT.MD) | Old queue-slice automation prompt |
| [CANONICAL_DELETION_LOG.MD](./CANONICAL_DELETION_LOG.MD) | Python migration deletion log |
| [CANONICAL_GLOSSARY.MD](./CANONICAL_GLOSSARY.MD) | Legacy vs APEX Python terminology map |
| [CANONICAL_MODULE_MAP.MD](./CANONICAL_MODULE_MAP.MD) | Python module-to-APEX-domain map |
| [CANONICAL_SPEC_INDEX.MD](./CANONICAL_SPEC_INDEX.MD) | APEX canonical spec index |
| [CANONICAL_TOMBSTONE_INDEX.MD](./CANONICAL_TOMBSTONE_INDEX.MD) | APEX migration tombstone index |
| [CI_ROOT_CAUSE_ANALYSIS_PROMPT.md](./CI_ROOT_CAUSE_ANALYSIS_PROMPT.md) | Old CI debugging prompt |
| [GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD](./GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD) | Python release gating and experiment registry |
| [MICROSERVICES_SPLIT_PLAN.MD](./MICROSERVICES_SPLIT_PLAN.MD) | Stub redirect (content already moved) |
| [MODULE_INVENTORY.md](./MODULE_INVENTORY.md) | Python ML module inventory |
| [MONITORING_CANONICAL.MD](./MONITORING_CANONICAL.MD) | Python APEX Prometheus metrics map |
| [PHASE_DESIGN_CHECKLIST.md](./PHASE_DESIGN_CHECKLIST.md) | Old three-phase system design checklist |
| [PHASE_QUICK_REFERENCE.md](./PHASE_QUICK_REFERENCE.md) | Old three-phase quick reference card |
| [QUEUE.MD](./QUEUE.MD) | Old queue protocol and conventions |
| [QUEUE_ARCHIVE.MD](./QUEUE_ARCHIVE.MD) | Old queue completed-item archive |
| [QUEUE_SCHEMA.md](./QUEUE_SCHEMA.md) | Old queue file schema |
| [QUEUE_STACK.csv](./QUEUE_STACK.csv) | Old queue stack CSV |
| [SYSTEM_SPECIFICATION.md](./SYSTEM_SPECIFICATION.md) | Old three-phase Python system specification |

### Old Python/APEX specs (moved from docs/specs/)

See [`specs/README.md`](./specs/README.md) for the full list.

### Root duplicates (canonical copy is in subdirectory)

| File | Canonical location |
|------|--------------------|
| [AUDIT_CODE_REVIEW.MD](./AUDIT_CODE_REVIEW.MD) | `docs/governance/audit_code_review.md` |
| [COINBASE_GRANULARITY.MD](./COINBASE_GRANULARITY.MD) | `docs/architecture/coinbase_granularity.md` |
| [COMMENTARY.MD](./COMMENTARY.MD) | `docs/foundation/commentary.md` |
| [DEFERRED_ROADMAP.MD](./DEFERRED_ROADMAP.MD) | `docs/backlog/deferred_roadmap.md` |
| [DEPLOY_CLOUD.MD](./DEPLOY_CLOUD.MD) | `docs/operations/deploy_cloud.md` |
| [FULL_AUDIT.md](./FULL_AUDIT.md) | `docs/governance/full_audit.md` |
| [GRACEFUL_SHUTDOWN.MD](./GRACEFUL_SHUTDOWN.MD) | `docs/operations/graceful_shutdown.md` |
| [KRAKEN_MARKET_DATA.MD](./KRAKEN_MARKET_DATA.MD) | `docs/architecture/kraken_market_data.md` |
| [MIGRATION_TO_SPEC_PIPELINE.MD](./MIGRATION_TO_SPEC_PIPELINE.MD) | `docs/architecture/migration_to_spec_pipeline.md` |
| [MLFLOW_PROMOTION.MD](./MLFLOW_PROMOTION.MD) | `docs/governance/mlflow_promotion.md` |
| [PER_ASSET_OPERATOR.MD](./PER_ASSET_OPERATOR.MD) | `docs/operations/per_asset_operator.md` |
| [PNL_LEDGER.MD](./PNL_LEDGER.MD) | `docs/architecture/pnl_ledger.md` |
| [QUESTDB_TRACES.MD](./QUESTDB_TRACES.MD) | `docs/architecture/questdb_traces.md` |
| [READY_TO_RUN.MD](./READY_TO_RUN.MD) | `docs/operations/ready_to_run.md` |
| [RISK_PRECEDENCE.MD](./RISK_PRECEDENCE.MD) | `docs/architecture/risk_precedence.md` |
| [RUNBOOKS.MD](./RUNBOOKS.MD) | `docs/operations/runbooks.md` |
| [SYSTEM_WALKTHROUGH.MD](./SYSTEM_WALKTHROUGH.MD) | `docs/architecture/system_walkthrough.md` |
| [WINDOWS_OPERATOR_UI.MD](./WINDOWS_OPERATOR_UI.MD) | `docs/operations/windows_operator_ui.md` |
