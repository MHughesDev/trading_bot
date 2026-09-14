# Phase 2 — Data API v2, the research cutoff and the Desk

**Completion: cutoff core complete and verified; most §5 endpoints not built.
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
