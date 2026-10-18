# Phase 5 — The model studio frontend (Set H Phase 5, re-cut)

**Completion: 0% (0 / 9 tasks), plus 9 `PARTIAL` (built and tested 2026-10-04 by R-N/FE; none is routed until the orchestrator applies the `App.tsx` lines, and the new server routes are not yet served).**

**Requirement IDs:** Set H 5.1–5.9
**Spec:** Set H phase files; Meridian design system
**Depends on:** Phase 4 (N-4.3) and N-0.2.
**Migration range:** none.

Set H's Phase 5 was written for the pre-Meridian UI. `ModelCreatePage.tsx`, `ModelDetailPage.tsx` and `ModelLineagePage.tsx` exist; the rest does not. Re-cut against the Meridian design system and
the `/mlops` suite (June 2026 consolidation) by N-0.2. Nothing here is started in this track: it needs `frontend/src/App.tsx` routing lines the orchestrator applies.

---

### N-5.1 Routing, navigation and the data layer — M
`PARTIAL` 2026-10-04 (R-N/FE) — `frontend/src/api/modelsStudio.ts` (typed client, plain TS types), `lib/models/queries.ts` (react-query hooks keyed under `['models', ...]`), `pages/ModelStudioPage.tsx` (one page = the whole route table: index, `new`, `:id/*`). Tests: `the-studio-routes-render-inside-a-plain-router` (4), `the-studio-client-calls-only-routes-the-server-serves` (2, reads the Rust `.route(...)` table segment by segment so `/api/models/studio/families` is not satisfied by `/api/models/{id}`). **Missing:** the `App.tsx` lazy import and `<Route path="/mlops/studio/*">` (orchestrator's file, so the page is built but not yet reached); the client-path test fails until `models_studio.rs` is registered in `routes/mod.rs` (it names the 9 unserved paths). No nav entry added: `NAV_SECTIONS` is outside this territory.

`/models` routes, nav entry and typed query hooks for the model endpoints.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.2 Command Center — M
`PARTIAL` 2026-10-04 (R-N/FE) — `pages/ModelCommandCenterPage.tsx` over `GET /api/models/studio/fleet`; counts, filter (all/training/stale/serving), the server's own stale reason and serving flag. Tests (`the-command-center-reads-the-fleet`, 5): skeleton while loading, empty with a create action, error with retry (not an empty fleet), populated flags, filter-to-nothing. **Missing:** not routed (App.tsx) and the fleet route is not served yet.

The fleet view: what is training, what is stale, what is serving.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.3 Create wizard — M
`PARTIAL` 2026-10-04 (R-N/FE) — `pages/ModelCreateWizardPage.tsx`, `lib/models/declaration.ts` (form to `DatasetDeclaration`; no purge/embargo field can be emitted; weights have no default). Six steps: family, universe and window, features, target and weights, validation, review. Review calls `POST /api/models/declarations/validate`, shows the derived purge/embargo/horizon/look-back/sample estimate, the cutoff clip and any lowering refusal; submit is disabled until the server accepted AND `lowering.ok`; then creates the model and `POST /train` with `declaration` + `family`. Tests (`the-wizard-derives-the-embargo-and-never-asks-for-it`, 12 + `declaration-and-verdict`, 12): derived embargo shown, no control named purge/embargo on any step (mutation-checked: adding an "Embargo (bars)" input fails it), body carries no purge/embargo key, missing weights refused before the server is asked, 422 shown against its field, lowering refusal disables submit, submit order create then train. **Missing:** not routed; the families/validate routes are not served yet; the HPO/hyperparameter step and `max_gpu_hours` input are not in the wizard (no served field to carry them).

Declares a `DatasetDeclaration` (N-1.1) step by step; shows the derived embargo, never an input for it.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.4 Cockpit shell and Overview — M
`PARTIAL` 2026-10-04 (R-N/FE) — `pages/ModelCockpitPage.tsx` (header + tabs overview/training/test/evaluations/versions/lineage, deep-linkable with `?v=`), `components/models/CockpitOverview.tsx`, `VerdictPanel.tsx`, `lib/models/useSelectedVersion.ts`. Overview: lineage summary, aliases, versions, scorecard WITH baselines and every usage rule, clustered importance (or the server's reason it is unavailable). Tests (`the-cockpit-overview-shows-baselines-and-verdicts`, 13): loading, error, tabs, aliases empty, a version with no baselines is stated, complexity flag, verdict read failure is "not cleared" not a pass, no-version empty, importance interval only when given. **Missing:** not routed; verdict and importance routes not served yet. The old `/mlops/:id` cockpit (`ModelDetailPage`) is untouched and still routed.

One page per model: lineage, versions, current alias, scorecard with baselines.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.5 Training console — L
`PARTIAL` 2026-10-04 (R-N/FE) — `components/models/TrainingConsole.tsx`: status, phase, progress, ETA, recorded loss curve, observed progress curve, best-earlier overlay, runs list, event log, cancel. Checkpoint cadence, `max_gpu_hours` and GPU hours used are read from the run's `metrics` and print "not reported" otherwise. Tests (`the-training-console-reports-only-what-the-run-reports`, 8): loading, empty, error, running run, "not reported" without the fields, values when reported, failed-run reason, cancel. **Missing:** live updates are polled (nothing consumes the `models.jobs` WebSocket lane, as the existing console already documents), so there is no push; the trainer does not yet report cadence or budget in run metrics, so today the console shows "not reported" for both; the lost-contact banner path (a poll failing after data was held) is not covered by a test.

Live progress with the checkpoint cadence and the `max_gpu_hours` budget.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.6 Test lab — M
`PARTIAL` 2026-10-04 (R-N/FE) — `components/models/TestLab.tsx`: run a version against saved cases and/or a latest-bars window (`GET /feature-vector`), each answer with latency and a PIT badge. Tests (`the-test-lab-states-the-pit-status-of-each-answer`, 9): loading, empty, error, run disabled until something is chosen, case run, window run with the feature source stated, "PIT not reported" when the answer says nothing (never "clean"), "PIT clean"/"PIT violation" only when the answer says so, a failed version is a row not a gap. **Missing:** `POST /versions/{v}/test` takes instances, not a historical as-of window, so the "window" is the most recent stored bars; and today's answer carries no PIT field, so the badge reads "not reported" until the server adds one.

Run a model version against a window and a case set; show the PIT status of the answer.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.7 Evaluations arena — L
`PARTIAL` 2026-10-04 (R-N/FE) — `components/models/EvaluationsArena.tsx` + `VerdictPanel.tsx` + `lib/models/verdict.ts`: candidate row beside baseline rows, a warning when baselines are not on identical folds, every usage rule shown, `not_computable` drawn as blocking ("Not computable: blocks") in the same weight as a fail, the headline never "Allowed" while a rule blocks even if the server said `allowed`. Tests (`the-arena-renders-a-not-computable-rule-as-blocking`, 11 + verdict helpers in `declaration-and-verdict`): mutation-checked (making `ruleBlocks` ignore `not_computable` fails 5 tests). **Missing:** not routed; verdict route not served yet; the candidate row uses the version's numeric `metrics` and prints "not reported" for any score the baselines have and it lacks (the verdict carries no candidate scores of its own).

Candidate against baselines on identical folds; the usage-rule verdicts (N-3.2) rendered, never hidden.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.8 Version timeline, promotion gate and deployments — M
`PARTIAL` 2026-10-04 (R-N/FE) — `components/models/PromotionPanel.tsx`: version timeline with aliases, gate checks, the verdict and baselines as evidence (a fail is visible), "Propose promotion" (rationale required), proposals list, Approve/Reject (note required) rendered only on `pending` proposals, a 403 shown as "only a human session can decide", deployments table. There is no direct promote control and `studioApi` has no promote method (tested). Tests (`promotion-is-a-proposal-a-human-decides`, 11). **Missing:** not routed; the evidence/proposal routes are not served yet; the page cannot tell a human session from an agent token in advance, so the decide controls show for any signed-in user and the server's 403 is the enforcement (the UI states the refusal); alias rollback and deployment creation are not offered here.

Promotion is a proposal a human decides (N-4.3); the page shows the evidence a decider needs.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system

### N-5.9 Lineage graph — M
`PARTIAL` 2026-10-04 (R-N/FE) — `lib/models/lineage.ts` (`buildHashLineage`), `components/models/LineageGraphView.tsx`, and `ModelLineagePage.tsx` extended with a Content hashes / Registry switch (content hashes is the default; the old registry graph is unchanged). Chain: dataset, features, model version, prediction series, backtest; versions naming the same hash share one node; an unrecorded link is a dashed "not recorded"/"none linked" node, never invented. Tests (`the-lineage-chain-is-keyed-by-content-hash`, 9, incl. the existing page). **Missing:** not routed in the cockpit by App.tsx (the existing `/mlops/graph` route already reaches the extended page); the registry does not serve dataset, feature-DAG, prediction-series or backtest hashes (`GET /lineage` still answers aliases only), so the chain reads the keys `dataset_id`, `feature_dag_hash`, `prediction_series_id`, `backtest_run_id` from each version's `metrics` and shows gaps where they are absent: today most links will be gaps until the registry records them.

### ADR rows (FE)

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-N-FE-01 | The studio is one page, `ModelStudioPage`, mounted at `/mlops/studio/*` and owning its own nested routes (index, `new`, `:id/*`). | One lazy page and `<Route>` per surface in `App.tsx`. | `App.tsx` is the orchestrator's file; one mount is one line of review, and a literal `studio` segment outranks the dynamic `/mlops/:id`. |
| ADR-N-FE-02 | The new Studio sits beside the existing `/mlops` pages (`MlOpsPage`, `ModelDetailPage`) and does not replace them. `ModelLineagePage` is extended, not duplicated. | Rewrite `ModelDetailPage` into the cockpit. | The existing cockpit has tests and live users of its direct promote and deployment controls; replacing it before the proposal route is served would remove a working path. |
| ADR-N-FE-03 | The Studio exposes no direct promote control; promotion is Propose, and Approve/Reject only on a `pending` proposal. `studioApi` has no promote method. | Reuse `PromotionGateModal` (direct promote with an override reason). | N-4.3: promotion is a human decision; a UI path that moves an alias without a proposal contradicts it. |
| ADR-N-FE-04 | "Propose promotion" stays available when the gate or a usage rule fails; the evidence is shown to the decider. | Disable Propose on a failing gate. | A proposal is not a promotion; blocking it hides the reasoning from the one person who decides, and the gate is re-evaluated server-side at decision time. |
| ADR-N-FE-05 | A usage rule that applies and is `not_computable` is drawn as blocking, and the headline is never "Allowed" while any rule blocks, even if the server said `allowed: true`. | Trust `allowed` alone. | ADR-N-05: an unmeasured thing is not a passed thing; the client cannot soften what the server's own rules say. |
| ADR-N-FE-06 | Decide controls show for any signed-in user on a pending proposal; the server's 403 is the enforcement and is shown as "only a human session can decide". | Hide them by guessing the credential type client-side. | The browser holds a human session by construction; a client-side guess adds a second gate that can disagree with the server's. |
| ADR-N-FE-07 | The Test Lab's "window" is the latest stored bars (feature-vector route) and says so; PIT status is read from the answer and otherwise reads "PIT not reported". | Offer an as-of date picker; default a missing PIT field to clean. | `POST /test` takes instances, not an as-of window; inventing the control or the "clean" would be a claim the server did not make. |
| ADR-N-FE-08 | The training console prints the checkpoint cadence and `max_gpu_hours` only from the run's `metrics`, otherwise "not reported". | Show the platform default or the manifest budget. | There is no platform default for `max_gpu_hours` (AT-66); an invented budget would read as a limit that is not enforced. |
| ADR-N-FE-09 | The lineage chain reads hashes from each version's `metrics` and draws unrecorded links as dashed gaps. | Draw prediction-series and backtest nodes from the strategy-used-by list. | A strategy name is not a content hash; linking by name would present an inference as a recorded fact. |
| ADR-N-FE-10 | `family.framework ?? 'sklearn'` is the framework in the created model's definition when a family declares none (statistical families). | Omit `framework`, or block creation for those families. | The existing definition validator needs a framework; `sklearn` is what the trainer routes a non-GPU family to. Revisit when the families route says otherwise. |
| ADR-N-FE-11 | The client-path test scans the Rust `.route(...)` table and matches segment by segment, with literal segments compared exactly. | Match client paths to `{param}` patterns with a regex. | A regex lets `/api/models/studio/families` pass as `/api/models/{id}`, which is the false pass this guard exists to prevent. |

Dataset to features to model to prediction series to backtest, by content hash.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the page is routed, renders from a real endpoint (never a client path the server does not serve: `every_client_path_is_served`), has loading, empty and error states, and `npm run lint` stays at 0.
**Traces:** Set H 5.x; Meridian design system
