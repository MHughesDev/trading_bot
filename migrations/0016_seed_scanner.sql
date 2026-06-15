-- Seed built-in Discovery (scanner) strategies.
-- These have actions: [] so the runtime infers StrategyKind::Discovery.
-- ON CONFLICT DO NOTHING preserves any user edits.

INSERT INTO strategy_definitions (strategy_id, definition_version, asset_class, definition_json)
VALUES (
  'ema_crossover_scanner',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "ema_crossover_scanner",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["ema_7", "ema_21"] }
    ],
    "nodes": [
      { "id": "c1", "type": "condition",
        "expr": "feature('ema_7') > feature('ema_21')" },
      { "id": "s1", "type": "signal", "when": "c1", "emit": "scanner_signal" }
    ],
    "actions": []
  }$json$::jsonb
),
(
  'rsi_overbought_scanner',
  '1.0',
  'crypto_spot_cex',
  $json${
    "strategy_id": "rsi_overbought_scanner",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
      { "lane": "market.bars.1m",     "instrument": "$bound_at_init" },
      { "lane": "features.technical", "instrument": "$bound_at_init",
        "features": ["rsi_14"] }
    ],
    "nodes": [
      { "id": "c1", "type": "condition",
        "expr": "feature('rsi_14') > 70" },
      { "id": "s1", "type": "signal", "when": "c1", "emit": "scanner_signal" }
    ],
    "actions": []
  }$json$::jsonb
)
ON CONFLICT (strategy_id) DO NOTHING;
