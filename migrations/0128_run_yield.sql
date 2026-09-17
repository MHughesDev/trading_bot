-- 0128 — what the run produced (AGENT-013 P5, §10; Set S task S-10.13).
--
-- ## The failure this measures
--
-- A run can end having produced no knowledge while every dashboard shows
-- activity. `agent_runs` already records steps, tokens, cost and trials — none
-- of which is an output. A run that spent nine trials and wrote nothing down was
-- indistinguishable, on every surface this platform had, from one that spent
-- nine trials and closed a question.
--
-- ## Counted here, weighted in Rust
--
-- The columns are counts. `knowledge_units` is deliberately **not** one of them:
-- it is `ledger`-side arithmetic over these counts (`harness::lifecycle::r#yield`),
-- and storing the derived number as well would be a second answer that drifts the
-- first time the weighting changes — a weighting that is going to change, because
-- a yield metric is a thing runs get optimised against the moment it exists.
--
-- `barren` is stored, though, and that is not a contradiction: it is the flag the
-- scorecard groups by and the usage page filters on, so it has to be indexable.
-- `a_barren_run_spent_a_trial` is the CHECK that keeps it honest — a run that
-- spent nothing cannot be barren, because barren means the platform's scarcest
-- resource was consumed and nothing came back.

ALTER TABLE agent_runs
  ADD COLUMN IF NOT EXISTS findings_established INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS findings_suggestive INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS findings_refuted INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS negatives INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS negatives_new_class INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS questions_opened INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS questions_closed INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS contradictions_surfaced INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS proposals INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS hypotheses_tested INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS hypotheses_refuted INT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS hypotheses_supported INT NOT NULL DEFAULT 0,
  -- NULL until the run finishes. A run still going is not barren, and a default
  -- of `false` would say it was not while nobody had looked.
  ADD COLUMN IF NOT EXISTS barren BOOLEAN;

-- Counts do not go backwards.
ALTER TABLE agent_runs
  ADD CONSTRAINT yield_counts_are_not_negative CHECK (
    findings_established >= 0
    AND findings_suggestive >= 0
    AND findings_refuted >= 0
    AND negatives >= 0
    AND negatives_new_class >= 0
    AND questions_opened >= 0
    AND questions_closed >= 0
    AND contradictions_surfaced >= 0
    AND proposals >= 0
    AND hypotheses_tested >= 0
    AND hypotheses_refuted >= 0
    AND hypotheses_supported >= 0
  );

-- A negative whose class was new for the subject is still a negative, so the
-- bonus count can never exceed the count it is a subset of.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_new_class_negative_is_a_negative CHECK (
    negatives_new_class <= negatives
  );

-- Same shape for hypotheses: refuted and supported are both outcomes of testing
-- one, so neither can exceed the tests and the two together cannot either.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_decided_hypothesis_was_tested CHECK (
    hypotheses_refuted + hypotheses_supported <= hypotheses_tested
  );

-- Barren means the scarcest resource was consumed and nothing came back. A run
-- that spent no trial is idle, or answering a lookup, or refused — none of which
-- is the failure this flags, and calling any of them barren would bury the one
-- that matters under the ones that do not.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_barren_run_spent_a_trial CHECK (
    barren IS NOT TRUE OR trials_spent > 0
  );

-- The scorecard groups by (tier, workflow) and never averages across them:
-- "12% barren" over everything is a number nobody can act on, and "`survey` on
-- the 7B is barren 60% of the time" names a workflow and a tier.
CREATE INDEX IF NOT EXISTS agent_runs_barren_idx
  ON agent_runs (workflow, barren, finished_at DESC)
  WHERE barren IS NOT NULL;
