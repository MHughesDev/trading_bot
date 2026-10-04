# Phase 6 — Selection strategies

**Completion: 100% (6 / 6 tasks).** U-6.1, U-6.2, U-6.4, U-6.5 DONE 2026-10-02; U-6.3 and U-6.6 DONE 2026-10-04.

**Requirement IDs:** SU-SEL-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §4, §5.2, §5.3; ADR-0040.
**Depends on:** Phases 1 and 3.
**Blocks:** nothing in this set.
**Migration range:** 0147–0148 (Phase 5 used only 0146, so this phase took the next two rather than leave a gap).

*What should we buy.* A selection strategy returns the assets that meet its criteria, ranked, with the attributes that decided it; a deployment can then run a trading strategy on whatever it selects.

---

### U-6.1 Universe provider — M
`DONE` 2026-10-02 — `strategy_engine::Membership` (`env.rs`) is the one type every environment
asks: a **logged** universe resolves through `dataplane::universe::members_known_as_of(log, t, t)` —
both clocks, so an addition recorded late is invisible until it is recorded — and a **fixed** one
(a list, or every initialised asset of a class, `asset_lifecycle`) is today's membership applied
to every instant and says so (`point_in_time() == false`; a portfolio run then carries
`universe.not_point_in_time`). A key with no symbol stays a member as `#<key>` and is excluded for
no data, never silently dropped. `backtest::env::universe::load` is the one loader — the portfolio
backtest, the selection runner and the live allocator all call it. **Found and fixed:** 0104 never
granted `dataplane.universe` / `universe_membership_event` to `app_role`, so the platform's first
live load would have been refused while every owner-run test passed (granted in 0147). Evidence:
`env::universe_tests` (later additions excluded, later removals included, the late-recorded one
invisible until known); `backtest/tests/pg_universe.rs` on a scratch PostgreSQL 17 through 0148,
including a `SET ROLE app_role` read that fails with the grant revoked.

A universe is a named as-of membership (`dataplane::universe`, `members_known_as_of`), an explicit asset list, or every initialised asset of a class; the provider supplies point-in-time membership to every environment.

**Files:** `crates/backtest/src/env/universe.rs`, `crates/strategy-engine/src/env.rs`.
**Acceptance:** a backtest's universe at t excludes instruments added later and includes ones since removed; the same universe definition yields the same members in backtest and live for the same instant.
**Traces:** FEAT-007 §4; Set P P-4.3.

### U-6.2 Cross-sectional evaluation — L
`DONE` 2026-10-02 — `strategy_engine::selection::SelectionEngine`. The engine's node evaluation moved
into one per-instrument routine (`instance.rs`) that the trading `Engine` and the selection engine
both run — byte-identical traces for every existing trading test. One `Instance` per member keeps
its own bars, indicators and stateful-operator state; a `cs_*` node waits until every member has
its input, is computed across them, and the members' passes resume; model requests from every
member go out together (`<member>::<node>`). The sink filters, scores, ranks (ties by instrument),
cuts at `top_n`, and accounts for every member — selected with attributes, or excluded with its
reason. Members are visited in instrument order and sums run in that order. `state.*` and fixed
instruments are refused in a selection at compile time. Evidence: `strategy-engine/tests/selection.rs`
8/8 — `cs_rank`/`cs_zscore` against a by-hand reference including a tie (mid-rank), a warming-up
member excluded with its reason, permuted universe and bar order give the same digest, temporal
state per member, one model call per member with an unanswered one excluded as a timeout;
`selection::tests` against scipy-convention references.

The engine evaluates a selection plan by mapping the per-instrument sub-graph over the universe, applying `cs_*` operators over the resulting vectors, and evaluating the `select` sink. Temporal state is kept per instrument.

**Files:** `crates/strategy-engine/src/selection.rs`.
**Acceptance:** `cs_rank` and `cs_zscore` match a reference computation; a member missing a score is excluded with its reason, never ranked as zero; results are identical regardless of universe iteration order.
**Traces:** FEAT-007 §5.3; ADR-0040.

