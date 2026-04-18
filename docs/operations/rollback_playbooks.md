# Rollback playbooks (APEX / FB-CAN-053)

**Spec:** [`APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`](../Human%20Provided%20Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md) §10.

Every **promotable** release candidate (`ReleaseCandidate` in `models/registry/release_ledger.json` or via `POST /governance/release-objects`) must carry a **`rollback`** block that is **actionable**:

| Field | Role |
|-------|------|
| `target_config_version` / `target_logic_version` / `target_model_family_ref` / `target_feature_family_refs` | Immutable pointers to the last known-good state (at least one required unless `instructions` alone is sufficient for your runbook). |
| `instructions` | Operator steps (redeploy prior YAML, swap checkpoint, toggle feature families, restart processes). **Minimum length** enforced by `validate_rollback_playbook` for simulation/shadow/live gates. |
| `trigger_conditions` | When to execute rollback (metric thresholds, failed gates, incident class). **Minimum length** enforced for non-research promotions. |
| `rollback_owner` | Accountable owner for execution. **Required** for promotion beyond **research**. |

**CI:** `scripts/ci_rollback_playbook.sh` runs `orchestration/rollback_validation.validate_rollback_playbook` on the same candidate fixture as `ci_canonical_contracts.sh`, so rollback viability is checked in the merge gate.

**Gates:** `evaluate_promotion_gates` adds **`rollback_playbook`** when playbook fields are missing or too short (FB-CAN-053).

**Related:** [`GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`](../GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD) · `GET /governance/rollback-playbook` (machine-readable requirements).
