# AI Trading Agent — Design Spec (System 1 / System 2)

**Status:** design only (no pipeline code yet). This spec is the build plan for adding an
LLM **reasoning agent** on top of the existing quantitative pipeline. Risk stays final.

---

## 1. Why this document exists

There are **two distinct "AIs"** in this repo, and they are easy to conflate:

1. **System 1 — the quant pipeline (exists, hot path).** `decision_engine/run_step.run_decision_tick`
   → `decision_engine/pipeline.DecisionPipeline.step` runs, synchronously in ~ms:
   forecaster (`forecaster_model/`, sklearn quantile in prod) → canonical orchestrator
   (regime → 3‑stage trigger → auction) → policy (`policy_model/`, NumPy MLP or heuristic) →
   `risk_engine.RiskEngine.evaluate` → signed `OrderIntent` → adapter. **No LLM. No reasoning.**
   Deterministic, auditable, every bar.

2. **System 2 — an LLM reasoning agent (this spec, new).** A model that can *research and reason*
   before acting. It does **not** replace System 1; it sits above it and is invoked **only when
   System 1 flags something worth deliberating on.**

The platform already pings on new data (`decision_engine/bar_event_trigger.publish_bar_closed` +
`BarDecisionTrigger`, flag `bar_close_decision_trigger_enabled`). Today that ping drives System 1.
This spec extends the ping to (conditionally) drive System 2.

**Latency reality:** System 1 finishes in ~1ms; the only thing that consumes the 60s minute
budget is an *optional* LLM call, and a reasoning model + a few tool calls fits comfortably in
that window (typically 5–30s). The constraint is **cost/rate**, not capability — hence gating.

---

## 2. Architecture: tiered (System 1 reflex, System 2 deliberation)

```
bar closes (t) ──push──▶ System 1 quant tick (ms) ──▶ proposal + signal flag
   ├─ no signal / low conviction ─▶ act on quant proposal (or hold). NO LLM call.
   └─ signal ─▶ ping System 2 agent ──(≤60s budget)──▶ verdict
                 • pulls extra context via MCP tools (research)
                 • returns {approve | veto | resize | set limit/stop | hold}
                 └─▶ routes through the SAME risk-gated path (/trade/order → RiskEngine → adapter)
```

- **System 1 stays primary and always-on.** It is the only thing in the per-bar hot path.
- **System 2 is event-driven and gated.** It never runs on every bar for every asset (crypto is
  24/7 ≈ 1,440 bars/day/asset; per-bar LLM calls are cost-prohibitive and non-deterministic).
- **Risk is final for both.** System 2 acts only through `execution/manual_order.submit_manual_order`
  → `RiskEngine.evaluate_manual_order` → signed intent (the same path the human UI and MCP use).

---

## 3. Push vs. pull — do both, deliberately

- **Push the *trigger* (platform → agent).** Reuse `publish_bar_closed`. When System 1 raises a
  flag, publish/enqueue an **agent-decision request**. This keeps it "platform calls the AI."
- **Push a minimal *decision packet*** in the request (baseline context — see §5), so the agent can
  decide even with zero further calls.
- **Pull *extra context* on demand.** The agent uses MCP tools (§6) to fetch only what it wants
  (recent bars, news/memory, positions, full forecast). This is what enables "do research," keeps
  prompts small, and avoids force-feeding everything every minute.

Rule of thumb: **push the trigger + baseline; pull the depth.**

---

## 4. Trigger gating — when does System 2 run?

Invoke the agent only when System 1 emits at least one **gate condition** (all tunable via
settings; default thresholds conservative):

| Gate | Source signal (System 1) | Rationale |
|---|---|---|
| Entry candidate | `TriggerOutput.trigger_valid` or strong `setup_score`/`trigger_strength` | A real entry is forming. |
| Regime change | `RegimeOutput.semantic` transition vs. previous tick | Context shift worth reasoning about. |
| Novelty / OOD | `CanonicalStateOutput.novelty` high or forecast `ood_score` high | Out-of-distribution → defer to reasoning. |
| Large asymmetry | forecast quantile skew (`q_high`/`q_low` imbalance) above threshold | Asymmetric edge worth confirming. |
| Open-position risk | drawdown / stop proximity / `data_integrity_alert` | Exit/scale-down deliberation. |

Add a **global rate limiter** (per-asset min interval between agent calls, e.g. ≥ N minutes, and a
max calls/hour budget) so a noisy market cannot spam the LLM. Cooldown after each agent action.

If no gate fires → **System 1 acts (or holds) with no LLM call.**

---

## 5. Decision-packet schema (pushed to the agent)

A compact, model-friendly JSON the agent receives on every ping (baseline context):

```jsonc
{
  "schema_version": 1,
  "symbol": "BTC-USD",
  "bar": { "ts": "2026-06-02T12:34:00Z", "interval_s": 60, "o":..,"h":..,"l":..,"c":..,"v":.. },
  "gate_reasons": ["entry_candidate", "regime_change"],
  "quant_proposal": {                 // System 1's recommendation
    "action": "buy|sell|hold|flatten",
    "size_fraction": 0.12,
    "order_type": "market|limit|stop|stop_limit",
    "confidence": 0.71
  },
  "forecast": { "horizons": [1,3,5], "q_low": [...], "q_med": [...], "q_high": [...],
                "regime": "trend_up", "ood_score": 0.18 },
  "risk_state": { "mode": "RUNNING", "drawdown_pct": 0.03, "position_fraction": 0.0,
                  "data_integrity_alert": false },
  "portfolio": { "equity_usd": 100000, "cash_usd": 88000, "position_qty": 0 }
}
```

