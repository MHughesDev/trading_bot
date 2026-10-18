# Phase 5 — Frontend Studio

**Completion: 0% (0 / 9 tasks)**

**Goal:** Build the **AI Model Studio** UI — an operations cockpit, not a settings
page. Discovery via living cards, depth via a per-model cockpit, testing via a
playground, lineage via the node canvas, training via a live console. Extremely
intuitive, extremely polished — and entirely in the existing design language so
it feels native on day one.

**Depends on:** Phase 1's frozen contract (can build in parallel with Phases 2–4
using stubbed data, then light up as each backend phase lands).

---

## Design language — what "sexy" means *here*

The app's aesthetic is restrained and premium: a glassmorphism nav pill with
spring physics (`GlassPillNav.tsx`), `--tb-*` CSS-variable theming (light/dark,
`index.css`), Inter + JetBrains Mono, Radix/shadcn primitives, and a React-Flow
canvas. "Sexy" = **elevated polish in that language**, never a different style:

- **Tokens only.** `bg-surface`, `text-text`, `border-border`, `text-accent`,
  `text-pnl-up/-down`, `--tb-*`. No hardcoded hex. Both themes must look
  deliberate.
- **Status as light.** Each lifecycle state gets a color + a soft glow pill
  (`shadow-[0_0_0_3px_rgba(...)]` halo); `Training`/`Evaluating` pulse subtly via
  framer-motion; `Active` glows accent/green; `Failed` dim red.
- **Motion with restraint.** Reuse the nav's spring (`stiffness 380, damping 30`);
  `layoutId` shared-element transitions for card→cockpit; `useReducedMotion`
  respected everywhere. Cards lift on hover (`-translate-y-0.5`, shadow).
- **Data as texture.** Mini sparklines (accuracy/loss) on cards via
  `lightweight-charts`; radial **scorecard rings** via `@radix-ui/react-progress`
  styled circular; JetBrains Mono for all metrics.
- **Depth via glass + gradient.** Section headers use a faint accent gradient
  wash; the cockpit header is a `bg-surface/80 backdrop-blur` bar like the nav.
- **Never a dead screen.** Every list has a designed empty state (icon + one-line
  + primary action), every async surface has a skeleton, every mutation a toast.

---

## Route map

| Route | Screen | Notes |
|-------|--------|-------|
| `/models` | **Command Center** | Cards (default) ⇄ table toggle; filters; create/compare |
| `/models/create` | **Create Wizard** | 4-step guided flow |
| `/models/:id` | **Cockpit** (default → Overview) | Tabbed; glass header |
| `/models/:id/versions` | Version Timeline | Vertical timeline + promote/rollback |
| `/models/:id/train` | Training Console | 3-pane live run |
| `/models/:id/test` | Test Lab | Per-kind playground |
| `/models/:id/evaluations` | Evaluations Arena | Compare + scorecards + regression |
| `/models/:id/deployments` | Deployment Control Room | Aliases, traffic, env |
| `/models/graph` | Lineage Graph | React-Flow canvas |

Tabs in the cockpit map to the sub-routes (Overview | Versions | Train | Test Lab
| Evaluations | Deployments | Settings).

---

## Tasks

