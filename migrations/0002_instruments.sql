-- Instrument metadata.
-- tick_size and lot_size are NUMERIC — never FLOAT.

CREATE TABLE IF NOT EXISTS instruments (
    instrument_id       TEXT PRIMARY KEY,
    asset_class         TEXT NOT NULL,          -- matches domain::AssetClass snake_case
    venue_id            TEXT NOT NULL,
    base_precision      INTEGER NOT NULL,
    quote_precision     INTEGER NOT NULL,
    tick_size           NUMERIC(30, 18) NOT NULL,   -- never FLOAT
    lot_size            NUMERIC(30, 18) NOT NULL,   -- never FLOAT
    trading_hours_json  JSONB NOT NULL,             -- serialized TradingSchedule
    halt_policy         TEXT NOT NULL,              -- 'haltable' | 'non_haltable'
    trust_tier          TEXT NOT NULL,              -- matches domain::TrustTier snake_case
    watermark_secs      INTEGER NOT NULL DEFAULT 2,
    active              BOOLEAN NOT NULL DEFAULT true,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS instruments_venue_asset
    ON instruments(venue_id, asset_class);
