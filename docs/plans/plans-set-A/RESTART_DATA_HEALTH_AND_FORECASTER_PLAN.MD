# Implementation Plan — Restart Data-Health + Real Forecaster Signal

> **Status:** proposed · **Authored:** 2026-06-02 · **Branch context:** `claude/sleepy-lamport-QXLNP`
> **Scope:** synthesizes the full session investigation — pipeline mapping, the forecaster-wiring
> discovery (FB-SPEC-02), the heuristic-vs-learned philosophy decision, and the restart data-health
> problem — into one dependency-ordered roadmap.
>
> This is the single source of truth for the work. Phases are ordered so the one shared seam
> (**real, contiguous OHLC history flowing into `pipeline.step`**) is built exactly once.

---

## 0. TL;DR — the unifying insight

Two problems that looked separate are the **same fix**:

1. **The forecaster produces no real signal.** Every forecaster mode (numpy / torch / the newly-wired
   quantile artifact) is fed a **fabricated flat price series** —
   [`ohlc_arrays_from_feature_row`](../../forecaster_model/inference/stub.py) repeats the latest
   `close` for `history_len` bars because `pipeline.step` only ever receives a single
   `feature_row: dict[str, float]`, never the rolling OHLC window.

2. **Restart corrupts the data picture.** On restart the live decision rollers cold-start empty
   ([`live_service.py:349`](../../app/runtime/live_service.py)), gap detection + backfill are **off by
   default**, the watermark is **write-only** (no runtime consumer), and the bar interval (`1s`) can't
   be Kraken-backfilled.

Both are solved by **warm-starting the rollers from persisted history and threading that real OHLC
window into the shared decision path**. Build that seam once (Phase C) and both the forecaster *and*
restart-recovery start working. Everything else sequences around it.

```
A (canonical interval)  ──►  B (store catch-up)  ──►  C (real history into decisions ⭐)  ──►  D (health gate)
        │                                                       │
        └───────────────────────────────────────────────────────┴──►  E (promotion/governance)  ──►  F (validation)
                                                                                                          │
                                                                                                          └──►  G (deferred: learned policy)
```

**Start with A → B → C.** A and B are low-risk and unblock everything; **C is the high-value,
high-care keystone.**

---

## 1. Invariants & constraints that bound every change

These are non-negotiable. Any phase that violates one is wrong, no matter how convenient.

| # | Invariant | Where it lives | Consequence for this plan |
|---|-----------|----------------|---------------------------|
| I1 | **live ≡ replay** — `run_decision_tick` is the *same* code for live and backtest | [`decision_engine/run_step.py`](../../decision_engine/run_step.py) (imported by both `live_service.py` and `backtesting/replay.py`) | Phase C must feed history identically in both paths. Guard with the replay-equivalence suite. This is *the* risk in the whole plan. |
| I2 | **Immutable run binding** (FB-CAN-077) — each `DecisionRecord` carries config/logic/dataset/seed + a tamper-evident hash | [`decision_engine/decision_record.py`](../../decision_engine/decision_record.py) | **No online weight mutation.** Model changes happen only via offline training → atomic promotion (Phase E), never by mutating a served artifact in place. |
| I3 | **Kraken `/public/OHLC` serves only ~720 candles per interval** | [`orchestration/real_data_bars.py`](../../orchestration/real_data_bars.py) | Backfill (B) and training (F) must pick an interval whose 720-window covers the lookback. First run: **1h / 30d** or **4h / 120d**, never **1m / 120d**. Residual gaps beyond the window are expected and must be handled, not assumed-away. |
| I4 | **Kraken cannot serve sub-minute `/OHLC`** | `fetch_symbol_bars_*` silently routes `1s` to `/Trades` | The canonical interval (A) must be ≥ 60s for backfill to work at all. |
| I5 | **Graceful fallback over crash** | already implemented in `_build_quantile_packet` | Any new history/health path that can fail must fall back (flat-bar stub, numpy forward, RECONCILING hold) rather than killing a live tick. |

---

## 2. Design philosophy — resolved

The session opened with the real question: *"do we let the decision model run off a strategy, or do we
let IT decide? And if IT decides, what's the point of the backtester?"* The resolution that shapes this
plan:

