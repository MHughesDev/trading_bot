-- 0165 — what the monitor measures, and what it files (COMP-007 §3, §4, §8.3,
-- §8.8; Set S S-7.5, S-7.6, S-7.9, S-7.10; Set T T-2.2, T-2.4).
--
-- Five things, each the receiving half of a decision that until now had nowhere
-- to be written down, and so was either a proxy or nothing at all:
--
--   1. monitor.feed_freshness      — a GAUGE of the last bar each live feed
--                                     delivered. The monitor has no ClickHouse
--                                     client and must not grow one, so the
--                                     process that observes the bar records it.
--   2. monitor.venue_reconciliation — the per-venue reconciliation verdict.
--                                     Until now "every venue reconciles" was
--                                     derived as "no venue is halted", which read
--                                     a connection-loss halt as a reconciliation
--                                     failure and a never-checked venue as clean.
--   3. monitor.platform_defect      — what a common-mode fleet divergence and a
--                                     failed drill file. Not a retry, not a log line.
--   4. monitor.drill_run            — the four fail-closed drills, with timings.
--   5. The promotion gate, in the schema: a deployment that is not paper cannot
--      be written without declared limits in three families, whoever writes it.
--
-- Plus a NOTIFY on monitor.events, so a push channel (the desk) learns of a state
-- change from the change itself rather than from the next poll.

-- ── 1. feed freshness: a gauge, not a census ────────────────────────────────
--
-- One row per live feed, overwritten. `dataplane.data_coverage_census` is an
-- append-only series keyed on `as_of`; `now() - last_bar` against a snapshot
-- over-states age by the time since the snapshot, which is the false-halt
-- direction. A gauge's age is the age.
--
-- `observed_at` is when the producer last affirmed the row, bar or no bar. A
-- row whose producer has gone quiet is *unmeasured*, not stale: the platform
-- being down is the lease's business, and halting every strategy for a
-- producer that died would give one fault two verdicts.
--
-- `session_open` is the producer's statement that bars are expected now. A
-- session-bound market that is shut is not a stale feed, and the producer —
-- which knows the asset class — is the one that can say so.
CREATE TABLE IF NOT EXISTS monitor.feed_freshness (
  symbol          TEXT        NOT NULL CHECK (length(btrim(symbol)) > 0),
  venue           TEXT        NOT NULL CHECK (length(btrim(venue)) > 0),
  -- The undeclared-timeframe case is kept out of the gauge by the CHECK rather
  -- than by a comment: a staleness verdict needs a multiple to measure against.
  timeframe_secs  INTEGER     NOT NULL CHECK (timeframe_secs > 0),
  last_bar_at     TIMESTAMPTZ NOT NULL,
  session_open    BOOLEAN     NOT NULL,
  observed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (symbol, venue, timeframe_secs)
);

COMMENT ON TABLE monitor.feed_freshness IS
  'COMP-007 §3 data family: the last bar each live feed delivered, written by the '
  'platform process that observed it (apps/platform health_jobs feed producer). '
  'A gauge: one row per feed, overwritten. Read by tbot-monitor, which raises '
  'Fact::HardDataStaleness past 10x the timeframe.';

GRANT SELECT, INSERT ON monitor.feed_freshness TO app_role;
GRANT UPDATE (last_bar_at, session_open, observed_at) ON monitor.feed_freshness TO app_role;

