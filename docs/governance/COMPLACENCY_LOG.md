# Complacency Log: Incomplete Work & Technical Debt

**Last Updated:** 2026-09-27 (v3 — the CRITICAL was stale; Set S ranges recorded)

**Set S migration ranges, recorded here so concurrent tracks cannot collide**
(MASTER §5). These are the *declared* ranges. Several were allocated against a
tree that had already moved past them, and the real numbers are recorded per task
in each phase file — the collision is documented as ADR-S3-08. Allocate from the
actual head, not from this table, and check `migrations/` before you take a number.

| Phase | Declared range | Actually shipped |
|---|---|---|
| 2 — One runtime | 0061–0064 | 0061, 0062 (as `0061_agent_effect_log.sql`) |
| 4 — Memory and evaluation | 0065–0069 (memory), 0070–0073 (evaluation) | 0122, 0123, 0124, 0125 |
| 7 — The monitor | 0074–0078 | 0074–0078 as declared |
| 3 — The catalogue | 0079–0081 | — |
| 5 — The language | 0082–0084 | — |
| 8 — Integration and honesty | 0085–0088 | 0134, 0135, 0136 |
| 9 — Context as retrieval | none needed | none — the range it declared was never used |
| 10 — Modes and the lifecycle | 0089–0091 | 0126, 0128, 0131 |
| 11 — Continuous research | 0092–0096 | 0138 |

Highest migration on disk at the time of writing: **0140**.
**Scope:** Entire repository — all stubs, placeholders, `#[ignore]` tests, incomplete integrations, and unimplemented features

> **Implementation plan:** every open item below (plus 8 findings from the
> 2026-06-13 sweep this log missed — dead `/auth/*` routes, frontend builder
> gaps, parse-only sizing, hot-path stage-3, unused graph/semantic infra) is
> sequenced in [`docs/plans/plan-sets/set-E/`](../plans/plan-sets/set-E/MASTER.md). See its Item → Phase Map
> for the cross-reference.

---

## Column Definitions

| Column | Meaning |
|--------|---------|
| **File Path** | Location of the incomplete work |
| **Category** | Type: Auth, Metrics, Collector, Adapter, API, DB, Tests, Tooling, Docs, Config |
| **Severity** | CRITICAL (blocks deployment), HIGH (breaks feature), MEDIUM (degrades feature), LOW (nice-to-have) |
| **Issue** | What's incomplete or lazy |
| **Current State** | What code/implementation exists now |
| **Required State** | What needs to exist for production readiness |
| **Phase** | Scheduled phase or "Post-Phase 6" |
| **Blocker** | Dependencies or things preventing completion |
| **Notes** | Additional context, related issues, cross-references |

---

## Master Table: All Incomplete Work

