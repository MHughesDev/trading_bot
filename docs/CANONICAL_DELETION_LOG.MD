# Canonical migration — deletion log

Records **removed** paths and **replacement** pointers so contributors can trace what left the tree and why. Complements [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md) §5.1.

| Removal (commit / slice) | What was removed | Replacement / notes |
|--------------------------|------------------|---------------------|
| FB-CAN-012 (2026-04-18) | `docs/Specs/MASTER_SPEC_RISK_STATE_GAP.MD` | Superseded by the repo vs canonical gap audit ([`REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md)) and APEX canonical risk/state specs. For app vs policy `RiskState` naming, see [`Specs/RISK_ENGINE.MD`](Specs/RISK_ENGINE.MD) and [`app/contracts/risk.py`](../app/contracts/risk.py). Historical audit reference: **FB-AUDIT-12** in [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD). |

**As-built Specs retained:** `docs/Specs/*.MD` stays the **implementation** map (see [`CANONICAL_MODULE_MAP.MD`](CANONICAL_MODULE_MAP.MD)); gap-audit §5.1 wholesale removal of those files is **not** required for canonical alignment. Use **FB-CAN-023** / migration queue items if a doc is truly obsolete.
