-- 0099 — the deep-backfill cursor (DATA-005 §12, DA-11; Set P P-1.1).
--
-- The platform has months of history at best. strict_v1 Gate 9 has a five-year
-- calendar floor and CPCV, walk-forward and regime work need years. The backfill
-- that closes that gap runs for hours against free endpoints that will ban a
-- client which misbehaves, so it must be three things at once: resumable across
-- worker restarts, idempotent under resubmission, and polite. This table is
-- where all three are recorded.
--
-- One row per (source, venue, instrument, timeframe, window). The key is the
-- job manifest, so a resubmitted window finds its own row rather than starting a
-- second fetch of the same pages.
--
-- Two deliberate choices:
--
--  * `status` separates 'paused' from 'failed'. They look identical in a status
--    column and mean opposite things: a failed job is something to fix, a paused
--    one is something to wait for. A rate-limited backfill that reports 'failed'
--    turns a venue's published limit into an incident report, and the operator
--    learns to ignore the column.
--  * `cursor` is an exclusive high-water mark in bar-CLOSE time, inside
--    [window_start, window_end). It is the only thing a restart needs: resume
--    from coalesce(cursor, window_start) and no page is fetched twice and none
--    is skipped.
--
-- Nothing here stores bars. Bars go to ClickHouse through BarStore, which is the
-- single writer and the single PIT reader (INV-02).

CREATE TABLE IF NOT EXISTS dataplane.backfill_state (
  -- dataplane.source.name: decides the declared vendor lag stamped on every row
  -- this window writes (ADR-P2-24). Coinbase/Kraken 60 s, Alpaca 900 s. It is
  -- part of the key because the same window from two sources is two different
  -- claims about when the data became knowable, not one claim fetched twice.
  source        TEXT        NOT NULL REFERENCES dataplane.source(name),
  venue         TEXT        NOT NULL REFERENCES dataplane.venue(name),
  -- The platform symbol ('BTC-USD'). Not the surrogate instrument_id: a backfill
  -- window is submitted before the instrument is necessarily known to the
  -- identity service, and BarStore allocates the id when the first bar lands.
  instrument    TEXT        NOT NULL CHECK (length(instrument) > 0),
  timeframe     TEXT        NOT NULL CHECK (timeframe IN ('1m','5m','15m','1h','4h','1d')),
  window_start  TIMESTAMPTZ NOT NULL,
  window_end    TIMESTAMPTZ NOT NULL,

  -- Exclusive high-water mark in bar-close time. NULL = nothing fetched yet.
  cursor        TIMESTAMPTZ,
  status        TEXT        NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','running','paused','completed','failed')),
  attempts      INTEGER     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  -- When a paused job may be claimed again. A pause without a time is a stall.
  backoff_until TIMESTAMPTZ,
  last_error    TEXT,
  pages_fetched BIGINT      NOT NULL DEFAULT 0 CHECK (pages_fetched >= 0),
  rows_written  BIGINT      NOT NULL DEFAULT 0 CHECK (rows_written >= 0),
  -- The jobs-service manifest hash of the window, when it was submitted through
  -- the job service. Recorded so a row and a job can be joined without guessing.
  manifest_hash TEXT,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

  PRIMARY KEY (source, venue, instrument, timeframe, window_start, window_end),
  CHECK (window_end > window_start),
  -- The cursor is inside its own window. A cursor past window_end would make the
  -- job look complete while leaving a hole; before window_start it would refetch
  -- history the window never claimed.
  CHECK (cursor IS NULL OR (cursor >= window_start AND cursor <= window_end)),
  -- A paused row must say when to come back. Enforced rather than expected: the
  -- one failure mode this whole table exists to avoid is a backfill that stopped
  -- and nobody noticed, and a pause with no deadline is exactly that.
  CHECK (status <> 'paused' OR backoff_until IS NOT NULL),
  -- A failed row must say why. 'failed' with no reason is a dead end for whoever
  -- picks it up next.
  CHECK (status <> 'failed' OR last_error IS NOT NULL),
  -- A completed row has reached the end of its window. This is the invariant
  -- that makes "resubmitting a completed window performs zero fetches" safe to
  -- rely on: completion is a fact about coverage, not a flag someone set.
  CHECK (status <> 'completed' OR cursor = window_end)
);

-- The claim query: which windows are runnable now?
CREATE INDEX IF NOT EXISTS backfill_state_runnable_idx
  ON dataplane.backfill_state (status, backoff_until)
  WHERE status IN ('pending','paused','running');

-- The census query (P-0.1/P-1.7): how deep is each instrument, per timeframe?
CREATE INDEX IF NOT EXISTS backfill_state_coverage_idx
  ON dataplane.backfill_state (instrument, timeframe, window_start);

GRANT SELECT, INSERT ON dataplane.backfill_state TO app_role;
GRANT UPDATE (cursor, status, attempts, backoff_until, last_error, pages_fetched, rows_written, manifest_hash, updated_at)
  ON dataplane.backfill_state TO app_role;