- **The forecaster is the learned component; the policy stays heuristic (for now).** The forecaster
  turns raw bars into a calibrated distribution (edge + uncertainty). The
  [`HeuristicTargetPolicy`](../../policy_model/policy/heuristic.py) turns that distribution into a
  position via an auditable formula (`raw = edge_scale·edge/unc; churn = 0.2·pos; clamp`). This is the
  **running** decision-maker.
- **We do NOT bake strategies into opaque weights yet.** The learned MLP/RL path
  ([`PolicyNetwork.update`](../../policy_model/policy/policy_network.py) is a no-op stub;
  `MultiBranchMLPPolicy` is random-init) stays **dormant scaffolding** until the forecaster produces a
  real edge and we have offline evidence to clone against. Jumping to a learned policy now would mean
  training a black box on top of a flat-line forecaster — garbage in.
- **The backtester stays meaningful precisely because of I1.** It is not "testing a hand-coded
  strategy" — it replays the *exact same decision code* over historical bars. It is how we measure
  whether the forecaster's signal + heuristic sizing actually makes money before risking capital. The
  backtester is the validation harness for the learned forecaster, not a relic of a strategy-based
  design.
- **The path to a learned policy is deferred, not abandoned** — see **Phase G**. Order:
  forecaster-produces-edge (C–F) → behavior-clone the heuristic
  ([`policy_model/training/behavior_cloning.py`](../../policy_model/training/behavior_cloning.py)) →
  actor-critic curriculum ([`actor_critic.py`](../../policy_model/training/actor_critic.py)). The
  current RL "runs" are `ppo_placeholder`/`sac_placeholder` (heuristic rollouts), so there is no real
  learned policy to promote today.

**One-line doctrine:** *Learn the forecast, keep the sizing auditable, validate via replay, and only
graduate to a learned policy once there's a real edge to clone.*

---

## 3. Current state — real vs. scaffolding

| Component | Representation | Reality | Wired to serving? |
|-----------|----------------|---------|-------------------|
| Forecaster | `QuantileForecasterArtifact` (sklearn `QuantileRegressor`, joblib) | **Real**, trained on Kraken OHLCV walk-forward | ✅ **as of this session** (FB-SPEC-02) via `NM_MODELS_FORECASTER_QUANTILE_PATH` |
| Forecaster | `ForecasterWeightBundle` (NPZ xLSTM) | ❌ only producer is `capture_forecaster_weights_from_seed` = `rng.normal()` (noise) | wired, but noise |
| Forecaster | Torch net | ❌ `train_forecaster_torch` is a "minimal/future" stub; no real checkpoint | wired, but empty |
| Policy | `HeuristicTargetPolicy` | **Real & running** (the formula above) | ✅ default in `PolicySystem.decide` |
| Policy | `PolicyNetwork` / `MultiBranchMLPPolicy` (MLP) | ❌ `update()` no-op; random-init weights | dormant |
| Risk | rule-based engine | **Real & running** | ✅ |

**Serving precedence (implemented this session):** `quantile_ohlc_v1` > torch > NPZ bundle > RNG.
Geometry (`feature_windows`, `history_length`, `num_regime_dims`) is read from
`artifact.config_snapshot` at predict time, so a train/serve config drift raises a catchable exception
→ falls back to the numpy path instead of crashing the tick.

**The catch (Gap 2):** all of the above is moot until Phase C, because every mode currently sees a flat
line.

---

## 4. ✅ Already done this session (FB-SPEC-02, partial)

Real-data quantile forecaster wired into the shared serving path. Necessary but **not yet useful** —
blocked on Phase C (flat-bar input).

- `app/config/settings.py:170` — `models_forecaster_quantile_path: str | None = None` + YAML parser
  (`models.forecaster_quantile_path`, settings.py:425).