-- ── 2. per-venue reconciliation verdicts ────────────────────────────────────
--
-- Append-only: the question an operator asks before resuming a venue is "when
-- did this venue last agree with our book", and an editable answer is not one.
CREATE TABLE IF NOT EXISTS monitor.venue_reconciliation (
  check_id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  venue               TEXT        NOT NULL CHECK (length(btrim(venue)) > 0),
  verdict             TEXT        NOT NULL
                      CHECK (verdict IN ('agreed', 'disagreed', 'unreadable')),
  instruments_checked INTEGER     NOT NULL CHECK (instruments_checked >= 0),
  -- Required whenever the verdict is not clean: "disagreed" with no numbers is a
  -- halt nobody can act on.
  detail              TEXT,
  checked_by          TEXT        NOT NULL CHECK (length(btrim(checked_by)) > 0),
  checked_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT venue_reconciliation_detail_when_not_clean
    CHECK (verdict = 'agreed' OR length(btrim(coalesce(detail, ''))) > 0),
  CONSTRAINT venue_reconciliation_never_by_an_agent
    CHECK (checked_by NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS venue_reconciliation_latest
  ON monitor.venue_reconciliation (venue, checked_at DESC);

CREATE OR REPLACE FUNCTION monitor.refuse_mutation() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS venue_reconciliation_append_only ON monitor.venue_reconciliation;
CREATE TRIGGER venue_reconciliation_append_only
  BEFORE UPDATE OR DELETE ON monitor.venue_reconciliation
  FOR EACH ROW EXECUTE FUNCTION monitor.refuse_mutation();

-- The rule a venue resume is held to (§8.3), in one place both the store and a
-- human reading the database can ask: a venue reconciles when it has never been
-- checked, or when its most recent check agreed. A venue whose last word was
-- "disagreed" or "unreadable" does not, however long ago that was.
CREATE OR REPLACE FUNCTION monitor.venue_reconciled(v TEXT) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
  SELECT coalesce(
    (SELECT verdict = 'agreed'
       FROM monitor.venue_reconciliation
      WHERE venue = v
      ORDER BY checked_at DESC, check_id DESC
      LIMIT 1),
    TRUE)
$$;

GRANT SELECT, INSERT ON monitor.venue_reconciliation TO app_role;
GRANT EXECUTE ON FUNCTION monitor.venue_reconciled(TEXT) TO app_role;

-- ── 3. platform defects ─────────────────────────────────────────────────────
--
-- Filed by a machine, resolved by a person. `dedupe_key` keeps one *open*
-- defect per underlying condition, so an hourly job that sees the same
-- common-mode divergence every hour files it once.
--
-- This is deliberately not `ledger::falsification::EvaluatorDefect`: that type
-- demands a sealed `Reproduction`, and a forward divergence is the one thing
-- nobody can replay. A row here is the filing; promoting it into the ledger is
-- the human who re-ran the backtests.
CREATE TABLE IF NOT EXISTS monitor.platform_defect (
  defect_id    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  source       TEXT        NOT NULL CHECK (source IN ('fleet_common_mode', 'drill_failed')),
  dedupe_key   TEXT        NOT NULL CHECK (length(btrim(dedupe_key)) > 0),
  summary      TEXT        NOT NULL CHECK (length(btrim(summary)) > 0),
  evidence     JSONB       NOT NULL,
  filed_by     TEXT        NOT NULL CHECK (length(btrim(filed_by)) > 0),
  filed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  status       TEXT        NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved')),
  resolved_by  TEXT,
  resolution   TEXT,
  resolved_at  TIMESTAMPTZ,
  CONSTRAINT platform_defect_never_by_an_agent
    CHECK (filed_by NOT LIKE 'agent:%'),
  -- Resolution is a person, with a sentence, at a time — all three or none.
  CONSTRAINT platform_defect_resolution_complete CHECK (
    (status = 'open' AND resolved_by IS NULL AND resolution IS NULL AND resolved_at IS NULL)
    OR (status = 'resolved' AND resolved_by LIKE 'human:_%'
        AND length(btrim(coalesce(resolution, ''))) > 0 AND resolved_at IS NOT NULL)
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS platform_defect_one_open_per_key
  ON monitor.platform_defect (dedupe_key) WHERE status = 'open';

CREATE OR REPLACE FUNCTION monitor.platform_defect_resolve_once() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'platform_defect rows are never deleted';
  END IF;
  IF OLD.status = 'resolved' THEN
    RAISE EXCEPTION 'platform_defect % is already resolved', OLD.defect_id;
  END IF;
  IF NEW.defect_id  IS DISTINCT FROM OLD.defect_id
  OR NEW.source     IS DISTINCT FROM OLD.source
  OR NEW.dedupe_key IS DISTINCT FROM OLD.dedupe_key
  OR NEW.summary    IS DISTINCT FROM OLD.summary
  OR NEW.evidence   IS DISTINCT FROM OLD.evidence
  OR NEW.filed_by   IS DISTINCT FROM OLD.filed_by
  OR NEW.filed_at   IS DISTINCT FROM OLD.filed_at THEN
    RAISE EXCEPTION 'only the resolution of platform_defect % may change', OLD.defect_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS platform_defect_resolve_once ON monitor.platform_defect;
CREATE TRIGGER platform_defect_resolve_once
  BEFORE UPDATE OR DELETE ON monitor.platform_defect
  FOR EACH ROW EXECUTE FUNCTION monitor.platform_defect_resolve_once();

GRANT SELECT, INSERT ON monitor.platform_defect TO app_role;
GRANT UPDATE (status, resolved_by, resolution, resolved_at) ON monitor.platform_defect TO app_role;

-- ── 4. drill runs ───────────────────────────────────────────────────────────
--
-- The four drills of Set T T-2.4, each with its outcome and timing. A failed
-- drill names the defect it filed; a drill against anything but paper cannot
-- be recorded, because the CHECK on `venue_class` admits nothing else.
CREATE TABLE IF NOT EXISTS monitor.drill_run (
  run_id        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  drill         TEXT        NOT NULL CHECK (drill IN (
                  'monitor_lost', 'divergence_planted', 'kill_switch_tripped', 'service_restored'
                )),
  trigger       TEXT        NOT NULL CHECK (trigger IN ('scheduled', 'on_demand')),
  venue_class   TEXT        NOT NULL CHECK (venue_class = 'paper'),
  started_at    TIMESTAMPTZ NOT NULL,
  finished_at   TIMESTAMPTZ NOT NULL,
  duration_ms   BIGINT      NOT NULL CHECK (duration_ms >= 0),
  passed        BOOLEAN     NOT NULL,
  measurements  JSONB       NOT NULL,
  failure       TEXT,
  defect_id     UUID        REFERENCES monitor.platform_defect (defect_id),
  run_by        TEXT        NOT NULL CHECK (length(btrim(run_by)) > 0),
  CONSTRAINT drill_run_ordered CHECK (finished_at >= started_at),
  CONSTRAINT drill_run_never_by_an_agent CHECK (run_by NOT LIKE 'agent:%'),
  -- A failed drill is a defect, not a retry: it says why and names what it filed.
  CONSTRAINT drill_run_failure_is_filed CHECK (
    passed OR (length(btrim(coalesce(failure, ''))) > 0 AND defect_id IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS drill_run_latest ON monitor.drill_run (drill, started_at DESC);

DROP TRIGGER IF EXISTS drill_run_append_only ON monitor.drill_run;
CREATE TRIGGER drill_run_append_only
  BEFORE UPDATE OR DELETE ON monitor.drill_run
  FOR EACH ROW EXECUTE FUNCTION monitor.refuse_mutation();

GRANT SELECT, INSERT ON monitor.drill_run TO app_role;

-- ── 5. the promotion gate, in the schema (S-7.9, COMP-002 §3 amendment) ─────
--
-- `strategy_deployment.environment` admits only 'paper' today (0142), so no
-- deployment can be live. The gate below is what still holds on the day that
-- CHECK is widened: a non-paper deployment row cannot be written unless the
-- deployment's own id (which is the `strategy_id` its orders carry) has current
-- limits in all three families `monitor::limits::REQUIRED_FOR_LIVE` names.
-- In the schema rather than in a route because no route promotes anything yet,
-- and whichever one is written later cannot forget a trigger.
CREATE OR REPLACE FUNCTION monitor.has_live_limits(sid TEXT) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
  SELECT count(DISTINCT family) = 3
    FROM monitor.limits
   WHERE strategy_id = sid
     AND family IN ('execution', 'behaviour', 'pnl')
$$;

CREATE OR REPLACE FUNCTION monitor.refuse_live_without_limits() RETURNS trigger AS $$
BEGIN
  IF NEW.environment IS DISTINCT FROM 'paper'
     AND NOT monitor.has_live_limits(NEW.deployment_id::text) THEN
    RAISE EXCEPTION
      'deployment % is %, and a strategy with no declared control limits in the execution, behaviour and pnl families cannot leave paper (COMP-007 §8.8)',
      NEW.deployment_id, NEW.environment
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS strategy_deployment_live_needs_limits ON public.strategy_deployment;
CREATE TRIGGER strategy_deployment_live_needs_limits
  BEFORE INSERT OR UPDATE ON public.strategy_deployment
  FOR EACH ROW EXECUTE FUNCTION monitor.refuse_live_without_limits();

GRANT EXECUTE ON FUNCTION monitor.has_live_limits(TEXT) TO app_role;
GRANT EXECUTE ON FUNCTION monitor.refuse_live_without_limits() TO app_role;

-- ── 6. the declared return baseline (S-7.8) ─────────────────────────────────
--
-- The P&L family standardises each day's return against the distribution the
-- strategy was promoted on — declared from a named backtest or paper run, never
-- re-derived from the window being judged (`monitor::pnl::ReturnBaseline` has no
-- constructor from a series, for that reason). Versioned like `monitor.limits`:
-- a new baseline is a new row and the old one stays legible.
CREATE TABLE IF NOT EXISTS monitor.return_baseline (
  strategy_id       TEXT             NOT NULL CHECK (length(btrim(strategy_id)) > 0),
  version           INTEGER          NOT NULL CHECK (version >= 1),
  mean_return       DOUBLE PRECISION NOT NULL CHECK (mean_return = mean_return),
  sigma             DOUBLE PRECISION NOT NULL CHECK (sigma > 0 AND sigma = sigma),
  periods_per_year  DOUBLE PRECISION NOT NULL CHECK (periods_per_year > 0),
  declared_from     TEXT             NOT NULL CHECK (length(btrim(declared_from)) > 0),
  declared_by       TEXT             NOT NULL CHECK (length(btrim(declared_by)) > 0),
  declared_at       TIMESTAMPTZ      NOT NULL DEFAULT now(),
  PRIMARY KEY (strategy_id, version),
  -- A baseline the watched party can set is not a baseline.
  CONSTRAINT return_baseline_never_by_an_agent CHECK (declared_by NOT LIKE 'agent:%')
);

DROP TRIGGER IF EXISTS return_baseline_append_only ON monitor.return_baseline;
CREATE TRIGGER return_baseline_append_only
  BEFORE UPDATE OR DELETE ON monitor.return_baseline
  FOR EACH ROW EXECUTE FUNCTION monitor.refuse_mutation();

GRANT SELECT, INSERT ON monitor.return_baseline TO app_role;

-- ── 7. events announce themselves ───────────────────────────────────────────
--
-- 0074 and 0075 notify on the lease and the switch; the event log did not, so
-- a push channel learned of a scale-down, a warning or a window only on its
-- next poll. The payload is the event id: listeners re-read the row rather
-- than trusting a payload that NOTIFY truncates at 8000 bytes.
CREATE OR REPLACE FUNCTION monitor.notify_event() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('monitor_events', NEW.event_id::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS monitor_events_notify_trg ON monitor.events;
CREATE TRIGGER monitor_events_notify_trg
  AFTER INSERT ON monitor.events
  FOR EACH ROW EXECUTE FUNCTION monitor.notify_event();
