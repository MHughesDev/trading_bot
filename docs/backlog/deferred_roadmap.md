# Deferred roadmap (FB-N1, FB-N3)

**Queue system:** [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) · [`QUEUE_STACK.csv`](QUEUE_STACK.csv) · [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)

**Queue:** Parent rows **FB-N1** / **FB-N3** in [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) are **Done** (**closed for now**, 2026-04-17); sub-slices **FB-N1-S1** / **FB-N3-S1** remain the delivered first steps. This file still describes **future** work if you **reopen** or add new backlog items.

Themes below are **intentionally out of scope** for full delivery in the current V1 trading stack. This file records **recommended next steps** when you promote them.

---

## FB-N1 — Multi-exchange execution

**Current state:** [`execution/router.create_execution_adapter`](../execution/router.py) selects **one** adapter: Alpaca (paper) or Coinbase (live), with optional **`NM_EXECUTION_ADAPTER`** overrides (`stub`, `mock_alpaca_paper`). [`execution/adapter_registry.py`](../execution/adapter_registry.py) lists supported venue keys for **`GET /status` → `execution_adapters`**.

**Suggested phases (when prioritized):**

1. **Registry + routing policy** — map `symbol` → venue; reject ambiguous symbols at config load.
2. **Second live adapter** — e.g. Kraken spot REST (new module under `execution/adapters/`), separate credentials, same `OrderIntent` contract.
3. **Risk** — per-venue and aggregate notional caps (extends `RiskEngine` + [`MULTI_ASSET_PORTFOLIO.MD`](Specs/MULTI_ASSET_PORTFOLIO.MD)).
4. **Control plane** — operator-visible venue health and adapter selection (beyond a single `NM_EXECUTION_MODE`).

**Non-goals for a first slice:** cross-venue netting, smart order routing, or unified margin.

---

## FB-N3 — Portfolio optimization

**Current state:** A small **equal-weight** reference (`1/n` fractions) is not bundled in-repo (removed **FB-CAN-020**; was `risk_engine/equal_weight.py`). Reintroduce inline or under `models/` when portfolio optimization is prioritized. No third-party optimizer is bundled.

**Suggested phases (when prioritized):**

1. **Objective** — max Sharpe vs min variance vs risk-parity; pick one for a first experiment.
2. **Inputs** — covariance from rolling returns (replay or offline) + constraints from `risk_max_*` settings.
3. **Integration** — optional hook in decision or risk layer **behind a flag** (default off) so live behavior is unchanged until explicitly enabled.

**Non-goals for a first slice:** live continuous optimization on every tick; dependencies on heavy proprietary solvers without an opt-in extra in `pyproject.toml`.
