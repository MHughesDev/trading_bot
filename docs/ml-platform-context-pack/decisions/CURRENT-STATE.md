# Current state vs. spec

> **Status note (2026-09-13):** this document is the **pre-change survey**, kept as the baseline it was written as. Phase 0's trial-ledger work has since landed — see `PROGRESS.md` for what changed and which invariants are now enforced by mechanism. Sections marked ⟢ below have been superseded.

Survey date: 2026-09-12. Read-only — no code touched. Evidence gathered by direct inspection of `migrations/*.sql`, `clickhouse/*.sql`, and the crates named below. Where a claim could not be fully verified in the time available, it is marked **(unconfirmed)**.

**Important framing:** `docs/ml-platform-context-pack/schemas/*.sql` (the pack's own `trial`, `campaign`, tenancy/RLS reference DDL) is documentation, not deployed schema. Nothing in `crates/`/`apps/`/`migrations/`/`clickhouse/` references it. Every "exists" claim below refers to the live system only.

---

## 1. What exists, partially exists, is absent — mapped to spec sections

### §0.1 / §1.2 — Four timestamps, `knowledge_time`

**Partially exists, and only in the newer table.**

- `clickhouse/06_market_bars_v2.sql` (the current, live bars table) has four real timestamp columns: `bar_open_time`, `event_time` (bar **close** — opposite convention from spec §1.2, see below), `available_time` (this *is* the spec's `knowledge_time` — comment explicitly ties it to "what the research cutoff and every PIT filter compare against"), and `ingested_time`.
- The retired table `market_bars` (`clickhouse/02_bars.sql`) only had `available_time` + `ingested_time`, and `available_time` there conflated bar-close with knowledge-time. It is frozen read-only as of 2026-09-11 per an in-file comment — good, it is not being written to, so it is not actively compounding the problem, but its history is exactly the kind of unreconstructable knowledge_time hole §6 of CLAUDE.md anticipates.
- **Gap vs spec:** no confirmed `NOT NULL` constraint audit on `available_time`, and no `venue_ts` column distinct from `event_time`/`bar_open_time` — the spec's four are `event_time, venue_ts, ingest_time, knowledge_time`; the live table's four are `bar_open_time, event_time(=close), ingested_time, available_time`. Conceptually two of the four map cleanly (ingest↔ingested_time, knowledge↔available_time) but `venue_ts` (the *exchange's own* clock, distinct from when the bar interval started/ended) does not exist as a separate column **(unconfirmed whether venue-reported time is ever captured upstream and dropped, or never captured)**.

**Verdict: INV-01 (`knowledge_time` on every fact table) is satisfied for `market_bars_v2` only.** Not satisfied for trades (`01_trades.sql` — needs separate check, not done here), not satisfied for anything in the trial/ledger tables (`outcome`/`result_json` blobs carry no knowledge_time), not satisfied for `instruments` (Postgres).

### §0.2 / §1.4 — Unadjusted prices, adjustment factors at read time

**Fully absent — but so is the failure mode it guards against.** No `corporate_action`, `dividend`, `split` table anywhere; no column named `adj_close`/`adjusted_close`. This is a genuinely crypto-only platform today (per memory and the asset-class survey below), so there is currently no corporate-action stream to get wrong. **This spec section is not yet a live risk — it becomes one the day equities/ETFs are ingested, not before.** Flag as Phase 0-adjacent but not urgent unless equities onboarding is imminent.

### §1.1 / INV-04 — Surrogate `instrument_id`, symbol never a key

**Violated, and this is the single most invasive gap in the whole survey.** `migrations/0002_instruments.sql`: `instrument_id TEXT PRIMARY KEY` — the "id" *is* the symbol string (e.g. `"BTC-USD"`), used as the join key everywhere (`market_bars_v2`, `market_trades`, backtest configs, positions, orders — everything). There is no opaque surrogate key layer at all.

This is not a column-add. Every foreign key in the system, across two databases, is typed as the symbol string. A migration to surrogate IDs touches ClickHouse bars/trades partitioning (repartition, since sort/partition keys are presumably symbol-derived), every Postgres FK, every Rust struct that carries `instrument_id: String`, and every API/agent-tool payload that names an instrument by string today. Treat this as its own multi-step migration project, not a Phase-0 checkbox done in an afternoon.

### §1.2 — Canonical bar shape, `DECIMAL(38,18)`, `TIMESTAMP(9)`, open-convention, sparse

**Mostly matches, with one real convention conflict.**
- Decimal storage: yes — `Decimal128(10)` on price columns in `01_trades.sql` / `06_market_bars_v2.sql`, comments explicitly reject `Float64`. Not `DECIMAL(38,18)` precision exactly (`Decimal128(10)` is 10 fractional digits, not 18) — **(unconfirmed whether 10 digits is sufficient for the stated crypto range 10⁻⁸–10⁵; likely fine for spot prices, worth an explicit check before treating as closed)**.
- Sparse storage: **(unconfirmed)** — not verified whether empty minutes are materialized or absent.
- **Open-vs-close convention conflict:** spec §1.2 says bar timestamp is unconditionally the bar's **open**. The live schema's `event_time` is documented as the bar's **close**, with `bar_open_time` computed by subtracting the period (`crates/backtest/src/store.rs:490-506`). This is exactly the kind of "half of all financial data bugs are bar-convention bugs" trap CLAUDE.md/§1.2 warns about — the two conventions coexist under different column names right now, which is survivable only as long as every consumer knows which column means what. This should be resolved explicitly (which column is the canonical "the" bar timestamp downstream tools default to) rather than left as tribal knowledge.

### §1.3 / INV-02 — Single PIT read path, no non-PIT path at any level

**This is the one clean win — already correct, do not touch casually.** `BarStore` (`crates/backtest/src/store.rs`) is the sole reader of `market_bars_v2`; every consumer (API routes, pipeline manager, bar persistence, sim executor, and a documented thin PIT façade in `crates/model-registry/src/data_view.rs`) goes through it, and PIT filtering (`pit_filter()`) is baked in by default rather than opt-in. This appears to already satisfy INV-02's *structure* (single reader module). What has **not** been verified: whether there's a CI test asserting no other module imports the raw ClickHouse table directly (INV-02's stated enforcement mechanism) — right now the invariant holds by code organization, not by an enforced gate. That's the gap: convention today, not mechanism yet.

### §1.5–§1.9 — Futures, options, DeFi, crypto-specific tables, quality flags

**All absent.** No `option_contract`, `roll_schedule`, `future_contract`, `chain_block`, `funding_rate` tables; no `quality_flags` bitmask column anywhere. `AssetClass` enum in `crates/domain/src/instrument.rs` *names* Option/FuturesExpiring/PerpetualSwap/Bond/Fx/Nft/PredictionMarket as concepts, and some routes wire default venues per asset class, but there is no specialized schema behind any of them — everything is the one generic `instruments` row. **Given the platform is crypto-only in practice today (per prior-session memory and the absence of any equities/futures/options ingestion code), this whole section of the spec is aspirational relative to current scope, not a regression to fix.** It matters once those asset classes are actually onboarded — building the schema ahead of the data existing is optional insurance, not urgent.

### §2 — Cross-asset time alignment, `asof` discipline

**No violation found, but also no infrastructure yet — there's nothing to align.** Single asset class (crypto/spot-like), so the multi-clock alignment problem the spec worries about doesn't exist yet. No SQL `ASOF JOIN` anywhere in the codebase; the one "AsOf" hit is an unrelated Rust PIT-cutoff type (`crates/model-registry/src/data_view.rs:28`), not a nearest-match join — no lookahead bug found. **INV-10 is vacuously satisfied today because the feature doesn't exist; it becomes a real invariant to enforce the day a second asset class with a different clock is added.**

### §3 — L1 dataset/feature plane

- **Feature runtime (§3.3, INV-14): already correct, single implementation.** `crates/features/src/rsi.rs`/`Ema` is the one indicator implementation, consumed identically by backtest sim, live strategy runtime, and the training-data path — no second/competing implementation found. This is the invariant the spec calls the "non-negotiable," and it already holds. **Do not let a second "fast path" feature implementation get introduced under time pressure — that's the one mistake CLAUDE.md §0.3 says is unfixable after the fact.**
- **Lookback declaration and enforcement (§3.2, INV-13): already correct.** `crates/backtest/src/requirements.rs` derives `warmup_bars` from declared feature specs and `sim_executor.rs` hard-fails when the widened data window has no coverage. Genuinely enforced, not just documented.
- **`feature_serving_log` / nightly consistency diff (§3.3, INV-14 second half): absent.** No served-feature logging table, no nightly recompute-and-diff job found. Given there's currently only one code path, the *risk* this guards against (silent divergence between two paths) doesn't exist yet — but the log itself is also the audit trail the spec wants "from day one," and it's cheap to add before a second path ever gets introduced.
- **Dataset content-hashing (§3.1, INV-12), split specs with computed embargo (§3.5, INV-15), label specs (§3.4): not found / not evaluated in this pass** — no `dataset_id`/`split_spec`/`label_spec` tables or equivalent hashing scheme located. Needs a dedicated look before Phase 1 starts; not covered by the agent survey in depth.

### §4 — L2 Trial Ledger

**This is the section with the most existing work, and the most dangerous partial state.**

- Real schema exists: `backtest_runs` (content-addressed `run_id`, immutability trigger), `backtest_studies` (question logged before result, no scoreboard column by design), `backtest_experiments` (monotonic trial counter, single-access holdout vault, state machine), `backtest_nulls`/`backtest_null_choices` (immutable null library, override-reason requirement), `backtest_gate_verdicts` (staged funnel with a "significance never naked" constraint) — migrations `0026`–`0030`. This is a genuinely spec-aligned design for several sub-problems (no bare score, mandatory pre-registration-like question logging, defensible null selection).
- **But none of it is live.** Every corresponding store module (`crates/backtest/src/run/store.rs`, `experiment/store.rs`, `study/store.rs`, `nulls/store.rs`) is explicitly an in-memory reference implementation, and the actual orchestrator (`SuiteManager` in `crates/backtest/src/suite.rs`) holds everything in a process-local `RwLock<HashMap>`. A grep for `INSERT INTO backtest_(runs|studies|experiments|nulls|gate_verdicts)` across the whole repo finds exactly one hit, in a test seed file — zero production writes. **The migrations describe a ledger that does not persist across a process restart.** This is worse than "not built" in one sense: the schema exists, which could create a false impression during review that the invariant is satisfied, when in fact every trial recorded today is lost on redeploy.
- **Missing fields relative to spec §4.1, even once wired to Postgres:** no `prev_hash`/`row_hash` (INV-19 hash chain), no `propensity` (INV-20/INV-16), no `censoring` (INV-17), no `exploration_flag` (INV-21), no `candidate_set` column, no typed `outcome_vector` (§4.3) — `result_json JSONB` is an opaque blob, which is closer to "a score by another name" than the spec's explicit no-`score`-column design, even though the schema comment says it avoids a scoreboard column.
- **A live, agent-reachable bypass exists (INV-16 is actively violated today, not just unfinished).** There are two parallel backtest systems: the Set J Experiment/Study/Run flow above, and an older, separate job tracker (`crates/api/src/routes/backtests.rs` + `crates/backtest/src/manager.rs`, table `backtest_jobs`, renamed off the old `backtest_runs` name). The legacy path has no Experiment, no trial counter, no null library, no gate funnel — and it **is wired to the MCP agent tool `create_backtest`** (`crates/mcp-server/src/tools/backtests.rs`, registered in `lib.rs`), which calls `POST /api/backtests` directly. An LLM agent today can dispatch a real backtest with zero ledger involvement of any kind. This is the clearest INV-16 violation in the survey and the one CLAUDE.md calls out by name ("no `--no-log`, no admin bypass... if a side door exists, an audit will find it was used") — it already exists, unintentionally, as a leftover from before Set J was built.
- **Decision log (§4.4):** partially exists in miniature — `backtest_null_choices` logs chosen-vs-recommended null with a mandatory override reason, which is a real "candidate set / chosen / why" record, but scoped only to null selection. No general-purpose decision log.
- **Forced exploration floor (§4.5, INV-21): absent.** No exploration-flag concept, no floor enforcement, nothing in any agent tool schema to omit (there's nothing to omit because the concept doesn't exist).

### §5 — Knowledge plane (asset embeddings, regimes, outcome tensor, insight store)

**Not evaluated in this pass beyond confirming absence of the underlying schema** (no `asset_embedding`, `regime_causal`/`regime_research` schema split, outcome-tensor table, or `insight` table found in the migrations searched). This is consistent with it being Phase 3 work in the spec's own ordering — nothing here should move ahead of Phase 0/1 regardless.

### §7 — Multi-tenancy

**Does not exist, and — separately from "not built yet" — may not be the right frame yet.** No `tenant_id`, no RLS anywhere. What exists is real per-user auth: `migrations/0013_auth.sql` (password hashes, sessions, resets), and a genuine scope system (`crates/api/src/auth/scopes.rs`) restricting agent/research sessions to an explicit capability whitelist, enforced by a DB trigger (migration 0038). This is single-tenant-with-multiple-users, not multi-tenant SaaS. **Every §7 invariant (INV-23's tenant grants, the feature firewall INV-24, RLS S-2) is currently vacuous because there is only one tenant.** That changes the Phase-0 priority: building tenant partitioning/RLS ahead of an actual second tenant is speculative infrastructure, not data-loss prevention — it does not belong in the "destroys unrecoverable information" category the checklist reserves for Phase 0, unless multi-tenant SaaS is imminent per business plans **(needs Mason's confirmation — see Open Questions)**.

---

## 2. Invariant-by-invariant status

| # | Invariant | Status | Note |
|---|---|---|---|
| INV-01 | `knowledge_time` on every fact table | **Partial** | Present (`available_time`) on `market_bars_v2` only; not on trades/instruments/ledger; NOT NULL not confirmed |
| INV-02 | Single PIT read path | **Mostly satisfied** | `BarStore` is the sole reader; enforcement is by code organization, not a CI test yet |
| INV-03 | Unadjusted prices, factors at read time | **Vacuously satisfied** | No corporate actions exist because no equities are ingested; will need building before that changes |
| INV-04 | Surrogate `instrument_id`, symbol never a key | **Violated** | `instrument_id TEXT PRIMARY KEY` = the symbol itself, used as FK everywhere |
| INV-05 | Prices `DECIMAL(38,18)`, no float | **Mostly satisfied** | `Decimal128(10)` on bars/trades (fewer fractional digits than spec, needs a range check); some analytics tables use `Float64` |
| INV-06 | Bar timestamp = OPEN, UTC, ns | **Conflicted** | Live schema's `event_time` is bar CLOSE; `bar_open_time` is derived. Two conventions coexist under different names |
| INV-07 | Options: IV not greeks | **N/A today** | No options data exists |
| INV-08 | Futures: continuous series as view, `decision_time` | **N/A today** | No futures data exists |
| INV-09 | DeFi: keyed on block_hash | **N/A today** | No DeFi data exists |
| INV-10 | `asof` backward-only, no `nearest` | **Vacuously satisfied** | No asof joins exist at all yet |
| INV-11 | Aligned features carry `_age`/`_quality` | **Not evaluated / likely absent** | No multi-clock alignment exists yet (single asset class) |
| INV-12 | Dataset identified by content hash | **Not found** | No dataset-spec hashing scheme located |
| INV-13 | Feature runtime enforces declared lookback | **Satisfied** | `requirements.rs` + `sim_executor.rs` hard-fail on insufficient window |
| INV-14 | One feature code path, logged & diffed | **Half satisfied** | Single code path is real; serving log + nightly diff job absent |
| INV-15 | `embargo_bars` computed, not typed; purge on `t1` | **Not found** | No split-spec table located |
| INV-16 | No compute without a `REGISTERED` trial row | ⟢ **Now partial** | Enforced by type across the Rust dispatch path; `create_backtest` removed from every agent surface. Legacy `POST /api/backtests` UI route still open — OQ-08 |
| INV-17 | Every trial recorded incl. failures; `censoring` set | ⟢ **Now enforced** | `backtest_trials.censoring` + DB CHECK on terminal states; failures and cache hits both recorded |
| INV-18 | Full OOS returns + per-fold predictions persisted | **Not evaluated** | Needs a dedicated look at what `result_json` actually contains |
| INV-19 | Hash-chained, append-only ledger | ⟢ **Now partial** | DB trigger computes `row_hash`; DELETE and registration-fact mutation refused. Lifecycle columns still advance in place — ADR-P0-06 |
| INV-20 | Every decision logs propensity etc.; no deterministic logging | ⟢ **Now enforced** | `propensity` NOT NULL unless `policy_id='legacy_unlogged'`, range-checked, in both DB and Rust |
| INV-21 | ≥5% forced exploration floor, not writable | **Violated (absent)** | Concept doesn't exist |
| INV-22 | Platform-computed `N_eff`, no self-report | **Not found** | No such computation located |
| INV-23 | Gate profiles immutable/versioned; tight grants | **Partial** | Gate funnel exists (`backtest_gate_verdicts`) and looks immutable by design, but unwritten in production; grant restrictions not evaluated |
| INV-24 | `info_class` firewall for cross-tenant features | **N/A today** | No multi-tenancy exists |

**Reading this table straight: 6 invariants are violated in ways that matter today (04, 06 as a live ambiguity, 16, 17, 19, 20, 21 — that's actually 7), 3 are satisfied or mostly satisfied by real mechanism (02, 13, and half of 14), and the remaining ~11 are either vacuously true (nothing to violate yet, given single-asset-class/single-tenant scope) or simply not evaluated in this pass and need a follow-up read (12, 15, 18, 22).**

---

## 3. Migrations needed vs. simple edits

**Needs a real migration (schema change + backfill + code touch-points across multiple crates):**
- Surrogate `instrument_id` (§1.1) — the largest single migration in this survey; touches both databases and every Rust struct/API payload naming an instrument.
- Wiring the existing Set J ledger schema (`backtest_runs/studies/experiments/nulls/gate_verdicts`) to a real Postgres pool instead of `InMemoryRunStore`/`SuiteManager`'s `RwLock<HashMap>` — this is "just" plumbing (the schema already exists) but touches the whole orchestration path in `crates/backtest/src/suite.rs`.
- Extending that same ledger schema with `prev_hash`/`row_hash`, `propensity`, `censoring`, `exploration_flag`, `candidate_set` — additive columns, but the hash-chain trigger and the "no compute without REGISTERED" executor guard are real logic, not just DDL.
- `knowledge_time` on trades and instruments (currently only on `market_bars_v2`) — additive column + backfill-with-sentinel per CLAUDE.md §6.
- Retiring the legacy `create_backtest` MCP tool / `backtest_jobs` path, or wiring it into the ledger — this is a decision (see Open Questions), not pure mechanics, but whichever way it goes touches migrations (either drop `backtest_jobs` or bring it into the trial-row-required world).

**Simple, additive edits (no backfill, no cross-crate touch):**
- Resolving the open/close naming ambiguity between `event_time` and `bar_open_time` — likely a documentation/consumer-contract fix plus possibly a rename, not a data migration.
- `feature_serving_log` table + nightly diff job — new table, no existing data affected.
- CI test asserting no module besides `BarStore` reads `market_bars_v2` directly (turns INV-02 from convention into mechanism) — a grep-based test, cheap.
- Exploration floor (INV-21) — new column + dispatcher-side enforcement, additive.

---

## 4. What is already correct — do not touch without reason

- **`BarStore` as the single PIT read path.** Already matches INV-02's intent structurally. The risk here is someone adding a "just for this one dashboard query" direct ClickHouse read under time pressure — that's the failure mode INV-02 exists to prevent, and it hasn't happened yet.
- **The single feature implementation (`crates/features`) serving both backtest and live.** This is §0.3's non-negotiable and it already holds. Any future "fast path" or "simplified live version" of a feature is the mistake to guard against, not a shortcut to take.
- **Declared-and-enforced lookback (`requirements.rs`).** Genuinely matches INV-13's intent (fails at registration/run-start, not silently).
- **The Set J *schema design itself*** (immutable-by-trigger runs, no-scoreboard studies, mandatory null-override-reason, staged gate funnel with "significance never naked"). The design is spec-aligned even though the wiring isn't — this should be extended and completed, not replaced with a different ledger design.
- **The auth/scope system** (`crates/api/src/auth/scopes.rs`, migration 0038's DB-triggered restriction). This is a genuine mechanism-not-policy enforcement pattern the spec asks for elsewhere (grants, not review) — worth reusing as the template when the ledger executor guard and the feature firewall get built.

---

## 5. Specific answers to the questions asked

- **Does the market data layer have `knowledge_time`?** Yes, on the current bars table only (`market_bars_v2.available_time`), not on trades or instruments, and not NOT-NULL-confirmed.
- **Are adjusted prices stored?** No — but only because no corporate-action-bearing asset class is ingested yet, not because of a deliberate unadjusted-at-read-time design.
- **Is there more than one feature implementation?** No — single implementation, already correct.
- **Can a training/backtest run happen without a ledger row?** ⟢ At survey time: yes, via the legacy `create_backtest` MCP tool → `backtest_jobs` path. **Since fixed for every agent surface** (tool removed, dispatch refuses the name, CI test keeps it closed). Still possible for a human through `POST /api/backtests` — OQ-08.
- **Are there non-PIT read paths?** None found for bars specifically (`BarStore` is universal). Not evaluated for trades or other tables.

---

## 6. Not evaluated in this pass — needs a follow-up read before Phase 1 planning is final

- Trades table's own timestamp/knowledge_time story (only bars were checked in depth).
- Whether `result_json`/`backtest_run_series` actually contains full OOS return series and per-fold predictions (INV-18) or only summaries.
- Dataset/split/label spec hashing (INV-12, INV-15) — no table found, but not exhaustively searched for an equivalent mechanism under a different name.
- `N_eff` computation (INV-22) — not located, not confirmed absent with high confidence.
- Sparse-storage confirmation for `market_bars_v2` (are empty minutes materialized?).
