# Canonical architecture specifications (APEX)

**Purpose:** Single entry point for **target system architecture** during the **FB-CAN-*** canonical migration. Use this page to decide which documents are **binding for “what we are building toward”** vs **historical or as-implemented**.

**Documentation last reviewed:** 2026-04-18 (FB-CAN-015: [`app/contracts/decision_snapshots.py`](../app/contracts/decision_snapshots.py) — input snapshot contracts).

---

## Precedence (highest first)

1. **`docs/Human Provided Specs/new_specs/canonical/*.md`** — **Canonical APEX** specifications (architecture, contracts, config, replay, monitoring, governance). These define the **intended** end state unless a queue item explicitly defers scope.
2. **`docs/reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`** — Authoritative **gap analysis** between the repo and canonical specs; use for migration ordering and queue ID mapping.
3. **`docs/Specs/*.MD`** — **As-built / as-implemented** maps of the **current** codebase. They remain the best description of **what runs today** but are **not** the target architecture contract during migration. See each file’s header note.
4. **Root-level `docs/Human Provided Specs/*.MD`** (e.g. `MASTER_SYSTEM_PIPELINE_SPEC.MD`) — **Legacy human intent** for the pre-canonical pipeline (forecaster/policy stack). Marked **superseded for target architecture** where they conflict with APEX canonical docs; kept for history and migration traceability.
5. **`docs/Human Provided Specs/new_specs/superseded/`** — Older draft packs; **do not** use for new design decisions.

When two documents conflict on **target** behavior, **canonical APEX** wins. When they conflict on **current code paths**, **`docs/Specs/`** + code win.

---

## Canonical spec files (index)

| Document | Topic |
|----------|--------|
| [`new_specs/canonical/APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md) | Unified master — domains, pipeline, philosophy |
| [`new_specs/canonical/APEX_Canonical_Configuration_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Canonical_Configuration_Spec_v1_0.md) | Configuration domains and versioning |
| [`new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md) | Release gating and promotion |
| [`new_specs/canonical/APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md) | Feature schema and data contracts |
| [`new_specs/canonical/APEX_State_Regime_Logic_Detail_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_State_Regime_Logic_Detail_Spec_v1_0.md) | State / regime / degradation |
| [`new_specs/canonical/APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md) | Trigger stages and math |
| [`new_specs/canonical/APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md) | Opportunity auction |
| [`new_specs/canonical/APEX_Execution_Logic_Detail_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Execution_Logic_Detail_Spec_v1_0.md) | Execution logic and feedback |
| [`new_specs/canonical/APEX_Replay_and_Simulation_Interface_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Replay_and_Simulation_Interface_Spec_v1_0.md) | Replay / simulation |
| [`new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md) | Monitoring and alerting |
| [`new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md) | Experiment registry |

---

## Related links

- **Governance (release gating + experiment registry):** [`GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`](GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD)
- **Deletion log (removed paths + replacements):** [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD)
- **Module map (runtime packages ↔ APEX domains):** [`CANONICAL_MODULE_MAP.MD`](CANONICAL_MODULE_MAP.MD)
- **Decision Service input snapshots (typed):** [`app/contracts/decision_snapshots.py`](../app/contracts/decision_snapshots.py) · builders [`app/contracts/snapshot_builders.py`](../app/contracts/snapshot_builders.py)
- **Queue program:** [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) · [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) §2.7 **FB-CAN-***
- **Gap audit:** [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md)
