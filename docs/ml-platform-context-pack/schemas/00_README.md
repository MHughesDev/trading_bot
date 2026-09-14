# Schemas

Executable DDL extracted and expanded from `spec/SPEC.md`. Postgres dialect for the system of record; Iceberg tables are expressed as Postgres-shaped DDL for readability and should be translated to your catalog's DDL.

Apply in numeric order. Every file is idempotent-safe to read but **not** to re-run — review before executing against an existing database.

| File | Contents | Spec § |
|---|---|---|
| `01_identity.sql` | instruments, symbols, venues | §1.1 |
| `02_market_data.sql` | bars, corporate actions, futures, options, crypto, chain | §1.2–1.9 |
| `03_datasets.sql` | feature/label/split specs, serving log, consistency diff | §3 |
| `04_ledger.sql` | trial ledger, decisions, trajectories, outcome type | §4, §14.6 |
| `05_knowledge.sql` | embeddings, regimes, outcome tensor, insights | §5 |
| `06_tenancy.sql` | RLS, grants, feature firewall | §7, §5.4, §14.3 |

**Before running any of this:** set Iceberg retention (`history.expire.max-snapshot-age-ms ≥ 90d`, `min-snapshots-to-keep ≥ 50`). Defaults are 5 days and 1, and shipping with them silently destroys reproducibility of everything older than five days.
