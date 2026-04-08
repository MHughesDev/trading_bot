# MLflow — manual promotion only (Master Spec)

- Training/orchestration jobs may **log** metrics and artifacts to MLflow.
- **Do not** configure automatic transition to Production stage in CI.
- Human reviews evaluation → approves → runs promotion (UI or one-off script).

`models/registry/mlflow_registry.py` `promote()` is intentionally a no-op for staging changes.

**CI guard:** `scripts/ci_mlflow_promotion_policy.sh` fails the build if `transition_model_version_stage` or `set_registered_model_alias` appears in Python (catch accidental auto-promotion code).
