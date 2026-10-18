# Phase 2 — Data API v2, the research cutoff and the Desk

**Completion: cutoff core complete and verified; §5 endpoints built except `quotes`/`book`
(`BLOCKED:UPSTREAM`, Set P); `data_snapshot_id` and Parquet delivery landed 2026-10-03.
DA-07 and DA-13 landed later, in Phase 6 — see
[phase-5-6](phase-5-6-toolbox-ui-and-evals.md).**

**Requirement IDs:** DA-01…DA-09, DA-13, DA-15 (scoped below).
**Spec:** [DATA-005](../../../specs/DATA-005-data-api-v2.md), ADR-0025.
**Migrations:** `0036_agent_projects_sessions.sql`, `0038_token_scopes.sql`.

---

## What was built

| Piece | Where |
|---|---|
| Research projects, the Desk, cutoff arithmetic | `crates/api/src/projects.rs` |
| Data API: `bars`, `catalog`, `live`; project CRUD | `crates/api/src/routes/data.rs` |
| Token scopes and the never-for-agents list | `crates/api/src/auth/scopes.rs` |
| PIT horizon on every bar read | `BarStore::as_of` (Phase 0, L-0.6) |
| `data_qc` grading | `crates/api/src/workers.rs` |

## The cutoff is a property of the reader, not of the request

A project carries a `research_cutoff`; the Desk's is `NULL`, meaning now. Every read
resolves the caller's project and clips to its horizon. There is no parameter that
turns this off, because there is no code path that reads bars without going through
a project.

Three database-level guarantees back it, and each exists because an API-level check
protects only the callers that go through that API:

- **The cutoff cannot move once the project has an experiment** (DA-05). Without this
  a researcher who disliked a result could move the cutoff forward, re-run, and
  present the second answer as the first, with nothing in the data showing it.
- **A project's kind cannot change.** Flipping research → desk would drop the cutoff
  and hand over the holdout in one statement.
- **A research project must have a cutoff and a Desk must not.** Both halves are
  `CHECK`ed, because either state is one the rest of the system cannot interpret.

## Clipping is silent in the data and loud in the manifest

An over-long window returns correct data for the part the project may see, plus
`cutoff_applied` naming the boundary. Refusing outright would teach nothing and
invite probing for the edge.

The Desk clips too — nobody reads the future — but reports `cutoff_applied: null`,
because that field means "a holdout was enforced here" and saying it on the Desk
would make an exploratory result look confirmatory.

## Verified on the live platform

Same request, two projects:

| | research project | Desk |
|---|---|---|
| requested end | 2026-12-31 | 2026-12-31 |
| effective end | **2026-06-13** (cutoff) | 2026-09-11 (now) |
| `cutoff_applied` | 2026-06-13 | none |
| catalogue, BTC-USD 1m | **41,395 bars** | **102,051 bars** |
| `data/live` | `403 live_data_desk_only` + fix | last bar, 16 s old |

The catalogue figures are the point: the holdout is not merely unreadable, it is
invisible. A catalogue that advertised bars the project cannot read would leak the
future one row at a time — "history ends on the 4th" is information.

## Authority

`RESEARCH_SCOPES` is seven entries and the agent gets exactly them. What is missing
is the design: `data.holdout`, `orders.place`, `orders.cancel`, `automations.arm`,
`models.promote`, `skills.admit`. `mint_token` does not take a scope parameter —
an orchestrator that could be *asked* for extra authority would eventually be asked
for it by a bug — and migration 0038 refuses the forbidden set at the database.

12 tests cover this (7 project integration, 5 authority), including
`the_database_refuses_a_dangerous_scope_on_a_project_bound_session`, which writes the
row directly to prove the API is not the only guard.

## What is not built

Of DATA-005 §5's endpoint table, only `catalog`, `bars` and `live` exist. Missing:
`coverage`, `search`, `trades`, `quotes`/`book`, `funding`, `open_interest`,
`option_chain`, `prediction_markets`, `fundamentals`, `macro`, `universe`,
`features`, and the two text endpoints. Most need data the platform does not collect
yet (Set P) or the feature engine (Set M).

**DA-07 and DA-13 were finished in Phase 6**, because the eval suite needs both: the
`noise` and `planted_edge` suites run on synthetic instruments, and the auditor suite
needs a grade gate to have fixtures to run against. `crates/api/src/data_admission.rs`
holds the grade thresholds and the admission rule (grade D is refused outright, and a
waiver cannot cover it); `crates/backtest/src/synthetic.rs` holds the seven
generators.

