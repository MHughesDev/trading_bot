# Implementation Summary — Human-First Platform + Decoupled AI (2026-06-02)

This pass reworked the system toward two goals: **(1) a human-first trading platform** and
**(2) a decoupled AI that the platform triggers and that acts through an MCP server** — with
**better PnL** as the north star. Everything below is additive and CI-safe (new tests green;
the only failing tests in this sandbox are pre-existing `auth_session_enabled` env artifacts).

## 1. What was already satisfied (verified, not rebuilt)

- **Charts handle no-data gracefully.** PnL panel, Plotly OHLC, and Lightweight-Charts all
  render a blank framed chart on empty/missing data (`chart_block.build_blank_figure`,
  `asset_page._render_ohlc_chart`, `charts/trading_chart`). Covered by
  `test_chart_bars_empty_table.py`, `test_chart_bars_connection_down.py`.
- **Minute bars are the atomic unit.** `market_data_bar_interval_seconds = 60`,
  `training_granularity_seconds = 60`; coherence enforced by `test_settings_interval_coherence.py`.
- **Reboot → gap-fill → stream, holding trading until data is ready.** Startup gap detection
  (`data_plane/storage/startup_gap_detection.py`), Kraken backfill
  (`orchestration/startup_canonical_backfill.py`), watermark sidecars, warm-start of rolling
  bars while the WS streams, and a data-health hard gate (`data_plane/health/data_health.py`)
  that sets `data_integrity_alert` → `RISK_BLOCK_DATA_HEALTH`. All implemented and tested.
- **Post-signup key editing** already existed (Account page + Setup wizard).

## 2. Changes implemented this pass

| Area | What | Key files |
|---|---|---|
| Backtesting (PnL) | Performance metrics (return, Sharpe/Sortino, max drawdown, CAGR, Calmar, vol) + round-trip trade journal (win rate, profit factor, expectancy) | `backtesting/metrics.py`, `backtesting/trade_ledger.py` |
| Human-first trading | `RiskEngine.evaluate_manual_order` (explicit qty, hard gates, reduce-only clamp) + `/trade/order` & `/trade/flatten` + asset-page Buy/Sell/Flatten panel | `risk_engine/engine.py`, `execution/manual_order.py`, `control_plane/api.py`, `control_plane/asset_page.py` |
| MCP server (decoupled AI action surface) | Tool registry + `PlatformBackend` HTTP client + read/act tools + import-guarded stdio server | `mcp_server/` |
| Platform → AI trigger | `market.bar.closed.v1` event + `publish_bar_closed` + `BarDecisionTrigger`; live loop emits on bar persist (flag-gated) | `decision_engine/bar_event_trigger.py`, `app/runtime/live_service.py` |
| Signup + venues | Multi-step signup flow (email→password→Alpaca→Webull, links+instructions); Webull credential storage/API; Webull on Account page | `control_plane/pages/99_Sign_up.py`, `app/runtime/user_venue_credentials.py`, `control_plane/pages/7_Account.py` |

Risk stays final: both the human `/trade/*` path and the MCP `place_order` tool route through
`RiskEngine` gates + HMAC signing before any adapter call.

## 3. Stub / pseudo-code inventory (production source)

Most are deliberate ML backlog placeholders (real PyTorch/RL training is a known multi-week
effort), not accidental gaps. Ranked by trading impact:

1. `forecaster_model/inference/stub.py:38` `build_forecast_packet_stub` — live forecasts use the
   methodology path; deep VSN/xLSTM weights are not trained.
2. `forecaster_model/training/torch_trainer.py:17,61` — placeholder/toy trainer (no real NN weights).
3. `policy_model/training/actor_critic.py`, `behavior_cloning.py` — RL trainers are stubs.
4. `policy_model/policy/policy_network.py:~26` `PolicyNetwork.update()` — no-op (policy never trains).
5. `policy_model/policy/critic.py`, `encoders.py` — random-weight value head / encoders.
6. `execution/coinbase_advanced_http.py:86` — Coinbase live supports **market only** (raises for limit/stop).
7. `orchestration/rl_real_data_eval.py:45`, `training_campaign.py:~254` — heuristic rollout placeholders for PPO/SAC.
8. `data_plane/memory/qdrant_memory.py` — placeholder 64-d embeddings (no semantic model).
9. `decision_engine/pipeline.py:82` cold-start synthetic OHLC fallback; `trigger_engine.py:99` permissive defaults.
10. `execution/adapters/alpaca_paper.py` — submits **market only** (strips limit/stop).

Note: several functions named `*_stub` (e.g. `orchestration/promotion.py:decide_forecaster_promotion_stub`) are fully implemented — the name is historical.

## 4. Forecaster / decision data-lifecycle analysis (the "3 inference passes" question)

**Finding:** there are **not** 3 redundant inference passes. The forecaster runs **one forward
pass** with five sequential stages (VSN → CNN encoder → multi-resolution xLSTM → regime-conditioned
fusion → quantile decoder). The multi-resolution branches are combined by a **regime-weighted sum**
(learned), not averaged and not re-run. The live serving model is actually
`sklearn.QuantileRegressor` per (horizon, quantile); the deep VSN/xLSTM path is currently a
random-weight stub used only as fallback.

**Recommendation (kept the design; do not add passes):**
- The specific worry ("3 passes averaged / all passed to the decision model") does **not** apply —
  no change needed there.
- The real PnL lever is **training quality on real bars**, not architecture: ensure the trained
  quantile artifact (`forecaster_model/training/real_data_fit.py`, fit on minute log-returns with
  walk-forward + conformal calibration) is the one served, and that the RNG/stub fallback is not
  silently used in production.
- `ensemble_variance` is currently diagnostic-only but is included in the 46-feature policy
  observation; if it is ~0 in production it adds noise. Worth revisiting **only** alongside a policy
  retrain (changing the observation vector breaks the fixed policy input dim, so it was intentionally
  not changed here).
- The newly-added backtest metrics + trade journal are the prerequisite to actually measure whether
  any forecasting change improves PnL.

## 5. Backtesting suite recommendation

**Keep and extend the in-house replay suite** (it has decision-path parity, determinism/provenance,
fault injection, multi-asset). The missing piece for PnL work — **aggregate metrics + a trade
journal** — was added this pass. Suggested next: parameter sweeps and walk-forward automation on top
of `orchestration/walkforward_triple.py`. Do **not** adopt backtrader/zipline/vectorbt; the in-house
replay is simpler and shares the live decision path.

## 6. Recommended next steps (out of scope for a single pass)

- **Real model training:** implement the PyTorch forecaster trainer and an RL (PPO/SAC) policy
  trainer to replace the stubs (items 1–5 above). This is the largest lever after measurement.
- **Order types:** extend the Alpaca/Coinbase adapters to honor limit/stop (currently market-only).
- **Webull execution adapter:** keys are now stored; an adapter is a separate effort (Webull's
  official API is limited/region-specific).
- **Flip the bar-close trigger on** (`bar_close_decision_trigger_enabled=true`) and run a
  `BarDecisionTrigger` to make decisions fully event-driven instead of tick-polled.
