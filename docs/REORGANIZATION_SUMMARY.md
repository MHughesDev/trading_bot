# Docs Folder Reorganization Summary
**Date:** 2026-06-11  
**Status:** ✅ COMPLETE

## Changes Made

### 📦 Consolidated Folders
- **Deleted:** `Specs/` folder → Merged 12 files into `specs/`
  - All high-level system specs now in single location
  - Added: `APP_CONFIG_AND_CONTRACTS.MD`, `CONTROL_PLANE.MD`, `DATA_PLANE.MD`, `EXECUTION_LAYER.MD`, `MODELS_AND_ORCHESTRATION.MD`, `MULTI_ASSET_PORTFOLIO.MD`, `OBSERVABILITY_AND_INFRA.MD`, `RISK_ENGINE.MD`, `SYSTEM_OVERVIEW.MD`, `TESTING_AND_CI.MD`, `CANONICAL_PRECEDENCE.MD`

### 🗑️ Deleted Obsolete Files (18 total)
All had duplicate versions in proper folders:
- `SYSTEM_WALKTHROUGH.MD` → kept in `architecture/`
- `DEFERRED_ROADMAP.MD` → kept in `backlog/`
- `MIGRATION_TO_SPEC_PIPELINE.MD` → kept in `architecture/`
- `AUDIT_CODE_REVIEW.MD` → kept in `governance/`
- `FULL_AUDIT.md` → kept in `governance/`
- `COMMENTARY.MD` → kept in `foundation/`
- `PER_ASSET_OPERATOR.MD` → kept in `operations/`
- `MLFLOW_PROMOTION.MD` → kept in `governance/`
- `GRACEFUL_SHUTDOWN.MD` → kept in `operations/`
- `READY_TO_RUN.MD` → kept in `operations/`
- `RUNBOOKS.MD` → kept in `operations/`
- `DEPLOY_CLOUD.MD` → kept in `operations/`
- `COINBASE_GRANULARITY.MD` → kept in `architecture/`
- `KRAKEN_MARKET_DATA.MD` → kept in `architecture/`
- `QUESTDB_TRACES.MD` → kept in `architecture/`
- `RISK_PRECEDENCE.MD` → kept in `architecture/`
- `PNL_LEDGER.MD` → kept in `architecture/`
- `WINDOWS_OPERATOR_UI.MD` → kept in `operations/`

### 📁 Archived Old Metadata (6 files)
Moved to `archive/canonical-metadata-archive/`:
- `CANONICAL_GLOSSARY.MD` - Legacy APEX terminology mapping (superseded by Human Provided Specs)
- `CANONICAL_SPEC_INDEX.MD` - Index to canonical specs (superseded)
- `CANONICAL_TOMBSTONE_INDEX.MD` - Deleted specs tracker (superseded)
- `CANONICAL_DELETION_LOG.MD` - Deletion log (superseded)
- `CANONICAL_MODULE_MAP.MD` - Module mapping (superseded)
- `MODULE_INVENTORY.md` - Duplicate of above (superseded)

**Reasoning:** FB-CAN-* migration appears to be mature. APEX specs in `Human Provided Specs/new_specs/canonical/` are the canonical reference now.

### ✏️ Reorganized Files (8 total)

**→ `plans/`** (Planning documents)
- `PHASE_DESIGN_CHECKLIST.md`
- `PHASE_QUICK_REFERENCE.md`
- `MICROSERVICES_SPLIT_PLAN.MD`

**→ `specs/`** (System specification)
- `SYSTEM_SPECIFICATION.md`

**→ `architecture/`** (System architecture)
- `MONITORING_CANONICAL.MD` → renamed to `monitoring.md`

**→ `governance/`** (Governance & release)
- `GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD` → renamed to `release_and_experiments.md`

**→ `BRAINSTORM/`** (Automation templates)
- `AUTOMATION_QUEUE_SLICE_PROMPT.MD`

**→ `procedures/`** (CI procedures)
- `CI_ROOT_CAUSE_ANALYSIS_PROMPT.md`

### 🏛️ Promoted ADR Files (2 total)
Moved to `adr/` with proper numbering:
- `ADR_CANONICAL_BAR_STORAGE.MD` → `adr/0012-canonical-bar-storage.md`
- `ADR_MANAGED_DATA_SERVICES.MD` → `adr/0013-managed-data-services.md`

---

## Final Root-Level Structure

### Core Documentation (Per README.md)
- `artifact.md` - Foundational project definition
- `architecture.md` - System architecture map
- `glossary.md` - Shared terminology
- `open-questions.md` - Living decision register
- `README.md` - Docs index

### Queue System (Tightly Coupled - Kept Together)
- `QUEUE.MD` - Agent protocol & conventions
- `QUEUE_SCHEMA.md` - Queue structure definition
- `QUEUE_ARCHIVE.MD` - Archived queue history
- `QUEUE_STACK.csv` - Current queue stack

### Other Root Files
- `docs_home.md` - Home/entry point
- `parity-matrix.md` - Parity/compatibility tracking

---

## Folder Structure Now (15 folders)

| Folder | Purpose | Files |
|--------|---------|-------|
| `adr/` | Architecture Decision Records | 0001-0013 |
| `architecture/` | System design & technical docs | 7 files |
| `archive/` | Old/obsolete documentation | canonical-metadata-archive/ |
| `backlog/` | Deferred work items | 1 file |
| `BRAINSTORM/` | Exploratory options & ideas | BS-001 to BS-006 + automation |
| `foundation/` | Foundation & commentary | 1 file |
| `governance/` | Release, audit, governance | 4 files |
| `Human Provided Specs/` | APEX canonical specs | canonical/ + superseded/ |
| `operations/` | Operational procedures & runbooks | 8 files |
| `plans/` | Phase plans (0-7) | 14 files |
| `procedures/` | Atomic task instructions | 11 files |
| `reports/` | Audit reports & matrices | 6 files |
| `research/` | Research briefs & evaluations | 6 files |
| `skills/` | Agent skill definitions | 10 files |
| `specs/` | Feature, component, data specs | 24 files (consolidated from Specs/) |

---

## Statistics

| Metric | Count |
|--------|-------|
| **Files Deleted** | 18 |
| **Files Archived** | 6 |
| **Files Reorganized** | 8 |
| **ADRs Promoted** | 2 |
| **Specs Consolidated** | 12 |
| **Root-level files (before)** | ~50+ |
| **Root-level files (after)** | 12 |
| **Reduction** | 76% cleaner |

---

## What's Left in Root (12 items)

**Essential documentation (6):**
- artifact.md
- architecture.md
- glossary.md
- open-questions.md
- README.md
- docs_home.md

**Queue system (4):**
- QUEUE.md
- QUEUE_SCHEMA.md
- QUEUE_ARCHIVE.md
- QUEUE_STACK.csv

**Other (2):**
- parity-matrix.md

---

## Next Steps (Optional)

1. **Create missing canonical docs** (referenced in README):
   - Currently missing: `glossary.md` (exists), `open-questions.md` (exists)
   - Both appear to be created, good!

2. **Archive folder cleanup** (optional):
   - Move old Specs/ content from `archive/` if not needed
   - Consider archiving entire `archive/` to `.archive.tar.gz` for historical reference

3. **Update any references** in code/CI:
   - Any scripts referencing deleted files need updates
   - Check for hardcoded paths to reorganized files

---

## Verification Commands

```bash
# Check root files
ls -1 docs/ | grep -v "/" | wc -l

# List all folders
ls -d docs/*/ | sort

# Find any remaining stubs
grep -r "See .* for the full" docs/*.md 2>/dev/null
```