### U-6.3 Running a selection — M
`DONE` 2026-10-04 (built 2026-10-02) — `backtest::selection_run`: `run_selection` resolves the revision, the
universe at the instant, and every member's bars **as the world stood then** (`BarStore::as_of`),
warms each member over earlier closes, and evaluates at the last close at or before the instant
asked; `evaluate_over` is the pure half. `POST /api/selections/run` and `GET /api/selections/runs`
(`routes/selections.rs`), journaled in `selection_run` (0147, append-only) with who asked — a web
session is a person, a service session the agent. The agent's `run_selection` /
`list_selection_runs` tools (MCP and in-app) call that same route and forward only the strategy and
the instant. Scheduled triggers now have an evaluator (`strategy_engine::schedule::Cron`, five-field
UTC; malformed schedules are refused at compile time) and drive selections in portfolio backtests
and in live deployments. Evidence: `backtest/tests/selection_run.rs` (as of day 5 the run sees day
5's membership; later bars change nothing — same digest; a bar recorded after the instant is
invisible; a member with no bars is excluded); `pg_universe.rs` (journal round trip, append-only).
**2026-10-04, the two gaps closed:** a screener someone *watches* runs on its own schedule —
`selection_watch` (0187: a pinned revision and schedule; one running watch per person and revision;
stopped once, never edited or deleted), `POST /api/selections/watch` (refused without a
`triggers.schedule`), `GET /api/selections/watches`, `DELETE /api/selections/watch/{id}`, and
`apps/platform/src/screener_jobs.rs`, which runs every watch due in a minute through `run_selection`
and journals it as `requested_by = 'schedule'`, once per fire (spawned at boot; source scan). And the
ClickHouse half ran against the server: `tests/selection_clickhouse.rs` loads BTC-USD and ETH-USD as
the store stood at 2026-10-02 20:00 UTC, excludes an unknown symbol for having no data, and selects
by close. The HTTP hop itself was not exercised — the platform is not started here — but each of its
three halves (Postgres, ClickHouse, the pure evaluation) is. "A person and the agent get the same list" rests on there being one route
and one function, with tests that the tool and the route add nothing that could differ.

An API and agent tool run a selection strategy now or as of a past instant and return the ranked list with scores and attributes; results are journaled; scheduled runs follow the strategy's triggers.

**Files:** `crates/api/src/routes/selections.rs`, `crates/mcp-server/src/tools/`, migration 0148.
**Acceptance:** a person and the agent get the same list for the same strategy and instant; a run as of a past date uses that date's membership and data only.
**Traces:** FEAT-007 §4, §10.

### U-6.4 Deployments bound to a selection — L
`DONE` 2026-10-02 — `POST /api/deployments` takes `target: {kind: "selection", …}`; the selection is
pinned to its revision, the parent is recorded with its target (0148) and declared;
`apps/platform/src/allocator.rs` runs it. Each rebalance runs the selection (journaled as asked by
the deployment) and applies `strategy_engine::allocation::rebalance` — **the same function the
portfolio backtest applies**: an entrant becomes a child deployment of the trading strategy on that
instrument carrying a `SleeveHandle`, which the live-runner supervisor starts; a leaver is told how
it leaves through its handle and its runner carries it out on its next bar (`close` flattens
through its own risk gate, `hold` stops, `let_strategy_exit` decides until flat); a stayer's capital
moves on its handle. A member sizes against its share of the deployment's capital, as in the
backtest. Weights sum to exactly one; a member offered no free capital waits rather than starting
empty. Every change is a row in `deployment_allocation_event` (append-only, with its capital and
selection run). Resumed at boot with each member's last capital. Evidence: allocator tests 6/6
(members start and stop within one rebalance; capital sums to the deployment's; one journal row per
change; an exit holds its capital until its runner stops; a member that cannot start is not one;
stopping holds every position); live-runner tests on the real paper engine (a member sizes against
its 2,000 sleeve, not the account; `close` flattens through the gate; `let_strategy_exit` stops
only when flat); PostgreSQL: parent/child records, the journal's CHECKs and append-only trigger,
and the production member port recording, declaring and resuming a member.
*Caveat carried from U-4.7:* the paper engine keeps one account per asset class, so members see
positions any other deployment holds in the same instrument.

A deployment's target may be a selection: on each rebalance the allocator diffs membership, starts trading instances for entrants, applies `on_removal` (`close`, `let_strategy_exit`, `hold`) to leavers, and allocates capital equal-weight or by the selection's `weight` attribute.

