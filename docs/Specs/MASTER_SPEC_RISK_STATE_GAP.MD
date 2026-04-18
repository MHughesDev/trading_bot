# Risk state: app vs policy vs master spec §12

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


**FB-AUDIT-12** — reference for operators and future work.

| Concern | `app.contracts.risk.RiskState` | `policy_model.objects.RiskState` | Master [`MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) §12 |
|--------|-------------------------------|-------------------------------------|----------------------------------------------------------------------------------------------------------|
| Role | Runtime **mode** + **telemetry** (drawdown, spread, data age) | Policy **envelope** for gate (limits, kill switch, long/short) | Twelve **risk gate** fields by name |
| Bridge | — | `policy_model.bridge.policy_envelope_from_app_settings` maps `AppSettings` + app `SystemMode` into policy limits | — |

**Gap:** The app model does **not** carry `concentration_limit`, `daily_loss_limit_remaining`, etc. as live telemetry today; those exist on the **policy** `RiskState` with defaults from settings. Closing the gap means threading measured concentration / daily PnL into `RiskEngine` / policy bridge when those features are prioritized.
