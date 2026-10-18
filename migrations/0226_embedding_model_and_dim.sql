-- 0226 — a stored vector names the model and the dimension that made it (Set R-b RB-1.5,
-- KM-06; AGENT-003 section 4).
--
-- knowledge.insight.embedding and agent_memory.entry.embedding are vector(768) columns
-- with nothing saying *which* model produced a row's vector. A second model of the same
-- width would be compared, silently, with the first model's vectors, and a similarity
-- between two models' coordinates means nothing. These columns make the model part of the
-- row, and a trigger makes "a vector with no model" unwritable.
--
-- A trigger rather than a CHECK, because a CHECK (even NOT VALID) is re-evaluated on
-- every UPDATE of a row, and rows embedded before this migration have a vector and no
-- model: a decay-score update of such a row would fail on a rule about a column it did not
-- touch. The trigger fires only when the embedding column is written, so legacy rows are
-- left alone until the embedding pass re-embeds them. Their NULL model differs from every
-- real model name, so the pass picks them up and retrieval ranks them as unembedded until
-- it has.
--
-- knowledge.insight is under forced row-level security, so a backfill here would see no
-- rows; re-embedding is the backfill.

CREATE OR REPLACE FUNCTION public.require_embedding_model()
RETURNS TRIGGER AS $$
BEGIN
  IF NEW.embedding IS NULL THEN
    NEW.embedding_model := NULL;
    NEW.embedding_dim := NULL;
  ELSIF NEW.embedding_model IS NULL OR btrim(NEW.embedding_model) = '' OR NEW.embedding_dim IS DISTINCT FROM 768 THEN
    RAISE EXCEPTION 'a stored vector must name its model and be 768-dimensional (model %, dim %)',
      NEW.embedding_model, NEW.embedding_dim;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

ALTER TABLE knowledge.insight
  ADD COLUMN IF NOT EXISTS embedding_model TEXT,
  ADD COLUMN IF NOT EXISTS embedding_dim   INT;

DROP TRIGGER IF EXISTS trg_insight_embedding_names_its_model ON knowledge.insight;
CREATE TRIGGER trg_insight_embedding_names_its_model
  BEFORE INSERT OR UPDATE OF embedding ON knowledge.insight
  FOR EACH ROW EXECUTE FUNCTION public.require_embedding_model();

ALTER TABLE agent_memory.entry
  ADD COLUMN IF NOT EXISTS embedding_model TEXT,
  ADD COLUMN IF NOT EXISTS embedding_dim   INT;

DROP TRIGGER IF EXISTS trg_entry_embedding_names_its_model ON agent_memory.entry;
CREATE TRIGGER trg_entry_embedding_names_its_model
  BEFORE INSERT OR UPDATE OF embedding ON agent_memory.entry
  FOR EACH ROW EXECUTE FUNCTION public.require_embedding_model();