Also missing:
- **Parquet responses and `parquet_extract` artifacts.** `bars` returns JSON rows
  with a manifest. The artifact path exists (Phase 1) but is not wired to data reads,
  so a large extract still travels as JSON.
- **DA-08 as-of universes.**
- **`data_snapshot_id`** on responses, so job manifests cannot yet pin a data version.

---

## Phase 2 completion (track LEGACY, 2026-10-03)

The "What is not built" list above is **superseded** by this block. The DATA track had already
added `coverage`, `universe`, `macro`, `prediction_markets`, `text`, `bars/derived` and
`funding`; this track added the rest and the two cross-cutting items. All code is in
`crates/api/src/routes/data.rs` (blocks marked "Set L Phase 2").

| Endpoint | Status | Source of data |
|---|---|---|
| `GET /api/data/search` | `DONE` | `BarStore::list_coverage` (horizon-clipped) + `instruments.asset_class`; ranked exact > prefix > substring, then bars covered |
| `GET /api/data/trades` | `DONE` | ClickHouse `market_trades`, point-in-time on `available_time`, `LIMIT 1 BY dedup_key`, an absent table is `null` + `measured:false`, never `[]` |
| `GET /api/data/open_interest` | `DONE` | `storage::clickhouse::forward::read_open_interest` (same three outcomes as `funding`) |
| `GET /api/data/option_chain` | `DONE` | `option_chain_snapshots`, newest snapshot with `knowledge_time <= as_of`; BSM greeks computed at read time from the stored IV, rate, dividend and underlying price (`dataplane::options::bsm_greeks`, INV-07), `null` when an input is missing |
| `GET /api/data/fundamentals` | `DONE` (CIK keyed) | `dataplane.fundamentals_fact`, `accepted_at <= as_of`, latest filing per `(tag, unit, period_end)`. **Takes `cik`, not `ticker`**: the platform keeps no ticker-to-CIK map and a wrong guess attaches another company's numbers; a `ticker` is refused with that reason (ADR-L-DA-1) |
| `GET /api/data/features` | `DONE` | `features::runtime` batch mode (`backfill_column`) over PIT bars densified on the master clock; scope `research:features`; `null` where a window is not full |
| `GET /api/data/quotes`, `/book` | `BLOCKED:UPSTREAM` | No quote or book history is collected. The order book is a live-only stream (`book_feed.rs` -> `live_bus`); there is no table to read. Needs Set P's book lane (P-0.x) to store snapshots first. An endpoint returning an empty list would assert "measured and found nothing" |

**Parquet.** Every endpoint above and `bars` accept `?format=json|parquet`. `parquet` encodes
the rows (`decimal128(38,10)` for prices and sizes, `timestamp(ns, UTC)`, `float64` only for
derived statistics, every column nullable so absent is null), stores them as a
`parquet_extract` artifact through `ArtifactRegistry::put` (content-addressed, expiring), and
answers `{handle, manifest, summary}` with a six-line summary. The default stays `json`
(ADR-L-DA-2: DATA-005 says Parquet by default; changing the default would break the
frontend and MCP clients that read `bars` rows, so the default moves when they do).
`tbot data bars --format parquet` + `tbot artifact download` is the agent path.

**`data_snapshot_id`.** Every manifest carries `data_snapshot_id` (`ds_` + 32 hex) and a
`snapshot` descriptor `{endpoint, params, as_of, rows_sha256}`: the id is the sha256 of the
canonical (RFC 8785) descriptor, so it pins *which version of the data* a read saw, and a
late restatement changes it even when every parameter is the same. A job manifest carries it
in `data_snapshot_id`, which is already part of the idempotency hash (JB-02), so two runs on
different data versions are different jobs. **Not yet built:** re-verifying a snapshot at run
time (recompute from the descriptor and compare). The descriptor carries what is needed.

**Evidence.** `routes::data::data_v2_tests` (8 tests pass): snapshot id pins data and moves
with rows/as_of/endpoint/window but not key order; Parquet round-trips a decimal exactly and
keeps nulls null; the trades query is PIT and deduplicated and escapes its input; the chain
query reads one snapshot known by `as_of`; greeks come from stored inputs and a missing IV is
not a zero; search ranking; an absent table is unmeasured, not empty.

**Route registration** is orchestrator-owned (`crates/api/src/routes/mod.rs`); the six lines are
in the track report. Until they land the handlers are unreached: that is the one remaining
step to close "built but unreached" for this block.

**Not done.** DA-08 as-of universes beyond the `universe/{name}` route the DATA track built.
Endpoint-level HTTP tests need the router (and so the registration above).