- `.env.example:130` — `NM_MODELS_FORECASTER_QUANTILE_PATH` documented.
- [`decision_engine/pipeline.py`](../../decision_engine/pipeline.py) — full wiring:
  `_global_model_paths_configured` (:83), `_ResolvedPaths.forecaster_quantile_path` (:103), resolver
  merge (:132/:146), cache key (:209), serving-mode log (:269),
  `_load_quantile_forecaster_if_configured` (:507), `_build_quantile_packet`, and the `step()` serving
  branch that prefers the quantile packet with graceful fallback.
- [`tests/test_quantile_forecaster_serving.py`](../../tests/test_quantile_forecaster_serving.py) — 2
  tests (loaded+served; absent→numpy fallback). Green. `ruff` clean. 22 existing
  pipeline/forecaster/settings tests still pass.
- Memory: [`forecaster-serving-reality.md`](../../.claude/...) records the wiring + the two gaps.

---

## 5. Phases

Each phase: **why → changes (with file:line) → acceptance → tests/gates → deps → risk.**

---

### Phase A — Canonical bar interval (foundation)

**Why:** `market_data_bar_interval_seconds = 1` ([settings.py:70](../../app/config/settings.py)) and
`training_data_granularity_seconds = 1` (settings.py:71) can't be Kraken-backfilled (I4) and don't
match anything trainable. Everything downstream assumes **one servable interval** shared by live bars,
backfill, and training.

**Changes**
- Default `market_data_bar_interval_seconds` and `training_data_granularity_seconds` → **60** (settings.py:70–71
  + `infra/.../default.yaml` `market_data.bar_interval_seconds` / `training_granularity_seconds`,
  parsed at settings.py:254/256).
- **Interval-coherence sweep** — every interval knob must agree (or be a documented multiple):
  - `.env.example:22–23` (`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`, `NM_TRAINING_DATA_GRANULARITY_SECONDS`)
  - `.env.example:32` microservice `NM_FEATURE_SERVICE_BAR_INTERVAL_SECONDS=1`
  - `NM_ASSET_INIT_BOOTSTRAP_GRANULARITY_SECONDS=60` (.env.example:106 — already 60, confirm it stays the anchor)
- Optional: keep a *separate* sub-second microstructure roller if tick features are ever wanted, but the
  **decision/forecaster** roller uses the canonical interval. (Defer unless needed.)

**Acceptance**
- Backfill of the canonical interval returns OHLC (not forced to `/Trades`).
- Serving bar interval == the interval baked into the artifact's `config_snapshot`.
- A single grep shows no remaining `= 1` interval default in the decision path.

**Tests/gates**
- New `tests/test_settings_interval_coherence.py`: asserts the decision-path interval defaults are ≥ 60
  and mutually consistent.
- Existing settings/env-precedence tests stay green.

**Deps:** none. **Risk:** LOW. Existing QuestDB `canonical_bars` rows are keyed by `interval_seconds`
(`max_canonical_bar_timestamp(symbol, interval_seconds)`), so old 1s rows are inert — stop writing 1s,
start writing 60s; gap detection keys on the new interval.

---

### Phase B — Persistent-store restart catch-up (Data-Health Stage 1)

**Why:** the backfill pipeline **already exists** but is off by default, and the watermark is write-only.
Turn on what's there, harden the seams.

**Changes**
- Default `questdb_startup_gap_detection` (settings.py:135) and `questdb_startup_kraken_backfill`
  (settings.py:137) → **True** (and the `default.yaml` `questdb.*` keys, settings.py:336/338). Keep
  `questdb_persist_canonical_bars` honest — backfill is meaningless if we don't persist live bars.
- **Watermark on every persisted bar, not just backfill.** Today
  [`write_canonical_through`](../../app/runtime/canonical_bar_watermark.py) is only called from backfill.
  Add a call in the live persist block ([`live_service.py:399`](../../app/runtime/live_service.py), the
  `questdb_persist_canonical_bars` path) so the sidecar always reflects the latest persisted bucket.
- **Consume the watermark at startup.** [`read_canonical_through`](../../app/runtime/canonical_bar_watermark.py)
  currently has no runtime consumer. Read it during startup gap detection
  ([`live_service.py:253`](../../app/runtime/live_service.py)) to bound the dark period:
  `gap = [watermark+interval … last_closed_bucket]`.
