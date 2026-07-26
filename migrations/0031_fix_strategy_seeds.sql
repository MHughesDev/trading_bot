-- Fix the seeded ema_crossover strategy.
--
-- The original seed (0015) had two problems:
--   1. Only an entry (buy) signal — no exit (sell) signal, so the strategy
--      accumulated a long position forever and never produced closed trades.
--   2. size_mode "percent_of_balance" which the backtest engine rejects
--      (only "fixed" is supported in v1.0).
--
-- This migration replaces it with a proper crossover definition:
--   • Buy 0.1 BTC when fast EMA (7) crosses above slow EMA (21).
--   • Sell 0.1 BTC when fast EMA (7) crosses below slow EMA (21).
-- Both signals fire only on the rising edge (transition from inactive to
-- active), matching the live strategy-runtime semantics.

UPDATE strategy_definitions
SET definition_json = $json${
  "strategy_id": "ema_crossover",
  "definition_version": "1.0",
  "asset_class": "crypto_spot_cex",
  "inputs": [
    { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
    { "lane": "features.technical", "instrument": "$bound_at_init",
      "features": ["ema_7", "ema_21"] }
  ],
  "nodes": [
    { "id": "c_bull", "type": "condition",
      "expr": "feature('ema_7') > feature('ema_21')" },
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
WHERE strategy_id = 'ema_crossover';