| File Path | Category | Severity | Issue | Current State | Required State | Phase | Blocker | Notes |
|-----------|----------|----------|-------|----------------|----------------|-------|---------|-------|
| ~~`crates/api/src/auth/session.rs:9-14`~~ | Auth | ~~**CRITICAL**~~ **RESOLVED 2026-09-27** | ~~Placeholder bearer token auth~~ | — | Done | — | — | **This entry was stale and is corrected, not closed by new work.** `session.rs:57-88` resolves the bearer token against `sessions` with `expires_at > now()` and returns that session's `scopes`; `auth::scopes::permits` gates on them, and `permits_monitor_control` is the single door every monitor POST goes through. The `!t.is_empty()` filter this entry described is a pre-check before the database lookup, not the authorisation. Verified by reading the extractor. |
| `config/default.toml` | Config | **LOW** | Hardcoded dev credentials | Postgres password `trading:trading` in TOML | Move all secrets to environment variables for production | Phase 1 | None | Downgraded: values are dev-environment defaults, not real secrets. Still must be replaced before any production deployment. |
| `crates/execution/src/coinbase.rs:1` | Adapter | **HIGH** | Coinbase execution adapter stub | Single comment: "stub (post-Phase 6 scope)" | Full REST implementation: place orders, query status, handle fills | Post-Phase 6 | Out of scope | Live Coinbase trading not possible. |
| `crates/execution/src/account/tradovate.rs` | Adapter | **HIGH** | Tradovate account adapter not implemented | Returns `Err(AccountSourceError::NotImplemented)` for all methods | Fetch balances, positions, transactions from Tradovate REST | Phase 4 | REST endpoints unknown | Now fails loudly (was silently returning empty). Account state queries error out. |
| `crates/execution/src/account/tradier.rs` | Adapter | **HIGH** | Tradier account adapter not implemented | Returns `Err(AccountSourceError::NotImplemented)` for all methods | Fetch balances, positions, transactions from Tradier REST | Phase 4 | REST endpoints known, API key auth | Now fails loudly (was silently returning empty). |
| `crates/api/src/routes/orders.rs` | API | **HIGH** | Manual order placement disabled | Returns `503 SERVICE_UNAVAILABLE` — position/mark-price data not yet wired | Fetch position from Postgres, mark price from Redis, P&L from ledger, active status from registry | Phase 2 | Postgres query builders, Redis integration | Fixed from false-permissive (was passing with position=0); now safely rejects until Phase 2 data wiring. |
| `crates/api/src/routes/dashboard.rs:47` | API | **HIGH** | Dashboard returns all-zero P&L rollup | Returns `RollupResponse` with all `Decimal::ZERO`; comment: "TODO: fetch from Postgres and Redis" | Fetch `pnl_lots`, `pnl_closes` from Postgres; compute realized/unrealized P&L and win rate | Phase 2 | Postgres pnl schema (migration 0008 now exists), Redis integration | Users see $0 P&L regardless of actual position state. |
| `crates/execution/src/venues/zerox.rs:57` | Adapter | **MEDIUM** | 0x adapter transaction hash query stub | Comment: "stub — should fetch firm quote" | Fetch real on-chain transaction status from 0x API | Phase 4 | 0x HTTP wiring | DEX order fills not verified on-chain. |
| `crates/execution/src/venues/zerox.rs:116` | Adapter | **MEDIUM** | 0x adapter order query stub | `query_order()` always returns `BrokerOrderState::Filled` with dummy values | Implement real order state tracking via 0x API | Phase 4 | 0x HTTP wiring | All paper DEX trades assumed filled immediately. |
| `crates/execution/src/paper/amm_swap.rs:1-30` | Adapter | **MEDIUM** | AMM/DEX paper simulator skeleton | Takes caller-supplied `FirmQuote` (mocked); no real quote source | Phase 4: HTTP wiring to 0x `/price` endpoint for real price impact | Phase 4 | 0x HTTP integration | Paper DEX trades use mocked quotes only. |
| `crates/observability/src/metrics.rs:1-17` | Metrics | **MEDIUM** | Prometheus metrics not implemented | No-op stubs: `increment_published()`, `increment_quarantined()`, `increment_gap_detected()` | Wire to real Prometheus registry; expose `/metrics` endpoint | Phase 2 | Prometheus dependency | Zero production visibility into event flow. |
| `crates/observability/src/correctness.rs:1-15` | Metrics | **MEDIUM** | Correctness metrics not implemented | No-op stubs: `record_consumer_lag()`, `record_quarantine_rate()`, `record_reconciliation_divergence()` | Implement real correctness metrics; validate data pipeline integrity at runtime | Phase 2 | Storage integration | Cannot detect data corruption or replay divergence. |
| `crates/mcp-server/src/` | Tests | **MEDIUM** | MCP server module has zero tests | 5 source files, 0 tests | Add tests for tool discovery, lane queries, instrument enumeration | Phase 5 | None | Tools not validated against real platform state. |
| `crates/builders/src/orderbook.rs` | Builder | **MEDIUM** | Order book reconstruction not implemented | Empty struct with only `new()` and `Default`; no methods | Full L2 order book from feed events; track bids/asks, handle gaps | Phase 2 | None | Strategies cannot access depth data or market structure. |
| `crates/reconciliation/src/freshness.rs:81-85` | Reconciliation | **MEDIUM** | Trading hours check uses UTC instead of instrument timezone | UTC-based session comparison only; comment: "simplified UTC-only comparison here since full tz support would require the chrono-tz crate (out of scope for Phase 2)" | Use `chrono-tz` to convert UTC to instrument's local trading timezone before comparing | Phase 2 | `chrono-tz` crate | Equity instruments in non-UTC timezones (e.g. NYSE = America/New_York) will get incorrect hours detection. **New finding.** |
| `crates/storage/src/parquet/compaction.rs` | Storage | **LOW** | Nightly compaction not implemented | Logs `tracing::warn` and returns `Ok(())`; does not compact | Scheduled nightly job that merges small Parquet files per partition | Phase 2 | None | Now logs visibly (was silent). Query performance degrades over time. |
| `crates/storage/src/postgres/users.rs:2` | Storage | **LOW** | User management placeholder | Only contains `count()` function; comment: "placeholder — to be filled in Phase 2" | Full user CRUD, role management, permission checks | Phase 2 | None | Users table exists but management operations incomplete. |
| `crates/mcp-server/src/tools/discovery.rs:4-5` | API | **LOW** | MCP discovery uses hardcoded static data | Returns fixed BTC-USDT, ETH-USDT, SOL-USDT on Binance; comment: "Phase 5 returns representative static data" | Query live platform registries: `list_lanes()` and `list_instruments()` | Phase 5 | API routes | Agents reason about hardcoded data, not real platform state. |
| `crates/api/src/ws/live.rs:141` | API | **LOW** | WebSocket serialization failure swallowed silently | `serde_json::to_string(msg).unwrap_or_default()` returns empty string on error | Return an error frame or log and skip; never send empty text message | Phase 2 | None | Client receives empty WS message instead of data if serialization fails. **New finding.** |
| `docs/observability/` | Docs | **LOW** | Missing operational procedures for metrics/correctness | No procedures exist | Document: metrics dashboard setup, alert thresholds, gap detection procedures | Phase 2 | None | Ops team has no guidance on monitoring system health. |
| `docs/procedures/user-management.md` | Docs | **LOW** | User management operational procedures missing | Does not exist | Create: user onboarding, role assignment, permission management | Phase 2 | None | Ops team has no guidance on access management. |