- **Re-detect residual gaps after backfill.** After
  [`run_startup_canonical_backfill`](../../orchestration/startup_canonical_backfill.py)
  (live_service.py:271), re-run `detect_canonical_bar_gaps` and **log + record** anything Kraken
  couldn't fill (720-cap per I3, delistings). These feed Phase D.

**Acceptance**
- Kill the app for > 1 interval, restart → the QuestDB gap is closed, or the residual is explicitly
  logged with bounds.
- The watermark sidecar advances during normal live operation (not just after a backfill).
- Startup logs "canonical_through=<ts>, gap=<n bars>, backfilled=<m>, residual=<k>".

**Tests/gates**
- `tests/test_startup_watermark_consumed.py`: seed a watermark + a fake QuestDB max(ts), assert the
  computed gap window is `[watermark+interval, last_closed]`.
- `tests/test_live_persist_writes_watermark.py`: persisting a closed bar updates the sidecar.
- Existing `startup_gap_detection` / `startup_canonical_backfill` tests stay green.

**Deps:** A. **Risk:** LOW–MED (enabling network backfill on startup; bounded by
`questdb_backfill_max_lookback_days`, settings.py:139).

---

### Phase C — Real OHLC history into decisions ⭐ KEYSTONE (forecaster Gap 2 + Data-Health Stage 2)

**Why:** this is the seam. Until the rolling OHLC window reaches `pipeline.step`, no forecaster can
produce a useful signal **and** the rollers stay cold on restart. One change closes both.

**Changes**
1. **`RollingBars.seed(rows)`** — new warm-start API on
   [`data_plane/bars/rolling.py`](../../data_plane/bars/rolling.py). Accepts a list/frame of completed
   OHLCV rows (same shape as `current_partial_row`), populates `_completed` (respecting
   `max_completed`), and sets `_bucket_start` to the last seeded bucket so the first live `on_tick`
   continues contiguously. (Today `RollingBars` has no seed path — it only grows from `on_tick`.)
2. **Warm-start at startup** — after Phase B backfill, seed each roller
   ([`live_service.py:349`](../../app/runtime/live_service.py)) from the last `history_length` closed
   bars in QuestDB (REST fallback via `fetch_symbol_bars_sync` if DB empty), contiguous up to the
   watermark.
3. **Thread `ohlc_history` through the shared path** — add an optional
   `ohlc_history: pl.DataFrame | None = None` (or `Sequence[dict]`) param to:
   - [`run_decision_tick`](../../decision_engine/run_step.py) (signature at run_step.py:39; passes it to
     `pipeline.step` at run_step.py:137)
   - [`DecisionPipeline.step`](../../decision_engine/pipeline.py) (signature at pipeline.py:329)
   - down into `build_forecast_packet_methodology` / `_build_quantile_packet`, which **use the real
     window when present** and fall back to
     [`ohlc_arrays_from_feature_row`](../../forecaster_model/inference/stub.py) only when it's absent.
4. **Populate it on both sides (I1):**
   - **Live:** `live_service` passes `roller.bars_frame_with_partial()` (or completed-only) into
     `run_decision_tick`.
   - **Replay:** [`backtesting/replay.py`](../../backtesting/replay.py) passes the trailing
     `history_length` window of the replay frame. **Same source semantics, same column order.**

**Acceptance**
- On restart, the first live tick sees **≥ `history_length` contiguous bars** (assert first live WS
  bucket == `watermark + interval`; log a warning + count if not).
- Forecaster `q_med` (and uncertainty) **vary across different windows** — i.e. the flat-line artifact
  is gone. New test asserts non-constant output across two distinct windows.
- **Replay-equivalence suite passes** for identical windows (live and replay produce byte-identical
  `DecisionRecord`s given the same history).

**Tests/gates**
- `tests/test_rolling_bars_seed.py`: seed N rows → `bars_frame_with_partial` returns them contiguous;
  first `on_tick` after seed continues the sequence.
- `tests/test_forecaster_varies_with_history.py`: two different real windows → different forecast
  packets (guards against silent flat-line regressions).
- **GATE:** the existing live≡replay equivalence test(s) must pass unchanged. This is the merge gate
  for the phase.

