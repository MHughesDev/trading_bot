-- 0076 — control limits, declared before the drawdown (COMP-007 §3, §2 P2; S-7.9).
--
-- P2 is the whole point of this table: a limit chosen while the loss is happening is
-- a limit chosen by the loss. So limits are declared at promotion, from a named
-- backtest or paper run, versioned, and never edited in place.
--
-- `declared_from` is NOT NULL and non-empty because a baseline whose provenance is
-- unrecorded is a number somebody typed. The whole difference between a control
-- limit and a guess is that one of them can be traced to a measurement.

CREATE TABLE IF NOT EXISTS monitor.limits (
  strategy_id   TEXT        NOT NULL,
  metric        TEXT        NOT NULL,
  version       INTEGER     NOT NULL CHECK (version >= 1),
  family        TEXT        NOT NULL,
  -- Whether crossing this limit records and notifies, or acts. Declared with the
  -- limit rather than derived from the metric name, because "which metrics are
  -- only advisory" is a decision about a particular strategy and belongs next to
  -- the numbers it applies to.
  severity      TEXT        NOT NULL DEFAULT 'breach',
  lower_limit   NUMERIC(38,18),
  upper_limit   NUMERIC(38,18),
  method        TEXT        NOT NULL,
  declared_from TEXT        NOT NULL CHECK (length(btrim(declared_from)) > 0),
  declared_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  declared_by   TEXT        NOT NULL,
  PRIMARY KEY (strategy_id, metric, version),

  -- A limit with neither bound is not a limit. A one-sided limit is legitimate
  -- (fill rate has a floor and no ceiling), so the constraint is "at least one".
  CONSTRAINT monitor_limits_has_a_bound
    CHECK (lower_limit IS NOT NULL OR upper_limit IS NOT NULL),
  CONSTRAINT monitor_limits_bounds_ordered
    CHECK (lower_limit IS NULL OR upper_limit IS NULL OR lower_limit <= upper_limit),
  CONSTRAINT monitor_limits_family CHECK (family IN (
    'execution', 'behaviour', 'exposure', 'pnl', 'data', 'venue', 'platform', 'lease'
  )),
  CONSTRAINT monitor_limits_severity CHECK (severity IN ('warning', 'breach')),
  CONSTRAINT monitor_limits_method CHECK (method IN (
    'paper_ewma_3sigma', 'backtest_3sigma', 'venue_rate_ceiling',
    'risk_gate_mirror', 'declared_absolute', 'mc_p95_drawdown', 'cusum'
  )),
  -- §5/§8.6 again, one layer down: an agent may not author a control limit either.
  -- A limit the watched party can widen is not a control.
  CONSTRAINT monitor_limits_never_declared_by_an_agent
    CHECK (declared_by NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS monitor_limits_current
  ON monitor.limits (strategy_id, metric, version DESC);

COMMENT ON TABLE monitor.limits IS
  'COMP-007 §3 + COMP-002 gate amendment: a strategy with no row here cannot be promoted to live.';

-- Changing a limit during a breach takes the two-step pattern and is logged with
-- the breach it overrode (§3, P2). A change that silently raised a ceiling while
-- the metric was outside it would be indistinguishable, afterwards, from a metric
-- that never breached.
CREATE TABLE IF NOT EXISTS monitor.limit_change (
  change_id        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  strategy_id      TEXT        NOT NULL,
  metric           TEXT        NOT NULL,
  from_version     INTEGER,
  to_version       INTEGER     NOT NULL CHECK (to_version >= 1),
  changed_by       TEXT        NOT NULL,
  changed_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  reason           TEXT        NOT NULL CHECK (length(btrim(reason)) > 0),
  -- The breach this change was made during, if any. NULL means the change was
  -- made while the metric was inside its limit, which is the ordinary case.
  overrode_event_id UUID       REFERENCES monitor.events (event_id),
  -- The typed confirmation the second step required, kept so the two-step is
  -- auditable rather than merely performed.
  confirmation     TEXT,
  CONSTRAINT monitor_limit_change_never_by_an_agent
    CHECK (changed_by NOT LIKE 'agent:%'),
  -- Two-step: a change made during a breach must carry the typed confirmation.
  CONSTRAINT monitor_limit_change_mid_breach_is_confirmed
    CHECK (overrode_event_id IS NULL OR (confirmation IS NOT NULL AND length(btrim(confirmation)) > 0))
);

CREATE INDEX IF NOT EXISTS monitor_limit_change_strategy
  ON monitor.limit_change (strategy_id, changed_at DESC);
