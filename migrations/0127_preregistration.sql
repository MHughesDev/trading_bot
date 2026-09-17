-- 0127 — a run commits before it looks (AGENT-013 §7.2, P2; Set S task S-10.3).
--
-- ## The door this closes
--
-- `mlops.trial.prereg_hash` already locks the effect size, the policy and the
-- campaign before the first execution. What it does not carry is **the
-- falsifier**, and that is the largest door left open: without one, a run can
-- look at a result and then decide what would have counted as failure. Every
-- statistic downstream still computes, the trial count is still honest, and the
-- conclusion is still worthless — because "it worked" and "we kept looking until
-- something worked" leave the same trace.
--
-- ## Append-only, like the ledger it sits beside
--
-- There is no `UPDATE` path. An amendment is a **new row** naming what it
-- supersedes, so the lineage is readable and a registration cannot be quietly
-- rewritten after a result. The same reason `mlops.trial` is append-only: a
-- record that can be edited after the fact is a record of the present, not of
-- what was committed to.
--
-- `trial_budget` carries over on an amendment, enforced in
-- `ledger::prereg::amend` rather than here, because "the same number as the row
-- you superseded" is a statement about two rows and a CHECK sees one. The
-- amendment cap is the constraint that makes it matter, and that one *is* here.

CREATE SCHEMA IF NOT EXISTS agent_lifecycle;

CREATE TABLE IF NOT EXISTS agent_lifecycle.preregistration (
    prereg_id           UUID PRIMARY KEY,
    run_id              UUID NOT NULL,
    -- SPEC §12.1: a vector and its constraints, never a scalar score.
    objective           JSONB NOT NULL,
    benchmark           JSONB NOT NULL,
    -- REQUIRED and positive. No default, for the reason `campaign.delta_practical`
    -- has none: a platform-wide default is a decision nobody made about what
    -- improvement is worth acting on, inherited silently by everything after it.
    delta_practical     DOUBLE PRECISION NOT NULL,
    window_start        TEXT NOT NULL,
    window_end          TEXT NOT NULL,
    -- Computed from the pipeline (SPEC §12.2), never typed by a caller.
    embargo_bars        INT NOT NULL,
    cost_model_version  TEXT NOT NULL,
    null_family         TEXT NOT NULL,
    trial_budget        INT NOT NULL,
    gate_profile        TEXT NOT NULL,
    primary_hypothesis  TEXT NOT NULL,
    -- The result that would refute it. The field this table exists for.
    falsifier           TEXT NOT NULL,
    -- `ledger::prereg::Preregistration::hash`, which includes the falsifier.
    prereg_hash         TEXT NOT NULL,
    supersedes          UUID REFERENCES agent_lifecycle.preregistration(prereg_id),
    amendment           INT NOT NULL DEFAULT 0,
    sealed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_delta_practical_is_positive CHECK (
        delta_practical > 0.0 AND delta_practical = delta_practical
    ),
    CONSTRAINT the_window_runs_forwards CHECK (window_start <= window_end),
    -- Empty strings are the shape a "required" field arrives in when nothing
    -- checked it. Each of these is required because a registration missing it is
    -- not a registration.
    CONSTRAINT a_registration_states_its_falsifier CHECK (btrim(falsifier) <> ''),
    CONSTRAINT a_registration_states_its_hypothesis CHECK (btrim(primary_hypothesis) <> ''),
    CONSTRAINT a_registration_names_its_null CHECK (btrim(null_family) <> ''),
    CONSTRAINT a_registration_names_its_cost_model CHECK (btrim(cost_model_version) <> ''),
    CONSTRAINT a_registration_names_its_gate_profile CHECK (btrim(gate_profile) <> ''),
    -- A first registration supersedes nothing and an amendment names what it
    -- replaces. The pair is what makes the lineage readable from any row.
    CONSTRAINT lineage_matches_the_amendment_count CHECK (
        (amendment = 0 AND supersedes IS NULL)
        OR (amendment > 0 AND supersedes IS NOT NULL)
    ),
    -- A protocol rewritten three times is not a protocol, and the run ends
    -- `protocol_unstable`. Enforced here as well as in `prereg::amend` because
    -- the Rust rule holds until the second writer — the same argument `0122`
    -- makes about provenance.
    CONSTRAINT a_run_amends_at_most_twice CHECK (amendment < 3)
);

-- The registration a run is currently under is the one with no successor. Looked
-- up per run on every ticket, so it is indexed that way.
CREATE INDEX IF NOT EXISTS preregistration_run_idx
    ON agent_lifecycle.preregistration (run_id, amendment DESC);

-- Every ticket a run spends carries the hash, so the hash is looked up by value.
CREATE INDEX IF NOT EXISTS preregistration_hash_idx
    ON agent_lifecycle.preregistration (prereg_hash);

-- One first registration per run. A second `amendment = 0` row would give the
-- run two roots and make the lineage ambiguous — which is a way to seal a
-- second, more convenient protocol beside the first rather than amending it.
CREATE UNIQUE INDEX IF NOT EXISTS preregistration_one_root_per_run
    ON agent_lifecycle.preregistration (run_id)
    WHERE amendment = 0;
