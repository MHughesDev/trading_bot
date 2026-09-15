-- Set P Phase 0 (P-0.3, P-0.4, P-0.5, P-0.6): the forward-only series' Postgres
-- side — source declarations, the series registry, and the cadence the silence
-- alarm measures against.
--
-- The observations themselves are high-volume and live in ClickHouse
-- (clickhouse/08_forward_series.sql). What lives here is what must be transactional
-- and referenced: which sources exist, what lag each declares, which series the
-- platform has promised to keep collecting, and how often each should produce a
-- row.
--
-- The phase plan wrote "two ClickHouse tables (migration 0098)". The migrations
-- directory is Postgres only — ClickHouse DDL is embedded and replayed by
-- `storage::clickhouse::migrate`. Recorded as a plan correction rather than worked
-- around.

-- ── Sources and their declared lags ───────────────────────────────────────────
-- Kraken Futures public endpoints, verified reachable from this machine on
-- 2026-09-15: GET /derivatives/api/v3/tickers returned 200 with 295 tickers, 279 of
-- them perpetuals carrying fundingRate and openInterest; GET
-- /derivatives/api/v3/historical-funding-rates?symbol=PF_XBTUSD returned 8,870
-- hourly rates covering 2025-09-10 to 2026-09-15. No 451 and no geo-block.
--
-- Both are declared `rest_history`, not `live_stream`, and that is the whole point.
-- A REST poll that catches a value one second after publication still did not
-- observe the publication — it observed our poll. Declaring them live would put the
-- sentinel's absence on rows that never earned it, and Gate 15 takes forward
-- evidence from observed rows only (Set P D-P3).
INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes) VALUES
  (17, 'kraken_futures_rest',    'rest_history', 3600000,
       'public /derivatives/api/v3/tickers poll: funding rate, prediction and open interest. Funding settles hourly, so a polled value is bounded by one settlement interval, not by the round trip.'),
  (18, 'kraken_futures_history', 'rest_history', 3600000,
       'public /derivatives/api/v3/historical-funding-rates. Explicit history: every row is a backfill and carries BACKFILLED_KNOWLEDGE_TIME.'),
  (19, 'tradier_chain_rest',     'rest_history', 900000,
       'Tradier option chain snapshots. Tradier documents a 15-minute delay on sandbox data and on production index data; the row is observed, but observed late, and those are not the same thing.')
ON CONFLICT (source_id) DO NOTHING;

-- ── The forward-series registry (P-0.6) ───────────────────────────────────────
-- A promise, recorded: the platform has undertaken to keep collecting this series,
-- at this cadence, from this date. The registry is what makes silence detectable —
-- without it, a collector that never starts and a collector that dies look exactly
-- alike, which is to say they look like nothing at all.
CREATE TABLE IF NOT EXISTS dataplane.forward_series (
  series_id          TEXT PRIMARY KEY,
  -- The ClickHouse table the observations land in.
  store_table        TEXT NOT NULL
                     CHECK (store_table IN ('funding_rates','open_interest','option_chain_snapshots')),
  venue              TEXT NOT NULL,
  source_name        TEXT NOT NULL REFERENCES dataplane.source(name),

  -- How often a row is expected. The alarm fires when the newest row is older than
  -- `cadence_secs * alarm_after_cadences`.
  cadence_secs       INT  NOT NULL CHECK (cadence_secs > 0),
  alarm_after_cadences INT NOT NULL DEFAULT 3 CHECK (alarm_after_cadences >= 1),

  -- Whether any history exists to backfill. Funding has a year at Kraken; open
  -- interest has none anywhere. The column exists so the difference is a recorded
  -- fact rather than folklore.
  backfill_available BOOLEAN NOT NULL,
  backfill_earliest  TIMESTAMPTZ,

  -- Disk budget (P-0.6 item 1), in bytes, and what happens when it is approached.
  -- 'downsample' degrades resolution and keeps the rows; 'none' means the series is
  -- small enough to keep at full resolution forever. There is deliberately no
  -- 'delete': on a series that cannot be re-collected, deletion is irreversible
  -- destruction of the only copy that will ever exist (FM-11).
  disk_budget_bytes  BIGINT NOT NULL CHECK (disk_budget_bytes > 0),
  retention_policy   TEXT NOT NULL DEFAULT 'none'
                     CHECK (retention_policy IN ('none','downsample')),
  downsample_after_days INT CHECK (downsample_after_days IS NULL OR downsample_after_days > 0),
  downsample_to_secs    INT CHECK (downsample_to_secs IS NULL OR downsample_to_secs > 0),

  -- When collection began. The forward clock's zero.
  collecting_since   TIMESTAMPTZ NOT NULL DEFAULT now(),
  enabled            BOOLEAN NOT NULL DEFAULT TRUE,
  notes              TEXT,

  -- A downsampling policy must say when and to what, or it is not a policy.
  CONSTRAINT forward_series_downsample_is_complete
    CHECK (retention_policy <> 'downsample'
           OR (downsample_after_days IS NOT NULL AND downsample_to_secs IS NOT NULL)),
  -- Downsampling coarsens; it never refines.
  CONSTRAINT forward_series_downsample_coarsens
    CHECK (downsample_to_secs IS NULL OR downsample_to_secs >= cadence_secs),
  -- A series that claims backfillable history must say how far back it goes.
  CONSTRAINT forward_series_backfill_states_its_reach
    CHECK (NOT backfill_available OR backfill_earliest IS NOT NULL)
);

