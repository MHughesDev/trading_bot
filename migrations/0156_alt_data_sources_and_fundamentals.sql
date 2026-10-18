-- Set P Phase 3 (P-3.1, P-3.2, P-3.5): the alternative-data sources, declared, and
-- the fundamentals store.
--
-- The collectors for SEC EDGAR, ALFRED and Alpaca news have existed since
-- 2026-09-22 with their source declarations in `dataplane` (SourceDecl::sec_edgar,
-- alfred_rest, …) and **no row in `dataplane.source`**, so nothing they produced
-- could be attributed, and `cargo xtask check-free-sources` could not see them.
-- Each is declared here, free, one statement at a time, with the lag its module
-- states (ADR-P2-24) and an explicit cost_class (0104: no DEFAULT, on purpose).

INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes, cost_class) VALUES
  (20, 'sec_edgar', 'rest_history', 86399000,
       'SEC EDGAR companyfacts + submissions. companyfacts reports a filed DATE only; the derived knowable instant is the end of that day. Observed acceptanceDateTime replaces it where the submissions index supplies one.',
       'free')
ON CONFLICT (source_id) DO NOTHING;
INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes, cost_class) VALUES
  (21, 'alfred_rest', 'rest_history', 86399000,
       'FRED/ALFRED vintages. realtime_start is a DATE; a vintage is knowable from the end of its publication day.',
       'free')
ON CONFLICT (source_id) DO NOTHING;
INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes, cost_class) VALUES
  (22, 'alpaca_news', 'rest_history', 0,
       'Alpaca News API (Benzinga). Knowledge time is publication time (created_at); a correction (updated_at) is a new revision row.',
       'free')
ON CONFLICT (source_id) DO NOTHING;

-- Kraken's downloadable OHLCVT archives (P-1.3). Its own row so the backfill
-- router can route the archive path beside the OHLC API path (both answered
-- 'kraken_rest' before, so only one could ever be registered).
INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes, cost_class) VALUES
  (23, 'kraken_archive', 'vendor_file', 60000,
       'Kraken OHLCVT history archives. Each file is verified against a declared sha256 before a row is read; a mismatch pauses the window.',
       'free')
ON CONFLICT (source_id) DO NOTHING;

-- The cost_class declarations, in the one form `cargo xtask check-free-sources`
-- reads (0104's convention): one statement per source, so adding a source means
-- adding a line that says which it is. The INSERTs above already set the column;
-- these are no-ops on a fresh database and the fence's evidence on every one.
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'sec_edgar'      AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'alfred_rest'    AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'alpaca_news'    AND cost_class IS NULL;
UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kraken_archive' AND cost_class IS NULL;

-- ── Fundamentals facts (DATA-005 §12; P-3.2) ──────────────────────────────────
-- `accepted_at` is the knowledge time. The period a number describes is NOT when
-- it became knowable, and a read keyed on `period_end` is the most common lookahead
-- bug in fundamentals research. Every read path filters on `accepted_at <= as_of`.
CREATE TABLE IF NOT EXISTS dataplane.fundamentals_fact (
  cik            BIGINT      NOT NULL CHECK (cik > 0),
  tag            TEXT        NOT NULL,
  unit           TEXT        NOT NULL,
  period_start   DATE,
  period_end     DATE        NOT NULL,
  fiscal_year    INT,
  fiscal_period  TEXT,
  form           TEXT        NOT NULL,
  accession      TEXT        NOT NULL,
  value          NUMERIC     NOT NULL,
  accepted_at    TIMESTAMPTZ NOT NULL,
  -- 'observed' when acceptanceDateTime came from the submissions index;
  -- 'derived' when it is the end of the filed day (the declared 86 399 s lag).
  knowledge      TEXT        NOT NULL CHECK (knowledge IN ('observed','derived')),
  source_id      INT         NOT NULL REFERENCES dataplane.source(source_id),
  ingest_time    TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- A filing cannot report a period that had not finished when it was accepted.
  CONSTRAINT fundamentals_accepted_after_period
    CHECK (accepted_at >= period_end::timestamptz),
  -- One value per filing per fact. A restatement arrives in a *later* filing,
  -- with its own accession, and is a new row: the original is never overwritten.
  PRIMARY KEY (cik, tag, unit, period_end, accession)
);

CREATE INDEX IF NOT EXISTS fundamentals_fact_asof
  ON dataplane.fundamentals_fact (cik, tag, accepted_at);

-- Append-only. A correction is a later filing, not an UPDATE: overwriting would
-- erase the value a study saw at the time.
DROP TRIGGER IF EXISTS trg_fundamentals_fact_immutable ON dataplane.fundamentals_fact;
CREATE TRIGGER trg_fundamentals_fact_immutable
  BEFORE UPDATE OR DELETE ON dataplane.fundamentals_fact
  FOR EACH ROW EXECUTE FUNCTION dataplane.refuse_delete();

GRANT SELECT, INSERT ON dataplane.fundamentals_fact TO app_role;
