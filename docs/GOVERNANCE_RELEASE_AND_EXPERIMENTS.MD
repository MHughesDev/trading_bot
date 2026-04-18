# Release gating and experiment registry (APEX)

**Slice:** FB-CAN-011 · **Specs:** [`APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md) · [`APEX_Research_Experiment_Registry_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md)

## What is implemented in code

| Capability | Location |
|------------|----------|
| Release candidate model, evidence package, rollback metadata, promotion gate evaluation | `orchestration/release_gating.py` |
| Evidence bundle JSON, canonical fingerprint, structured diff vs baseline YAML (**FB-CAN-026**) | `orchestration/release_evidence.py`, `scripts/build_release_evidence_bundle.py` |
| Control plane: current bundle + diff vs pasted `default.yaml` | `GET /governance/release-evidence`, `POST /governance/release-evidence/diff` (`control_plane/api.py`) |
| JSON ledger path (optional) | `models/registry/release_ledger.json` via `default_release_ledger_path()` |
| Experiment records, registry queries, lifecycle validation, bidirectional link to release ledger (**FB-CAN-027**) | `models/registry/experiment_registry.py` |
| Experiment HTTP API (list/get/upsert/delete) | `GET/POST/DELETE /governance/experiments` in `control_plane/api.py` |
| Default experiment registry file | `models/registry/experiment_registry.json` |
| CLI validation of a candidate JSON against gates | `scripts/validate_release_gates.py` |
| CI merge gate (checked-in fixture must pass `--target live`) | `scripts/ci_canonical_contracts.sh` (GitHub Actions **lint-test** job) |

## Operator workflow (minimal)

1. **Experiments:** Append or update rows in `experiment_registry.json` using the `ExperimentRecord` schema (or call `upsert_experiment` from tooling). Use `query_experiments` for filters (domain, status, tags, notes substring, linked release).
2. **Release candidates:** Append `ReleaseCandidate` entries to `release_ledger.json` (or build in Python and `write_release_ledger`).
3. **Evidence bundle (config fingerprint + optional diff):** `python3 scripts/build_release_evidence_bundle.py --out evidence.json` (add `--baseline app/config/default.yaml` to include a structured diff vs that YAML merge). Attach or reference `evidence.json` from the promotion PR.
4. **Control plane:** `GET /governance/release-evidence` returns the running process bundle; `POST /governance/release-evidence/diff` with body `{"baseline_yaml": "<full default.yaml text>"}` returns a diff vs the running merged canonical config.
5. **Experiments (FB-CAN-027):** Use `GET /governance/experiments` with optional query filters; `POST /governance/experiments` with a full `ExperimentRecord` JSON (mutating routes require API key / session as other control-plane mutators). Linking `linked_release_candidate` to a `release_id` requires that release to exist in `models/registry/release_ledger.json` first; the ledger’s `linked_experiment_ids` is updated automatically. Status transitions follow the spec §4 table (invalid transitions return 422).
6. **Promotion:** Run `python3 scripts/validate_release_gates.py --candidate <file.json> --target live` (or `shadow` / `simulation`). Exit code `0` = gates pass; `2` = blocked (see printed JSON for `blocked_gates`).

## Notes

- This does **not** auto-promote registered MLflow models; CI still enforces no programmatic promotion in application code (`scripts/ci_mlflow_promotion_policy.sh`).
- Canonical config versioning remains in `app/config/canonical_config.py` (`CanonicalMetadata.config_version`); release records should reference the same version strings for traceability.
