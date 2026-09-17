-- 0129 — the campaign a run drives, and the worker sub-runs that propose for it
-- (AGENT-013 §8.5, AGENT-010 §9, AGENT-005 §29; Set S task S-10.11).
--
-- ## One object, one hash
--
-- `mlops.campaign` has had a DEFINE since 0043 and `agent_lifecycle.preregistration`
-- has had one since 0127, and until now a run driving a campaign would have had
-- two: two objectives, two `delta_practical`s, two hypotheses, and no rule about
-- which one a later reader should believe. §8.5 says they are the same object.
--
-- `campaign_run` is where that is enforced rather than asserted. It carries the
-- `prereg_id` the campaign was defined from and the `prereg_hash` that
-- registration sealed, and both are REQUIRED — so a campaign a run drives cannot
-- exist without the registration it came from, and the registration's hash is
-- the campaign's hash by construction.
--
-- ## Why the brief is stored and not rebuilt
--
-- The worker sub-run at HYPOTHESIZE reads a *projection* of the parent's context
-- store. The parent is suspended on `Effect::Await` while its campaign runs, so
-- the only way to rebuild its store at dispatch time would be to replay its whole
-- effect log — for every generation, in a job worker, to arrive at the answer the
-- parent already had.
--
-- Storing it also buys the property fan-out needs: every worker off one campaign
-- reads the same bytes, so the provider cache sees one prefix rather than one per
-- worker. `brief_digest` is what makes that checkable afterwards rather than
-- merely intended.
--
-- ## `withheld` is a count, not a promise
--
-- The projection excludes the parent's reasoning by construction — the records
-- are not in the store the worker selects from (`harness::context::projection`).
-- What is recorded here is how many records the parent had that the worker did
-- not, so a reader can see that the exclusion happened and how large it was
-- without the reasoning itself being copied anywhere near the worker.

CREATE SCHEMA IF NOT EXISTS agent_lifecycle;

CREATE TABLE IF NOT EXISTS agent_lifecycle.campaign_run (
    campaign_id     UUID PRIMARY KEY,
    -- The parent Research run. It suspends on `job_id` and resumes on the
    -- campaign's terminal event.
    run_id          UUID NOT NULL,
    tenant_id       TEXT NOT NULL,
    -- The registration this campaign's DEFINE *is*. Not a copy of its fields:
    -- the row it was built from.
    prereg_id       UUID NOT NULL REFERENCES agent_lifecycle.preregistration(prereg_id),
    -- `ledger::prereg::Preregistration::hash`. Denormalised on purpose — every
    -- trial the campaign spends carries this hash, and a join per ticket to
    -- learn a value that cannot change is a join for nothing.
    prereg_hash     TEXT NOT NULL,
    -- The `JobKind::Campaign` job driving it.
    job_id          TEXT NOT NULL,

    -- ── the pinned projection (AGENT-005 §29) ────────────────────────────────
    -- 'worker' for a campaign. Named rather than assumed so a brief assembled
    -- under a different projection is visible as one.
    projection      TEXT NOT NULL,
    -- The bytes every worker sub-run of this campaign is given.
    brief           TEXT NOT NULL,
    -- sha256 of `brief`. Two workers whose digests differ did not share a prefix,
    -- whatever the intent was.
    brief_digest    TEXT NOT NULL,
    -- Record ids rendered into the brief, in render order.
    shown           JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- How many of the parent's records the worker does not get.
    withheld        INT NOT NULL,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_campaign_run_names_its_registration CHECK (btrim(prereg_hash) <> ''),
    CONSTRAINT a_campaign_run_names_its_job CHECK (btrim(job_id) <> ''),
    -- The three `harness::context::projection::Projection` variants. A campaign
    -- uses 'worker'; the others are here because this column records which
    -- projection was used and a column that can only hold one value records
    -- nothing.
    CONSTRAINT a_projection_is_one_of_three CHECK (
        projection IN ('critic', 'worker', 'full')
    ),
    -- A brief that is empty is a worker given nothing and asked to propose.
    CONSTRAINT a_brief_has_content CHECK (btrim(brief) <> ''),
    CONSTRAINT a_brief_has_a_digest CHECK (btrim(brief_digest) <> ''),
    CONSTRAINT withheld_is_countable CHECK (withheld >= 0),
    CONSTRAINT shown_is_a_list CHECK (jsonb_typeof(shown) = 'array')
);

