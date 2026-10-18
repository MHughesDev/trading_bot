-- Renumbered from 0124 when claude/knowledge-time-provenance-repair was merged into main
-- (0124 was already taken by 0124_memory_retrievals.sql and applied).
--
-- Track 20 (knowledge-time provenance repair): the census must be able to say how
-- much of its "observed" count is worth anything.
--
-- Measured 2026-09-15 by P-0.1, against the live ClickHouse: all 158,259 rows in
-- `market_bars_v2` carry `available_time = event_time`, minimum and maximum lag both
-- zero, for every source including the three REST backfill sources. INV-01 is not
-- partially violated there. It is uniformly violated, which is worse than an absent
-- column: embargo computation, PIT filtering and every dataset's
-- `max_knowledge_lag_ms` read that column as true.
--
-- Migration 0097 gave the census a derived/observed split. On that store the split
-- would have reported 158,259 observed rows and looked healthy, because nothing was
-- flagged. The split is only meaningful beside a count of rows whose claim does not
-- survive inspection, and that is what this column is.
--
-- The predicate is `dataplane::bar::is_unflagged_sentinel_violation`: a
-- `knowledge_time` earlier than `bar close + the source's declared_vendor_lag_ms`,
-- carrying no `BACKFILLED_KNOWLEDGE_TIME` flag. Too early *with* a flag is an honest
-- hole and is already counted as derived. Too early *without* one is a fiction.
--
-- Traces: INV-01; ADR-P2-24 (OQ-01); Set P D-P2, D-P3.

ALTER TABLE dataplane.data_coverage_census
  ADD COLUMN IF NOT EXISTS unflagged_sentinel_bars BIGINT;

-- Deliberately NULLABLE, and deliberately not defaulted to 0.
--
-- A census reading taken before this audit existed did not measure the condition. To
-- backfill it with 0 would be to assert, of every historical reading, that it saw no
-- violations -- which is exactly the class of fabrication this whole migration exists
-- to undo. NULL means "this reading cannot say", and the immutability trigger on this
-- table means it will stay NULL: a census row is an observation and is never edited.
COMMENT ON COLUMN dataplane.data_coverage_census.unflagged_sentinel_bars IS
  'Rows counted as observed whose knowledge_time is earlier than their source could have published, with no BACKFILLED_KNOWLEDGE_TIME flag. A subset of observed_knowledge_bars. NULL = reading predates the audit (migration 0251); it is never backfilled with 0. Above zero is an alarm, not a measurement.';

ALTER TABLE dataplane.data_coverage_census
  DROP CONSTRAINT IF EXISTS census_sentinel_count_is_non_negative;
ALTER TABLE dataplane.data_coverage_census
  ADD CONSTRAINT census_sentinel_count_is_non_negative
  CHECK (unflagged_sentinel_bars IS NULL OR unflagged_sentinel_bars >= 0);

-- The violating rows are drawn from the observed side of the split, never the
-- derived side: a flagged row is already counted as derived and must not be counted
-- twice. A value exceeding the observed count would mean the two disagree about what
-- they are counting.
ALTER TABLE dataplane.data_coverage_census
  DROP CONSTRAINT IF EXISTS census_sentinel_count_fits_in_observed;
ALTER TABLE dataplane.data_coverage_census
  ADD CONSTRAINT census_sentinel_count_fits_in_observed
  CHECK (unflagged_sentinel_bars IS NULL OR unflagged_sentinel_bars <= observed_knowledge_bars);

-- The number an operator actually wants: rows this series may honestly offer Gate 15
-- as forward-test evidence (D-P3). Stored and generated so it cannot drift from its
-- inputs, and NULL for a reading that predates the audit rather than silently equal
-- to the unaudited observed count.
ALTER TABLE dataplane.data_coverage_census
  ADD COLUMN IF NOT EXISTS defensibly_observed_bars BIGINT
  GENERATED ALWAYS AS (observed_knowledge_bars - unflagged_sentinel_bars) STORED;

COMMENT ON COLUMN dataplane.data_coverage_census.defensibly_observed_bars IS
  'observed_knowledge_bars minus unflagged_sentinel_bars: the series stock of Gate-15-admissible forward evidence (Set P D-P3). NULL when the reading predates migration 0251.';

-- Find the alarming series first. Partial, because zero is the expected reading and
-- an index over the healthy majority would earn nothing.
CREATE INDEX IF NOT EXISTS census_provenance_alarm
  ON dataplane.data_coverage_census (as_of DESC, unflagged_sentinel_bars DESC)
  WHERE unflagged_sentinel_bars > 0;
