-- 0186 — what a forward recorder needs to resume after a restart (Set T T-0.4).
--
-- A deployment recorded as forward evidence (`mlops.paper_deployment`) keeps
-- its realised series in `mlops.forward_observation`. Rebuilding that series
-- exactly after a restart — rather than from the *current* balance, which would
-- silently put it on another base — takes two more things nothing stored:
--
-- - the mark of every bar the recorder opened, closed or re-anchored at
--   (`strategy_forward_mark`): the last one is the mark the resumed series
--   continues from, and a re-anchor's mark verifies the segment after it;
-- - every fill of the deployment with its fee and the price its decision was
--   made at (`strategy_forward_fill`): cash and position are folded from them.
--
-- A fee the venue did not report is NULL, never 0: an unknown cost recorded as
-- zero makes the forward series cheaper than it was, which is the one
-- direction a forward comparison must never be wrong in. A resume refuses a
-- series with an unknown fee rather than guess one.
--
-- Append-only: the evidence a divergence verdict rests on is never edited.

CREATE TABLE IF NOT EXISTS strategy_forward_mark (
  forward_deployment_id UUID NOT NULL REFERENCES mlops.paper_deployment (deployment_id),
  tenant_id             TEXT NOT NULL CHECK (length(tenant_id) > 0),
  bar                   TIMESTAMPTZ NOT NULL,
  mark                  NUMERIC(38,18) NOT NULL CHECK (mark > 0),
  kind                  TEXT NOT NULL CHECK (kind IN ('open','close','reanchor')),
  written_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (forward_deployment_id, bar)
);

CREATE TABLE IF NOT EXISTS strategy_forward_fill (
  forward_deployment_id UUID NOT NULL REFERENCES mlops.paper_deployment (deployment_id),
  tenant_id             TEXT NOT NULL CHECK (length(tenant_id) > 0),
  -- 1, 2, 3… per forward deployment, in the order the fills were applied.
  seq                   BIGINT NOT NULL CHECK (seq > 0),
  side                  TEXT NOT NULL CHECK (side IN ('buy','sell')),
  -- This fill's quantity — the delta, never the cumulative figure.
  filled                NUMERIC(38,18) NOT NULL CHECK (filled > 0),
  fill_price            NUMERIC(38,18) NOT NULL CHECK (fill_price > 0),
  charge                NUMERIC(38,18) CHECK (charge IS NULL OR charge >= 0),
  decision_price        NUMERIC(38,18) NOT NULL CHECK (decision_price > 0),
  filled_at             TIMESTAMPTZ NOT NULL,
  written_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (forward_deployment_id, seq)
);

CREATE OR REPLACE FUNCTION strategy_forward_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION '% is append-only (Set T T-0.4)', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_forward_mark_append_only ON strategy_forward_mark;
CREATE TRIGGER trg_strategy_forward_mark_append_only
  BEFORE UPDATE OR DELETE ON strategy_forward_mark
  FOR EACH ROW EXECUTE FUNCTION strategy_forward_append_only();

DROP TRIGGER IF EXISTS trg_strategy_forward_fill_append_only ON strategy_forward_fill;
CREATE TRIGGER trg_strategy_forward_fill_append_only
  BEFORE UPDATE OR DELETE ON strategy_forward_fill
  FOR EACH ROW EXECUTE FUNCTION strategy_forward_append_only();

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') THEN
    GRANT SELECT, INSERT ON strategy_forward_mark, strategy_forward_fill TO app_role;
    -- 0044's default privileges grant every new public table DELETE and UPDATE
    -- to app_role; an append-only table takes them back (as 0167 does).
    REVOKE UPDATE, DELETE ON strategy_forward_mark, strategy_forward_fill FROM app_role;
  END IF;
END $$;