-- One campaign per run per registration; a run that defines twice has amended,
-- and an amendment is a new registration and therefore a new campaign.
CREATE INDEX IF NOT EXISTS campaign_run_run_idx
    ON agent_lifecycle.campaign_run (run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS campaign_run_job_idx
    ON agent_lifecycle.campaign_run (job_id);

-- A campaign is defined once. `refuse_mutation` is the same trigger
-- `mlops.campaign` uses, and for the same reason: a DEFINE that can be edited
-- after a result is not a DEFINE.
DROP TRIGGER IF EXISTS trg_campaign_run_immutable ON agent_lifecycle.campaign_run;
CREATE TRIGGER trg_campaign_run_immutable
    BEFORE UPDATE OR DELETE ON agent_lifecycle.campaign_run
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── sub-runs (AGENT-010 §9) ──────────────────────────────────────────────────
--
-- A sub-run is a run with a parent, a role, a projection and its own effect log.
-- This table is the parent's side of the `Delegation` pair: what was dispatched,
-- under which projection, and what came back.
--
-- `outcome` is nullable because the row is written *before* the sub-run finishes
-- — the same ordering `campaign_driver::advance` uses for its child jobs, and for
-- the same reason. A crash between dispatching and recording leaves work nothing
-- points at, which the next fold re-derives; a crash the other way leaves a row
-- claiming a sub-run that does not exist, and no replay fixes that.

CREATE TABLE IF NOT EXISTS agent_lifecycle.sub_run (
    sub_run_id      UUID PRIMARY KEY,
    parent_run_id   UUID NOT NULL,
    tenant_id       TEXT NOT NULL,
    -- AGENT-010 §9's three roles.
    role            TEXT NOT NULL,
    projection      TEXT NOT NULL,
    -- The job carrying it. A sub-run is compute like anything else.
    job_id          TEXT NOT NULL,
    -- Set when the sub-run belongs to a campaign phase.
    campaign_id     UUID REFERENCES agent_lifecycle.campaign_run(campaign_id),
    phase           TEXT,
    -- The campaign fold's length when this was dispatched. With `campaign_id` it
    -- is the idempotency key: a driver restarted between dispatching and
    -- recording finds this row rather than dispatching a second worker.
    seq             BIGINT,
    -- The brief this sub-run actually received, as a digest. Compared against
    -- the campaign's, this is what makes "every worker got the same bytes" a
    -- checkable claim rather than an intention.
    brief_digest    TEXT NOT NULL,
    withheld        INT NOT NULL,
    -- The proposal, once it arrives. `harness`-side shape:
    -- `research::propose::Proposal`.
    outcome         JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    settled_at      TIMESTAMPTZ,

    CONSTRAINT a_role_is_one_of_three CHECK (role IN ('critic', 'worker', 'reader')),
    CONSTRAINT a_sub_run_projection_is_one_of_three CHECK (
        projection IN ('critic', 'worker', 'full')
    ),
    CONSTRAINT a_sub_run_names_its_job CHECK (btrim(job_id) <> ''),
    CONSTRAINT a_sub_run_records_its_brief CHECK (btrim(brief_digest) <> ''),
    CONSTRAINT sub_run_withheld_is_countable CHECK (withheld >= 0),
    -- A campaign sub-run carries its position; a standalone one carries neither.
    -- Half a position is not a key, and a driver restarting would dispatch again.
    CONSTRAINT a_campaign_sub_run_knows_where_it_is CHECK (
        (campaign_id IS NULL AND phase IS NULL AND seq IS NULL)
        OR (campaign_id IS NOT NULL AND phase IS NOT NULL AND seq IS NOT NULL)
    ),
    -- A settled sub-run has a time and an outcome; an unsettled one has neither.
    CONSTRAINT a_settled_sub_run_has_both CHECK (
        (settled_at IS NULL AND outcome IS NULL)
        OR (settled_at IS NOT NULL AND outcome IS NOT NULL)
    )
);

-- The idempotency key the driver reads before dispatching.
CREATE UNIQUE INDEX IF NOT EXISTS sub_run_one_per_campaign_position
    ON agent_lifecycle.sub_run (campaign_id, phase, seq)
    WHERE campaign_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS sub_run_parent_idx
    ON agent_lifecycle.sub_run (parent_run_id, created_at DESC);