---

## Completed Since Last Scan (v1 → v2)

| Item | What Was Done |
|------|--------------|
| `migrations/0006_asset_class_registry.sql` | **CREATED** — `asset_class_registry` + `data_type_registry` tables with seed data |
| `migrations/0007_credential_store.sql` | **CREATED** — `venue_credentials` table (fills sequence gap) |
| `migrations/0008_pnl_schema.sql` | **CREATED** — `pnl_lots` + `pnl_closes` tables per C-073/C-105 |
| `apps/platform/src/main.rs` | **GUARDRAIL ADDED** — refuses to bind non-loopback address while auth is placeholder (M-17) |
| `xtask/src/main.rs` | **IMPLEMENTED** — real grep-based f64 scanner; exits 1 on violations; skips test code |
| `crates/collectors/src/crypto/mod.rs` | **CLEANED** — stale `TODO(Phase 1)` comment removed (Kraken IS wired) |
| `crates/collectors/src/equity/mod.rs` | **CLEANED** — stale `TODO(Phase 6)` comment removed (Alpaca IS wired) |
| `crates/graph/src/populate.rs:226` | **CLEANED** — stale "never populated" comment removed (edges ARE populated) |
| `crates/execution/src/account/tradovate.rs` | **IMPROVED** — returns `Err(NotImplemented)` instead of silent `Ok(vec![])` |
| `crates/execution/src/account/tradier.rs` | **IMPROVED** — returns `Err(NotImplemented)` instead of silent `Ok(vec![])` |
| `crates/execution/src/account_source.rs` | **IMPROVED** — `NotImplemented` variant added to `AccountSourceError` |
| `crates/api/src/routes/orders.rs` | **FIXED** — now returns 503 instead of false-permissive position=0 defaults |
| `crates/storage/src/parquet/compaction.rs` | **IMPROVED** — now logs `tracing::warn` instead of silently returning `Ok(())` |
| `crates/collectors/src/crypto/kraken.rs` | **FIXED** — wire deserialization uses `String` instead of `f64` for price/qty |
| `crates/collectors/src/equity/alpaca_data.rs` | **FIXED** — wire deserialization uses `String` instead of `f64` for price/size |
| `crates/collectors/src/options/tradier.rs` | **FIXED** — `bidsize`/`asksize` renamed with `#[serde(rename)]`; helper fns renamed |
| `crates/features/src/ema.rs` | **FIXED** — `price` parameter renamed to `value` (indicator math, not monetary storage) |
| `crates/features/src/rsi.rs` | **FIXED** — `price`/`prev_price` renamed to `value`/`prev_value` |

