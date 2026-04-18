# Release gating and experiment registry (APEX)

**Slice:** FB-CAN-011 · **Specs:** [`APEX_Config_Management_and_Release_Gating_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md) · [`APEX_Research_Experiment_Registry_Spec_v1_0.md`](Human%20Provided%20Specs/new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md)

## What is implemented in code

| Capability | Location |
|------------|----------|
| Release candidate model, evidence package, rollback metadata, promotion gate evaluation | `orchestration/release_gating.py` |
| JSON ledger path (optional) | `models/registry/release_ledger.json` via `default_release_ledger_path()` |
| Experiment records, registry queries, link to release candidate | `models/registry/experiment_registry.py` |
| Default experiment registry file | `models/registry/experiment_registry.json` |
| CLI validation of a candidate JSON against gates | `scripts/validate_release_gates.py` |

## Operator workflow (minimal)

1. **Experiments:** Append or update rows in `experiment_registry.json` using the `ExperimentRecord` schema (or call `upsert_experiment` from tooling). Use `query_experiments` for filters (domain, status, tags, notes substring, linked release).
2. **Release candidates:** Append `ReleaseCandidate` entries to `release_ledger.json` (or build in Python and `write_release_ledger`).
3. **Promotion:** Run `python3 scripts/validate_release_gates.py --candidate <file.json> --target live` (or `shadow` / `simulation`). Exit code `0` = gates pass; `2` = blocked (see printed JSON for `blocked_gates`).

## Notes

- This does **not** auto-promote registered MLflow models; CI still enforces no programmatic promotion in application code (`scripts/ci_mlflow_promotion_policy.sh`).
- Canonical config versioning remains in `app/config/canonical_config.py` (`CanonicalMetadata.config_version`); release records should reference the same version strings for traceability.
