-- 0184 — where an instrument's tier-1 fingerprint is kept before there is a
-- population to embed it against (SPEC §5.2-5.3, checklist 3.1/3.4; OPEN-TASKS C1.3).
--
-- `knowledge.asset_embedding` needs `retrieval_vec vector(48) NOT NULL`, and that
-- vector is a property of the *population*: the whitener is fitted over every
-- instrument and refuses below 60 (`embedding.MIN_POPULATION`). A platform with
-- twelve instruments therefore has twelve fingerprints and nothing it can legally
-- put in `asset_embedding`. Without somewhere to keep the fingerprints, the
-- `instrument_profile` job would have had to throw them away every run and
-- recompute them when the sixtieth instrument arrived.
--
-- So this is the fingerprint's own table: append-only, one row per computation,
-- keyed by when it became knowable. `tier1` is in `dimension_order` with NULL for
-- an absent dimension — NULL is "not measured", and a zero would be a measurement
-- nobody made. Market-public like `asset_embedding`: no tenant column.

CREATE TABLE IF NOT EXISTS knowledge.instrument_fingerprint (
  instrument_id   BIGINT NOT NULL,
  venue_id        INT    NOT NULL,
  window_spec     TEXT   NOT NULL,                 -- '21d@1m'
  dimension_order TEXT[] NOT NULL,
  tier1           REAL[] NOT NULL,                 -- NULL elements are not-measured
  absent          JSONB  NOT NULL DEFAULT '{}'::jsonb,
  bars_used       INT    NOT NULL CHECK (bars_used > 0),
  knowledge_time  TIMESTAMPTZ NOT NULL,
  info_class      TEXT   NOT NULL DEFAULT 'market_public',
  PRIMARY KEY (instrument_id, venue_id, window_spec, knowledge_time),
  CONSTRAINT chk_fingerprint_shape
    CHECK (cardinality(tier1) = cardinality(dimension_order) AND cardinality(tier1) > 0)
);
CREATE INDEX IF NOT EXISTS idx_instrument_fingerprint_latest
  ON knowledge.instrument_fingerprint (window_spec, instrument_id, venue_id, knowledge_time DESC);

-- Granted in the table's own migration. The platform writes it; everything that
-- reads an embedding may read a fingerprint; nothing updates or deletes one.
GRANT SELECT, INSERT ON knowledge.instrument_fingerprint TO app_role;
GRANT SELECT ON knowledge.instrument_fingerprint TO agent_role, internal_ml_role;
REVOKE UPDATE, DELETE, TRUNCATE ON knowledge.instrument_fingerprint
  FROM app_role, agent_role, internal_ml_role;
