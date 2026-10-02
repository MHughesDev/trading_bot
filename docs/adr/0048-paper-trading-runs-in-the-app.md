# ADR-0048: Paper Trading Runs in the App, Never on a Broker's Paper Account

**Status:** Accepted — **owner decision 2026-10-02**: "make it where the paper trading is happening through an in app paper trading software, not through alpaca paper trading"
**Date:** 2026-10-02
**Supersedes:** the "Alpaca paper account" execution system of [ADR-0006](0006-three-system-broker-architecture-coinbase-alpaca-market-simulator.md). Alpaca stays as an equity **data** source and as a candidate **live** equity adapter; it is no longer a paper venue.
**Related:** [ADR-0044](0044-native-simulated-venue-shares-the-paper-venue-core.md) (the shared venue core), [ADR-0046](0046-risk-gate-in-every-environment.md), [ADR-0047](0047-decision-journal-and-replay-parity.md); Set T T-0.4; migration 0141.

## Context

ADR-0006 named Alpaca's paper account as the platform's paper venue, and Set T built
the forward-evidence pipeline (Gate 15) around it: `reconciliation::forward::PaperVenue`
had one variant, `AlpacaPaper`, and migration 0105 constrained
`mlops.paper_deployment.venue` to `'alpaca_paper'`. That made Alpaca paper credentials a
human gate (ROADMAP H-7) standing between the platform and its first forward
deployment.

The running platform never routed paper orders to Alpaca. The manual order door and the
hot path both fill through `execution::paper::PaperTradingEngine`, the in-app paper
venue. The dependency on Alpaca existed only in the forward-evidence design and the
schema.

The owner has decided that paper trading happens in the app.

## Decision

1. **The paper venue is the platform's own** — `execution::paper::PaperTradingEngine`,
   exposed behind the `execution::Venue` port as `PaperVenue`. It is the venue core
   every backtest runs on (ADR-0044), on the wall clock with live marks instead of on a
   simulated clock with bars.
2. **`reconciliation::forward::PaperVenue` has one variant, `Platform`
   (`"platform_paper"`).** `AlpacaPaper` is removed. The one-variant shape stays, so
   naming any other venue is still a source change and roadmap gate H-4.
3. **Migration 0141** replaces 0105's venue CHECK with `venue IN ('platform_paper')`.
4. **ROADMAP H-7 (Alpaca paper credentials) is withdrawn.** Deploying a strategy to
   paper needs no credential. T-0.4 is now blocked only on the engineering that is
   recorded against it.
5. No code path sends a paper order to a broker's paper endpoint. `AlpacaBroker`
   keeps its constructor for a future live equity adapter. That adapter is gated by H-4
   like every live path.

## What forward evidence on this venue measures, and what it does not

This is the consequence that matters. It is written down so that a Gate 15 verdict is
never read for more than it says.

Set T's original plan refused the in-app engine for Gate 15. Its reason was that
"reconciling a simulator against the same simulator measures nothing". That reason is
half right, and after ADR-0044 it is sharper. The paper venue and the backtest venue now
share one fill model *by construction*. A forward series from in-app paper therefore
**cannot** reveal that the simulator's fills or costs are wrong. Both sides of the
comparison come from the same simulator.

It still measures a great deal:

- **The strategy on data nobody had when it was built.** Signal decay, regime change
  and overfitting all show up as realised-versus-backtest divergence. Since both sides
  share one cost model, the divergence is attributable to the *strategy*, which makes
  it cleaner evidence about the strategy than a broker's paper fills would be.
- **The pipeline end to end, in observed time.** Live marks, bar construction, cadence,
  the risk gate, journaling, the restart path and reconciliation all run in observed
  time. This is what DONE-3's calendar is for.
- **Replay parity.** ADR-0047's nightly replay of paper through the backtest should
  agree *exactly*. Any disagreement is a bug in one of the two drivers, not market noise.

Fill realism is the one thing it does not test: whether simulated fills and costs match
a real market. That question moves to the first live trades. Each live fill is compared
against what the simulator would have charged for the same order at the same moment.
The comparison is recorded as its own measurement, the **fill-model check**, and is not
folded into Gate 15. Until then, every Gate 15 verdict on `platform_paper` is labelled
as resting on *modelled fills*.

A broker's paper account would not have closed this gap either. Broker paper fills are
also simulated. Alpaca describes its paper environment as not modelling market impact
or slippage. It is a different simulator, not a market.

## Consequences

- **Good:** No credential stands between the platform and its first forward deployment.
  Paper and backtest are one venue core with two drivers, which is the owner's
  "similar but not the same". Paper behaviour no longer depends on a third party's
  paper-engine quirks, outages or rate limits.
- **Bad:** Forward evidence is silent on fill realism until live trading starts. That
  check is now a named, tracked item rather than something assumed covered.
- **Neutral:** Alpaca remains the equity data feed (ADR-0006) and a candidate live
  equity adapter.
