# QuestDB — decision traces

When `NM_QUESTDB_PERSIST_DECISION_TRACES=true` (or `questdb.persist_decision_traces` in YAML), `app/runtime/live_service.py` writes each `decision_trace` JSON to the `decision_traces` table via `QuestDBWriter.insert_decision_trace_dict`.

**Backup / restore:** follow QuestDB ops for your deployment (volume snapshots or `questdb` export). The `decision_traces.details` column holds full JSON for audit.

**Bars:** `QuestDBWriter.insert_bar` writes to **`canonical_bars`** (FB-AP-014: `interval_seconds`, `schema_version`). Legacy **`bars`** DDL may still exist on old volumes; new inserts use **`canonical_bars`**. Batching from the live path is a follow-up.
