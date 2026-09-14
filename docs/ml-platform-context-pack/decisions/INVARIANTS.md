# Invariants

24 statements that must be true of the running system. Each is a build-blocking constraint, not a preference. Each names the mechanism that enforces it — if the mechanism is "review" or "documentation," it is not enforced.

Format: `INV-nn · statement · [enforced by] · (spec §)`

---

## Data plane

**INV-01** — Every table recording a fact about the world has a `knowledge_time` column, and it is written at ingest, never derived later.
*[schema NOT NULL constraint + ingest test]* (§0.1, §1.2)

**INV-02** — There is exactly one read path for market data, and it is point-in-time correct. No non-PIT path exists at any permission level, including development and test.
*[code structure: single reader module; CI test asserts no other module imports the raw table]* (§1.3)

**INV-03** — Prices are stored unadjusted. Adjustment factors are separate, bitemporal, and composed at read time.
*[schema: no `adj_close` column may exist; CI test greps for forbidden column names]* (§0.2, §1.4)

**INV-04** — `instrument_id` is the only key. `symbol` never appears in a primary key or foreign key.
*[schema + CI test over information_schema]* (§1.1)

**INV-05** — Prices are `DECIMAL(38,18)`. No float or double column holds a price.
*[schema + CI test over information_schema]* (§1.2)

**INV-06** — Bar timestamps are the bar OPEN, in UTC, at nanosecond precision.
*[ingest validator + single documented convention]* (§1.2)

**INV-07** — Options store implied volatility with `iv_model`, `iv_rate`, `iv_div`. No greek is stored.
*[schema; greeks computed in the read layer only]* (§1.6)

**INV-08** — Futures continuous series are derived views, never stored as facts. Roll rules carry `decision_time`. Back-adjusted series set `non_reproducible=true` on any dataset that uses them, and that flag propagates to every trial.
*[schema + dataset-spec validator]* (§1.5)

**INV-09** — DeFi observations key on `(chain_id, block_number, block_hash)`. Block number alone is never a key.
*[schema]* (§1.8)

**INV-10** — `asof` joins are backward-only. `strategy='nearest'` is unreachable from application code.
*[wrapper library; the raw function is not exported; CI test asserts no direct import]* (§2)

**INV-11** — Aligned features carry a companion `_age_minutes` and `_quality` column. Forward-fill without exposed age does not occur.
*[feature runtime contract + test]* (§2)

## Datasets and features

**INV-12** — A dataset is identified by a content hash over its full spec, including calendar version, quality exclusion mask, adjustment policy and runtime image digest. Identical `dataset_id` implies byte-identical data.
*[hash function + reproducibility test that rebuilds and compares]* (§3.1)

**INV-13** — Features execute under a windowed view that physically cannot read further back than their declared `lookback_bars`. A feature needing more fails at registration.
*[feature runtime enforces the window; registration test]* (§3.2)

**INV-14** — Exactly one feature implementation serves both backfill and live. Every live serve is logged and diffed nightly against a recomputation.
*[single code path + `feature_consistency_diff` populated; alarm on `code_drift`]* (§3.3)

**INV-15** — `embargo_bars` is computed from the pipeline, never typed by a user. Purge is on `t1`. A lowered embargo or `purge_on='t0'` requires a recorded override that surfaces in every comparison involving that trial.
*[split-spec validator + comparison UI flag]* (§3.5, §12.2)

## The ledger

**INV-16** — No compute is dispatched without a `trial` row in state `REGISTERED` carrying `config_hash`, `prereg_hash` and `propensity`. There is no bypass at any permission level.
*[executor refuses jobs without a REGISTERED trial_id; CI test attempts a bypass and asserts refusal]* (§4.2·1)

**INV-17** — Every trial is recorded, including crashed, cancelled, pre-empted, gate-failed and exploratory. Every terminal transition sets `censoring`.
*[state machine; no terminal path omits the write]* (§4.2·2, §9)

**INV-18** — Full OOS return series and raw per-fold predictions are persisted for every trial that produces them.
*[executor contract + completeness test]* (§4.2·3–4)

**INV-19** — The ledger is hash-chained and append-only. Corrections append with `supersedes`. A daily signed anchor is written to WORM storage.
*[DB trigger computes `row_hash`; no UPDATE grant on the trial table; chain verification job]* (§4.2·8, §4.6)

**INV-20** — Every decision logs `candidate_set`, `chosen`, `propensity`, `policy_id/version`, `exploration_flag` and `decision_tier`. Deterministic logging policies are not permitted.
*[decision API requires the fields; a null propensity is rejected]* (§4.4)

**INV-21** — At least 5% of dispatched trials per campaign are uniform-random exploration draws. The floor is not writable by an agent and not exposed in any tool schema.
*[dispatcher enforces; tool schema omits the field; alarm if achieved fraction < floor]* (§4.5)

## Evaluation and governance

**INV-22** — `N_eff` is computed by the platform from stored return series across the tenant's whole ledger, including gate-failures. No API accepts a self-reported trial count.
*[no parameter exists; computation is server-side only]* (§12.4)

**INV-23** — Gate profiles are immutable and versioned. Agents hold no grant on gate thresholds, `delta_practical` after DEFINE, N_eff computation, the deflation code path, the sealed holdout beyond its single rate-limited call, or `regime_research`.
*[DB grants + versioned profile table with no UPDATE]* (§12.3, §14.3 Tier C)

**INV-24** — Only `platform_physics`, `methodology` and `market_public` information classes cross a tenant boundary. A global model whose feature list contains any other class fails the build.
*[`info_class` column on every feature and insight + build-blocking CI test]* (§7.3)

---

## Two that are not numbered because they are settings, not code

**S-1** — Iceberg `history.expire.max-snapshot-age-ms` ≥ 90 days and `min-snapshots-to-keep` ≥ 50, set **before the first experiment runs**. Every registered artifact gets an Iceberg tag transactionally at registration. Monthly audit asserts 100% tag coverage.
*Defaults are 5 days and 1. Shipping with defaults silently destroys reproducibility of everything older than five days, with no error.* (§0)

**S-2** — Postgres: `FORCE ROW LEVEL SECURITY` on every tenant table; `SET LOCAL` only for tenant context; RLS policies reviewed for the fact that they combine with `OR`.
*Table owners bypass RLS without FORCE. Bare `SET` bleeds tenant context across pooled connections — a cross-tenant leak that passes every test written against a non-pooled connection.* (§7.1)
