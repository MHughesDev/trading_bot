# docs/archive/ Content Audit (G-0.1)

**Date:** 2026-06-13
**Purpose:** Classify every file in docs/archive/ before Phase 3 deletion.

Classifications:
- **DUPLICATE** — 100% superseded by a named live file; safe to delete.
- **PARTIAL** — overlaps but has unique sections; live file should absorb delta before deletion.
- **UNIQUE** — content not present elsewhere; keep.

---

## Audit Table

| File | Classification | Live Equivalent | Notes |
|------|----------------|-----------------|-------|
| `ADR_CANONICAL_BAR_STORAGE.MD` | DUPLICATE | `docs/adr/0012-canonical-bar-storage.md` | Byte-identical to live ADR |
| `ADR_MANAGED_DATA_SERVICES.MD` | DUPLICATE | `docs/adr/0013-managed-data-services.md` | Byte-identical to live ADR |
| `AUDIT_CODE_REVIEW.MD` | DUPLICATE | `docs/governance/audit_code_review.md` | Redirect stub only |
| `AUTOMATION_QUEUE_SLICE_PROMPT.MD` | UNIQUE | — | Python-era queue automation prompt; no live equivalent |
| `CANONICAL_DELETION_LOG.MD` | UNIQUE | — | FB-CAN migration deletion log; historical reference |
| `CANONICAL_GLOSSARY.MD` | UNIQUE | — | Legacy→APEX terminology mapping; historical reference |
| `CANONICAL_MODULE_MAP.MD` | UNIQUE | — | Python module→APEX-domain map; historical |
| `CANONICAL_SPEC_INDEX.MD` | UNIQUE | — | Python-era spec index; superseded by Rust architecture |
| `CANONICAL_TOMBSTONE_INDEX.MD` | UNIQUE | — | Tombstone/removal index (FB-CAN-058); historical |
| `CI_ROOT_CAUSE_ANALYSIS_PROMPT.md` | UNIQUE | — | CI debugging prompt; no live equivalent |
| `COINBASE_GRANULARITY.MD` | DUPLICATE | `docs/architecture/coinbase_granularity.md` | Redirect stub only |
| `COMMENTARY.MD` | DUPLICATE | `docs/foundation/commentary.md` | Redirect stub only |
| `DEFERRED_ROADMAP.MD` | DUPLICATE | `docs/backlog/deferred_roadmap.md` | Redirect stub only |
| `DEPLOY_CLOUD.MD` | DUPLICATE | `docs/operations/deploy_cloud.md` | Redirect stub only |
| `FULL_AUDIT.md` | DUPLICATE | `docs/governance/full_audit.md` | Redirect stub only |
| `GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD` | DUPLICATE | `docs/governance/release_and_experiments.md` | Redirect stub only |
| `GRACEFUL_SHUTDOWN.MD` | DUPLICATE | `docs/operations/graceful_shutdown.md` | Redirect stub only |
| `KRAKEN_MARKET_DATA.MD` | DUPLICATE | `docs/architecture/kraken_market_data.md` | Redirect stub only |
| `MICROSERVICES_SPLIT_PLAN.MD` | DUPLICATE | `docs/plans/special-plans/MICROSERVICES_SPLIT_PLAN.MD` | Redirect stub only |
| `MIGRATION_TO_SPEC_PIPELINE.MD` | DUPLICATE | `docs/architecture/migration_to_spec_pipeline.md` | Redirect stub only |
| `MLFLOW_PROMOTION.MD` | DUPLICATE | `docs/governance/mlflow_promotion.md` | Redirect stub only |
| `MODULE_INVENTORY.md` | UNIQUE | — | Python module inventory (2026-06-04); historical |
| `MONITORING_CANONICAL.MD` | UNIQUE | — | Python APEX monitoring spec; superseded but historical |
| `PER_ASSET_OPERATOR.MD` | DUPLICATE | `docs/operations/per_asset_operator.md` | Redirect stub only |
| `PHASE_DESIGN_CHECKLIST.md` | DUPLICATE | `docs/plans/reference/PHASE_DESIGN_CHECKLIST.md` | Redirect stub only |
| `PHASE_QUICK_REFERENCE.md` | DUPLICATE | `docs/plans/reference/PHASE_QUICK_REFERENCE.md` | Redirect stub only |
| `PNL_LEDGER.MD` | DUPLICATE | `docs/architecture/pnl_ledger.md` | Redirect stub only |
| `QUESTDB_TRACES.MD` | DUPLICATE | `docs/architecture/questdb_traces.md` | Redirect stub only |
| `QUEUE.MD` | UNIQUE | — | Python queue protocol; superseded by Rust system |
| `QUEUE_ARCHIVE.MD` | UNIQUE | — | Python queue history; legacy artifact |
| `QUEUE_SCHEMA.md` | UNIQUE | — | Python queue schema (FB-CAN-002 era); historical |
| `QUEUE_STACK.csv` | UNIQUE | — | Python queue backlog CSV; legacy |
| `README.md` | UNIQUE | — | Archive context-setter explaining Python→Rust transition |
| `READY_TO_RUN.MD` | DUPLICATE | `docs/operations/ready_to_run.md` | Redirect stub only |
| `RISK_PRECEDENCE.MD` | DUPLICATE | `docs/architecture/risk_precedence.md` | Redirect stub only |
| `RUNBOOKS.MD` | DUPLICATE | `docs/operations/runbooks.md` | Redirect stub only |
| `SYSTEM_SPECIFICATION.md` | UNIQUE | — | Python three-phase architecture spec; different from live Rust specs |
| `SYSTEM_WALKTHROUGH.MD` | DUPLICATE | `docs/architecture/system_walkthrough.md` | Redirect stub only |
| `WINDOWS_OPERATOR_UI.MD` | DUPLICATE | `docs/operations/windows_operator_ui.md` | Redirect stub only |
| `canonical-metadata-archive/` | UNIQUE | — | Python→Rust migration metadata (CANONICAL_* files + MODULE_INVENTORY); all historical |
| `specs/` | PARTIAL | `docs/archive/specs/` | Archive specs subdir; contains older spec versions — live docs/specs/ is authoritative |

---

## Summary

- **DUPLICATE (redirect stubs + ADR copies):** 22 files → safe to delete in Phase 3
- **PARTIAL:** 1 directory (`specs/`) → live `docs/specs/` is authoritative; archive stubs can be removed
- **UNIQUE:** 17 files + 1 directory → keep in archive
