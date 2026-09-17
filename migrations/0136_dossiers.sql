-- 0136 — the dossier a human reads cold (AGENT-006 §5.8, AGENT-012 §5; Set S
-- task S-8.7). Planned as `0085`; the range in the phase file predates thirty
-- other migrations.
--
-- ## Why it is stored rather than rendered on demand
--
-- Because the point of a dossier is that it can be read in six months. Rendering
-- one from live tables would make it a view of the platform *now* — the runs
-- would have been re-scored, the memory entries superseded, the questions closed
-- by later work — and a reviewer would be reading a document that had quietly
-- changed since the decision it justified was made.
--
-- So a dossier is a snapshot, written once at the end of the run that produced
-- it, and `verify` is the separate, explicit act of asking whether it still
-- holds. The two are different questions and the platform can answer both only
-- if it keeps the first one still.
--
-- ## `evidence` beside `body`
--
-- `body` is the document. `evidence` is the bundle the claims were aligned
-- against — `[{id, text}]` as `harness::report::claims` read it. Kept because
-- `verify` re-derives: it re-renders each evidence id from the platform as it
-- stands and re-runs the alignment, and "the number changed" and "the claim was
-- never supported" are different answers that need the original to tell apart.

CREATE TABLE IF NOT EXISTS mlops.dossier (
    -- The trial this rests on. `harness::report::dossier::compile` refuses a
    -- dossier without one, and this is that value.
    ticket      TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    run_id      UUID,
    -- The wire shape the desk reader renders (AGENT-012 §5).
    body        JSONB NOT NULL,
    -- The evidence bundle as it stood when the claims were aligned.
    evidence    JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_dossier_names_its_trial CHECK (btrim(ticket) <> ''),
    CONSTRAINT a_dossier_has_a_body CHECK (jsonb_typeof(body) = 'object'),
    CONSTRAINT a_dossier_keeps_its_bundle CHECK (jsonb_typeof(evidence) = 'array')
);

-- A snapshot that can be edited is not a snapshot.
DROP TRIGGER IF EXISTS trg_dossier_no_change ON mlops.dossier;
CREATE TRIGGER trg_dossier_no_change
    BEFORE UPDATE OR DELETE ON mlops.dossier
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE INDEX IF NOT EXISTS dossier_run_idx ON mlops.dossier (tenant_id, run_id, created_at DESC);

-- ## Comments
--
-- A reviewer's notes on a dossier, which are not part of the dossier. They are a
-- separate table for the same reason the dossier is immutable: a document that
-- gains sentences after the decision it justified is a document nobody can cite.
--
-- Comments **are** editable in the one way that matters — they can be added —
-- and are never merged into `body`.
CREATE TABLE IF NOT EXISTS mlops.dossier_comment (
    comment_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    ticket      TEXT NOT NULL REFERENCES mlops.dossier(ticket) ON DELETE CASCADE,
    tenant_id   TEXT NOT NULL,
    -- Who wrote it. A person's id, or `agent:<run_id>` — spelled rather than a
    -- boolean, because "a human raised this" and "the critic raised this" are
    -- read very differently and a flag would lose which.
    author      TEXT NOT NULL CHECK (btrim(author) <> ''),
    -- The claim it is about, when it is about one.
    claim       TEXT,
    body        TEXT NOT NULL CHECK (btrim(body) <> ''),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS dossier_comment_idx
    ON mlops.dossier_comment (tenant_id, ticket, created_at);
