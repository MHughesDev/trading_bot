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

-- ## Designs (AGENT-013 §5.5, §6.3; Set S task S-10.5)
--
-- A design binds a hypothesis to the exact thing that will test it, and records
-- what the power check found. `observations` and `required_observations` are
-- stored because the refusal is the interesting row: a design that was refused
-- never gets here, so what is kept is the evidence that the window *was* long
-- enough — which is the half a reader needs when the result later disappoints.

CREATE TABLE IF NOT EXISTS agent_lifecycle.design (
    design_id             UUID PRIMARY KEY,
    run_id                UUID NOT NULL,
    hypothesis_id         UUID NOT NULL,
    -- An exact version. `latest` is refused at the route, because a design bound
    -- to it produces a result nobody can reproduce a week later.
    strategy_version      TEXT NOT NULL,
    split_spec_id         TEXT,
    null_family           TEXT NOT NULL,
    timeframe             TEXT NOT NULL,
    observations          DOUBLE PRECISION NOT NULL,
    required_observations DOUBLE PRECISION,
    bound_at              TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_design_names_an_exact_version CHECK (
        btrim(strategy_version) <> '' AND lower(strategy_version) <> 'latest'
    ),
    CONSTRAINT a_design_names_its_null CHECK (btrim(null_family) <> ''),
    -- A stored design passed the power check, so its window holds at least what
    -- the check demanded. A row that did not would be a design the platform
    -- refused and then wrote down anyway.
    CONSTRAINT a_stored_design_had_the_power_it_needed CHECK (
        required_observations IS NULL OR observations >= required_observations
    )
);

CREATE INDEX IF NOT EXISTS design_run_idx
  ON agent_lifecycle.design (run_id, bound_at DESC);

-- ## Proposals (AGENT-013 P6, §6.8, §7.5; Set S task S-10.9)
--
-- The only doors into rings 1 and 2, and the row is deliberately inert: it
-- changes nothing. A human approves it on the desk and **the registry write
-- happens then, as that human** — not as the agent, not with the agent's token,
-- and not at a moment the agent chose.
--
-- `decided_by` is NOT NULL on any decided row, and that is the load-bearing
-- column: it is what makes "who approved this" a fact rather than a
-- reconstruction from timestamps.

CREATE TABLE IF NOT EXISTS agent_lifecycle.proposal (
    proposal_id  UUID PRIMARY KEY,
    run_id       UUID NOT NULL,
    kind         TEXT NOT NULL,
    -- Derived from the kind in `ledger::proposal`, stored so a reader does not
    -- re-derive it, and CHECKed against the kind here so the two cannot disagree.
    ring         SMALLINT NOT NULL,
    envelope     TEXT NOT NULL,
    -- What the decision is made from. A proposal with no dossier asks a human to
    -- decide from the agent's own summary of the agent's own work.
    dossier_ref  TEXT NOT NULL,
    subject      TEXT NOT NULL,
    rationale    TEXT NOT NULL DEFAULT '',
    decision     TEXT NOT NULL DEFAULT 'pending',
    -- The human. NULL while pending, required once decided.
    decided_by   UUID,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at   TIMESTAMPTZ,

    CONSTRAINT a_proposal_kind_is_one_of_three CHECK (
        kind IN ('promote_strategy', 'change_limits', 'deploy_paper')
    ),
    -- The ring follows the kind. Two columns that could disagree are two answers
    -- to "how far does this reach", and the looser one is the one somebody reads.
    CONSTRAINT the_ring_follows_the_kind CHECK (
        (kind IN ('promote_strategy', 'change_limits') AND ring = 1)
        OR (kind = 'deploy_paper' AND ring = 2)
    ),
    -- Every proposal falls under an approval envelope. `none` is not available:
    -- a proposal that needed no approval would not need to be a proposal.
    CONSTRAINT a_proposal_is_gated CHECK (
        envelope IN ('spend', 'promotion', 'sealed_holdout', 'tier3_memory')
    ),
    CONSTRAINT a_proposal_cites_a_dossier CHECK (btrim(dossier_ref) <> ''),
    CONSTRAINT a_proposal_names_its_subject CHECK (btrim(subject) <> ''),
    CONSTRAINT a_decision_is_one_of_three CHECK (
        decision IN ('pending', 'approved', 'rejected')
    ),
    -- A decided proposal names the human who decided it and when. Pending names
    -- neither. "Who approved this" must be a fact rather than something
    -- reconstructed from timestamps afterwards.
    CONSTRAINT a_decision_names_a_human_and_a_time CHECK (
        (decision = 'pending' AND decided_by IS NULL AND decided_at IS NULL)
        OR (decision <> 'pending' AND decided_by IS NOT NULL AND decided_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS proposal_pending_idx
  ON agent_lifecycle.proposal (created_at DESC)
  WHERE decision = 'pending';

CREATE INDEX IF NOT EXISTS proposal_run_idx
  ON agent_lifecycle.proposal (run_id, created_at DESC);