**Deps:** A, B. **Risk:** **HIGH** — mutates the shared decision contract (I1). Mitigation: identical
history source both paths; keep the stub fallback; gate on the equivalence suite; land behind nothing
else until green.

---

### Phase D — Continuity health gate (Data-Health Stage 3)

**Why:** with real history flowing, the system must *refuse to trade on bad history* rather than
silently acting on a gap-poisoned window.

**Changes**
- New `data_plane/health/data_health.py` — per-symbol `DataHealth` check: **contiguous** (interior gap
  ≤ tolerance), **deep** (≥ `history_length`), **recent** (latest bar ≤ staleness threshold). Built from
  the seeded roller + residual-gap output of Phase B.
- Wire into risk: until `DataHealth` is clean, drive the existing **`apex_data_integrity_alert`**
  hard-override → `RECONCILING` mode (no-new-entries / reduce-only;
  `RISK_BLOCK_PAUSE_NEW_ENTRIES`) in [`risk_engine/engine.py`](../../risk_engine/engine.py).
- Observability: Prometheus gauge (`data_health_ok{symbol}`) + a dashboard health strip
  (`control_plane/`).

**Acceptance**
- Restart with an unfillable residual gap (I3) → system **holds** (no new entries) with a reason code
  and the gauge reads 0; once a clean contiguous window exists, it resumes and the gauge reads 1.
- The gate flips **both** ways under test.

**Tests/gates**
- `tests/test_data_health_gate.py`: gappy window → RECONCILING + reason code; clean window → trading
  resumes.
- Risk-precedence tests stay green (the override must sit at the right precedence per
  [`docs/RISK_PRECEDENCE.MD`](../RISK_PRECEDENCE.MD)).

**Deps:** C. **Risk:** MED (touches risk precedence — keep it a *hard pre-trade gate*, not a sizing
tweak).

---

### Phase E — Forecaster promotion + governance + cleanup (P1–P4)

**Why:** training already writes a champion joblib and a *decision*, but **nothing acts on the decision
to update the served path.** Close the loop, bind per-asset, and tidy layering.

**Changes**
- **P1 — make promotion real.** Today
  [`decide_forecaster_promotion_stub`](../../orchestration/promotion.py) (promotion.py:45) only returns
  a `PromotionDecision` + writes `promotion_decision.json`; the campaign calls it at
  [`training_campaign.py:295`](../../orchestration/training_campaign.py) but **no file is copied to the
  served path.** Add a promotion *effect*: on `decision == "promote"`, atomically copy/symlink the
  champion joblib to the served location (or update the active-set manifest), **gated by** the existing
  [`evaluate_promotion_gates`](../../orchestration/release_gating.py) (release_gating.py:154),
  `validate_rollback_for_promotion` (:73), and
  [`post_release_probation`](../../orchestration/post_release_probation.py). Honor I2 — promotion is an
  atomic file swap, never an in-place mutation of a live artifact.
  - Operational tie-in: the nightly scheduler (FB-AP-035/036,
    `NM_SCHEDULER_NIGHTLY_PER_ASSET_FORECASTER`) becomes the auto-retrain→gate→promote loop once this
    lands.
- **P2 — per-asset manifest binding.** `forecaster_quantile_path` is in global settings but **not** in
  the per-asset manifest. Add it to:
  - [`AssetModelManifest`](../../app/contracts/asset_model_manifest.py) (currently has
    `forecaster_weights_path` / `forecaster_torch_path`, not quantile)
  - the manifest-only resolver branch in `pipeline.py` (the `_paths_from_manifest`-style function near
    pipeline.py:110 sets torch/weights but not quantile; the *merge* branch at pipeline.py:146 already
    falls back via `getattr`)
  - **`models/registry/active_set.py` `_MANIFEST_MODEL_KEYS`** (active_set.py:20) — currently
    `(forecaster_checkpoint_id, forecaster_conformal_state_path, forecaster_weights_path,
    policy_mlp_path)`; it's **missing both `forecaster_torch_path` and `forecaster_quantile_path`**, so
    the FB-SPEC-06 active-set manifest can't bind either. Add both.
