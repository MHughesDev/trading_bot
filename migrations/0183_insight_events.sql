-- 0183 — the insight store's confirmations and contradictions, each with evidence
-- (SPEC §5.6, ADR-P3-04, ADR-C-05; OPEN-TASKS C1.5).
--
-- `knowledge.insight` has had `support_n`, `contradicted_n` and `decay_score` since
-- 0055 and nothing wrote them: SPEC §5.6 says stale beliefs are *the dominant
-- long-horizon memory failure* and these columns exist to catch them, and unread
-- and unwritten they catch nothing.
--
-- A confirmation or a contradiction is itself a claim about the world, so it
-- carries the same REQUIRED evidence an insight does: at least one trial id.
-- `cardinality`, not `array_length` — the latter is NULL for an empty array and a
-- CHECK that evaluates to NULL passes (the 0055 trap).
--
-- Append-only by grant: the counts on the insight row are a summary of this log,
-- and a log whose rows could be edited would make the summary unverifiable.
-- RLS is FORCEd on the tenant, as for every table in this schema.

CREATE TABLE IF NOT EXISTS knowledge.insight_event (
  event_id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id          TEXT NOT NULL,
  insight_id         UUID NOT NULL REFERENCES knowledge.insight(insight_id),
  kind               TEXT NOT NULL CHECK (kind IN ('confirm','contradict')),
  evidence_trial_ids UUID[] NOT NULL CHECK (cardinality(evidence_trial_ids) >= 1),
  note               TEXT,
  recorded_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_insight_event_insight
  ON knowledge.insight_event (tenant_id, insight_id, recorded_at);

ALTER TABLE knowledge.insight_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.insight_event FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.insight_event;
CREATE POLICY tenant_isolation ON knowledge.insight_event
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The grants are in the same migration as the table (owner-connected tests cannot
-- see a missing one; `pg_insight_grants` checks them as app_role).
GRANT SELECT, INSERT ON knowledge.insight_event TO app_role, agent_role;
REVOKE UPDATE, DELETE, TRUNCATE ON knowledge.insight_event FROM app_role, agent_role;
