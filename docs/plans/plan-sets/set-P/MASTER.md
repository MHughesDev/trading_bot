# Data Reach — Set P

**Completion: 79% left (4 / 30 tasks). 4 done, 5 partial, 1 BLOCKED:UPSTREAM, 20 not started.**

> **This line read `0% (0 / 30 tasks). NOT STARTED.` until 2026-09-21** — while
> this set’s own phase files recorded real work. It was wrong by the maximum
> possible amount for six days, because the test that holds every other count
> honest scans `phase-*.md`, and a MASTER is not a phase. The file whose name
> promises the summary was the only one outside the mechanism that checks
> summaries. `every_set_master_counts_its_own_set` now covers it.

**Status:** DRAFT (2026-09-15) — ready for execution.
**Created:** 2026-09-15
**Stage:** [0 and 1](../../ROADMAP.md#4-the-stages) — **starts immediately, concurrently with Set S.**
**Derived from:** [DATA-005 §7, §8, §11, §12](../../../specs/DATA-005-data-api-v2.md),
[BS-007 06_DATA §6](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD),
[ADR-0027](../../../adr/0027-free-only-data-sources.md) (free-only sources),
ADR-P2-24 (the `knowledge_time` sentinel), `docs/artifact.md` FM-11.
**Requirement IDs:** DA-07, DA-08, DA-10, DA-11, DA-12, DA-14 · sources S1–S8.
**Migration range:** 0097–0104.

**Migrations shipped:** 0100 — `data_qc_reports` (P-2.2), 2026-09-15.
**Scope class:** End-state architecture. Phases are build-ordering, not feature-gating.

---

## 1. Why this set exists, and why it is first

Set S makes the agent good at research. Set P decides whether there is anything for it to
be right about.

Three facts, each independently sufficient:

1. **The platform's history is thin and its depth has never been measured.** The last
   recorded check found BTC-USD holding **22 one-hour bars** (DATA-005 S1, 2026-09-08).
   `strict_v1` Gate 9 has a five-year calendar floor; CPCV, walk-forward and regime work
   need years. Nobody currently knows how far the platform is from either.
2. **Several series cannot be backfilled at all.** Funding rates, open interest, options
   chains and order-book depth are *forward-collect only* under a free-only data policy.
   There is no vendor to buy 2024 from. **Every day a collector is not running is a
   permanent, unfillable hole in every study that ever uses that series** — `artifact.md`
   FM-11, the only failure mode in the artifact marked unrecoverable at any cost.
3. **Backfill is cheap and boring.** Phase 0 is days of work. Deferring it behind 189
   interesting tasks trades a permanent loss for a temporary convenience.

**The first task of this set is to measure, not to build.** P-0.1 publishes what history
actually exists before anything is collected, because "we have thin data" is currently a
belief, and the size of the gap decides whether `strict_v1` is reachable this decade or
whether `paper_v1` (ADR-P2-14) carries the programme for years.

---

## 2. Scope

### In scope

| Area | Today | Set P delivers |
|---|---|---|
| Coverage knowledge | unmeasured; one anecdote from 2026-09-08 | a published per-instrument, per-timeframe census, refreshed nightly |
| Bar history | months at best, from live collection | resumable deep backfill from three free venues, targeting ≥ 3 years of 1m for BTC-USD and ETH-USD |
| Forward-only series | **none collected** | funding, open interest, crypto option chains, equity chain snapshots — running from Phase 0 |
| Data quality | `data_qc` specified, grades unenforced end to end | the §7 checks, A–D grades, refusal at Experiment creation, waivers |
| Alternative data | none | FRED/ALFRED vintages, EDGAR fundamentals, Alpaca news and corporate actions, Kalshi |
| Derived access | fixed timeframes | server-side resampling, information-driven bars, PIT universes |
| Source policy | ADR-0027 by convention | a CI check that fails the build on a paid source |

### Out of scope

- **Paid feeds.** DA-14: adding one requires revisiting ADR-0027 with the user. The CI
  check in P-4.5 makes that a decision rather than a drift.
- **The Data API surface itself** — DA-01…DA-06, DA-09, DA-13, DA-15 belong to Set L
  Phase 2, which is complete for the cutoff core. Set P adds *sources and quality*, not
  endpoints, except where a new store needs a read path.
- **Equities/futures/options schema beyond snapshots.** Blocked on roadmap gate H-6 (is
  equities onboarding real?); the option-chain snapshot tables are built because they are
  forward-only, not because the asset class is onboarded.
- **Retention policy for raw events** — Q-8, still deferred. P-0.6 sets a disk budget for
  the forward snapshots this set switches on, which is a narrower question.

---

## 3. Locked decisions carried into this set

| ID | Decision | Source |
|---|---|---|
| D-P1 | Free sources only. A paid source is an ADR, enforced by a CI check, not a review habit | ADR-0027, DA-14 |
| D-P2 | Backfilled rows carry an honest `knowledge_time`: bar close + declared vendor lag, plus `BACKFILLED_KNOWLEDGE_TIME`. **Flag, never block** — the sentinel is absorbed structurally by `EmbargoInputs.max_knowledge_lag_ms` | ADR-P2-24 (OQ-01) |
| D-P3 | Gate 15's forward-test evidence comes from observed-knowledge rows only. The sentinel never reaches money | ADR-P2-24 |
| D-P4 | Bars are append-only in `market_bars_v2` with `timeframe` in the sorting key; revisions are retained, never overwritten | DA-16, Set L Phase 0 |
| D-P5 | Measure before collecting. The census is published before the first backfill job runs, so the programme's evidence clock is a number rather than a feeling | this set, §1 |
| D-P6 | Forward-only collectors are switched on **before** the backfill work, because backfill loses nothing by waiting a week and forward collection loses exactly one week | `artifact.md` FM-11 |

---

## 4. Phase summary

| Phase | Title | Tasks | Requirement IDs | Migrations | State |
|---|---|---|---|---|---|
| 0 | [Measure, and start the irreversible clock](phase-0-measure-and-collect.md) | 6 | S5, S6, S8, DA-12 | 0097–0098 | NOT STARTED |
| 1 | [Deep backfill](phase-1-deep-backfill.md) | 7 | S1, DA-11 | 0099 | NOT STARTED |
| 2 | [Quality and eligibility](phase-2-quality-and-eligibility.md) | 6 | DA-07 | 0101 | 5 / 6 — P-2.3 (enforcement) deliberately last |
| 3 | [Alternative data](phase-3-alternative-data.md) | 6 | S2, S3, S4, S7, DA-11 | 0102–0103 | NOT STARTED |
| 4 | [Derived access and the source fence](phase-4-derived-access.md) | 5 | DA-08, DA-10, DA-14 | 0104 | NOT STARTED |

### Dependency graph

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 4
   │                        ▲
   └──▶ Phase 3 ────────────┘
```

Phase 0 blocks everything only because it creates `backfill_state` and the census. Phases
1 and 3 are independent of each other and can run as two branches. Phase 2 needs history
to grade, so it follows Phase 1 for bars but can be built against Phase 0's census.

**Set P does not depend on Set S at any point**, and Set S does not depend on Set P until
its research runs want depth. They are two branches in one workspace, sharing only
`Cargo.toml`, `Cargo.lock` and `crates/collectors/src/lib.rs`
([roadmap §7](../../ROADMAP.md#7-concurrency-and-collision-control)).

---

## 5. The agent contract

As [Set S §5](../set-S/MASTER.md#5-the-agent-contract), with these differences:

- **Migration numbers come only from 0097–0104.** Set S holds 0061–0096; taking one of
  those collides with a concurrent track.
- **Every collector task is rate-limit-first.** A backfill that gets the platform banned
  from a free endpoint costs more than the history it fetched. Declare the limit, honour it
  in code, and prove it with a test that asserts the limiter refuses to exceed it — not a
  `sleep` in a loop.
- **Never write `market_bars`.** Write through `BarStore::insert_collected`, which targets
  v2. A CI guard already exists (`cargo xtask check-bars-v1-frozen`); do not work around it.
- **`knowledge_time` is not optional and not guessable.** Every row this set writes states
  whether its knowledge time was observed or derived, and derived rows carry the flag. A
  row that cannot say which does not get written.
- **Verify terms before building a source.** DATA-005 §11 marks S5, S6 and S8 "verify
  terms". If a source's terms forbid it, record that in the task as `BLOCKED:UPSTREAM` with
  the quoted term and move to the next source; do not build it anyway.

---

## 6. Exit criteria

1. **DATA-005 §10.3** — BTC-USD and ETH-USD have ≥ 3 years of continuous 1m history graded
   A or B, and the census page shows it.
2. **DATA-005 §10.2** — `data_qc` grades a series with planted bad ticks, stale runs and a
   gap correctly, and Experiment creation on a grade-D series fails with a teachable error.
3. **DATA-005 §10.4** — an EDGAR fundamentals request as of date *t* never returns a value
   filed after *t*.
4. **Forward-only series have unbroken coverage from the day Phase 0 landed**, with a
   monitored gap alarm — the acceptance that matters most, because it is the only one that
   degrades silently.
5. Every source in the platform is free, proven by a CI check that fails on a new paid one.
6. A dataset built from any of these sources carries its sources' maximum declared lag into
   `EmbargoInputs.max_knowledge_lag_ms`, so the sentinel cannot flatter a result.

---

## 7. Coverage

| Source | Criteria | Tasks |
|---|---|---|
| DATA-005 §10.1 | (Set L Phase 2, complete) | — |
| DATA-005 §10.2 | data_qc grading and refusal | P-2.1…P-2.3, P-2.6 |
| DATA-005 §10.3 | ≥ 3 years 1m, graded | P-1.1…P-1.7, P-2.2 |
| DATA-005 §10.4 | EDGAR PIT | P-3.2 |
| DATA-005 §10.5 | (Set L Phase 6, complete) | — |
| DA-07 | grade gating | P-2.3 |
| DA-08 | PIT universes | P-4.3, P-4.4 |
| DA-10 | resampling, information-driven bars | P-4.1, P-4.2 |
| DA-11 | S1–S4 resumable jobs | P-1.1…P-1.4, P-3.1…P-3.4 |
| DA-12 | forward option snapshots | P-0.4, P-0.5 |
| DA-14 | no paid source | P-4.5 |
| FM-11 | forward-only collection | P-0.3, P-0.4, P-0.5, P-0.6 |

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| A backfill gets the platform rate-limited or banned from a free endpoint | declared limits honoured by a limiter with a test that proves refusal; resumable cursors so a ban is a pause, not a restart |
| The census reveals depth is far worse than believed, and the programme's assumptions change | that is the *point* of measuring first (D-P5); the finding goes to the roadmap as a decision for the user, not into a plan revision |
| Forward collectors run but write dishonest `knowledge_time` — the v2 bug that already happened once | D-P2, and a test that asserts an observed row and a backfilled row of the same bar are distinguishable |
| Forward collection silently stops and nobody notices for a month | P-0.6's gap alarm is an acceptance criterion, not a nice-to-have; silence is the failure mode |
| Option-chain snapshots fill the disk | P-0.6 sets a disk budget and a downsampling rule before the first snapshot, not after |
| A source's terms forbid what the spec assumed | §5: record the quoted term, mark `BLOCKED:UPSTREAM`, move on; never build against terms |
| Grade gating lands and refuses every existing Experiment | grades are published before enforcement (P-2.6 before P-2.3), so the blast radius is known before the switch |

---

## 9. Progress log

| Date | Phase | Task | Note |
|---|---|---|---|
| 2026-09-15 | — | — | Set P created from DATA-005 §7/§8/§11/§12 and 06_DATA §6, placed at Stage 0 of the roadmap ahead of Set S on the evidence-clock argument. Nothing implemented. |
