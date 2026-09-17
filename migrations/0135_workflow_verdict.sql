-- 0135 — a battery's conclusion, so a gate can demand it (AGENT-006 §6,
-- AGENT-008 P8; Set S task S-8.4).
--
-- ## The question it answers
--
-- Until this table, a gate's entry condition was the prior gate's verdict and
-- nothing else. The batteries §6 describes could be run, or skipped, and the
-- funnel could not tell the difference — so a model that had not run them and
-- said it had was indistinguishable from one that had.
--
-- A row here is written by the **workflow engine**, at the end of a battery it
-- ran itself, from the steps it dispatched. It is not written by a tool the
-- model calls, and there is no route that accepts a verdict with no battery
-- behind it. That is the property the gate leans on: the gate reads rows, and a
-- model cannot write one by being convincing.
--
-- ## Why the family id is the key
--
-- Because it is the thing the battery already minted, before its first step was
-- admitted (`0134`). Every reservation the battery made carries it, so a verdict
-- and the looks it rests on join without a second identifier — and a verdict
-- whose family has no reservations is visibly a verdict with no looks behind it.
--
-- ## Append-only, and not superseded in place
--
-- A battery can be re-run; the later row is a second row. `check` in
-- `backtest::gates::tickets` takes the newest by `created_at` rather than the
-- best, so re-running until it passes leaves every attempt visible and does not
-- let a caller choose which one counts.

CREATE TABLE IF NOT EXISTS mlops.workflow_verdict (
    -- Minted by the engine as `wf_<workflow>_<uuid>`; TEXT rather than UUID
    -- because the prefix is load-bearing in logs and refusals.
    family_id    TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    workflow     TEXT NOT NULL,
    -- One of the battery's own closed set. Not CHECKed against a list here: the
    -- sets are per-battery and live in `mcp_server::tools::workflows`, and a
    -- CHECK naming twenty-odd strings would be a second vocabulary to keep in
    -- step. `backtest::gates::tickets` enumerates what each checkpoint
    -- *accepts*, which is the check that matters and the one that defaults to
    -- refusing.
    verdict      TEXT NOT NULL,
    -- Why, in the battery's own words. Stored because a refusal that says only
    -- "fails" costs the reader the run that produced it.
    because      TEXT NOT NULL,
    -- What the battery was run over — the experiment id. A gate looks a ticket
    -- up by this and by the workflow name.
    subject      TEXT NOT NULL,
    -- The run that asked for it, when there is one.
    run_id       UUID,
    -- Every step, its status and its ids, as the engine rendered them. The
    -- evidence bundle §6 calls permanent for the leakage audit: kept here rather
    -- than in a transient result, so a verdict cited months later can still be
    -- taken apart.
    steps        JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Whether every step that was meant to run did. A battery that could not
    -- complete has a verdict — `inconclusive`, `suspect`, `reject` — and this
    -- says which kind of answer it is.
    complete     BOOLEAN NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_verdict_names_its_battery CHECK (btrim(workflow) <> ''),
    CONSTRAINT a_verdict_is_a_word CHECK (btrim(verdict) <> ''),
    CONSTRAINT a_verdict_gives_a_reason CHECK (btrim(because) <> ''),
    CONSTRAINT a_verdict_names_its_subject CHECK (btrim(subject) <> '')
);

-- A verdict is a record of what a battery concluded. Editing one afterwards
-- would make it a record of the present.
DROP TRIGGER IF EXISTS trg_workflow_verdict_no_change ON mlops.workflow_verdict;
CREATE TRIGGER trg_workflow_verdict_no_change
    BEFORE UPDATE OR DELETE ON mlops.workflow_verdict
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- "What has this experiment's batteries concluded, newest last" — the query a
-- checkpoint makes.
CREATE INDEX IF NOT EXISTS workflow_verdict_subject_idx
    ON mlops.workflow_verdict (tenant_id, subject, workflow, created_at);
