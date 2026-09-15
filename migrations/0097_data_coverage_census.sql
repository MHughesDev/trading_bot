-- Set P Phase 0 (P-0.1): the coverage census.
--
-- "The platform has thin data" was, until this table existed, an inference from a
-- single observation on 2026-09-08 — BTC-USD holding 22 one-hour bars. The whole
-- roadmap's stage ordering, and the question of whether `strict_v1`'s five-year
-- Gate 9 floor is reachable this decade, rested on a number nobody had.
--
-- One row per (instrument, venue, timeframe, as_of). `as_of` is in the key on
-- purpose: the census is a TIME SERIES, not a snapshot that overwrites its own
-- history. Watching coverage accrue is the only way to see a collector die, and a
-- table that keeps just the latest reading cannot show it (DATA-005 §7; Set P D-P5).
--
-- The census reads through `BarStore` and writes nothing to the bars themselves.
--
-- Venue is part of the key even though the phase plan wrote the grain as
-- (instrument, timeframe, as_of). A deliberate refinement: BTC-USD holds 1m bars on
-- coinbase, kraken and binance with wildly different depth, and the point-in-time
-- reader keeps venues apart precisely because blending them fabricates a price
-- nobody traded (SPEC §1.7). A census that summed across venues would report a
-- continuous span no single venue has.

CREATE TABLE IF NOT EXISTS dataplane.data_coverage_census (
  -- Grain.
  instrument              TEXT        NOT NULL,
  venue                   TEXT        NOT NULL,
  timeframe               TEXT        NOT NULL,
  as_of                   TIMESTAMPTZ NOT NULL,

  period_secs             INT         NOT NULL CHECK (period_secs > 0),

  -- What kind of clock this series is on.
  --   'historical' — backfillable; a hole today can be filled tomorrow.
  --   'forward'    — collect-only; a hole today is permanent (artifact.md FM-11).
  -- The distinction is the whole reason Set P Phase 0 exists, so it is a column
  -- rather than a convention.
  series_kind             TEXT        NOT NULL DEFAULT 'historical'
                          CHECK (series_kind IN ('historical','forward')),

  -- Span. NULL only for a series with no rows at all.
  first_bar               TIMESTAMPTZ,
  last_bar                TIMESTAMPTZ,

  -- Counts. Expected comes from the instrument's trading calendar, so a weekend on
  -- a session calendar is not a hole.
  expected_bars           BIGINT      NOT NULL CHECK (expected_bars >= 0),
  present_bars            BIGINT      NOT NULL CHECK (present_bars >= 0),
  missing_bars            BIGINT      NOT NULL CHECK (missing_bars >= 0),
  -- Basis points, an integer: the census must hash stably across machines, and a
  -- float percentage would not.
  missing_bps             INT         NOT NULL CHECK (missing_bps BETWEEN 0 AND 10000),

  longest_gap_periods     BIGINT      NOT NULL CHECK (longest_gap_periods >= 0),
  -- The number that decides whether a walk-forward study is possible at all. A
  -- large `present_bars` over a swiss-cheese span does not.
  longest_continuous_bars BIGINT      NOT NULL CHECK (longest_continuous_bars >= 0),
  longest_continuous_secs BIGINT      NOT NULL CHECK (longest_continuous_secs >= 0),

  -- Knowledge provenance, split. A census that reported only "how many bars" would
  -- flatter a series whose entire history is a backfill stamped as though it had
  -- been watched live — exactly the bug that shipped in market_bars_v2 (ADR-P2-24).
  -- Gate 15 takes forward evidence from `observed_knowledge_bars` only (D-P3).
  derived_knowledge_bars  BIGINT      NOT NULL CHECK (derived_knowledge_bars >= 0),
  observed_knowledge_bars BIGINT      NOT NULL CHECK (observed_knowledge_bars >= 0),

  -- What the depth permits, in the gate funnel's own vocabulary.
  strict_v1_satisfied     BOOLEAN     NOT NULL,
  strict_v1_shortfall_days INT,
  paper_v1_satisfied      BOOLEAN     NOT NULL,
  paper_v1_shortfall_days INT,

  -- Age of the newest row at `as_of`, in seconds. The silence alarm (P-0.6) reads
  -- this for forward series; for historical series it is informational.
  last_row_age_secs       BIGINT,

  computed_at             TIMESTAMPTZ NOT NULL DEFAULT now(),

  PRIMARY KEY (instrument, venue, timeframe, as_of),

  -- Every bar is on exactly one side of the provenance line. A row that cannot say
  -- which does not get written (Set P §5).
  CONSTRAINT census_provenance_partitions_bars
    CHECK (derived_knowledge_bars + observed_knowledge_bars = present_bars),
  -- Missing is a derived quantity and may not disagree with its inputs.
  CONSTRAINT census_missing_is_consistent
    CHECK (missing_bars = greatest(expected_bars - present_bars, 0)),
  -- A continuous run cannot be longer than the bars that exist.
  CONSTRAINT census_run_fits_in_series
    CHECK (longest_continuous_bars <= present_bars),
  -- A satisfied floor has no shortfall; an unsatisfied one states its shortfall in
  -- days or admits it has no data to measure. Silence is not an option.
  CONSTRAINT census_strict_shortfall_iff_unsatisfied
    CHECK ((strict_v1_satisfied AND strict_v1_shortfall_days IS NULL)
        OR (NOT strict_v1_satisfied AND (strict_v1_shortfall_days IS NOT NULL OR present_bars = 0))),
  CONSTRAINT census_paper_shortfall_iff_unsatisfied
    CHECK ((paper_v1_satisfied AND paper_v1_shortfall_days IS NULL)
        OR (NOT paper_v1_satisfied AND (paper_v1_shortfall_days IS NOT NULL OR present_bars = 0))),
  -- An empty series has no span and no floor.
  CONSTRAINT census_empty_series_has_no_span
    CHECK (present_bars > 0 OR (first_bar IS NULL AND last_bar IS NULL
                                AND NOT strict_v1_satisfied AND NOT paper_v1_satisfied)),
  CONSTRAINT census_span_is_ordered
    CHECK (first_bar IS NULL OR last_bar IS NULL OR first_bar <= last_bar)
);

-- The census page sorts by depth ascending, thinnest first (P-0.2).
CREATE INDEX IF NOT EXISTS census_by_depth
  ON dataplane.data_coverage_census (as_of DESC, longest_continuous_secs ASC);
-- One series' history, for the accrual chart and the silence alarm.
CREATE INDEX IF NOT EXISTS census_series_history
  ON dataplane.data_coverage_census (instrument, venue, timeframe, as_of DESC);
CREATE INDEX IF NOT EXISTS census_forward_series
  ON dataplane.data_coverage_census (series_kind, as_of DESC)
  WHERE series_kind = 'forward';

-- A census reading is an observation. Correcting one by overwriting it would erase
-- the evidence that the number changed, which is the only signal a collector death
-- ever gives (Set P §8: silence is the failure mode). Re-running writes a new
-- `as_of`; it never edits an old one.
DROP TRIGGER IF EXISTS trg_census_immutable ON dataplane.data_coverage_census;
CREATE TRIGGER trg_census_immutable
  BEFORE UPDATE OR DELETE ON dataplane.data_coverage_census
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

GRANT SELECT ON dataplane.data_coverage_census TO app_role, internal_ml_role, backtest_role, agent_role;
GRANT INSERT ON dataplane.data_coverage_census TO app_role;