---

## By Severity

### 🔴 CRITICAL (Blocks Production Deployment) — 0 Items

**The one CRITICAL this log carried was stale, and that is worse than it sounds.**

1. ~~**Auth Placeholder (M-17)** — `crates/api/src/auth/session.rs`~~ —
   **corrected 2026-09-27.** The entry said any non-empty bearer token was
   accepted. `session.rs:57-88` resolves the token against `sessions` with an
   `expires_at > now()` check and returns the session's scopes;
   `auth::scopes::permits` gates on them. The claim had been false for some time.
   - **Why this mattered more than the item itself:** a register whose only
     CRITICAL is false trains its readers to discount the colour. The next real
     CRITICAL entered a log where the top severity had already cried wolf. That is
     the failure this correction is for, and it is the same shape as the leakage
     claim this programme retracted and the `by_regime` empty array it served for
     months — a signal that reads as measured when it was never checked.
   - The loopback guardrail is a separate control and is **not** removed by this
     correction. Whether to bind on a non-loopback address is a deployment
     decision with its own review, not a consequence of the auth entry being wrong.

---

### 🟠 HIGH (Breaks Feature) — 5 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Coinbase live execution | Post-Phase 6 | Out of scope | No live crypto execution |
| Tradovate/Tradier account state | Phase 4 adapters | REST endpoints | Account queries error out (loudly) |
| Manual order placement | Disabled (503) | Postgres/Redis not wired | Orders blocked until Phase 2 |
| Dashboard P&L rollup | Phase 2 | Postgres + Redis | Always shows $0 P&L |
| 0x DEX execution | Phase 4 | 0x HTTP wiring | DEX trades unverified / assumed filled |

---

### 🟡 MEDIUM (Degrades Feature) — 6 Items

| Feature | Status | Blocker | Impact |
|---------|--------|---------|--------|
| Prometheus metrics | Phase 2 | None | No production visibility |
| Correctness metrics | Phase 2 | Storage integration | No data integrity validation |
| Order book builder | Phase 2 | None | No depth/market structure data |
| MCP server tests | Phase 5 | None | Tools not validated |
| AMM paper simulator | Phase 4 | 0x HTTP wiring | Mock prices only |
| Freshness timezone check | Phase 2 | `chrono-tz` crate | Wrong hours for non-UTC equity instruments (**new**) |

---

### 🔵 LOW (Nice-to-Have) — 5 Items