-- The series this phase switches on. Cadences are the venue's own, not ours:
-- Kraken Futures settles perpetual funding hourly, so an hourly poll is the
-- finest resolution that carries information.
INSERT INTO dataplane.forward_series
  (series_id, store_table, venue, source_name, cadence_secs, alarm_after_cadences,
   backfill_available, backfill_earliest, disk_budget_bytes, retention_policy,
   downsample_after_days, downsample_to_secs, notes)
VALUES
  ('kraken_futures.funding', 'funding_rates', 'kraken_futures', 'kraken_futures_rest',
   3600, 3, TRUE, '2025-09-10T08:00:00Z', 2147483648, 'none', NULL, NULL,
   'Hourly perpetual funding. 8,870 hourly rates were reachable for PF_XBTUSD on 2026-09-15, so roughly a year is backfillable — but only roughly a year, and the window rolls.'),
  ('kraken_futures.open_interest', 'open_interest', 'kraken_futures', 'kraken_futures_rest',
   3600, 3, FALSE, NULL, 2147483648, 'none', NULL, NULL,
   'Ticker open interest. The venue publishes the CURRENT value and offers no history endpoint: every hour not sampled is permanently lost (FM-11).')
ON CONFLICT (series_id) DO NOTHING;

CREATE INDEX IF NOT EXISTS forward_series_enabled
  ON dataplane.forward_series (enabled, store_table) WHERE enabled;

-- ── The silence alarm's state (P-0.6 item 2) ──────────────────────────────────
-- One row per series, updated by the monitor. Kept in Postgres rather than derived
-- on the fly so that "the alarm fired at 03:14 and nobody looked" is a fact with a
-- timestamp, not an inference from a log that has rotated.
CREATE TABLE IF NOT EXISTS dataplane.forward_series_alarm (
  series_id        TEXT PRIMARY KEY REFERENCES dataplane.forward_series(series_id),
  last_row_at      TIMESTAMPTZ,
  last_checked_at  TIMESTAMPTZ NOT NULL,
  state            TEXT NOT NULL CHECK (state IN ('ok','stale','never_collected')),
  -- How far past the deadline the newest row is. Negative while healthy.
  overdue_secs     BIGINT NOT NULL,
  raised_at        TIMESTAMPTZ,
  cleared_at       TIMESTAMPTZ,
  -- A raised alarm states when it was raised; a clear one does not pretend to.
  CONSTRAINT alarm_raised_iff_not_ok
    CHECK ((state = 'ok' AND raised_at IS NULL) OR (state <> 'ok' AND raised_at IS NOT NULL)),
  CONSTRAINT alarm_never_collected_has_no_row
    CHECK (state <> 'never_collected' OR last_row_at IS NULL)
);

GRANT SELECT ON dataplane.forward_series, dataplane.forward_series_alarm
  TO app_role, internal_ml_role, backtest_role, agent_role;
GRANT INSERT, UPDATE ON dataplane.forward_series_alarm TO app_role;
GRANT INSERT ON dataplane.forward_series TO app_role;
GRANT UPDATE (enabled, disk_budget_bytes, retention_policy, downsample_after_days, downsample_to_secs, notes)
  ON dataplane.forward_series TO app_role;
