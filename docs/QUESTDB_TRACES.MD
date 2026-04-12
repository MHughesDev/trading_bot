# QuestDB — decision traces

When `NM_QUESTDB_PERSIST_DECISION_TRACES=true` (or `questdb.persist_decision_traces` in YAML), `app/runtime/live_service.py` writes each `decision_trace` JSON to the `decision_traces` table via `QuestDBWriter.insert_decision_trace_dict`.

**Backup / restore:** follow QuestDB ops for your deployment (volume snapshots or `questdb` export). The `decision_traces.details` column holds full JSON for audit.

**Bars:** `QuestDBWriter.insert_bar` writes to **`canonical_bars`** (FB-AP-014: `interval_seconds`, `schema_version`). Legacy **`bars`** DDL may still exist on old volumes; new inserts use **`canonical_bars`**.

When **`NM_QUESTDB_PERSIST_CANONICAL_BARS=true`** (or `questdb.persist_canonical_bars` in YAML), `app/runtime/live_service.py` inserts each **completed** OHLC bucket from the Kraken WS path (same roll-up as features; **no** partial-bar rows). Traces-only mode does not require this flag.