| Item | Status | Phase | Impact |
|------|--------|-------|--------|
| Parquet compaction | Warns but no-ops | Phase 2 | Query perf degrades |
| User management (storage) | Placeholder | Phase 2 | User CRUD ops missing |
| MCP dynamic discovery | Static data | Phase 5 | Agents use hardcoded instrument list |
| WS serialization silent failure | `unwrap_or_default` | Phase 2 | Empty frame sent on serialize error (**new**) |
| Ops documentation | Missing | Phase 2 | No runbooks for metrics/user mgmt |

---

### ⚪ Dead code — 2 items, recorded 2026-09-27 (Set S S-0.6)

Recorded because a dead symbol is cheap to leave and expensive to read: the next
person to touch the module has to work out whether it is load-bearing, and the
honest answer belongs written down rather than re-derived.

| Symbol | State | Who removes it |
|---|---|---|
| `Section::Tools` | **Already gone.** Removed by Set S phase 1 (S-1.x, "`Section::Tools` is deleted — nothing ever rendered it"): the schemas do not travel in that section, so it rendered nothing and was budgeted for anyway. Confirmed by grep — no hits anywhere in `crates/`. | Done; entry kept so the register shows the claim was checked rather than assumed. |
| `harness::context::ContextReport::needs_model_summary` | **Still dead, and dead in the more awkward direction: it is written and never read.** Set twice (`context.rs:490`, `transcript.rs:257`) and read exactly once, by an assertion in `transcript.rs:331`. There is no production reader, so nothing acts on it — a run that needs a model summary is flagged to nobody. | **No Set S task claims it.** That is the finding, not an omission of this entry: it is either a control that was never finished (something should read it and decide to summarise) or a field that should go. Deciding which is a small piece of design work, not a deletion, so it is not being deleted here on the strength of a grep. |

---

## Phase Dependencies

```
Phase 2:
  ├─ Auth implementation (M-17)          [CRITICAL path — unlock network binding]
  ├─ Manual order gate real data         [Postgres position + Redis mark price]
  ├─ Dashboard P&L rollup                [Postgres pnl_lots/pnl_closes + Redis]
  ├─ Prometheus metrics wiring           [None]
  ├─ Correctness metrics wiring          [Storage integration]
  ├─ Order book builder                  [None]
  ├─ Parquet compaction                  [None]
  ├─ User management (storage)           [None]
  ├─ Freshness timezone (chrono-tz)      [Add crate]
  ├─ WS serialization error handling     [None]
  └─ Ops documentation                   [None]

Phase 4:
  ├─ Tradovate account REST adapter      [REST endpoints unknown]
  ├─ Tradier account REST adapter        [REST endpoints + API key]
  ├─ 0x adapter HTTP wiring              [0x API]
  └─ AMM paper simulator (real quotes)   [0x HTTP integration]

Phase 5:
  ├─ MCP dynamic discovery               [Platform API routes]
  └─ MCP server test coverage            [None]

Post-Phase 6:
  └─ Coinbase live execution             [Out of scope]
```

---

## Risk Assessment

### Deployment Risk: **MEDIUM** (down from HIGH after v1 fixes)

| Risk | Severity | Mitigation |
|------|----------|-----------|
| ~~Auth completely bypassed~~ | ~~CRITICAL~~ **not a risk** | Corrected 2026-09-27: the token is resolved against `sessions` with an expiry check and scopes are enforced. The loopback guardrail remains, as a separate control. |
| Manual orders disabled | HIGH | 503 returned; safe state until Phase 2 |
| Dashboard shows $0 P&L | HIGH | Users cannot see trading performance |
| DEX trades unverified | MEDIUM | Implement on-chain tx verification before Phase 4 |
| No observability | MEDIUM | Wire Prometheus before production |
| Wrong trading hours for non-UTC equities | MEDIUM | Add `chrono-tz` before equity go-live |
| WS silent serialization failure | LOW | Add error logging/frame before Phase 2 |

### Readiness Checklist

