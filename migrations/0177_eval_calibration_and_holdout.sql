-- 0177 -- the evaluation side's own state: token calibration and the eval holdout
-- (Set S tasks S-4.12 and S-4.17; AGENT-009 section 4.2 and section 5, P7).
--
-- Two tables, one migration, because both are "the eval harness remembers
-- something about itself" and both have the same grant to get right.
--
--   agent_token_calibration  what the context assembler's chars/4 estimate gets
--                            wrong, per model, measured from real runs
--                            (agent_runs.prompt_tokens vs est_prompt_tokens, 0062).
--   eval_holdout_draw        every draw of the sealed 20% of each eval suite, with
--                            the harness version that asked for it. A re-draw that
--                            is not recorded is a holdout nobody can audit.
--
-- Both GRANT to app_role in this file. 0044 moved the runtime pool onto a member
-- of app_role, and tests connect as the owner, so a missing grant passes CI and
-- fails in the running platform (the defect 0145 and 0166 fixed twice). The test
-- that covers this file switches to app_role before it touches either table.

CREATE TABLE IF NOT EXISTS agent_token_calibration (
  model            TEXT PRIMARY KEY CHECK (btrim(model) <> ''),
  -- chars-per-token the estimator should divide by for this model. The harness
  -- prior is 4.0; the clamp is AGENT-009 section 5: a measurement outside
  -- [2.5, 5.0] is a bad sample, not a tokenizer.
  bytes_per_token  DOUBLE PRECISION NOT NULL
                     CHECK (bytes_per_token >= 2.5 AND bytes_per_token <= 5.0),
  -- Fewer than ten runs is a number about those runs, and a row is not written.
  sample_n         INTEGER NOT NULL CHECK (sample_n >= 10),
  -- Share of the sampled runs the calibrated estimate still OVER-estimated.
  -- The budget suite reads this; section 5 wants it to stay at 0.95 or above.
  over_estimate_rate DOUBLE PRECISION NOT NULL
                     CHECK (over_estimate_rate >= 0 AND over_estimate_rate <= 1),
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS eval_holdout_draw (
  id               BIGSERIAL PRIMARY KEY,
  salt             TEXT NOT NULL CHECK (btrim(salt) <> ''),
  -- Which harness version asked for the draw. "Who wanted another look" is the
  -- question a holdout discipline exists to answer (AGENT-009 section 4.2).
  requested_by     TEXT NOT NULL CHECK (btrim(requested_by) <> ''),
  harness_version  TEXT NOT NULL CHECK (btrim(harness_version) <> ''),
  drawn_on         DATE NOT NULL,
  corpus_size      INTEGER NOT NULL CHECK (corpus_size > 0),
  sealed_ids       TEXT[] NOT NULL CHECK (cardinality(sealed_ids) >= 1),
  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- One draw per salt: changing the salt is the only way to re-draw, so the same
  -- salt twice is the same draw recorded twice.
  CONSTRAINT eval_holdout_draw_salt_unique UNIQUE (salt)
);

-- Append-only. A draw is a fact about the past; correcting it is a new draw with
-- a new salt, which is exactly the visible edit the discipline wants.
CREATE OR REPLACE FUNCTION eval_holdout_draw_immutable() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % refused', TG_TABLE_NAME, TG_OP;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_eval_holdout_draw_immutable ON eval_holdout_draw;
CREATE TRIGGER trg_eval_holdout_draw_immutable
  BEFORE UPDATE OR DELETE ON eval_holdout_draw
  FOR EACH ROW EXECUTE FUNCTION eval_holdout_draw_immutable();

-- Every attempt to promote a context policy, win or lose (S-9.12). A losing v2 is
-- recorded with the weights and the corpus fingerprint that produced it, so a
-- re-fit that changes the weights without changing the corpus shows up as exactly
-- that rather than as a policy that quietly started passing.
CREATE TABLE IF NOT EXISTS context_policy_promotion (
  id                 BIGSERIAL PRIMARY KEY,
  policy_version     TEXT NOT NULL CHECK (btrim(policy_version) <> ''),
  weights_json       JSONB NOT NULL CHECK (jsonb_typeof(weights_json) = 'object'),
  corpus_size        INTEGER NOT NULL CHECK (corpus_size >= 0),
  corpus_fingerprint TEXT NOT NULL CHECK (btrim(corpus_fingerprint) <> ''),
  promoted           BOOLEAN NOT NULL,
  refused            TEXT[] NOT NULL DEFAULT '{}',
  report             TEXT[] NOT NULL DEFAULT '{}',
  harness_version    TEXT NOT NULL CHECK (btrim(harness_version) <> ''),
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- A promotion is a promotion: it has no refusals. A refusal has at least one.
  CONSTRAINT promotion_matches_refusals CHECK (promoted = (cardinality(refused) = 0))
);

DROP TRIGGER IF EXISTS trg_context_policy_promotion_immutable ON context_policy_promotion;
CREATE TRIGGER trg_context_policy_promotion_immutable
  BEFORE UPDATE OR DELETE ON context_policy_promotion
  FOR EACH ROW EXECUTE FUNCTION eval_holdout_draw_immutable();

-- The grants are what the Rust does: the calibration job upserts (INSERT and
-- UPDATE), the draw recorder only inserts, and nothing deletes.
GRANT SELECT, INSERT, UPDATE ON agent_token_calibration TO app_role;
GRANT SELECT, INSERT ON eval_holdout_draw TO app_role;
GRANT USAGE, SELECT ON SEQUENCE eval_holdout_draw_id_seq TO app_role;
GRANT SELECT, INSERT ON context_policy_promotion TO app_role;
GRANT USAGE, SELECT ON SEQUENCE context_policy_promotion_id_seq TO app_role;

-- memory_retrievals (0124) is read by the monthly memory report and by the
-- retention sweep (S-4.16). 0044's default privileges cover tables created by the
-- same owner afterwards, but a database restored from a dump does not carry them,
-- so the grant is made explicitly. Guarded: a database that never ran 0124 has no
-- such table.
DO $$
BEGIN
  IF to_regclass('public.memory_retrievals') IS NOT NULL THEN
    GRANT SELECT, INSERT, UPDATE ON public.memory_retrievals TO app_role;
    GRANT USAGE, SELECT ON SEQUENCE public.memory_retrievals_id_seq TO app_role;
  END IF;
END
$$;
