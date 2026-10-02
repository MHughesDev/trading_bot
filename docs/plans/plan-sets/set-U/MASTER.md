# Unified Strategy Engine — Set U

**Completion: 65% left (19 / 54 tasks). 19 done, 0 partial, 35 not started.**

**Status:** ACCEPTED (2026-10-01) — Phase 0 starts immediately.
**Created:** 2026-10-01
**Derived from:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) and ADR-0039 through
ADR-0047, written after the 2026-10-01 audit of every path a strategy takes. Folds in the unbuilt
halves of FEAT-004 §3, FEAT-005 §3–§4 and §9, and FEAT-006 §5–§6.
**Requirement IDs:** SU-CON, SU-ENG, SU-VEN, SU-BT, SU-LIVE, SU-MOD, SU-SEL, SU-AUTH, SU-PAR.
**Migration range:** 0141–0149. 0141 went to [ADR-0048](../../../adr/0048-paper-trading-runs-in-the-app.md) (paper runs in the app, 2026-10-02); the phases' own migrations moved up one.
**Scope class:** End-state architecture.

---

## 1. Why this set exists

The owner's direction, 2026-10-01: a strategy must behave *exactly the same* in backtest, paper and
live; backtest and paper should *work similarly but not exactly the same*; there are two kinds of
strategy — one that answers *what should we buy*, one that returns the trade decision — and a strategy
says everything that goes into a trade, so whatever the trading terminal can do, a strategy can do; any
model trained in MLOps can be called with the right types in and out; models, indicators and
thresholds compose in any blend; and people and AI write, use and test strategies identically.

The audit found the platform further from that than its plan files suggested. Three different
programs evaluate a strategy. **On paper, indicators are never computed**, so an indicator strategy
never trades. **No strategy can call a model in any mode.** Every model's answer is forced into a
forecast shape and missing model inputs are filled with zeros. Strategies can only place market
orders. Selection logic exists and nothing calls it. Backtests skip the risk gate. The full list is
FEAT-007 §1.

This set replaces all of that with **one pure engine** that every environment drives, **typed
dataflow graphs** of two kinds, **trade intents** a shared planner turns into orders, **typed model
contracts** resolved point-in-time, and a **nightly replay** that proves paper and backtest still
decide identically.

---

## 2. Scope

| Area | Today | Set U delivers |
|---|---|---|
| Evaluation | three evaluators that disagree | one pure, deterministic `strategy-engine` |
| Indicators on paper | never computed | computed inside the engine from raw bars, everywhere |
| Strategy shape | conditions → signals → market orders | typed dataflow graph; `trading` and `selection` kinds |
| What a strategy can do | market buy/sell | hold, target, market/limit/stop/stop-limit/trailing, bracket, protect, close, add, reduce, cancel |
| Models | forecast-shaped, zero-filled, never called | typed contracts; any kind; chained; point-in-time in backtests; pinned versions |
| Selection | unreached | ranked selections; deployments that trade whatever a selection picks; portfolio backtests |
| Backtest venue | market-only facade, no fills back | SimulatedVenue on the paper venue's core |
| Risk gate | paper only | every environment |
| Authoring | four surfaces, three grammars | one canonical graph; text, visual and builder views; one validator |
| Parity | asserted | journaled and replayed nightly |

### Out of scope

- Enabling live trading. Live mode stays refused by policy; this set makes the live environment
  *correct*, not *on*.
- Set T's proposal and approval path. Phase 4 writes the deployment record Set T needs and stops there.
- New indicator families (DATA-006 owns them) and new model families (FEAT-006 owns them).

---

## 3. Decisions this set implements

