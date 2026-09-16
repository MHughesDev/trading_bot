-- 0077 — scoring the monitor itself (COMP-007 §4.1; S-7.12).
--
-- The platform measures its strategies; it does not get to exempt its own
-- instruments from measurement. A 3σ band on a noisy intraday metric breaches by
-- construction, and a monitor that cries wolf trains the operator to click through
-- it — which is the alarm-fatigue failure, except that here the thing being ignored
-- is the last line before money.
--
-- So every breach is labelled afterwards, precision per metric family is computed
-- monthly, and a family below 0.5 is flagged for re-derivation rather than left to
-- erode trust quietly.

CREATE TABLE IF NOT EXISTS monitor.breach_label (
  event_id     UUID        PRIMARY KEY REFERENCES monitor.events (event_id),
  family       TEXT        NOT NULL,
  label        TEXT        NOT NULL,
  labelled_by  TEXT        NOT NULL,
  labelled_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  note         TEXT,
  CONSTRAINT monitor_breach_label_value CHECK (label IN ('true_positive', 'false_positive')),
  CONSTRAINT monitor_breach_label_family CHECK (family IN (
    'execution', 'behaviour', 'exposure', 'pnl', 'data', 'venue', 'platform', 'lease'
  )),
  -- 'human:<user>' when the operator who resumed said so, 'auto' when a
  -- scaled-down strategy was restored to full with no intervening change (§4.1).
  CONSTRAINT monitor_breach_label_never_by_an_agent
    CHECK (labelled_by NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS monitor_breach_label_family
  ON monitor.breach_label (family, labelled_at DESC);

-- Published beside the strategy scorecards. Stored rather than computed on read so
-- that a month's number is fixed once the month closes and a later relabelling is
-- visible as a recomputation rather than as history quietly changing.
CREATE TABLE IF NOT EXISTS monitor.family_precision (
  period_month     DATE        NOT NULL,
  family           TEXT        NOT NULL,
  true_positives   INTEGER     NOT NULL CHECK (true_positives >= 0),
  false_positives  INTEGER     NOT NULL CHECK (false_positives >= 0),
  precision_value  NUMERIC(38,18),
  flagged          BOOLEAN     NOT NULL DEFAULT FALSE,
  computed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (period_month, family),

  CONSTRAINT monitor_family_precision_family CHECK (family IN (
    'execution', 'behaviour', 'exposure', 'pnl', 'data', 'venue', 'platform', 'lease'
  )),
  -- A family with no labelled breaches has no precision — not a precision of zero,
  -- and not a precision of one. Both of those are claims the data cannot support,
  -- and both would flag or clear a family on no evidence at all.
  CONSTRAINT monitor_family_precision_unlabelled_is_null CHECK (
    (true_positives + false_positives = 0) = (precision_value IS NULL)
  ),
  CONSTRAINT monitor_family_precision_range CHECK (
    precision_value IS NULL OR (precision_value >= 0 AND precision_value <= 1)
  ),
  -- The flag is a function of the number, not an opinion about it: below 0.5 is
  -- flagged, at or above is not, and an unmeasured family is never flagged.
  CONSTRAINT monitor_family_precision_flag_follows_the_number CHECK (
    flagged = (precision_value IS NOT NULL AND precision_value < 0.5)
  )
);

COMMENT ON TABLE monitor.family_precision IS
  'COMP-007 §4.1: a family below 0.5 has its limits re-derived from data rather than left to erode trust.';
