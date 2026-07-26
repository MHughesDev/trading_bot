-- Three additional seed strategies using fixed sizing (sim-compatible).
-- ON CONFLICT (strategy_id) DO NOTHING so user-edited copies are preserved.

INSERT INTO strategy_definitions (strategy_id, definition_version, asset_class, definition_json)
VALUES

-- ── 1. RSI Mean Reversion ────────────────────────────────────────────────────
-- Buys each new oversold dip (RSI < 30) and exits each overbought spike
-- (RSI > 70).  Rising-edge semantics mean each fresh dip/spike fires exactly
-- once per crossing, so positions don't stack on a sustained extreme.
(
  'rsi_mean_reversion',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "rsi_mean_reversion",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["rsi_14"] }
    ],
    "nodes": [
      { "id": "c_oversold",   "type": "condition",
        "expr": "feature('rsi_14') < 30" },
      { "id": "s_buy",        "type": "signal", "when": "c_oversold",   "emit": "go_long" },
      { "id": "c_overbought", "type": "condition",
        "expr": "feature('rsi_14') > 70" },
      { "id": "s_sell",       "type": "signal", "when": "c_overbought", "emit": "go_flat" }
    ],
    "actions": [
      { "on_signal": "go_long", "type": "place_order",
        "order": { "side": "buy",  "size_mode": "fixed", "size": "0.1" } },
      { "on_signal": "go_flat", "type": "place_order",
        "order": { "side": "sell", "size_mode": "fixed", "size": "0.1" } }
    ]
  }$json$::jsonb
),

-- ── 2. EMA Trend + RSI Filter ────────────────────────────────────────────────
-- Enters long when EMA(7) > EMA(21) AND RSI is below 70 (trend confirmed,
-- not yet overbought).  Exits on the EMA cross down regardless of RSI.
-- The AND is expressed as a product of two 0/1 sub-comparisons > 0.
(
  'ema_rsi_trend',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "ema_rsi_trend",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["ema_7", "ema_21", "rsi_14"] }
    ],
    "nodes": [
      { "id": "c_bull", "type": "condition",
        "expr": "(feature('ema_7') > feature('ema_21')) * (feature('rsi_14') < 70) > 0" },
      { "id": "s_long", "type": "signal", "when": "c_bull", "emit": "go_long" },
      { "id": "c_bear", "type": "condition",
        "expr": "feature('ema_7') < feature('ema_21')" },
      { "id": "s_flat", "type": "signal", "when": "c_bear", "emit": "go_flat" }
    ],
    "actions": [
      { "on_signal": "go_long", "type": "place_order",
        "order": { "side": "buy",  "size_mode": "fixed", "size": "0.1" } },
      { "on_signal": "go_flat", "type": "place_order",
        "order": { "side": "sell", "size_mode": "fixed", "size": "0.1" } }
    ]
  }$json$::jsonb
),

-- ── 3. Triple EMA ────────────────────────────────────────────────────────────
-- Requires all three EMAs stacked in the same direction before trading.
-- Long entry: EMA(7) > EMA(21) > EMA(50) — short, medium, and long all bullish.
-- Exit: EMA(7) < EMA(21) AND EMA(21) < EMA(50) — all three stacked bearish.
-- Fewer signals than a simple crossover; each one carries stronger confirmation.
-- Warm-up: EMA(50) needs ~250 bars (50 × 5) before the first tradable bar.
(
  'triple_ema',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "triple_ema",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["ema_7", "ema_21", "ema_50"] }
    ],
    "nodes": [
      { "id": "c_bull", "type": "condition",
        "expr": "(feature('ema_7') > feature('ema_21')) * (feature('ema_21') > feature('ema_50')) > 0" },
      { "id": "s_long", "type": "signal", "when": "c_bull", "emit": "go_long" },
      { "id": "c_bear", "type": "condition",
        "expr": "(feature('ema_7') < feature('ema_21')) * (feature('ema_21') < feature('ema_50')) > 0" },
      { "id": "s_flat", "type": "signal", "when": "c_bear", "emit": "go_flat" }
    ],
    "actions": [
      { "on_signal": "go_long", "type": "place_order",
        "order": { "side": "buy",  "size_mode": "fixed", "size": "0.1" } },
      { "on_signal": "go_flat", "type": "place_order",
        "order": { "side": "sell", "size_mode": "fixed", "size": "0.1" } }
    ]
  }$json$::jsonb
)

ON CONFLICT (strategy_id) DO NOTHING;