- [x] Migrations 0006–0008 created
- [ ] Migrations 0006–0008 applied and tested against live DB
- [ ] Secrets moved to environment variables (prod deploy)
- [x] Auth placeholder isolated (loopback guardrail active)
- [ ] Phase 2 auth (token validation) implemented
- [ ] Manual order gate wired to real position/mark/P&L data
- [ ] Dashboard rollup shows real P&L
- [ ] Prometheus metrics wired and tested
- [ ] Order book builder implemented
- [ ] MCP server has test coverage > 50%
- [ ] Freshness check uses correct instrument timezone
- [ ] Docker Compose environment starts cleanly
- [ ] One end-to-end trade (paper) completes without errors
- [ ] API security review passed before network exposure


---

## Set S phase 8 — what is not closed, and the phase that would close it

Added 2026-09-17, on the phase-8 exit review. S-8.9's rule is that **every acceptance
criterion carries evidence or a recorded, justified exception**, and these are the
exceptions. Each names the thing that would close it, so the next person does not have
to re-derive why it is open.

The format differs from the 2026-06 scan above because the question differs: that scan
asked "what is wrong", and this asks "what did we ship knowing it was incomplete".

| # | Exception | Why it shipped this way | What closes it |
|---|---|---|---|
| S8-1 | **Three of the six §6 batteries cannot run.** (Was five; `leakage_audit` joined the runnable set 2026-09-21 — its bar-boundary step was built and mis-declared, and its other two gaps are deferred by design *and conditional*, so the battery is offered and refuses the inputs those steps would have covered.) `leakage_audit`, `hypothesis_precheck`, `precapital_battery`, `signal_identity` and `go_live_readiness` are declared in full and are absent from the catalogue; only `selection_bias_verdict` is offered. | Sixteen stages across the five name a tool nobody has built. A battery that ran the steps that exist would emit a verdict *shaped* like the real one, and nothing downstream could tell the difference (ADR-S8-01). | The sixteen owed tools, enumerated by `mcp_server_lib::tools::workflows::owed()` with what each needs and why its verdict is unreachable without it. `precapital_battery` is one tool from running; `leakage_audit` is three. | **2026-09-21, second pass:** `hypothesis_precheck` joined too — both its owed steps were reads over tables that already existed, and the "capability delta" rule was `FailureClass::retry_condition`, already built. Of the twelve stages still owed, one (`capacity`) turned out to be answerable by `estimate_liquidity_cost` and is now wired, and three more have an *adjacent* tool that answers a **different** question — recorded at the declaration so the next person repeats neither mistake.
| S8-2 | **Two of the four gate ticket requirements are not enforced.** Gate 2 demands both `leakage_audit` and `selection_bias_verdict` as of 2026-09-21 — flipped by the invariant, which failed the moment the battery became runnable and named the flag. Gate 3 and the paper door still demand nothing. | Enforcing a ticket nothing can mint refuses every experiment on the platform, forever. The requirement is declared with `enforced: false` and an invariant test holds that flag equal to whether its battery can run — so this closes itself, loudly, on the day the battery lands (ADR-S8-02). | S8-1. `crates/invariants/tests/gates_demand_tickets.rs` fails until `enforced` is flipped to match. |
| S8-3 | **Reservation settlement is still unwired.** `mlops.trial_reservation.settled_trial_id` exists and nothing sets it, so an authorised look that never ran is not distinguishable from one that did. | Inherited from ADR-S10-10 and not closed here. Threading a reservation id through tool → API → suite → Run is a change to four layers and was not phase 8's subject. | A phase that threads the id through the compute path. Until then no reconciliation is claimed anywhere, and the migration's own header says so. |
| S8-4 | **Reviewer precision over time is not tracked**, though S-8.6 asks for it. | It needs a corpus of critiques whose findings were later confirmed or dismissed, and nothing records the second half — a critique is stored with the dossier and no one marks a finding as having been right. | A disposition on each critique finding, written when a human acts on the dossier. The comment thread (`mlops.dossier_comment`) is where that would attach. |
| S8-5 | **The per-conversation sandbox launcher is a deployment contract, not a component.** `POST /api/code/run` creates the workspace and shells `TBOT_SANDBOX_CMD`; nothing in this repository starts or tears down a container. | The honest alternatives were worse: running the script in the platform process when no sandbox is configured (the hole), or putting the Docker socket inside the API container (a different hole). With the variable unset the tool refuses and says why (ADR-S8-06). | A launcher component with a container lifecycle, or a deployment that supplies the command. The image, the executor, the refusals and the CI build are all in place. |
| S8-6 | **A dossier's `funnel` and `quoted_untrusted` sections are empty arrays.** The desk renders both and the compiler cannot see that they are never filled. | The funnel view is per-experiment and a dossier is per-run, and nothing in the run record says which experiment it advanced. `quoted_untrusted` needs the fetched-content provenance chain, which `agent_memory.entry` records per entry and not per quotation. | Joining a run to its experiment, and a per-quotation provenance record. Both are reads; neither is a new mechanism. |
| S8-7 | **`claims::extract` catches only claims containing a number.** A qualitative overclaim in the prose is not extracted and therefore cannot block. | Stated in the module header rather than implied. Post-hoc span alignment is a numeric operation; a prose-level overclaim check is a model judgement, which is what the critic is for. | The checklist critic's `SuggestiveNotPresentedAsEstablished` covers the most common case by reading the platform's confidence label. A broader check would need the fresh-session model critic (S-8.6's other half) to run on every report. |
| S8-8 | **About thirty-five string literals across the crates have swallowed line continuations** — runs of ten or more spaces mid-sentence, from an editing process that ate `\` continuations. Most are user-visible refusal messages. | Pre-existing, found while writing this phase, and unrelated to any of it. Fixing them inline would have put a thirty-file whitespace diff inside a feature change. | A sweep, spawned as its own task. The detection script is in that task's prompt. |

---

## Set S S-8.9 close-out — the `Verified by:` pass (2026-10-04)

Added 2026-10-04 by the S-8.9 pass over AGENT-005 to AGENT-014 (ten specs, 104 acceptance
criteria). **98 now carry a `Verified by:` naming tests that exist (each name was grepped
against the tree), 33 are ticked, 6 still read `[--]`.** A criterion is ticked only when the
tests in the tree prove all of it; anything resting on a live model, a live Postgres, a
running platform or a browser says **Partial** in its own slot and stays unticked. Same
question as the section above: not "what is wrong", but "what did we say was proved, and
what is it actually resting on".

| # | Exception | Why it is this way | What closes it |
|---|---|---|---|
| S8-9 | **The `judgement` suite's scorecard rows are synthetic.** `evals/judgement/tapes/*` are the outputs of five declared reference policies (`TierPolicy`), not recordings of any model; the cards are labelled `reference-policy/<tier>` and `not a model`. AGENT-009 AC-8 is therefore Partial: the machinery (46 cases a tier, 37 on a PR, 9 sealed, per-(tier, workflow) cells, the gate) is proved; no tier's judgement is measured. | A model run needs a provider credential nobody has stored (gate H-2). Labelling a hand-made tape as a frontier result would be the failure the suite exists to catch. | One frontier run over the 46 cases on a stated machine, n >= 30 per cell (S-10.16). ADR-S10-J1. |
| S8-10 | **Sixty-five criteria are Partial, most because nothing has run against a live model.** AGENT-007 AC-2, AC-9 (a model writing the citing finding); AGENT-009 AC-1, AC-5, AC-6, AC-7; AGENT-010 AC-2, AC-3, AC-4, AC-7, AC-10 (kill and resume is simulated by replaying a truncated log); AGENT-013 AC-1 to AC-5, AC-13, AC-15; AGENT-014 AC-1, AC-5, AC-6; AGENT-011 AC-7 (measured once on a local 7B, not on the `research` suite). Every one of those slots says which half is proved. | Gate H-2 (an Anthropic credential) and S-4.14 (`BLOCKED:HUMAN`): one 30-iteration frontier run and one local run. | S-4.14, then each task's own live clause. |
| S8-11 | **Many Partial slots rest on tests that need live Postgres and were not re-run in this pass.** They are written, CI runs them (`--ignored`), and the slot says "Written, not re-run". Re-run here: the harness crate, the desk's frontend tests (72 files, 681 tests), the evals runner (49 tests). | Postgres on 5433 has no pgvector and the platform DB is not on 5432, and the shared target directory's lock was held by other tracks for most of the pass. | `cargo test -p api -- --ignored` against a migrated database. |
| S8-12 | **Six criteria still read `[--]`:** AGENT-005 AC-2 (a live 30-iteration cache run, S-4.14) and AC-10 (the twelve ADR-0034 conflicts: nobody re-checked them), AGENT-006 AC-5 (templated-tool header schemas), AC-9 (summary <= 400 tokens on the reference dataset) and AC-10 (the AGENT-002 pointer and the README listing), AGENT-008 AC-3 (a lookup on the 7B in <= 3 calls, live). | None of them is covered by phases 4, 9, 10 or 11, and a slot with no test to name stays empty rather than being filled with a plausible sentence. | The phase that owns each (AGENT-006 AC-5, AC-9 are phase 3's), or a live run. |
| S8-13 | **The Proposed to Implemented move and the supersession marks are not done.** `docs/specs/README.MD` is the user's file and is under a standing no-touch rule; the nine specs stay `Proposed` there and AGENT-001 to 004 and COMP-006 are not marked superseded. | A standing instruction, recorded in `TRACK-RULES.md` and the project memory, outranks the S-8.9 body. | The owner makes the README edit, or lifts the rule. |
| S8-14 | **AGENT-012 AC-7 and AC-8 (contrast, reflow, 375px) are proved statically.** The tests read `desk.css` and the token rules; no browser rendered the desk at 375px or switched the theme mid-run, because every desk surface sits behind sign-in and the platform binary starts live collectors and was not started. The contrast report and the icon-label sweep have no test at all. | Same as S-6.1's note. | A browser pass against a running platform, and a contrast check in CI. |
| S8-15 | **AGENT-013 AC-8, AC-9, AC-12 lean on files another track was still writing** (`backtest/tests/reproduce.rs`, `api::repro_drift`, `api::compare_candidates`, all untracked during this pass). Their slots name tests that exist but say Partial and "owned by the S-LIFECYCLE track"; they were not run. | Concurrent work in the same tree. | That track's own close-out of S-10.8 and S-10.10; then re-read these three slots. |

---

## Changelog

| Date | Version | Changes |
|------|---------|---------|
| 2026-06-10 | v1 | Initial scan — 23 items found (3 CRITICAL, 7 HIGH, 6 MEDIUM, 4 LOW) |
| 2026-09-17 | v3 | Set S phase 8 exit: eight recorded exceptions, each with the phase that would close it. Not a repository scan — these are things shipped knowingly incomplete, which is a different list from the one above. |
| 2026-10-04 | v4 | Set S S-8.9 close-out: seven further recorded exceptions (S8-9 to S8-15) from filling the `Verified by:` slots of AGENT-005 to AGENT-014. |
| 2026-06-10 | v2 | Post-implementation pass — 18 items completed/improved; 2 new findings added (freshness timezone, WS serialization); overall risk downgraded from HIGH to MEDIUM |

---

**Last Scan Date:** 2026-06-10 (v2)
**Scan Depth:** Repository-wide (all crates, apps, migrations, config, docs)
**Open Items:** 21 (1 CRITICAL, 5 HIGH, 6 MEDIUM, 5 LOW)
**Completed Since v1:** 18 items
**New Findings (v2):** 2 items
