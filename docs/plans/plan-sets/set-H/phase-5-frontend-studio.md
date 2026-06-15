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

### ☐ H-5.1 Routing, nav entry, data layer scaffolding — M
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

### ☐ H-5.2 Command Center (`/models`) — L
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

### ☐ H-5.3 Create Wizard (`/models/create`) — L
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

### ☐ H-5.4 Cockpit shell + Overview tab (`/models/:id`) — L
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

### ☐ H-5.5 Training Console (`/models/:id/train`) — L
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

### ☐ H-5.6 Test Lab (`/models/:id/test`) — L
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

### ☐ H-5.7 Evaluations Arena (`/models/:id/evaluations`) — L
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

### ☐ H-5.8 Version Timeline + Promotion Gate + Deployments — L
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

### ☐ H-5.9 Lineage Graph (`/models/graph` + per-model) — M
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