**Files:** `apps/platform/src/allocator.rs`, `crates/domain/src/strategy_graph/deployment.rs`, migration 0149.
**Acceptance:** membership changes start and stop instances within one rebalance; `close` flattens a leaver; capital allocations sum to the deployment's capital; every change is journaled.
**Traces:** FEAT-007 §4; ADR-0040 decision 4.

### U-6.5 Portfolio backtest — L
`DONE` 2026-10-02 — `backtest::portfolio::run_portfolio`: one clock across every member's bars,
one account (`SimOrders` now holds a sleeve per instrument over one `SimulatedVenue` — the
single-instrument run is the one-sleeve case, its traces unchanged), the selection re-run on its
timeframe or schedule over the universe of that instant, rebalances applied through the shared
`allocation::rebalance`, each member trading through its own engine and sleeve. Reachable through
the trial ledger: `RunConfig.target` (absent from the hash when `None`, so every existing `run_id`
holds) routes a v3 run to `V3RunExecutor`'s portfolio path, and an Experiment created with a
`target` makes every one of its Runs a portfolio run; the manifest records every allocation change
and which instance opened each trade. Evidence: `backtest/tests/portfolio.rs` 6/6 — A, B and C
enter and leave exactly as the selection and the point-in-time universe say (C prices highest from
day 0 but enters only on day 7, when it is listed); trades and decisions only while a member;
every trade attributed to `<instrument>#<n>`; P&L summed per quote currency; `hold` and
`let_strategy_exit`; a noon schedule and a Monday-only rebalance. A mutation giving the selection
every instrument ever listed fails all four core tests. *Not run:* the executor's load path
against ClickHouse (none here).

### U-6.6 A trading strategy reads its selection's attributes — M
`DONE` 2026-10-04 — `PortfolioView.selection` (`SelectionContext`: the selection's name, revision and
the member's numeric attributes; absent from the serialised view when `None`, so journaled views and
trace digests from before it are unchanged) and `NodeKind::SelectionAttr`, compiled from
`selection_attr(selection, attr)` (a selection strategy is refused it, `selection_attr_in_selection`).
`allocation::member_attributes` is the one conversion: the portfolio backtest sets each running member's
sleeve after every rebalance and `SimOrders::view_of` shows it; the live allocator sets each member's
`SleeveHandle` (its selection port now returns the selection's name) and the runner's view shows it. The
attribute is matched by the selection's name or revision, and is missing with the reason when the
instance is unbound, bound to another selection, or was given no such attribute. **Found by the
acceptance test:** an attribute read back through `Decimal::to_f64` lost its last digit (200.48000000000002
→ 200.48); it now reads back through the decimal's own digits. Evidence: backtest `portfolio.rs` —
`a_member_reads_the_attribute_its_latest_selection_gave_it` (every bar-close decision of every member
equals the attribute its latest selection gave it, exactly) and `…naming_another_selection_reads_missing…`;
platform — `a_member_on_paper_reads_what_its_selection_gave_it` (the same conversion, exact) and
`a_deployment_not_bound_to_a_selection_reads_missing_with_the_reason`; engine
`a_selection_strategy_is_refused_selection_attr`.

Found closing U-6.4. FEAT-007 §4 lets a trading strategy read "a selection's attributes" through
`selection_attr`, and the compiler refuses it as not built. The allocator already holds each
member's attributes at every rebalance: put them on the member's `SleeveHandle` (live) and sleeve
(portfolio backtest), expose them on the portfolio view, and compile `selection_attr` to read them —
missing, with the reason, when the instance is not bound to a selection.

**Files:** `crates/strategy-engine/src/{compile,instance,portfolio}.rs`,
`crates/strategy-runtime/src/live.rs`, `crates/backtest/src/{portfolio,env/mod}.rs`,
`apps/platform/src/{allocator,live_runner}.rs`.
**Acceptance:** a member reads its selection's `weight` attribute identically in a portfolio
backtest and on paper; an instance not bound to a selection reads it as missing with a reason.
**Traces:** FEAT-007 §4, §5.3.

A backtest of a selection-bound deployment runs one simulated clock and one account across every member over time, re-running the selection on its schedule.

**Files:** `crates/backtest/src/runner.rs`, `crates/backtest/src/portfolio.rs`.
**Acceptance:** an instrument enters and leaves the portfolio as membership changes, with trades only while a member; P&L aggregates per numeraire; the run's trade list attributes every trade to its instance.
**Traces:** FEAT-007 §4; ADR-0044.
