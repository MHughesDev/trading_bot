-- 0132 — the gate reserves a look; the compute site registers the trial
-- (AGENT-013 §7.2, INV-16, SPEC §4.5; ADR-S10-10, resolves OQ-20).
--
-- ## The question this table answers, which nothing answered
--
-- `CallGate::admit` posted to `POST /api/ledger/trials` and that route did not
-- exist, so every tool declaring a trial was refused and the research loop could
-- not run. The route could not be written because `mlops.trial`'s required
-- columns — `config_hash`, `config`, `dataset_id`, `code_hash`, `image_digest`,
-- `seed_set` — are all facts about **compute**, and a tool call has chosen none
-- of them. `run_sweep` arrives with arguments; the sweep picks the configs and
-- the data slice afterwards.
--
-- The two halves are two questions:
--
--   * *May this run spend, and has it spent one?* — about the run and the call.
--   * *What compute ran, under what config, over what data?* — about the Run.
--
-- `mlops.trial` has always answered the second. Nothing answered the first.
--
-- ## Why not fold it into `mlops.trial`
--
-- Two grains in one table changes what `trials_counted`, the exploration floor's
-- denominator and `N_eff` mean — and every selection-adjusted statistic on this
-- platform reads one of those. A separate table leaves all of them reading
-- exactly what they read before.
--
-- ## Why not drop the gate's row instead
--
-- `report_kill` and `report_evaluator_defect` declare a trial and post to
-- `/api/falsification/kills`, which registers nothing. Letting the compute site
-- be the only registrar would make two of the seven trial-spending tools spend
-- nothing at all — a silent under-count, which is the direction that matters.
--
-- ## Settlement is not wired, and this file does not pretend otherwise
--
-- `settled_trial_id` exists and **nothing sets it**: threading the reservation
-- id through tool → API → suite → Run is a separate change. So an authorised
-- look that never ran is not yet distinguishable from one that did. The column
-- is here because adding it later would mean a second migration over a table
-- that is append-only by then, and because its absence would be read as "this
-- design has no settlement" rather than "settlement is owed".

CREATE TABLE IF NOT EXISTS mlops.trial_reservation (
    reservation_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    -- The run that made the call. Nullable: a front door with no agent run
    -- behind it still spends against a budget, and refusing the reservation for
    -- want of a run id would refuse the call.
    run_id          UUID,

    tool            TEXT NOT NULL,
    -- Stamped so evidence stays comparable across a tool's behaviour changes
    -- (AGENT-006 §2.7), the same reason the ticket carries it.
    tool_version    INT  NOT NULL,
    -- `TrialKind` as the catalogue declares it.
    kind            TEXT NOT NULL,

    -- The run's sealed pre-registration. **NOT NULL**, which is the strengthening
    -- this table makes possible: AGENT-013 §7.2 says every ticket a sealed run
    -- spends carries the hash, and until now no path delivered one — the gate
    -- carried it on `TrialRequest` and the registrar never put it in the body.
    --
    -- Safe as NOT NULL because the two conditions coincide: `Prereg::NotRequired`
    -- is set only for `TaskDemand::SingleLookup`, and so is `Spending::ChatTurn`,
    -- which refuses before the registrar is reached. A trial-spending call that
    -- gets here has always sealed one.
    prereg_hash     TEXT NOT NULL,

    -- The call's arguments by content. The audit envelope holds the arguments
    -- themselves; this is here so a reservation can be matched to the compute it
    -- authorised without a second copy of a payload that may be large.
    arguments_hash  TEXT NOT NULL,

    -- Set when the compute this authorised registers its own trial. Nothing sets
    -- it yet (ADR-S10-10).
    settled_trial_id UUID REFERENCES mlops.trial(trial_id),
    settled_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_reservation_names_its_tool CHECK (btrim(tool) <> ''),
    CONSTRAINT a_reservation_carries_its_registration CHECK (btrim(prereg_hash) <> ''),
    CONSTRAINT a_reservation_hashes_its_arguments CHECK (btrim(arguments_hash) <> ''),
    -- The catalogue's `TrialKind`, minus `none`: a reservation exists because a
    -- tool declared a trial, so "no trial" is not a value it can hold.
    CONSTRAINT a_reservation_kind_is_one_the_catalogue_declares CHECK (
        kind IN ('search', 'falsification')
    ),
    -- Settled names both, or neither. A `settled_at` with no trial is a row
    -- claiming the look ran and declining to say what ran.
    CONSTRAINT a_settled_reservation_names_its_trial CHECK (
        (settled_trial_id IS NULL AND settled_at IS NULL)
        OR (settled_trial_id IS NOT NULL AND settled_at IS NOT NULL)
    )
);

-- A reservation is a record of what was authorised. Editing one afterwards would
-- make it a record of the present, which is the argument `mlops.trial` makes for
-- itself — except for settlement, which is the one legitimate later write, so
-- this refuses DELETE and lets UPDATE through rather than blocking both.
DROP TRIGGER IF EXISTS trg_trial_reservation_no_delete ON mlops.trial_reservation;
CREATE TRIGGER trg_trial_reservation_no_delete
    BEFORE DELETE ON mlops.trial_reservation
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- "How many looks has this run taken" — the budget question, per run.
CREATE INDEX IF NOT EXISTS trial_reservation_run_idx
    ON mlops.trial_reservation (tenant_id, run_id, created_at DESC);

-- The reconciliation this table is shaped for and does not yet perform: looks
-- that were authorised and never settled.
CREATE INDEX IF NOT EXISTS trial_reservation_unsettled_idx
    ON mlops.trial_reservation (tenant_id, created_at DESC)
    WHERE settled_trial_id IS NULL;
