-- 0239 -- the comments an author wrote on a strategy (S-5.15; AGENT-011 section 3).
--
-- "Comments are part of the language": a reviewer, the model on its next turn and
-- whoever opens the dossier in six months cannot recover why a threshold is 0.6
-- from the number. Until now they were parsed, addressed (`strategy_lang::intent`)
-- and then dropped at the stored form, because ADR-S5-07 refused a comment field
-- on `domain::strategy_def`: a comment-like field there would make two
-- behaviourally identical documents hash differently, and every rewording of a
-- note would be a new revision and a new apparent trial.
--
-- This table keeps them BESIDE the definition instead, keyed by the revision
-- hash. The hash is computed over the canonical definition, which does not
-- contain a comment, so storing, rewording or deleting a note changes nothing
-- the ledger or `N_eff` can see. That is the point, and it is why this is a side
-- table and not a column on `strategy_revision`.
--
-- Last text wins: the same logic saved again with different comments replaces the
-- notes for that (user, revision). Comments are the author's, so the key carries
-- the user; two tenants who happen to hold the same content hash do not see each
-- other's notes.

CREATE TABLE IF NOT EXISTS strategy_revision_notes (
  user_id       UUID NOT NULL,
  revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
  -- `header`, `param fast`, `entry.long[0]`, `exit[2]` ... as `intent::notes` addresses them.
  clause        TEXT NOT NULL CHECK (btrim(clause) <> ''),
  lines         TEXT[] NOT NULL CHECK (cardinality(lines) >= 1),
  -- Display order: the source order of the clauses.
  position      INTEGER NOT NULL CHECK (position >= 0),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, revision_hash, clause)
);

GRANT SELECT, INSERT, UPDATE, DELETE ON strategy_revision_notes TO app_role;
