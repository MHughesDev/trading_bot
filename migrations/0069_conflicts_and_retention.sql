-- 0069 — contradictions and retention (AGENT-007 §2.2.1, §2.4, §2.5;
-- Set S tasks S-4.15 and S-4.16).
--
-- ── Why conflicts are an edge table and not an array column ─────────────────
--
-- §2.2 gives every entry a `conflicts_with[]`, and in the first draft nothing
-- computed it — which would have left P5 as a column that is permanently empty.
-- Making it a table rather than an array is what makes the link a *fact about
-- the pair*: it carries the quantity they disagree on, the reason, and who
-- resolved it, and the pair is retrieved together into every brief matching
-- either side.
--
-- **Neither side is downgraded.** §2.2.1 step 3, and it is the least intuitive
-- rule here: a later finding is not more true for being later. Auto-downgrading
-- the older one is exactly how a single bad measurement quietly overwrites a
-- good one, which is the poisoning this whole check exists to prevent.
-- Resolution is a research act — a third finding citing both — and `resolved_by`
-- is the only thing that closes a row.

CREATE TABLE IF NOT EXISTS knowledge.memory_conflict (
  conflict_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id    TEXT NOT NULL,
  -- Ordered so a pair cannot be stored twice under two spellings.
  entry_lo     UUID NOT NULL REFERENCES knowledge.memory_entry(entry_id) ON DELETE CASCADE,
  entry_hi     UUID NOT NULL REFERENCES knowledge.memory_entry(entry_id) ON DELETE CASCADE,
  CONSTRAINT a_conflict_has_two_sides CHECK (entry_lo <> entry_hi),
  CONSTRAINT the_pair_is_ordered CHECK (entry_lo < entry_hi),
  -- What they disagree about. Two findings about different quantities never
  -- contradict, so this is never NULL.
  quantity     TEXT NOT NULL CHECK (length(quantity) > 0),
  why          TEXT NOT NULL CHECK (length(why) > 0),
  detected_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- The third finding that distinguishes them. Only this closes a conflict.
  resolved_by  UUID REFERENCES knowledge.memory_entry(entry_id),
  resolved_at  TIMESTAMPTZ,
  CONSTRAINT resolution_is_a_finding CHECK (
    (resolved_by IS NULL) = (resolved_at IS NULL)
  ),
  UNIQUE (entry_lo, entry_hi, quantity)
);

CREATE INDEX IF NOT EXISTS idx_memory_conflict_open
  ON knowledge.memory_conflict (tenant_id, detected_at DESC)
  WHERE resolved_by IS NULL;

ALTER TABLE knowledge.memory_conflict ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.memory_conflict FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.memory_conflict;
CREATE POLICY tenant_isolation ON knowledge.memory_conflict
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT ON knowledge.memory_conflict TO app_role, agent_role;
GRANT INSERT, UPDATE ON knowledge.memory_conflict TO app_role;

-- ── Retention (§2.4) ────────────────────────────────────────────────────────
--
-- A store that only grows does not fail loudly. Retrieval ranks, so the brief
-- keeps filling — with old, narrowly-true findings — and the agent's
-- understanding gets *worse* over time while every dashboard stays green.
--
-- There is no DELETE anywhere below. `archived` means out of retrieval, still
-- addressable by id, still resolvable in the desk: the same append-only
-- discipline the ledger has, applied to the knowledge that cites it.

CREATE TABLE IF NOT EXISTS knowledge.memory_retention_run (
  run_at        TIMESTAMPTZ PRIMARY KEY DEFAULT now(),
  tenant_id     TEXT NOT NULL,
  total         INT NOT NULL,
  live          INT NOT NULL,
  marked_stale  INT NOT NULL,
  archived      INT NOT NULL,
  -- §2.4's last paragraph: a degrading brief becomes a number before it becomes
  -- bad research, which is the only point at which anyone can act on it.
  cited_share   DOUBLE PRECISION NOT NULL CHECK (cited_share BETWEEN 0 AND 1),
  by_kind       JSONB NOT NULL DEFAULT '{}'::jsonb
);

ALTER TABLE knowledge.memory_retention_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.memory_retention_run FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.memory_retention_run;
CREATE POLICY tenant_isolation ON knowledge.memory_retention_run
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON knowledge.memory_retention_run TO app_role;

-- Whether an evidence chain points at an entry. Nothing this returns TRUE for is
-- ever archived (§2.4's last line), so an old dossier's citations do not break.
CREATE OR REPLACE FUNCTION knowledge.cited_by_evidence_chain(p_entry_id UUID)
RETURNS BOOLEAN AS $$
  SELECT EXISTS (
    SELECT 1 FROM knowledge.memory_entry parent
     WHERE parent.evidence_ids::text LIKE '%' || p_entry_id::text || '%'
       AND parent.entry_id <> p_entry_id
  ) OR EXISTS (
    SELECT 1 FROM knowledge.memory_conflict c
     WHERE (c.entry_lo = p_entry_id OR c.entry_hi = p_entry_id)
       AND c.resolved_by IS NULL
  );
$$ LANGUAGE sql STABLE;

GRANT EXECUTE ON FUNCTION knowledge.cited_by_evidence_chain(UUID) TO app_role;