### ◐ H-5.1 Routing, nav entry, data layer scaffolding — M
**Status: PARTIAL (2026-10-03).** Done: the routes are mounted at `/mlops`, `/mlops/create`, `/mlops/graph`, `/mlops/leaderboard`, `/mlops/:id` (App.tsx) behind the nav entry "Models"; the typed client is `frontend/src/api/mlops.ts` and the hooks `frontend/src/hooks/useMlOps.ts` (the spec's `models.ts`/`useModels.ts` names were not used); `useModelRuns` polls fast while a run is live and keeps retrying after a failed read. Client/server mismatches found and fixed: `promote` sent `reason` (the server reads `override_reason`/`environment`), `compareEvals` sent `a,b` (the server reads `versions=a,b`), `usedBy` and `listEvals` types did not match the served shapes. `tsc`, `eslint` (0 problems) and `vite build` are clean for these files. **Missing:** the `api/ws.ts` subscribe helper for `models.jobs` -- the registry broadcasts on that lane but no WebSocket route consumes it, so the Studio polls (see H-5.5). 2026-10-04: `modelsApi.trainingCurve` / `modelsApi.events` and the hooks `useTrainingCurve` / `useModelEvents` were added for the two new routes. Evidence: `frontend/src/components/mlops/__tests__/the-command-center-reads-the-registry.test.tsx` (6).

- `App.tsx`: add lazy routes for the map above (code-split like `StrategyCreationPage`).
- `GlassPillNav.tsx`: add `{ path: '/models', label: 'AI Models', icon: Brain }`
  to `SECTIONS` (lucide `Brain`).
- `frontend/src/api/models.ts`: typed axios client + **TS types mirroring the
  Phase-1 contract** (`AiModel`, `ModelVersion`, `RunSnapshot`, `EvaluationRun`,
  `Scorecard`, `AliasMap`, `LineageGraph`, …) — hand-written like `api/backtests.ts`.
- `frontend/src/hooks/useModels.ts`: TanStack Query hooks (`useModels`,
  `useModel`, `useVersions`, `useRun`, `useEvaluations`) with the
  backtest-style **adaptive `refetchInterval`** (fast while a run is active).
- `frontend/src/api/ws.ts`: subscribe helper for the `models.jobs` lane.

**Acceptance:** `/models` renders behind the nav pill; types compile against the
contract; a query hook returns list data; no eslint/tsc errors.

### ◐ H-5.2 Command Center (`/models`) — L
**Status: PARTIAL (2026-10-03).** Done in `frontend/src/pages/MlOpsPage.tsx`: cards/table toggle persisted in localStorage (failure-tolerant), kind/status/asset-class/search filters sent to the list route, sort (name, kind, status, primary metric, version, updated; a model with no value sorts last in both directions), status pills, accuracy sparkline (inline SVG over versions reporting the same metric; "no history yet" below two points), headline metric of the PRODUCTION version, used-by chips (unknown until the read answers), skeleton cards and rows, designed empty and filtered-empty states, and an error state that is not the empty state. **Missing:** p95 latency and cost per 1k are not on the cards (no per-model rollup exists; the card says "not measured"); "Import" has no backend; both themes were not visually checked in a browser; and (2026-10-04) used-by chips now read a working route (H-4.5 fixed), though the card chips were not re-verified against live strategies. Evidence: `frontend/src/components/mlops/__tests__/the-command-center-reads-the-registry.test.tsx` (6), `frontend/src/components/mlops/__tests__/studio.helpers.test.ts` (40, includes summaries, sort, persistence).

The landing cockpit. Header with title + subtitle + actions
(`+ Create Model`, `Import`, `Compare`, view toggle). Filter bar
(kind · status · asset class · search). **Model cards** (default grid,
`md:grid-cols-2 xl:grid-cols-3`):

```
┌─────────────────────────────────────────┐
│ ◐ kind-icon   EgoGemma-Core      ●Active │   ● = glowing status pill
│ local reasoning · forecaster · v14       │
│ ╭───────── accuracy sparkline ─────────╮ │   lightweight-charts mini
│ Acc 87.4%   p95 182ms   $0.00/1k         │   JetBrains Mono badges
│ used by:  Chat  Forecasting  Search      │   "used-by" chips (derived)
│ [ Open ]  [ Test ]  [ Train ]            │
└─────────────────────────────────────────┘
```

Power-user **table view** (sortable: name, kind, status, primary metric, p95,
cost, version, updated) via the toggle. Cards lift on hover; clicking does a
`layoutId` shared-element transition into the cockpit.

**Acceptance:** cards render real list data with status pills, sparkline,
metric/used-by; filters + search work; table toggle persists (zustand or URL);
empty state + skeletons present; both themes clean.

### ◐ H-5.3 Create Wizard (`/models/create`) — L
**Status: PARTIAL (2026-10-03).** The 4-step wizard exists (`ModelCreatePage.tsx`: kind cards, framework/engine, data/options, identity with a live slug preview). Fixed this pass: the external-LLM path sent an empty `adapter` block it never collected -- it now collects provider, model, endpoint and cost per 1k (decimal string), will not advance without valid values, and sends them; a 422 `errors: [{path, message}]` is shown with its paths (field-level hint on the adapter inputs and a list on the final step) instead of "Failed to create model". **Missing:** the spec's "Source" step (fine-tune / import checkpoint / clone) and the feature-set / dataset-window / label-spec picker are not present (framework + options stand in); validation errors for non-adapter fields appear only on the final step, not beside the field. Evidence: `frontend/src/components/mlops/__tests__/the-wizard-collects-the-adapter-it-sends.test.tsx` (3).

Guided 4-step flow (not a raw form), each step a card-choice screen with a
progress rail and back/next (framer-motion step transitions):
1. **Purpose** → `model_kind` (Forecaster, Signal/Ranker, Trade-Decision,
   Risk/Sizing, Embedding, External-LLM Adapter) as selectable cards.
2. **Source** → from-scratch · fine-tune existing · import checkpoint · clone
   internal · register external adapter.
3. **Data** → pick/define feature set + dataset window + label spec (or
   "draft only, train later"); adapters skip to step 4.
4. **Identity** → display name (renamable) + auto `slug` (immutable, shown) +
   initial version note. Submit → `POST /api/models` → cockpit.

**Acceptance:** produces a valid `ModelDefinition` per kind; validation errors
surface inline (field-level, matching the API's 422 shape); slug preview updates
live; adapter path collects the `adapter` block; reduced-motion safe.

### ◐ H-5.4 Cockpit shell + Overview tab (`/models/:id`) — L
**Status: PARTIAL (2026-10-03).** Done in `ModelDetailPage.tsx`: glass header with status pill, rename (PATCH), archive, a Lineage link, tab deep-links via `?tab=`; Overview shows identity and aliases, a scorecard whose rings are the PRODUCTION version's latest evaluation (0-100, clamped -- the old rings multiplied by 100) and which names the version it describes, p95 latency and error rate rolled from `GET .../traces` ("not measured" with no traces; cost per 1k is not recorded), used-by chips from the real `{strategy_id, display_name}` shape, and an activity feed. **2026-10-04 (track F-UI):** `GET /api/models/{id}/events` (`routes/model_training_progress.rs`, owner-scoped, inference traces excluded) now feeds the activity feed, so a rename, promotion, alias set, rollback or archive shows in it (`eventText` / `buildActivity`; `the-loss-curve-is-the-recorded-series.test.tsx` 7 cases). **Still missing:** the sub-routes `/models/:id/versions` etc. are tabs, not routes; cost per 1k is not recorded on any trace; not checked in a browser. Evidence: `frontend/src/components/mlops/__tests__/the-cockpit-overview-reads-what-the-registry-serves.test.tsx` (4).

Glass header (name + status pill + kind/version + alias chips + actions menu:
Rename, Archive, Compare). Tab bar (sub-routes). **Overview** answers "what is
this, is it good, where is it used":
- **Identity** (name, kind, framework, current/production version, status, asset
  class).
- **Health** scorecard — radial rings (Quality/Speed/Cost/Safety/Reliability +
  Overall) + p95 latency, error rate, cost/1k from trace rollups.
- **Recent activity** feed from `model_events` (promoted, archived, dataset
  attached, eval passed) as a compact timeline.
- **Used in** strategy chips (links to those strategies).

**Acceptance:** rename flows through `PATCH` and shows in activity (rename-with-
history, never a silent string edit); scorecard rings reflect real
`scorecard_json`; health pulls trace rollups; deep-link to a tab works.

### ◐ H-5.5 Training Console (`/models/:id/train`) — L
**Status: PARTIAL (2026-10-03).** Done: 3-pane console (`ModelTrainTab.tsx`, `TrainLive.tsx`) with config, start, stop, a progress ring with phase, elapsed and an ETA from the observed rate, a progress-over-time curve, the event feed of observed phase changes, a "lost contact" banner with retry (polling retries every 5 s after a failed read), and on completion an "Evaluate vN" action plus a this-run-versus-best-earlier-version table by metric direction. **2026-10-04 (track F-UI): the loss curve exists.** The trainer already published `{epoch, train_loss, val_loss}` on every progress frame; the registry merged each over the last in memory. Each epoch is now appended to `mlops.training_metric` (migration 0201, granted to `app_role` insert/select only), served by `GET /api/models/{id}/runs/{run_id}/curve` (owner-scoped; null = not measured), polled every 2 s while live, and drawn by `LossCurve` (train solid, validation dashed; a run with no validation split draws one line and says so). Evidence: `crates/model-registry/tests/pg_training_curve.rs` (3 tests against a scratch DB: frames to curve, first report of an epoch stands, null stays NULL, other owner and other model's run refused, `app_role` can append and never edit), 3 unit tests in `manager/curve.rs`, and 7 vitest cases (mutation-checked: a null loss drawn as 0 fails 2). **Still missing:** the `models.jobs` WebSocket lane still has no consumer (polling instead, H-ADR-5.5a), the previous-best "overlay" is still a table drawn after the run rather than the earlier version's curve (a version does not record its run's id), and tokens/s is not reported by the trainer. **Earlier note, superseded:** there were no live loss/metric curves; the console polled; the overlay was a table. Evidence: `frontend/src/components/mlops/__tests__/studio.helpers.test.ts` (appendSample, etaSeconds, compareToBest).

The signature "sexy" surface — a 3-pane live run:

```
┌── Config ────────┬── Live Training ───────────────┬── Run Events ──┐
│ dataset / window │  loss curve (train vs val)      │ epoch 12 done  │
│ base version     │  metric curve (acc/AUC)         │ checkpoint     │
│ hyperparams      │  progress ring + ETA + tok/s    │ eval started   │
│ output name      │  vs previous-best overlay       │ warning…       │
│ [ Start ] [Stop] │                                 │                │
└──────────────────┴────────────────────────────────┴────────────────┘
```

Config left; center = live curves (`lightweight-charts`) fed by the `models.jobs`
**WS lane** with poll fallback, a circular progress ring (Radix progress, styled),
elapsed/ETA, and an optional overlay of the previous best version's curve;
right = streaming event/log feed. Start/Stop/Retry. On completion, a toast + CTA
to evaluate.

**Acceptance:** starting a train shows live, smoothly-updating curves + progress
without manual refresh; stop cancels; the previous-best overlay toggles; logs
stream; reconnects cleanly if the socket drops.

### ◐ H-5.6 Test Lab (`/models/:id/test`) — L
**Status: PARTIAL (2026-10-03).** Two surfaces exist (`ModelTestTab.tsx`): LLM adapter, and a feature-vector form used for every other kind, with a latency readout and save/load/delete of test cases. Fixed this pass: the LLM surface sent no prompt (the server used its default "Hello"); it now sends `{prompt}` and shows latency, tokens, cost and trace id. **Missing:** per-kind bodies (signal-ranker table, trade-decision action/confidence, embedding vector/neighbours, forecaster chart with confidence bands and forecast-vs-actual), LLM temperature/max-tokens/top-p, and save-as-case on the LLM surface; the backend returns only `predictions` for non-LLM kinds. Evidence: `frontend/src/components/mlops/__tests__/the-prompt-typed-in-the-test-lab-reaches-the-model.test.tsx` (1).

A playground that **adapts to `model_kind`** (shared chrome, kind-specific body):
- **Forecaster** → instrument + window inputs → prediction chart with confidence
  bands + forecast-vs-actual overlay + latency/cost.
- **Signal/Ranker** → universe input → ranked table with scores.
- **Trade-Decision** → feature input → action + confidence + (if classifier)
  top-classes + explanation.
- **Embedding** → text in → vector preview + nearest neighbors + similarity.
- **External-LLM Adapter** → prompt + params (temp/max-tokens/top-p) → response +
  tokens + latency + cost + trace id.

Every run shows a metrics strip and can **save as a test case** (`POST
…/test-cases`) for reproducibility/regression.

**Acceptance:** each kind renders its correct surface and returns a real result
via the backend; metrics strip (latency/cost) populates; save-case round-trips;
switching kinds never shows the wrong controls.

### ◐ H-5.7 Evaluations Arena (`/models/:id/evaluations`) — L
**Status: PARTIAL (2026-10-03).** Rebuilt `ModelEvalsTab.tsx`. The old tab was broken against the real routes (it read `version`/`status:'completed'` from run snapshots that carry neither, and compared with the wrong query). Now: the list is the server's evaluation snapshots, each succeeded one read by id for its version, scorecard and regression verdict; the production row is pinned; running and failed evaluations are listed as such; compare takes two versions, shows aligned metric deltas (direction-aware, "not comparable" where one side never reported a metric), the regression checks, and a winner badge -- only when BOTH sides carry a scorecard (the server scores a missing one as 0.0 and would crown the evaluated side). **Missing:** a confusion matrix, a backtest P&L panel and forecast-versus-actual are drawn only if the evaluation recorded them, and the registry records none today (forecast-vs-actual is on the Forecast Quality tab); the list is the in-memory snapshots, so evaluations from before the last server start are absent (needs a route over `evaluation_runs`). Evidence: `frontend/src/components/mlops/__tests__/a-winner-needs-two-measurements.test.tsx` (5), `frontend/src/components/mlops/__tests__/studio.helpers.test.ts` (compare, deltas, pinning). Mutation-checked: crowning a side without a scorecard fails 3 tests.

Answer "is this version better than production?":
- Eval **list/table** (version · dataset · primary metric · latency · cost ·
  regression verdict · status) with the production row pinned.
- **Compare** two versions side-by-side (`…/evaluations/compare`): metric deltas
  with a **winner badge**, regression report, metric-movement charts, confusion
  matrix (classifier), forecast-vs-actual diff (forecaster), and the backtest P&L
  panel (Phase 3.4).

**Acceptance:** compare renders aligned metrics + winner + regression verdict for
two versions; classifier shows a confusion matrix; forecaster shows
predicted-vs-actual; backtest P&L appears when attached.

### ◐ H-5.8 Version Timeline + Promotion Gate + Deployments — L
**Status: PARTIAL (2026-10-03).** Done: version timeline on a vertical rail (status, aliases held, headline metrics, notes, expandable artifact/hash/metric drawer); `PromotionGateModal.tsx` -- paper/live, the client-known checks (evaluated, artifact hash, rollback) with the regression check left to the server, the confirm reads `promoted` from the body (a refused gate is HTTP 200), shows the server's `gate_checks` on refusal, and an override requires a reason that is sent as `override_reason` (the old modal sent `reason`, which was dropped, and closed on any 200); Deployment Control Room (`ModelDeploymentsTab.tsx`) with all four alias slots, per-environment (paper/live) allocation bars, a deploy form whose traffic slider validates against active deployments (`<= 100`, the server's rule), and rollback behind a confirm that names the version it lands on. **Missing:** rollback is confirm-then-click, not one click, and its audit row is written by the server but cannot be read back (no events route); candidate/staging/fallback cannot be set (no route) and deployments cannot be edited or stopped, so "traffic-split sliders" apply at creation only. Evidence: `frontend/src/components/mlops/__tests__/a-refused-gate-is-not-a-promotion.test.tsx` (5), `frontend/src/components/mlops/__tests__/a-traffic-split-over-100-is-not-a-deployment.test.tsx` (3), helper tests (gate parsing, traffic, rollback target). Mutation-checked: closing on any resolved promote fails 2 tests.

- **Versions** (`/models/:id/versions`): vertical timeline (v14 Active, v13
  passed eval, v12 regressed, v11 archived) with per-version scorecard badge,
  metrics, dataset/run/code refs, creator, notes; expand to a version drawer.
- **Promotion gate modal**: the checklist (passed eval · faster · no regression ·
  artifact hash verified · rollback available) with ✓/✗ and a confirm/cancel;
  live promote disabled until checks pass (override path logs a reason).
- **Deployment Control Room** (`/models/:id/deployments`): alias→version map
  (`production`/`candidate`/`staging`/`fallback`), env toggles (paper/live),
  traffic-split sliders for A/B, one-click rollback.

**Acceptance:** promoting goes through the modal and moves the alias (reflected
instantly across cards/overview); rollback is one click + audited; traffic split
validates ≤100%; the timeline reads cleanly at a glance.

### ◐ H-5.9 Lineage Graph (`/models/graph` + per-model) — M
**Status: PARTIAL (2026-10-03).** `GET /api/models/{id}/lineage` returns only `{model_id, aliases, note}` -- no nodes or edges -- so the old page always rendered "No lineage data yet". `ModelLineagePage.tsx` now DERIVES the graph from served data (versions, training runs, deployments linked to their version, aliases, used-by strategies attached to production) with themed custom nodes, pan/zoom/minimap, click-through to the cockpit tabs, and says it is derived. **2026-10-04: still PARTIAL; the used-by branch now has real data (H-4.5).** **Missing:** dataset nodes and the eval-suite branch (nothing served names a dataset), a real cross-model global graph (`/mlops/graph` without `?model=` lists models instead), and virtualisation for very large graphs (runs are capped at 12). Evidence: `frontend/src/components/mlops/__tests__/studio.helpers.test.ts` (deriveLineage, layoutLineage).

Reuse `@xyflow/react` (the strategy builder canvas) and the `.tb-node` styling to
render `dataset_version → training_run → model_version → deployment → strategy`,
plus the eval-suite branch. Custom node types per entity (color-coded headers like
the existing nodes), read-only with pan/zoom/minimap, click-to-cockpit. A
per-model lineage view powers a cockpit "Lineage" affordance; `/models/graph`
shows the global graph.

**Acceptance:** the graph renders real lineage from `GET …/lineage`; nodes are
themed and navigable; large graphs stay responsive (virtualized/limited);
matches the canvas look already in the app.

---

## Component inventory

**Reuse (no new dep):** `components/ui/{button,card,badge,dialog,select,input,
label,toast}`, `@radix-ui/react-progress` (rings/bars), `@radix-ui/react-tabs`
(cockpit tabs), `lightweight-charts` (curves/sparklines), `@xyflow/react`
(lineage), `framer-motion` (transitions/pulse), `lucide-react` (icons),
TanStack Query, the `--tb-*` tokens + `cn()`.

**New (Studio-local):** `StatusPill`, `ScorecardRing`, `MetricSparkline`,
`UsedByChips`, `ModelCard`, `KindIcon`, `WizardStepper`, `LiveCurve`,
`PromotionGateModal`, `TrafficSlider`, lineage node components. All token-styled;
no new design primitives.

---

## Phase 5 exit criteria

- The full route map renders, native to the app's look, in both themes, with
  skeletons/empty states/toasts throughout and reduced-motion honoured.
- Command center → cockpit → train/test/eval/versions/deployments/lineage all
  function against the real backend (or clearly-marked stubs where a later backend
  phase is still pending).
- A user can, end-to-end and beautifully: create → train (watch live) → test →
  evaluate/compare → promote (gated) → deploy → see it used by a strategy → roll
  back — without leaving the Studio.
