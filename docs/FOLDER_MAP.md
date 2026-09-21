# Docs Folder Map - Reorganized Structure
**Last Updated:** 2026-06-11

```
docs/
│
├─ README.md                    # 📋 Main index (entry point per README.md)
├─ artifact.md                  # 📍 Foundational project definition (SC-N, FM-N)
├─ architecture.md              # 🏗️ System architecture & component map
├─ glossary.md                  # 📚 Shared terminology
├─ open-questions.md            # ❓ Living decision register (Q-N)
├─ docs_home.md                 # 🏠 Alternative home/entry point
├─ parity-matrix.md             # 📊 Parity/compatibility tracking
│
├─ QUEUE.MD                     # 🔄 Queue system: agent protocol & conventions
├─ QUEUE_SCHEMA.md              # 🔄 Queue schema & structure definition
├─ QUEUE_ARCHIVE.MD             # 🔄 Archived queue items (historical)
├─ QUEUE_STACK.csv              # 🔄 Current queue stack (data file)
│
├─ adr/                         # 🏛️ Architecture Decision Records (ADR-0001 through 0038)
│   ├─ 0001-rust-modular-monolith-with-satellite-collectors.md
│   ├─ 0002-decimal-money-newtypes-no-f64.md
│   ├─ ... (through 0011)
│   ├─ 0012-canonical-bar-storage.md                 # (NEW - moved from root)
│   └─ 0013-managed-data-services.md                 # (NEW - moved from root)
│
├─ architecture/                # 🏗️ System architecture & technical documentation
│   ├─ system_walkthrough.md
│   ├─ migration_to_spec_pipeline.md
│   ├─ monitoring.md                                  # (NEW - moved from root as MONITORING_CANONICAL)
│   ├─ risk_precedence.md
│   ├─ pnl_ledger.md
│   ├─ coinbase_granularity.md
│   ├─ kraken_market_data.md
│   └─ questdb_traces.md
│
├─ specs/                       # 📋 Feature, component, data, and integration specs
│   ├─ COMP-001-data-quality-and-ingestion.md
│   ├─ COMP-002-execution-and-risk-gate.md
│   ├─ COMP-003-ui-streaming-gateway.md
│   ├─ COMP-004-storage-and-replay.md
│   ├─ DATA-001-event-envelope-and-payloads.md
│   ├─ DATA-002-instrument-metadata.md
│   ├─ DATA-003-timestamps-and-identity.md
│   ├─ DATA-004-strategy-definition-format.md
│   ├─ FEAT-001-strategy-system.md
│   ├─ INTG-001-mcp-server.md
│   ├─ APP_CONFIG_AND_CONTRACTS.MD                   # (NEW - from consolidated Specs/)
│   ├─ CONTROL_PLANE.MD                              # (NEW - from consolidated Specs/)
│   ├─ DATA_PLANE.MD                                 # (NEW - from consolidated Specs/)
│   ├─ EXECUTION_LAYER.MD                            # (NEW - from consolidated Specs/)
│   ├─ MODELS_AND_ORCHESTRATION.MD                   # (NEW - from consolidated Specs/)
│   ├─ MULTI_ASSET_PORTFOLIO.MD                      # (NEW - from consolidated Specs/)
│   ├─ OBSERVABILITY_AND_INFRA.MD                    # (NEW - from consolidated Specs/)
│   ├─ RISK_ENGINE.MD                                # (NEW - from consolidated Specs/)
│   ├─ SYSTEM_OVERVIEW.MD                            # (NEW - from consolidated Specs/)
│   ├─ SYSTEM_SPECIFICATION.md                       # (NEW - moved from root)
│   ├─ TESTING_AND_CI.MD                             # (NEW - from consolidated Specs/)
│   ├─ CANONICAL_PRECEDENCE.MD                       # (NEW - from consolidated Specs/)
│   └─ README.md                 # Spec index
│
├─ plans/                       # 📅 Formal plan copies (all plan sets A–G)
│   ├─ README.md
│   ├─ plan-sets/               # All plan sets live here
│   │   ├─ set-A/               # Original Phase A–7 refactor plans
│   │   ├─ set-B/ … set-E/      # Subsequent refactor and optimization sets
│   │   ├─ set-F/               # AI Agent MCP Platform
│   │   └─ set-G/               # Documentation restructuring
│   ├─ reference/               # Checklists and quick-reference docs
│   └─ special-plans/           # Microservices split and similar one-off plans
│
├─ procedures/                  # 🔧 Atomic step-by-step task instructions
│   ├─ README.md
│   ├─ add-a-venue.md
│   ├─ add-adr.md
│   ├─ add-doc.md
│   ├─ ... (through execute-plan.md)
│   └─ CI_ROOT_CAUSE_ANALYSIS_PROMPT.md              # (NEW - moved from root)
│
├─ operations/                  # 🚀 Operational procedures, runbooks, deployment
│   ├─ deploy_cloud.md
│   ├─ runbooks.md
│   ├─ graceful_shutdown.md
│   ├─ ready_to_run.md
│   ├─ windows_operator_ui.md
│   ├─ per_asset_operator.md
│   └─ rollback_playbooks.md
│
├─ governance/                  # 🏛️ Release governance, audits, MLflow promotion
│   ├─ audit_code_review.md
│   ├─ full_audit.md
│   ├─ mlflow_promotion.md
│   └─ release_and_experiments.md                    # (NEW - moved from root as GOVERNANCE_RELEASE_...)
│
├─ reports/                     # 📊 Audit reports, coverage matrices, acceptance reports
│   ├─ AUDIT_REPORT_2026-04-13_full.md
│   ├─ AUDIT_REPORT_TEMPLATE.md
│   ├─ CANONICAL_ACCEPTANCE_AUDIT_REPORT.json
│   ├─ CANONICAL_SPEC_COVERAGE_MATRIX.MD
│   ├─ IMPLEMENTATION_SUMMARY_2026-06-02.md
│   └─ REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md
│
├─ research/                    # 🔬 Research briefs, technology evaluations, trade-off analyses
│   ├─ README.md
│   ├─ broker-venue-selection.md
│   ├─ rust-trading-stack-evaluation.md
│   ├─ CONCLUSIONS.md
│   ├─ DATA_SOURCES_REFERENCE.md
│   ├─ OPEN_QUESTIONS.md
│   └─ SKILL.md
│
├─ skills/                      # 🎯 Agent skill definitions that compose procedures
│   ├─ README.md
│   ├─ add-doc.md
│   ├─ analyze-impact.md
│   ├─ create-adr.md
│   ├─ ... (through explain-provenance.md)
│   └─ execute-plan.md
│
├─ BRAINSTORM/                  # 💭 Exploratory options, brainstorming, design thinking
│   ├─ README.md
│   ├─ TEMPLATE_BRAINSTORM.md
│   ├─ BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD
│   ├─ BS-002_QUEUE_VS_UI_GAP_ANALYSIS.MD
│   ├─ ... (through BS-006)
│   └─ AUTOMATION_QUEUE_SLICE_PROMPT.MD              # (NEW - moved from root)
│
├─ backlog/                     # 📝 Deferred work items, future roadmap
│   └─ deferred_roadmap.md
│
├─ foundation/                  # 📖 Foundation notes, commentary, principles
│   └─ commentary.md
│
├─ Human Provided Specs/        # 🔐 User-provided APEX system specifications (canonical)
│   ├─ README.md
│   ├─ TEMPLATE_HUMAN_SPEC.md
│   └─ new_specs/
│       ├─ MANIFEST.json
│       ├─ README.txt
│       ├─ canonical/                 # APEX canonical system specs (v2.1)
│       │   ├─ APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md
│       │   ├─ APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md
│       │   ├─ ... (9 spec documents)
│       │   └─ APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md
│       └─ superseded/               # Previous APEX spec versions
│           ├─ APEX_UNIFIED_Full_System_Master_Spec_v2_0.md
│           ├─ APEX_Decision_System_Master_Spec_v1_1_FINAL.md
│           └─ ... (3 others)
│
├─ archive/                     # 📦 Old/obsolete documentation (historical reference)
│   ├─ specs/
│   ├─ canonical-metadata-archive/   # (NEW - archived obsolete metadata)
│   │   ├─ CANONICAL_GLOSSARY.md
│   │   ├─ CANONICAL_SPEC_INDEX.md
│   │   ├─ CANONICAL_TOMBSTONE_INDEX.md
│   │   ├─ CANONICAL_DELETION_LOG.md
│   │   ├─ CANONICAL_MODULE_MAP.md
│   │   └─ MODULE_INVENTORY.md
│   └─ (other old files)
│
├─ REORGANIZATION_SUMMARY.md    # 📋 This reorganization (new file)
└─ FOLDER_MAP.md                # 🗺️ This folder map (new file)
```

