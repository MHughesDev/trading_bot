-- 0144 — the decision journal (Set U U-4.5; ADR-0047).
--
-- One record per decision per deployment, carrying enough to replay the step:
-- the event and the account the engine saw (`inputs`), the full decision trace
-- (every node's value, every model request and answer, every rule's outcome,
-- the resolved intents, the planned orders, the refusals), what the risk gate
-- said about each order, and the `missing` reasons that held a rule. Phase 8's
-- nightly job replays paper through the backtest and compares digests.
--
-- Append-only: a journal that could be edited is not evidence. Rows leave only
-- by age, through the retention prune, per the data budget.

CREATE TABLE IF NOT EXISTS strategy_decision_journal (
  deployment_id   UUID NOT NULL REFERENCES strategy_deployment (deployment_id),
  -- 1, 2, 3… per deployment, in the order the decisions were made.
  seq             BIGINT NOT NULL CHECK (seq > 0),
  tick            BIGINT NOT NULL CHECK (tick >= 0),
  decided_at      TIMESTAMPTZ NOT NULL,
  cause           TEXT NOT NULL CHECK (cause IN ('bar_close','fill','order_closed','schedule')),
  -- sha256 of the canonical trace — what the parity job compares.
  digest          TEXT NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
  -- sha256 of `inputs`.
  input_digest    TEXT NOT NULL CHECK (input_digest ~ '^[0-9a-f]{64}$'),
  inputs          JSONB NOT NULL,
  trace           JSONB NOT NULL,
  gate_verdicts   JSONB NOT NULL DEFAULT '[]'::jsonb,
  missing_reasons JSONB NOT NULL DEFAULT '{}'::jsonb,
  written_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (deployment_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_strategy_decision_journal_age
  ON strategy_decision_journal (decided_at);

CREATE OR REPLACE FUNCTION strategy_decision_journal_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'strategy_decision_journal is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_decision_journal_append_only ON strategy_decision_journal;
CREATE TRIGGER trg_strategy_decision_journal_append_only
  BEFORE UPDATE ON strategy_decision_journal
  FOR EACH ROW EXECUTE FUNCTION strategy_decision_journal_append_only();