| ADR | Decision |
|---|---|
| [0039](../../../adr/0039-one-deterministic-strategy-engine-environments-are-adapters.md) | one deterministic engine; environments are adapters |
| [0040](../../../adr/0040-strategies-are-typed-dataflow-graphs-of-two-kinds.md) | typed dataflow graphs; `trading` and `selection` kinds; deployments bind them |
| [0041](../../../adr/0041-decisions-are-trade-intents-planned-into-orders.md) | trade intents and one order planner |
| [0042](../../../adr/0042-decision-cadence-bar-close-fills-and-schedules.md) | decide on bar close, own fills, schedules — never every tick |
| [0043](../../../adr/0043-typed-model-contracts-resolved-per-environment.md) | typed model contracts; point-in-time resolution; pinned versions |
| [0044](../../../adr/0044-native-simulated-venue-shares-the-paper-venue-core.md) | native SimulatedVenue on the paper core; market_simulator as reference — **reverses the owner's 2026-06-08 choice of default backtest venue** |
| [0045](../../../adr/0045-one-language-front-end-graph-is-canonical.md) | one language front-end; the graph is canonical |
| [0046](../../../adr/0046-risk-gate-in-every-environment.md) | the risk gate in every environment |
| [0047](../../../adr/0047-decision-journal-and-replay-parity.md) | decision journal and nightly replay parity |
| [0048](../../../adr/0048-paper-trading-runs-in-the-app.md) | paper trading runs in the app, never on a broker's paper account (owner, 2026-10-02) |

---

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 0 | [The contracts](phase-0-contracts.md) | 5 | — | DONE 2026-10-01 |
| 1 | [The engine core](phase-1-engine-core.md) | 9 | — | DONE 2026-10-01 |
| 2 | [Venues and the risk gate](phase-2-venues-and-risk.md) | 6 | — | 5 / 6 — U-2.6 open |
| 3 | [The backtest environment](phase-3-backtest-environment.md) | 5 | — | NOT STARTED |
| 4 | [Paper and live](phase-4-paper-and-live.md) | 6 | 0142–0144 | NOT STARTED |
| 5 | [Models](phase-5-models.md) | 7 | 0145–0147 | NOT STARTED |
| 6 | [Selection strategies](phase-6-selection.md) | 5 | 0148–0149 | NOT STARTED |
| 7 | [Authoring — identical for people and models](phase-7-authoring.md) | 6 | — | NOT STARTED |
| 8 | [Parity, proven; old paths retired](phase-8-parity-and-retirement.md) | 5 | — | NOT STARTED |

### Dependency graph

```
Phase 0 ──▶ Phase 1 ──┬──▶ Phase 3 (backtest) ──┬──▶ Phase 6 (selection)
   │                  │        ▲                │
   │        Phase 2 ──┴──▶ Phase 4 (paper/live) ┴──▶ Phase 8 (parity, retirement)
   │                            ▲                        ▲
   ├──▶ Phase 5 (models) ───────┘                        │
   └──▶ Phase 7 (authoring) ─────────────────────────────┘
```

Phases 1, 2, 5 and 7 can proceed in parallel once Phase 0 lands; they share only the Phase 0 types.

---

## 5. The agent contract

As [Set S §5](../set-S/MASTER.md#5-the-agent-contract), with these differences:

- **Migration numbers come only from 0142–0149** (0141 is ADR-0048's).
- **The engine stays pure.** `strategy-engine` may not depend on `execution`, `risk`, `storage`,
  `model-registry`, `tokio` or any clock; a Cargo-graph test enforces it from Phase 3 on. A task that
  needs I/O inside the engine is a design error, not a dependency to add.
- **No second evaluator.** Until Phase 8 retires them, the old evaluators may be *called* by the old
  paths only; nothing new may call `evaluate_signals`, `preview_signals`' loop, or a v1
  `StrategyInstance`.
- **No silent zero, no `NaN`.** Absence is `missing(reason)` everywhere, and an intent that depends on
  it is `hold`. A default value standing in for a missing one is a defect, whatever it saves.
- **Every order passes the risk gate**, in every environment, and a test proves it per entry point.
- **No live-venue code path is enabled.** The live environment is built and tested against paper.

---

## 6. Exit criteria

FEAT-007 §15, all ten. In short: one engine evaluates every strategy everywhere (guard); a paper
journal replays through the backtest byte-identically; an indicator strategy fires on paper unaided;
a chain of two models and an indicator backtests point-in-time and paper-trades; every model output
type is consumed without reshaping; every intent executes in both venues; selections rank, deploy
and backtest as portfolios; every backtest order passes the gate; and a person and a model produce the
same graph and the same errors.