- **P3 — layering fix.** Serving now imports from `forecaster_model/training/real_data_fit.py`
  (`QuantileForecasterArtifact.load`, `predict_quantile_forecast_packet`). Move the **inference half**
  to `forecaster_model/inference/quantile_infer.py`; serving should not import from `training/`.
  (sklearn-on-serving is unavoidable — it's needed to unpickle — but the *module boundary* should be
  clean.)
- **P4 — doc update.** The [`training_campaign.py`](../../orchestration/training_campaign.py) header
  ("offline ≠ live until FB-SPEC-02 wires checkpoints") is **partially stale** after this session.
  Update it to: serving wiring done (FB-SPEC-02); real-equivalence pending Phase C.

**Acceptance**
- A training run that beats the champion **auto-promotes** behind the gates; the served path updates
  atomically; a failed gate keeps the champion.
- Multi-symbol + manifest mode binds `forecaster_quantile_path` per asset (env, per-asset manifest, and
  active-set manifest all resolve it).
- Serving imports no longer reach into `training/`.

**Tests/gates**
- Extend `tests/test_promotion_decision.py` with a "promote → served path updated atomically; gate fail
  → unchanged" case.
- Extend `tests/test_pipeline_asset_binding.py` + `tests/test_active_model_set.py` for the quantile key.
- Import-linter / a small test asserting `decision_engine` doesn't import `forecaster_model.training`.

**Deps:** A, C. **Risk:** MED (promotion touches the served artifact — guard with gates + atomic swap +
rollback validation).

---

### Phase F — Validation campaign (close the loop)

**Why:** prove the forecaster actually produces an edge before trusting it with sizing.

**Changes / runbook**
1. Run [`orchestration/training_campaign.py`](../../orchestration/training_campaign.py) at the canonical
   interval, **1h bars / 30-day lookback first** (per I3), then 4h/120d.
2. Point `NM_MODELS_FORECASTER_QUANTILE_PATH` at the champion artifact (or let P1 auto-promote it).
3. Run [`backtesting/replay.py`](../../backtesting/replay.py) with warm-started bars + real history →
   read PnL / Sharpe / pinball loss.

**Decision rule**
- **Edge present** → shadow → probation → promote (Phase E gates).
- **No edge** → iterate **features, not capacity.** Do **not** jump to the torch net or a learned
  policy. Add/repair features, re-fit the quantile model, re-validate. Capacity without signal just
  overfits.

**Acceptance**
- First **real** PnL/Sharpe numbers exist and are reproducible (live≡replay, I1).
- The "offline ≠ live" disclaimer (P4) is fully retired once replay and live agree on a sample window.

**Deps:** A–E (especially C). **Risk:** LOW (measurement, not mutation).

---

### Phase G — Learned policy curriculum (DEFERRED)

**Why deferred:** there is no real edge to clone until F shows one, and a learned policy on a flat-line
forecaster is garbage-in. Captured here so it isn't lost.

**Path (in order, post-F):**
1. **Behavior-clone the heuristic** —
   [`policy_model/training/behavior_cloning.py`](../../policy_model/training/behavior_cloning.py) learns
   to imitate `HeuristicTargetPolicy` over real decision traces. This gives a *trained* MLP that matches
   today's auditable behavior (safe starting point).
2. **Actor-critic curriculum** — [`actor_critic.py`](../../policy_model/training/actor_critic.py) /
   `policy_model/training/protocol.py`, replacing the `ppo_placeholder`/`sac_placeholder` heuristic
   rollouts with real RL.
3. **Implement `PolicyNetwork.update`** ([`policy_network.py`](../../policy_model/policy/policy_network.py),
   currently a no-op returning `{"loss": 0.0}`) and add a promotion gate analogous to Phase E for the
   policy component.

**Hard constraint:** still **offline** training → atomic promotion (I2). No online learning in the live
loop — it would break the immutable run binding.

---

## 6. Cross-cutting checklists