This mirrors existing contracts (`app/contracts/forecast_packet.ForecastPacket`,
`app/contracts/risk.RiskState`, `app/contracts/decisions.ActionProposal`) — the builder is a thin
adapter, not new modelling.

---

## 6. Tools the agent pulls (MCP)

Already in `mcp_server/`: `get_bars`, `get_positions`, `get_pnl`, `get_latest_decision`,
`place_order` (now market/limit/stop/stop_limit + TIF), `flatten_position`, `set_asset_lifecycle`.

**New read tools to add for research:**
- `get_forecast(symbol)` — full current `ForecastPacket` + diagnostics.
- `get_indicators(symbol, studies[], timeframe)` — values from `charts.indicators` (reuse the
  registry just built) so the agent can inspect RSI/MACD/Bollinger/etc.
- `get_news(symbol, lookback)` — recent headlines / memory hits (`data_plane/memory/`), if/when a
  news source is wired.
- `get_decision_history(symbol, n)` — recent decision records for continuity.

The agent's **only** mutating tool remains `place_order` / `flatten_position` (risk-gated).

---

## 7. Agent output contract

The agent returns a structured verdict (validated before any action):

```jsonc
{
  "verdict": "approve | veto | resize | replace | hold",
  "size_fraction": 0.08,            // for resize/replace
  "order_type": "limit",            // optional override
  "limit_price": "64950.00",        // optional
  "stop_price": null,
  "rationale": "string (logged, not traded on)",
  "confidence": 0.66
}
```

- **approve** → submit the quant proposal unchanged.
- **veto** → suppress the quant entry (hold).
- **resize/replace** → submit with the agent's size/type/prices.
- **hold** → no order.

Every verdict routes through `RiskEngine` + signing. **No raw text → trades** (existing
`OrderIntent` metadata validator stays). The agent cannot bypass risk, change config, or exceed
caps; those are enforced server-side.

---

## 8. Model choice & thinking harness

- **System 1:** unchanged (quantile regressor + NumPy policy). Keep deterministic.
- **System 2:** a **reasoning-capable LLM with tool use** (extended thinking + the MCP tools).
  - **Tiered models:** a fast/cheaper model for routine gated pings; escalate to a stronger
    reasoning model for high-stakes gates (large size, regime break, OOD). Configurable per gate.
  - **Harness:** a bounded agentic loop — *receive packet → (optional) pull tools → reason →
    emit verdict* — with a **hard wall-clock timeout** (< 60s, default ~30s) and a **max tool-call
    budget**. On timeout/error → **fall back to System 1's proposal** (fail safe, never block the bar).
  - Prompt-cache the static context (instructions, tool schemas, symbol profile) to cut cost/latency.

---

## 9. Safety, failure modes, cost

- **Risk is final** — agent actions are just risk-gated manual orders.
- **Fail safe** — any agent timeout/exception → use the quant proposal (or hold); never stall the loop.
- **Determinism boundary** — replay/backtest stays System 1 only (no LLM in `replay_deterministic`
  paths) so backtests remain reproducible. The agent is a **live-only** overlay.
- **Cost controls** — gating + per-asset cooldown + hourly budget + tiered models. Log every call
  (gate reasons, tokens, latency, verdict, outcome) for cost/PnL attribution.
- **Auditability** — persist the decision packet, the agent verdict + rationale, and the resulting
  order alongside the existing decision record.

---

## 10. Build plan (phases)

1. **Packet + gate (no LLM):** add a `signal flag` + decision-packet builder to System 1; extend
   `BarDecisionTrigger` to enqueue an agent request when a gate fires (still no model — log only).
2. **Agent runner (shadow mode):** wire the LLM loop + new MCP read tools; run in **shadow** (emit
   verdicts, log them, do **not** trade). Measure latency, cost, and agreement with System 1.
3. **Live overlay (flagged):** allow the agent to act through `/trade/order`, behind a settings
   flag, with the rate limiter + fail-safe. Start with veto-only (can suppress, not initiate).
4. **Full verdicts:** enable resize/replace/initiate once shadow metrics justify it.

Each phase is independently shippable and reversible via settings.

---

## 11. Open questions (decide before Phase 3)

- Which gates should be **veto-only** vs. allowed to **initiate** trades?
- Per-asset vs. portfolio-level agent (does it reason about one symbol or the whole book)?
- News/sentiment source for `get_news` (none wired today; `data_plane/memory/` is the seam).
- Model tier mapping per gate, and the monthly cost ceiling.

---

### Cross-references (current code)
- Loop: `decision_engine/run_step.py`, `decision_engine/pipeline.py`
- Push trigger: `decision_engine/bar_event_trigger.py`, settings `bar_close_decision_trigger_enabled`
- Risk-gated action path: `execution/manual_order.py`, `risk_engine/engine.py`, `control_plane/api.py` (`/trade/order`)
- Agent tool surface: `mcp_server/` (`backend.py`, `tools.py`)
- Indicator values for `get_indicators`: `charts/indicators.py`
