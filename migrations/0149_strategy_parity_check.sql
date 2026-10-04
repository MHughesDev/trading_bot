-- 0149 — the nightly parity check's verdicts (Set U U-8.2; ADR-0047 decision 2).
--
-- Each night the previous day's paper decision journal (0144) is replayed through
-- the engine as the backtest builds it. One row per replayed segment — a runner
-- start (its genesis) and the decisions after it — saying whether every decision
-- came out byte-identical, which decision first differed and in which field, or
-- why the segment could not be replayed. A `diverged` row is an engine-boundary
-- defect, a P1 like a feature-consistency code drift; the self-monitor reads it.
--
-- Append-only: a verdict that could be edited is not evidence.

CREATE TABLE IF NOT EXISTS strategy_parity_check (
  check_id       BIGSERIAL PRIMARY KEY,
  deployment_id  UUID NOT NULL REFERENCES strategy_deployment (deployment_id),
  -- The plan revision the replay compiled.
  revision       TEXT NOT NULL,
  -- The day being checked: decisions with window_start <= decided_at < window_end.
  window_start   TIMESTAMPTZ NOT NULL,
  window_end     TIMESTAMPTZ NOT NULL CHECK (window_end > window_start),
  -- The segment's records.
  first_seq      BIGINT NOT NULL CHECK (first_seq > 0),
  last_seq       BIGINT NOT NULL CHECK (last_seq >= first_seq),
  verdict        TEXT NOT NULL CHECK (verdict IN ('identical','diverged','unreplayable')),
  decisions      BIGINT CHECK (decisions IS NULL OR decisions > 0),
  -- For `diverged`: the first record that differed, the field, both values.
  diverged_seq   BIGINT,
  field          TEXT,
  recorded       JSONB,
  replayed       JSONB,
  -- For `unreplayable`: why.
  reason         TEXT,
  checked_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (verdict <> 'identical'    OR decisions IS NOT NULL),
  CHECK (verdict <> 'diverged'     OR (diverged_seq IS NOT NULL AND field IS NOT NULL)),
  CHECK (verdict <> 'unreplayable' OR reason IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_strategy_parity_check_checked
  ON strategy_parity_check (checked_at DESC);
CREATE INDEX IF NOT EXISTS idx_strategy_parity_check_deployment
  ON strategy_parity_check (deployment_id, window_start);

CREATE OR REPLACE FUNCTION strategy_parity_check_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'strategy_parity_check is append-only (Set U U-8.2)';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_parity_check_append_only ON strategy_parity_check;
CREATE TRIGGER trg_strategy_parity_check_append_only
  BEFORE UPDATE OR DELETE ON strategy_parity_check
  FOR EACH ROW EXECUTE FUNCTION strategy_parity_check_append_only();

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') THEN
    GRANT SELECT, INSERT ON strategy_parity_check TO app_role;
    GRANT USAGE ON SEQUENCE strategy_parity_check_check_id_seq TO app_role;
    GRANT SELECT ON strategy_decision_journal, strategy_deployment TO app_role;
  END IF;
END $$;
