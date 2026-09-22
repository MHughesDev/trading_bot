-- 0118 — the insight store's read path (SPEC §5.6, checklist 3.10, AGENT-007
-- §2.2/§2.4/§4, ADR-P3-08…P3-11).
--
-- 0055 created `knowledge.insight` and granted it. Nothing read it and nothing
-- aged it, which makes it a write-only log — the same thing as no memory at
-- all. This migration adds the columns retrieval needs and the log that makes
-- retrieval auditable.
--
-- Four ideas, each a column or a constraint rather than a convention:
--
--   1. **Metadata before similarity.** `scope` already holds the structured
--      coordinates; it gains a GIN index so a containment filter runs *before*
--      any distance is computed. An insight about BTC 1h trend is not relevant
--      to an ETH 1d mean-reversion question however similar the sentences are,
--      and a pure-kNN store answers that question wrongly with confidence.
--
--   2. **Staleness is shown, never silently applied.** `window_start`,
--      `window_end`, `valid_until` and `platform_capabilities` are what let
--      retrieval say "this claim is about a period that ended eight months ago"
--      or "this claim predates the execution model it assumes". Dropping stale
--      rows quietly is the degradation SPEC §5.6 names; flagging them is not.
--
--   3. **Decay removes from retrieval, never from the table.** There is no
--      DELETE path here. `archived_at` and the retrieval floor both take a row
--      out of the answer set while leaving the row addressable: "we believed
--      this and stopped" is itself knowledge, and a claim that vanishes when
--      contradicted can be rediscovered and re-believed forever.
--
--   4. **Memory is not evidence.** `knowledge.memory_retrieval.purpose` has a
--      CHECK admitting exactly one value. A retrieval row that claims to be
--      evidence for an Experiment cannot be written. That is a mechanism; a
--      paragraph in a design doc is not.
--
-- Deliberately **no** index on `insight.embedding` (ADR-P3-05): exact kNN over
-- a metadata-filtered set is about a millisecond at this scale, and an
-- approximate index's recall depends on its own build state, which makes a
-- neighbour list something an attacker can probe.

-- ── retrieval metadata ──────────────────────────────────────────────────────
ALTER TABLE knowledge.insight
  ADD COLUMN IF NOT EXISTS tags                  TEXT[]      NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS window_start          TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS window_end            TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS valid_until           TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS platform_capabilities TEXT[]      NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS embedding_version     TEXT,
  ADD COLUMN IF NOT EXISTS archived_at           TIMESTAMPTZ;

COMMENT ON COLUMN knowledge.insight.tags IS
  'Free tags that travel with the claim. `tainted` marks a claim derived from '
  'untrusted text and MUST survive retrieval — a taint that is dropped on read '
  'is worse than no taint, because the reader now believes it was checked.';
COMMENT ON COLUMN knowledge.insight.platform_capabilities IS
  'What the platform could do when the claim was formed. A claim that assumed '
  'market orders only is not a current claim once limit orders exist.';
COMMENT ON COLUMN knowledge.insight.archived_at IS
  'Out of retrieval, still addressable by id (AGENT-007 §2.4). Never a DELETE.';

-- An embedding with no version is a vector nobody can reproduce or compare
-- against the next one. They travel together or not at all.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'insight_embedding_versioned'
  ) THEN
    ALTER TABLE knowledge.insight
      ADD CONSTRAINT insight_embedding_versioned
      CHECK ((embedding IS NULL) = (embedding_version IS NULL));
  END IF;
END $$;

-- Metadata first: this index is what the WHERE clause uses, and it runs before
-- a single distance is computed.
CREATE INDEX IF NOT EXISTS idx_insight_scope_gin
  ON knowledge.insight USING gin (scope jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_insight_tags_gin
  ON knowledge.insight USING gin (tags);
CREATE INDEX IF NOT EXISTS idx_insight_retrievable
  ON knowledge.insight (tenant_id, tier, decay_score DESC)
  WHERE archived_at IS NULL;

-- ── the retrieval log (AGENT-007 §4) ────────────────────────────────────────
--
-- What was retrieved, for which run, why, and whether it was later cited. Two
-- jobs: it is AGENT-009's memory-recall signal, and it is the audit trail that
-- answers "did a belief enter this result" without having to reason about it.
CREATE TABLE IF NOT EXISTS knowledge.memory_retrieval (
  retrieval_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id    TEXT NOT NULL,
  run_id       TEXT,
  query_hash   TEXT NOT NULL,
  insight_id   UUID NOT NULL REFERENCES knowledge.insight(insight_id),
  rank         INT  NOT NULL CHECK (rank >= 0),
  why          TEXT NOT NULL,
  -- The whole point. Retrieved memory informs hypothesis generation and
  -- nothing else; it is never evidence in an Experiment, because evidence is
  -- computed from Studies that were actually run. A caller that wants to
  -- record a retrieval as evidence has to get past a CHECK, and cannot.
  purpose      TEXT NOT NULL DEFAULT 'hypothesis_generation'
                 CHECK (purpose = 'hypothesis_generation'),
  cited        BOOLEAN NOT NULL DEFAULT FALSE,
  retrieved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_memory_retrieval_run
  ON knowledge.memory_retrieval (tenant_id, run_id, retrieved_at DESC);

ALTER TABLE knowledge.memory_retrieval ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.memory_retrieval FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.memory_retrieval;
CREATE POLICY tenant_isolation ON knowledge.memory_retrieval
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT, UPDATE ON knowledge.memory_retrieval TO app_role, agent_role;
