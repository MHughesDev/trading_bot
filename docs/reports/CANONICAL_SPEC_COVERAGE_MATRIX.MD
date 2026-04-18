# Canonical spec coverage matrix (FB-CAN-070)

**Machine-readable source:** [`CANONICAL_SPEC_COVERAGE_MATRIX.json`](CANONICAL_SPEC_COVERAGE_MATRIX.json) (`schema_version: 1`)

**Purpose:** Map **[`REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md) §3** domains (A–O) to canonical spec references, **FB-CAN-*** queue IDs, primary implementation paths, and test glob patterns. Use this as the **coverage sign-off artifact** before final migration closeout (**FB-CAN-078** acceptance audit).

**CI:** `python3 scripts/ci_canonical_coverage_matrix.py` (also run from `scripts/ci_canonical_gates.sh`) validates:

- JSON shape and `schema_version`
- every `implementation_paths` entry exists in the repo
- every `queue_id` is registered in `scripts/generate_queue_stack.py`
- each row has at least one test file matching its `test_path_patterns`

## Operator checklist (closure)

See the `migration_signoff_checklist` array in the JSON file. In short:

1. Gap domains A–O are represented in the matrix rows.
2. Listed paths remain accurate when code moves (update the JSON).
3. Open **FB-CAN-*** items are tracked in the queue system until closed.
4. Run **FB-CAN-078** automated acceptance audit before declaring migration complete.

## Related

- **Code ↔ domain map:** [`CANONICAL_MODULE_MAP.MD`](../CANONICAL_MODULE_MAP.MD)
- **Canonical spec index:** [`CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD)
