-- §1.1 Identity. Surrogate keys only; symbols are bitemporal attributes. [INV-04]

CREATE TABLE venue (
  venue_id            INT PRIMARY KEY,
  name                TEXT NOT NULL,
  chain_id            INT,
  session_model       TEXT NOT NULL CHECK (session_model IN
                        ('continuous_24_7','rth_plus_ext','chain_block')),
  calendar_id         TEXT,           -- pinned exchange_calendars identifier
  calendar_version    TEXT,           -- enters dataset hashes [INV-12]
  quality_tier        SMALLINT NOT NULL CHECK (quality_tier BETWEEN 1 AND 3)
);

CREATE TABLE instrument (
  instrument_id       BIGINT PRIMARY KEY,        -- opaque, never reused
  asset_class         TEXT NOT NULL CHECK (asset_class IN
                        ('equity','etf','future','option','crypto','perp','pool')),
  base_instrument_id  BIGINT REFERENCES instrument(instrument_id),
  first_seen          TIMESTAMPTZ NOT NULL,
  last_seen           TIMESTAMPTZ,
  static_attrs        JSONB NOT NULL DEFAULT '{}'   -- immutable facts only
);

CREATE TABLE instrument_symbol (
  instrument_id  BIGINT NOT NULL REFERENCES instrument(instrument_id),
  venue_id       INT    NOT NULL REFERENCES venue(venue_id),
  symbol         TEXT   NOT NULL,
  valid_from     TIMESTAMPTZ NOT NULL,
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,            -- [INV-01]
  PRIMARY KEY (instrument_id, venue_id, symbol, valid_from, knowledge_time)
);
CREATE INDEX ON instrument_symbol (symbol, venue_id, valid_from);

-- Tick/lot sizes change over time and are PIT facts.
CREATE TABLE instrument_trading_rule (
  instrument_id  BIGINT NOT NULL,
  venue_id       INT NOT NULL,
  tick_size      DECIMAL(38,18),
  lot_size       DECIMAL(38,18),
  valid_from     TIMESTAMPTZ NOT NULL,
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, valid_from, knowledge_time)
);
