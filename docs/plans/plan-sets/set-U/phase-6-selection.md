# Phase 6 — Selection strategies

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** SU-SEL-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §4, §5.2, §5.3; ADR-0040.
**Depends on:** Phases 1 and 3.
**Blocks:** nothing in this set.
**Migration range:** 0147–0148.

*What should we buy.* A selection strategy returns the assets that meet its criteria, ranked, with the attributes that decided it; a deployment can then run a trading strategy on whatever it selects.

---

### U-6.1 Universe provider — M
`NOT STARTED`

A universe is a named as-of membership (`dataplane::universe`, `members_known_as_of`), an explicit asset list, or every initialised asset of a class; the provider supplies point-in-time membership to every environment.

**Files:** `crates/backtest/src/env/universe.rs`, `crates/strategy-engine/src/env.rs`.
**Acceptance:** a backtest's universe at t excludes instruments added later and includes ones since removed; the same universe definition yields the same members in backtest and live for the same instant.
**Traces:** FEAT-007 §4; Set P P-4.3.

### U-6.2 Cross-sectional evaluation — L
`NOT STARTED`

The engine evaluates a selection plan by mapping the per-instrument sub-graph over the universe, applying `cs_*` operators over the resulting vectors, and evaluating the `select` sink. Temporal state is kept per instrument.

**Files:** `crates/strategy-engine/src/selection.rs`.
**Acceptance:** `cs_rank` and `cs_zscore` match a reference computation; a member missing a score is excluded with its reason, never ranked as zero; results are identical regardless of universe iteration order.
**Traces:** FEAT-007 §5.3; ADR-0040.

### U-6.3 Running a selection — M
`NOT STARTED`

An API and agent tool run a selection strategy now or as of a past instant and return the ranked list with scores and attributes; results are journaled; scheduled runs follow the strategy's triggers.

**Files:** `crates/api/src/routes/selections.rs`, `crates/mcp-server/src/tools/`, migration 0147.
**Acceptance:** a person and the agent get the same list for the same strategy and instant; a run as of a past date uses that date's membership and data only.
**Traces:** FEAT-007 §4, §10.

### U-6.4 Deployments bound to a selection — L
`NOT STARTED`

A deployment's target may be a selection: on each rebalance the allocator diffs membership, starts trading instances for entrants, applies `on_removal` (`close`, `let_strategy_exit`, `hold`) to leavers, and allocates capital equal-weight or by the selection's `weight` attribute.

**Files:** `apps/platform/src/allocator.rs`, `crates/domain/src/strategy_graph/deployment.rs`, migration 0148.
**Acceptance:** membership changes start and stop instances within one rebalance; `close` flattens a leaver; capital allocations sum to the deployment's capital; every change is journaled.
**Traces:** FEAT-007 §4; ADR-0040 decision 4.

### U-6.5 Portfolio backtest — L
`NOT STARTED`

A backtest of a selection-bound deployment runs one simulated clock and one account across every member over time, re-running the selection on its schedule.

**Files:** `crates/backtest/src/runner.rs`, `crates/backtest/src/portfolio.rs`.
**Acceptance:** an instrument enters and leaves the portfolio as membership changes, with trades only while a member; P&L aggregates per numeraire; the run's trade list attributes every trade to its instance.
**Traces:** FEAT-007 §4; ADR-0044.
