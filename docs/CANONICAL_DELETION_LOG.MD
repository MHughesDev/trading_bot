# Canonical migration — deletion log

Records **removed** paths and **replacement** pointers so contributors can trace what left the tree and why. Complements [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md) §5.1.

| Removal (commit / slice) | What was removed | Replacement / notes |
|--------------------------|------------------|---------------------|
| FB-CAN-012 (2026-04-18) | `docs/Specs/MASTER_SPEC_RISK_STATE_GAP.MD` | Superseded by the repo vs canonical gap audit ([`REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md)) and APEX canonical risk/state specs. For app vs policy `RiskState` naming, see [`Specs/RISK_ENGINE.MD`](Specs/RISK_ENGINE.MD) and [`app/contracts/risk.py`](../app/contracts/risk.py). Historical audit reference: **FB-AUDIT-12** in [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD). |
| FB-CAN-019 (2026-04-18) | `decision_engine/action_generator.py`, `tests/test_action_generator_routes.py` | Standalone route→`forecast.returns_5`→proposal helper was unused on the live/replay path (`run_decision_tick` → `DecisionPipeline` → `PolicySystem` + `run_spec_policy_step`). Proposal sizing semantics remain in [`spec_policy_proposal.py`](../decision_engine/spec_policy_proposal.py) (`SPEC_POLICY_DELTA_TO_SIZE_FRACTION_SCALE`). |
| FB-CAN-020 (2026-04-18) | `risk_engine/equal_weight.py`, `tests/test_equal_weight.py` | Deferred **FB-N3** reference stub not wired into `RiskEngine`; canonical sizing is [`canonical_sizing.py`](../risk_engine/canonical_sizing.py). Historical queue ref: **FB-N3-S1** in [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD). |
| FB-CAN-021 (2026-04-18) | `get_execution_adapter` in [`execution/router.py`](../execution/router.py) | Thin duplicate of **`create_execution_adapter`**; all call sites use **`create_execution_adapter`** (`ExecutionService`, tests). |
| FB-CAN-022 (2026-04-18) | Top-level **`risk:`** block in `default.yaml` | Superseded by **`apex_canonical.domains.risk_sizing`** + flat `AppSettings` / **`NM_RISK_*`**; **`load_settings`** rejects YAML **`risk:`**. |

**As-built Specs retained:** `docs/Specs/*.MD` stays the **implementation** map (see [`CANONICAL_MODULE_MAP.MD`](CANONICAL_MODULE_MAP.MD)); gap-audit §5.1 wholesale removal of those files is **not** required for canonical alignment. Use **FB-CAN-023** / migration queue items if a doc is truly obsolete.
