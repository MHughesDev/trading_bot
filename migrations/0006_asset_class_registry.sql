-- Asset class and data type registries (Phase 1).
-- Referenced by the graph crate and strategy manifests.

CREATE TABLE IF NOT EXISTS asset_class_registry (
    asset_class_id   TEXT PRIMARY KEY,
    display_name     TEXT NOT NULL,
    description      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS data_type_registry (
    data_type_id     TEXT PRIMARY KEY,
    asset_class_id   TEXT NOT NULL REFERENCES asset_class_registry(asset_class_id),
    display_name     TEXT NOT NULL,
    description      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Seed: canonical asset classes.
INSERT INTO asset_class_registry (asset_class_id, display_name) VALUES
    ('crypto',        'Cryptocurrency'),
    ('equity',        'Equity'),
    ('futures',       'Futures'),
    ('options',       'Options'),
    ('forex',         'Foreign Exchange'),
    ('fixed_income',  'Fixed Income')
ON CONFLICT (asset_class_id) DO NOTHING;

-- Seed: canonical data types per asset class.
INSERT INTO data_type_registry (data_type_id, asset_class_id, display_name) VALUES
    ('crypto.trade',        'crypto',   'Trade'),
    ('crypto.quote',        'crypto',   'Quote'),
    ('crypto.orderbook',    'crypto',   'Order Book'),
    ('crypto.candle',       'crypto',   'Candle'),
    ('equity.trade',        'equity',   'Trade'),
    ('equity.quote',        'equity',   'Quote'),
    ('equity.orderbook',    'equity',   'Order Book'),
    ('equity.candle',       'equity',   'Candle'),
    ('futures.trade',       'futures',  'Trade'),
    ('futures.quote',       'futures',  'Quote'),
    ('futures.candle',      'futures',  'Candle'),
    ('options.trade',       'options',  'Trade'),
    ('options.quote',       'options',  'Quote'),
    ('forex.quote',         'forex',    'Quote'),
    ('forex.candle',        'forex',    'Candle'),
    ('fixed_income.quote',  'fixed_income', 'Quote')
ON CONFLICT (data_type_id) DO NOTHING;
