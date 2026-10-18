# docs/archive/

Files here are superseded. They belong to one of two categories:

1. **Old Python/APEX system** — written before the Rust rewrite (June 2026). References the three-phase ML pipeline, APEX canonical specs, queue system, QuestDB, Qdrant, and the old Python module structure. All superseded by the Rust rewrite documented in `docs/plans/`, `docs/adr/`, and `docs/specs/`.

2. **Root-level duplicates** — files that were migrated to a proper subdirectory (`docs/architecture/`, `docs/operations/`, `docs/governance/`, etc.) and left as stale copies at the root.

Do not reference these files from active docs. If a file here turns out to still be relevant, move it to the correct subdirectory using `docs/procedures/add-doc.md`.

---

## Index

> **Rebuilt 2026-10-03 (Set G remainder item 2 / G-3.1 sub-step 4).** The 24 files that
> [`AUDIT.md`](./AUDIT.md) classified DUPLICATE were deleted in commit `5febac2`
> (2026-06-13). This index now lists only what is actually on disk; the deleted files are
> recorded in the last table with their live replacements, without links to the old paths.

### Audit

| File | What it is |
|------|------------|
| [AUDIT.md](./AUDIT.md) | The 2026-06-13 Set G Phase 0 catalogue: every archive entry classified DUPLICATE / PARTIAL / UNIQUE |

### Old Python/APEX system (superseded by Rust rewrite)

| File | Was |
|------|-----|
| [AUTOMATION_QUEUE_SLICE_PROMPT.MD](./AUTOMATION_QUEUE_SLICE_PROMPT.MD) | Old queue-slice automation prompt |
| [CANONICAL_DELETION_LOG.MD](./CANONICAL_DELETION_LOG.MD) | Python migration deletion log |
| [CANONICAL_GLOSSARY.MD](./CANONICAL_GLOSSARY.MD) | Legacy vs APEX Python terminology map |
| [CANONICAL_MODULE_MAP.MD](./CANONICAL_MODULE_MAP.MD) | Python module-to-APEX-domain map |
| [CANONICAL_SPEC_INDEX.MD](./CANONICAL_SPEC_INDEX.MD) | APEX canonical spec index |
| [CANONICAL_TOMBSTONE_INDEX.MD](./CANONICAL_TOMBSTONE_INDEX.MD) | APEX migration tombstone index |
| [CI_ROOT_CAUSE_ANALYSIS_PROMPT.md](./CI_ROOT_CAUSE_ANALYSIS_PROMPT.md) | Old CI debugging prompt |
| [MODULE_INVENTORY.md](./MODULE_INVENTORY.md) | Python ML module inventory |
| [MONITORING_CANONICAL.MD](./MONITORING_CANONICAL.MD) | Python APEX Prometheus metrics map |
| [QUEUE.MD](./QUEUE.MD) | Old queue protocol and conventions |
| [QUEUE_ARCHIVE.MD](./QUEUE_ARCHIVE.MD) | Old queue completed-item archive |
| [QUEUE_SCHEMA.md](./QUEUE_SCHEMA.md) | Old queue file schema |
| [QUEUE_STACK.csv](./QUEUE_STACK.csv) | Old queue stack CSV |
| [SYSTEM_SPECIFICATION.md](./SYSTEM_SPECIFICATION.md) | Old three-phase Python system specification |
| [canonical-metadata-archive/](./canonical-metadata-archive/) | Second copy of the `CANONICAL_*` files plus `MODULE_INVENTORY.md` (Python→Rust migration metadata) |

### Old Python/APEX specs (moved from docs/specs/)

See [`specs/README.md`](./specs/README.md) for the full list.

### Deleted in `5febac2` (no longer on disk)

These were DUPLICATE in [`AUDIT.md`](./AUDIT.md): redirect stubs or byte-identical copies.
Use the live file instead.

| Deleted archive file | Live file |
|------|--------------------|
| `ADR_CANONICAL_BAR_STORAGE.MD` | [`docs/adr/0012-canonical-bar-storage.md`](../adr/0012-canonical-bar-storage.md) |
| `ADR_MANAGED_DATA_SERVICES.MD` | [`docs/adr/0013-managed-data-services.md`](../adr/0013-managed-data-services.md) |
| `AUDIT_CODE_REVIEW.MD` | [`docs/governance/audit_code_review.md`](../governance/audit_code_review.md) |
| `COINBASE_GRANULARITY.MD` | [`docs/architecture/coinbase_granularity.md`](../architecture/coinbase_granularity.md) |
| `COMMENTARY.MD` | [`docs/foundation/commentary.md`](../foundation/commentary.md) |
| `DEFERRED_ROADMAP.MD` | [`docs/backlog/deferred_roadmap.md`](../backlog/deferred_roadmap.md) |
| `DEPLOY_CLOUD.MD` | [`docs/operations/deploy_cloud.md`](../operations/deploy_cloud.md) |
| `FULL_AUDIT.md` | [`docs/governance/full_audit.md`](../governance/full_audit.md) |
| `GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD` | [`docs/governance/release_and_experiments.md`](../governance/release_and_experiments.md) |
| `GRACEFUL_SHUTDOWN.MD` | [`docs/operations/graceful_shutdown.md`](../operations/graceful_shutdown.md) |
| `KRAKEN_MARKET_DATA.MD` | [`docs/architecture/kraken_market_data.md`](../architecture/kraken_market_data.md) |
| `MICROSERVICES_SPLIT_PLAN.MD` | [`docs/plans/special-plans/MICROSERVICES_SPLIT_PLAN.MD`](../plans/special-plans/MICROSERVICES_SPLIT_PLAN.MD) |
| `MIGRATION_TO_SPEC_PIPELINE.MD` | [`docs/architecture/migration_to_spec_pipeline.md`](../architecture/migration_to_spec_pipeline.md) |
| `MLFLOW_PROMOTION.MD` | [`docs/governance/mlflow_promotion.md`](../governance/mlflow_promotion.md) |
| `PER_ASSET_OPERATOR.MD` | [`docs/operations/per_asset_operator.md`](../operations/per_asset_operator.md) |
| `PHASE_DESIGN_CHECKLIST.md` | [`docs/plans/reference/PHASE_DESIGN_CHECKLIST.md`](../plans/reference/PHASE_DESIGN_CHECKLIST.md) |
| `PHASE_QUICK_REFERENCE.md` | [`docs/plans/reference/PHASE_QUICK_REFERENCE.md`](../plans/reference/PHASE_QUICK_REFERENCE.md) |
| `PNL_LEDGER.MD` | [`docs/architecture/pnl_ledger.md`](../architecture/pnl_ledger.md) |
| `QUESTDB_TRACES.MD` | [`docs/architecture/questdb_traces.md`](../architecture/questdb_traces.md) |
| `READY_TO_RUN.MD` | [`docs/operations/ready_to_run.md`](../operations/ready_to_run.md) |
| `RISK_PRECEDENCE.MD` | [`docs/architecture/risk_precedence.md`](../architecture/risk_precedence.md) |
| `RUNBOOKS.MD` | [`docs/operations/runbooks.md`](../operations/runbooks.md) |
| `SYSTEM_WALKTHROUGH.MD` | [`docs/architecture/system_walkthrough.md`](../architecture/system_walkthrough.md) |
| `WINDOWS_OPERATOR_UI.MD` | [`docs/operations/windows_operator_ui.md`](../operations/windows_operator_ui.md) |