### Interval-coherence checklist (Phase A, but re-verify after each phase)
- [ ] `market_data_bar_interval_seconds` (settings.py:70)
- [ ] `training_data_granularity_seconds` (settings.py:71)
- [ ] `default.yaml` `market_data.bar_interval_seconds` / `training_granularity_seconds`
- [ ] `NM_FEATURE_SERVICE_BAR_INTERVAL_SECONDS` (.env.example:32)
- [ ] `NM_ASSET_INIT_BOOTSTRAP_GRANULARITY_SECONDS` (.env.example:106)
- [ ] artifact `config_snapshot` interval (must match serving)

### Promotion ↔ nightly tie-in (Phase E)
- [ ] `NM_SCHEDULER_NIGHTLY_ENABLED` + `NM_SCHEDULER_NIGHTLY_PER_ASSET_FORECASTER` exercise P1's
      promote-effect end to end.

---

## 7. Risk register

| Risk | Phase | Severity | Mitigation |
|------|-------|----------|------------|
| **live≡replay drift** — silently corrupts all backtest results | C | **High** | Identical history source both paths; equivalence suite is the merge gate; keep stub fallback. |
| Kraken 720-candle cap under-covers long/fine lookbacks | A/B/F | Med | Pick a servable interval (≥60s); handle residual gaps explicitly (B→D); 1h/30d first. |
| Mixed-interval QuestDB rows after the 1s→60s switch | A | Low | Rows keyed by `interval_seconds`; stop writing old, leave old rows inert. |
| Startup backfill makes network calls on boot | B | Low–Med | Bounded by `questdb_backfill_max_lookback_days`; failure falls back to RECONCILING (D), not a crash. |
| Promotion overwrites a good champion | E | Med | Atomic swap + `evaluate_promotion_gates` + `validate_rollback_for_promotion` + probation. |
| Health gate mis-precedence (sizing vs hard block) | D | Med | Implement as a hard pre-trade override per `docs/RISK_PRECEDENCE.MD`; test both directions. |
| sklearn import on serving path | E | Low | Unavoidable for unpickling; fix the *module boundary* (P3), not the dependency. |

---

## 8. Test & gate matrix

| Phase | New tests | Gate (must stay green) |
|-------|-----------|------------------------|
| A | `test_settings_interval_coherence` | settings / env-precedence suite |
| B | `test_startup_watermark_consumed`, `test_live_persist_writes_watermark` | `startup_gap_detection`, `startup_canonical_backfill` suites |
| C | `test_rolling_bars_seed`, `test_forecaster_varies_with_history` | **live≡replay equivalence suite** (the keystone gate) |
| D | `test_data_health_gate` | risk-precedence suite |
| E | extend `test_promotion_decision`, `test_pipeline_asset_binding`, `test_active_model_set`; import-boundary test | full pipeline/forecaster suites |
| F | replay produces reproducible PnL/Sharpe | live≡replay on a sample window |
| G | behavior-cloning fidelity vs heuristic | (deferred) |

---

## 9. Recommended execution order & first-PR boundary

1. **PR 1 — Phase A** (interval + coherence). Small, low-risk, unblocks everything.
2. **PR 2 — Phase B** (enable + harden restart catch-up). Low-risk, independently shippable.
3. **PR 3 — Phase C** (the keystone). Land alone, behind the equivalence gate. **Do not** bundle other
   phases into this PR.
4. **PR 4 — Phase D** (health gate).
5. **PR 5 — Phase E** (promotion + binding + layering + doc).
6. **Phase F** — operational run, not a code PR (or a small validation-harness PR).
7. **Phase G** — separate epic, post-F.

**If you only do three things:** A → B → C. That alone makes both the forecaster and restart-recovery
real.

---

## 10. Open decisions (none blocking; defaults chosen)

- **Canonical interval value** — plan assumes **60s**. If you want finer microstructure features later,
  add a *second* roller rather than dropping the canonical interval below Kraken's backfill floor.
- **History window shape into `pipeline.step`** — completed-only vs completed+partial. Default:
  **completed-only** for determinism (replay has no "partial"); the partial bucket stays a live-only
  feature concern.
- **Promotion target** — copy-to-served-path vs update active-set manifest. Default: **active-set
  manifest** (FB-SPEC-06 path) so per-asset binding and audit stay in one place.