## 🎯 Key Improvements

### Cleaner Root (76% reduction)
- **Before:** 50+ scattered files in root
- **After:** 12 focused core files + queue system

### Single Source of Truth for Specs
- **Before:** Specs/ and specs/ (duplicate folders)
- **After:** specs/ only (24 consolidated files)

### Proper Categorization
- All planning docs → docs/plans/plan-sets/
- All operational docs → operations/
- All governance → governance/
- All ADRs properly numbered → adr/

### Obsolete Metadata Archived
- Old canonical indexes consolidated to archive/
- Canonical reference now: Human Provided Specs/ (APEX v2.1)

## 📌 Quick Navigation

| Need | Location |
|------|----------|
| System overview | `artifact.md` + `architecture.md` |
| Architecture decisions | `adr/` (0001-0038) |
| System specs | `specs/` (detailed) + `Specs/` README (overview) |
| Implementation plans | `docs/plans/plan-sets/` (set-A through set-G) |
| How-to procedures | `procedures/` + `skills/` |
| Operational docs | `operations/` + `runbooks.md` |
| APEX canonical specs | `Human Provided Specs/new_specs/canonical/` |
| Research & exploration | `research/` + `BRAINSTORM/` |
| Approved decisions | `open-questions.md` |
| Terminology | `glossary.md` |
| Queue system | Root: `QUEUE.md`, `QUEUE_SCHEMA.md`, etc. |

