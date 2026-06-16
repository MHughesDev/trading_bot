# Phase 6 — Publish contract & workbench UX

**Completion: 0% (0 / 9 tasks)**

**Goal:** Land **the one seam that matters** — the immutable, point-in-time
distributional **publish contract** downstream surfaces call — and the **workbench
UX** that makes the whole suite usable: distribution and calibration charts, the
leaderboard and report viewer, and the greenfield **Ensemble Builder** and
**Pipeline Builder**, with templates and a run queue. Top-level navigation finally
reads **Models · Ensembles · Pipelines**.

**Depends on:** Phases 1–5 (the contract serves Phase 1 distributions calibrated in
Phase 4; the UI renders Phase 2 reports and Phase 5 pipelines).
**Blocks:** downstream surfaces (Strategies/Backtest/Live) consuming published
forecasters — out of scope here, unblocked by here.

---

## Design notes

**The publish contract (the handoff).** A published artifact (model, ensemble, or
inference pipeline) exposes one stable seam:

```
predict(asset, timeframe, timestamp)
   -> CalibratedForecast {
        quantile_levels: [..], quantiles_return: [..], median_return,   // sorted, return units
        sigma,
        risk: { var: {0.95:_, 0.99:_}, es: {..}, skew, spread },        // derived from quantiles
        version, produced_at, point_in_time: true
      }
```

- **Point-in-time correct:** `timestamp` is an `as_of` ceiling; the contract never
  reads a future bar (Phase 0 guard).
- **Immutable & versioned:** a consumer that pinned version N keeps getting version
  N's exact behavior — published bundles never mutate (new behavior ⇒ new version).
- **Train/serve parity:** the published path is the Phase 1/4 inference path the
  evaluation ran — one bundle, no second implementation (D-9). A CI assert pins this.

This is the same `domain::model::forecast` distribution from Phase 1 with risk
read-outs attached; it is **not** a new model — the suite still stops here and never
trades.

**Frontend** is greenfield for Ensembles/Pipelines but reuses the Set H Studio
chrome: React 19 + Vite + Tailwind 4, `@xyflow/react` for DAGs, `lightweight-charts`,
the `--tb-*` tokens, the `api/models.ts` client patterns, and the `useModels` hook
style. New surfaces mirror `ModelStudioPage`/`ModelDetailPage`.

---

## Tasks

### ☐ I-6.1 Distributional publish contract — M
Implement `predict(asset, timeframe, timestamp) -> CalibratedForecast` over the
inference path for published models, ensembles, and inference pipelines; resolve via
the existing alias hot-map; enforce the `as_of` (point-in-time) ceiling and version
immutability. Add `GET /api/models/{ref}/predict` (+ the existing gateway path).
**Acceptance:** predict returns a sorted calibrated distribution for a pinned version; the same pin returns identical behavior after a new version is promoted; a future `timestamp` is refused/clamped, never leaked.

### ☐ I-6.2 Derived risk read-outs (VaR/ES/skew/spread) — M
Compute VaR, ES, skew, and interval spread **from the published quantiles** (one
source of truth) and attach them to `CalibratedForecast.risk`. These are read-outs of
the model's own distribution — not a risk engine (ADR-0005 untouched).
**Acceptance:** VaR/ES at 95/99% match a direct computation from the quantiles on a fixture; skew/spread populate; values are f64 return-units (no `Price`/`Size`).

### ☐ I-6.3 Train/serve parity guarantee (CI-pinned) — S
Add a test asserting the **published predict path == the eval predict path** (same
bundle loader, same feature reconstruction, same calibration), so the two can never
drift. Wire it into CI.
**Acceptance:** a deliberate divergence (e.g. a serve-only scaler tweak) fails the parity test; the honest path passes; the test runs in `just test`.

### ☐ I-6.4 Registry tags, annotations & templates/presets — M
Extend the registry with free-form **tags** + **annotations** on models/ensembles/
pipelines (search/filter), and a **templates/presets** store (a starter spec users
fork) feeding the Create wizards. Migrations (Postgres 0026+: `model_tags`,
`spec_templates`).
**Acceptance:** an artifact can be tagged/annotated and found by tag; a preset is forkable into a new draft; tag search is exposed in the list API.

### ☐ I-6.5 Frontend: distribution & calibration charts — L
Build reusable React components: a **fan/quantile chart** (distribution over horizon),
a **PIT histogram**, a **reliability diagram**, and a **coverage-vs-nominal** plot,
fed by the Phase 2 report API. Add them to the model/ensemble Detail cockpit.
**Acceptance:** a model's report renders a fan chart + PIT + reliability + coverage from real eval data; charts use `--tb-*` tokens and match Studio styling.

### ☐ I-6.6 Frontend: leaderboard & report viewer — M
Build the leaderboard page (rank by metric, filter by asset/timeframe/regime, DM-vs-
leader badges) and a shareable report viewer (all Phase 2 sections + export).
**Acceptance:** the leaderboard ranks real evaluation runs with significance badges; a report opens, renders every section, and exports to a shareable file.

### ☐ I-6.7 Frontend: Ensemble Builder — L
Build the greenfield Ensemble surface (route `/ensembles`): roster picker (search
models), combiner selector (LOP/CRPS-weighted/stacking), weight-floor/temperature
controls, calibration toggle, then train-weights → evaluate (reusing the Phase 2 UI).
Mirror `ModelDetailPage` for the ensemble cockpit.
**Acceptance:** a user composes a 3-model ensemble, picks a combiner, trains weights, and views its evaluation report — all from the UI, hitting the Phase 4 API.

### ☐ I-6.8 Frontend: Pipeline Builder + run queue — L
Build the greenfield Pipeline surface (route `/pipelines`): a `@xyflow/react` DAG
canvas (data→features→target→train→calibrate→evaluate→register), a fan-out matrix
editor (asset×timeframe×window), a schedule control, and a **run queue / compute
panel** showing concurrency, queued/running/cached runs, and per-cell fan-out status.
**Acceptance:** a user builds a DAG, sets a fan-out matrix, runs it, and watches per-cell progress + the run queue update live over WS.

### ☐ I-6.9 Frontend: top-level nav (Models · Ensembles · Pipelines) + templates — M
Add `/ensembles` and `/pipelines` to `GlassPillNav`/`App.tsx` so the suite's
navigation matches the spec's three-way split; surface templates/presets in the
Create wizards.
**Acceptance:** the nav shows Models · Ensembles · Pipelines; each lands on its command center; presets are forkable from the Create flow.

---

## Phase 6 exit criteria

- A published model/ensemble/inference-pipeline exposes the immutable, point-in-time
  `predict → calibrated distribution + risk read-outs` contract; a pinned version's
  behavior is stable across later promotions.
- A CI test pins train/serve parity (published path == eval path).
- The registry supports tags/annotations/templates.
- The UI renders distribution/calibration/leaderboard/report surfaces and provides
  working Ensemble and Pipeline builders with a run queue; nav reads
  Models · Ensembles · Pipelines.
- The suite still stops at the published forecaster — no order, size, or P&L anywhere.
- `cargo test --workspace` + sidecar pytest + `frontend` build green; `just lint`,
  `just fmt-check`, `just check-money` green.
