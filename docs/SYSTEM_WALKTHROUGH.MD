# System walkthrough (AI / operator summary)

This document gives a **step-by-step** picture of how NautilusMonster runs end-to-end. It reflects the **master system pipeline** ([`MASTER_SYSTEM_PIPELINE_SPEC.MD`](Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD)): **`ForecastPacket`** from **`forecaster_model`** (VSN → CNN → multi-resolution xLSTM → fusion → quantiles) + **`PolicySystem`**, **Coinbase-only market data**, and **`execution_mode = paper`** (Alpaca paper execution).

---

## 1. Concepts that apply everywhere

1. **Market data** always comes from **Coinbase** (WebSocket / REST in the live loop; historical bars in replay).
2. **One shared decision step** — `run_decision_tick` in `decision_engine/run_step.py` — is used by **live** and **backtest** so behavior stays aligned.
3. **Decision path** — `DecisionPipeline.step` (see `decision_engine/pipeline.py`):
   - Build OHLC history from the latest feature row and run **`build_forecast_packet_methodology`** (full **`ForecasterModel`** / xLSTM reference forward).
   - Derive **`RegimeOutput`** for observability from the packet’s soft regime vector.
   - Run **`run_spec_policy_step`**: map packet → **`ForecastOutput`** for metrics, build **portfolio / execution / policy risk** state, call **`PolicySystem.decide`**, turn **`ExecutionPlan`** → **`ActionProposal`**.
4. **Risk is final** — `RiskEngine.evaluate` approves or blocks the proposal; only then does execution see an `OrderIntent` (signing path per settings).
5. **Execution venue** is chosen by **`execution.mode`** (`paper` | `live`): **Alpaca** for paper, **Coinbase** for live (`execution/router.py`). This does **not** change the decision math above.

---

## 2. Live trading (real-time loop)

**Entry:** `python -m app.runtime.live_service` → `run_live_loop` in `app/runtime/live_service.py`.

**Step-by-step:**

1. Load **`AppSettings`** (YAML + `NM_*` env).
2. Open **Coinbase WebSocket** for configured symbols; create **`DecisionPipeline`**, **`RiskEngine`**, **`ExecutionService`** (adapter from `execution_mode`).
3. Optionally: **QuestDB** for decision traces, **Qdrant** + memory retrieval, **RSS/news** sentiment refresh, **product metadata** cache for tradability.
4. For each normalized WS message:
   - Update **rolling minute bars** and build a **feature row** (tick overlay + bar features via `enrich_bars_last_row`).
   - **`run_decision_tick`** with `mid_price`, `position_signed_qty`, **`portfolio_equity_usd=risk_engine.current_equity`**, spread, timestamps, tradability.
5. Inside **`run_decision_tick`**: `pipeline.step` → **`spec_policy`** path → **`PolicySystem`** → proposal; then **`risk_engine.evaluate`** → optional **`TradeAction`**.
6. Log / persist **decision trace**; if the risk engine approved a trade, **`ExecutionService.submit_order`** → **Alpaca** (paper) or **Coinbase** (live).
7. Update **in-memory positions** after a successful submit; optional **position reconcile** task when `execution_mode == paper` and reconcile is enabled.

**Takeaway:** Live is **async**, **streaming**, same master pipeline as replay; **venue** is paper vs live from config only.

---

## 3. Paper trading (as configured in this repo)

**Paper** here means **`execution_mode: paper`** (default in `app/config`), which routes orders to the **Alpaca paper** adapter — **not** a separate code path for decisions.

- Same **`run_live_loop`** and **`run_decision_tick`** as live.
- You still consume **Coinbase** prices/bars for signals.
- Set **`NM_ALPACA_API_KEY`** / **`NM_ALPACA_API_SECRET`** for Alpaca; optional **`NM_POSITION_RECONCILE_ENABLED=true`** to align positions with the broker on an interval.

**Takeaway:** “Paper” is **execution venue + credentials**, not a different forecast/policy stack.

---

## 4. Live execution on Coinbase (real money path)

- Set **`execution.mode`** to **`live`** (and **`NM_EXECUTION_LIVE_ADAPTER=coinbase`** per defaults).
- **`create_execution_adapter`** returns **`CoinbaseExecutionAdapter`**; the loop is unchanged — still Coinbase data, still **`spec_policy`** decisions, still **`RiskEngine`** gating.
- Ensure **signing / risk** settings match production policy (e.g. **`NM_RISK_SIGNING_SECRET`**, never **`NM_ALLOW_UNSIGNED_EXECUTION`** in production).

**Takeaway:** Switching paper → live only changes **where orders go** after the same risk-approved intent.

---

## 5. Backtesting / replay

**Entry:** `backtesting/replay.py` — **`replay_decisions`** (and **`replay_multi_asset_decisions`** for multi-symbol).

**Step-by-step:**

1. Provide a **Polars** OHLCV frame sorted by time; optional **`FeaturePipeline`** and starting position.
2. For each bar (cumulative window): **`enrich_bars_last_row`** → feature dict (same enrichment idea as live).
3. **`run_decision_tick`** with bar **close** as **`mid_price`**, same **`pipeline`** and **`risk_engine`** as live.
4. If **`track_portfolio=True`**: load **`BacktestExecutionParams`** from settings, simulate **slippage / fees**, update **`PortfolioTracker`**; optionally pass **`available_cash_usd`** and **`portfolio_equity_usd`** into the tick when settings enable replay cash gating.

**Takeaway:** Replay is **offline** and **bar-driven**, but **one decision+risk function** — it runs the same **`ForecastPacket` + `PolicySystem`** logic as live.

---

## 6. Quick config reference (defaults described above)

| Topic | Typical setting |
|--------|------------------|
| Forecaster lineage label | `NM_MODELS_FORECASTER_CHECKPOINT_ID` (optional) |
| Conformal JSON on hot path | `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` (optional) |
| Execution venue | `NM_EXECUTION_MODE=paper` \| `live` |

For deeper detail, see [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](MIGRATION_TO_SPEC_PIPELINE.MD), [`docs/Specs/SYSTEM_OVERVIEW.MD`](Specs/SYSTEM_OVERVIEW.MD), and [`docs/Specs/FORECASTER_AND_POLICY.MD`](Specs/FORECASTER_AND_POLICY.MD).
