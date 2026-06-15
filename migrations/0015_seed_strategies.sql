-- Seed default strategy definitions so every user (new and existing) has
-- a working starting point in the Strategy Builder.
-- ON CONFLICT DO NOTHING means user-modified versions are never overwritten.

INSERT INTO strategy_definitions (strategy_id, definition_version, asset_class, definition_json)
VALUES (
  'ema_crossover',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "ema_crossover",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",       "instrument": "$bound_at_init" },
      { "lane": "features.technical",   "instrument": "$bound_at_init",
        "features": ["ema_7", "ema_21"] }
    ],
    "nodes": [
      { "id": "c1", "type": "condition",
        "expr": "feature('ema_7') > feature('ema_21')" },
      { "id": "s1", "type": "signal", "when": "c1", "emit": "entry" }
    ],
    "actions": [
      {
        "on_signal": "entry",
        "type": "place_order",
        "order": { "side": "buy", "size_mode": "percent_of_balance", "size": "0.02" }
      }
    ]
  }$json$::jsonb
),
(
  'rsi_oversold_bounce',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "rsi_oversold_bounce",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["rsi_14"] }
    ],
    "nodes": [
      { "id": "c1", "type": "condition",
        "expr": "feature('rsi_14') < 30" },
      { "id": "s1", "type": "signal", "when": "c1", "emit": "entry" }
    ],
    "actions": [
      {
        "on_signal": "entry",
        "type": "place_order",
        "order": { "side": "buy", "size_mode": "percent_of_balance", "size": "0.01" }
      }
    ]
  }$json$::jsonb
),
(
  'ema_trend_sell',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "ema_trend_sell",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["ema_7", "ema_21"] }
    ],
    "nodes": [
      { "id": "c1", "type": "condition",
        "expr": "feature('ema_7') < feature('ema_21')" },
      { "id": "s1", "type": "signal", "when": "c1", "emit": "entry" }
    ],
    "actions": [
      {
        "on_signal": "entry",
        "type": "place_order",
        "order": { "side": "sell", "size_mode": "percent_of_balance", "size": "0.02" }
      }
    ]
  }$json$::jsonb
)
ON CONFLICT (strategy_id) DO NOTHING;
