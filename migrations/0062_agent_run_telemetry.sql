-- What a finished run has to be able to answer (AGENT-010 §4, AGENT-005 §7,
-- AGENT-009 §5; Set S task S-2.8).
--
-- These ten are not reporting columns. Each one is read by a later phase to answer a
-- question that cannot be answered any other way once the run is over:
--
--   * the budget only works if the ESTIMATE can be compared against what the provider
--     actually counted. S-1.18 chose p90 for the calibration factor; the ratio it is
--     a percentile of is `est_prompt_tokens / prompt_tokens`, and neither number
--     survives the run without a column;
--   * `prompt_tokens` is NOT `tokens_in`. With caching on, a 40,000-token prompt of
--     which 39,000 was a cache hit bills 1,000 — so `tokens_in` answers "what did this
--     cost" and cannot answer "how big was the prompt". Both are wanted and they are
--     different numbers;
--   * a run's honesty is `trials_spent` against `n_eff_after`. A run that spent four
--     trials and moved N_eff by four searched; one that spent four and moved it by
--     one searched the same idea four times, and the difference is invisible without
--     both;
--   * `harness_version`, `adapter` and `transport` are what makes a comparison
--     between two runs legitimate. Two runs on different transports are two different
--     experiments, and an eval that averages them is measuring the mixture.
--
-- All ten are nullable. A run that crashed before it could fill them is still a run,
-- and NOT NULL here would mean either refusing to record the crash or inventing a
-- value for it — and an invented zero in `trials_spent` is worse than a null, because
-- a null cannot be averaged by accident.

ALTER TABLE agent_runs
    -- The harness that drove it. A comparison across versions is not a comparison,
    -- and S-2.6 refuses to replay a log written by a different one.
    ADD COLUMN IF NOT EXISTS harness_version   TEXT,
    -- The profile by NAME. `profile_id` already exists and is an id; a name is what a
    -- person reads in an eval scorecard, and an id that was renamed underneath is an
    -- id that silently changes what a historical row meant.
    ADD COLUMN IF NOT EXISTS profile_name      TEXT,
    -- Which `ModelAdapter` served it, and how it decoded.
    ADD COLUMN IF NOT EXISTS adapter           TEXT,
    ADD COLUMN IF NOT EXISTS transport         TEXT,

    -- How big the prompts were, summed: fresh + cache-read + cache-written. This is
    -- `llm::Usage::prompt_tokens`, and it is the number a budget may be compared
    -- against.
    ADD COLUMN IF NOT EXISTS prompt_tokens     BIGINT
        CHECK (prompt_tokens IS NULL OR prompt_tokens >= 0),
    -- What the harness THOUGHT they would be, by the same arithmetic the assembler
    -- budgeted with. The pair is the calibration (S-1.18); one without the other is
    -- a number with nothing to check it.
    ADD COLUMN IF NOT EXISTS est_prompt_tokens BIGINT
        CHECK (est_prompt_tokens IS NULL OR est_prompt_tokens >= 0),
    ADD COLUMN IF NOT EXISTS cache_read_tokens BIGINT
        CHECK (cache_read_tokens IS NULL OR cache_read_tokens >= 0),

    -- The honesty pair. Trials spent, and where the family's effective trial count
    -- ended up.
    ADD COLUMN IF NOT EXISTS trials_spent      INT
        CHECK (trials_spent IS NULL OR trials_spent >= 0),
    ADD COLUMN IF NOT EXISTS n_eff_after       DOUBLE PRECISION
        CHECK (n_eff_after IS NULL OR n_eff_after >= 0),

    -- How it ended, as one word from the loop's own vocabulary. `outcome_json` holds
    -- the whole `Outcome`; this is the part an index can group by, because "how many
    -- runs fenced last week" should not require parsing every row.
    ADD COLUMN IF NOT EXISTS outcome           TEXT;

-- The read every eval does: outcomes by profile over a window.
CREATE INDEX IF NOT EXISTS agent_runs_outcome_idx
    ON agent_runs (profile_name, outcome, created_at DESC)
    WHERE outcome IS NOT NULL;

-- The read the calibration does: runs that carry both halves of the ratio.
CREATE INDEX IF NOT EXISTS agent_runs_calibration_idx
    ON agent_runs (harness_version, created_at DESC)
    WHERE prompt_tokens IS NOT NULL AND est_prompt_tokens IS NOT NULL;

COMMENT ON COLUMN agent_runs.prompt_tokens IS
    'How big the prompts were, summed across the run: fresh + cache-read + '
    'cache-written. NOT tokens_in, which is what was billed — with caching on the '
    'two differ by the cache hit rate, and only this one may be compared against a '
    'context budget.';

COMMENT ON COLUMN agent_runs.est_prompt_tokens IS
    'What the harness estimated for the same requests. The pair (est, actual) is the '
    'ratio S-1.18 calibrates on; either number alone has nothing to check it.';

COMMENT ON COLUMN agent_runs.n_eff_after IS
    'The family N_eff after this run. With trials_spent it says whether the run '
    'searched or re-searched: four trials that moved N_eff by four is a search, four '
    'that moved it by one is the same idea four times.';
