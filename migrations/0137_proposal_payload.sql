-- 0137 — an approved proposal carries what would be written (AGENT-013 §7.5;
-- Set S task S-10.9).
--
-- ## What was missing
--
-- A proposal named `kind`, `subject` and `subject_id` — enough to *arm* an
-- automation, whose whole content is "this row, on". It is not enough to change
-- a risk limit: `monitor::limits::ControlLimit` has a metric, a family, a
-- severity, a version, bounds, a method and the run its baseline came from, and
-- a proposal carried none of them.
--
-- So `change_limits` approvals reported `not_performed` by name, which was
-- honest and useless. The alternative — letting the approval invent the missing
-- fields — is the failure ADR-S10-09 records for `subject_id`, sited at the one
-- moment the platform acts on somebody's authority.
--
-- ## Validated at creation, not at approval
--
-- The payload is checked against the real type when the proposal is *made*. A
-- proposal that reaches a human, gets approved, and only then turns out to be
-- unwritable is the worst possible ordering: somebody has already decided.
--
-- ## Why an object and not columns
--
-- Because the shape is per kind. `deploy_paper` needs nothing, `change_limits`
-- needs eight fields, and `promote_strategy` needs whatever the promoted set
-- turns out to be — a decision this task is still waiting on. Eight nullable
-- columns that are all NULL for two of the three kinds would be a schema that
-- says nothing about which fields belong together.
--
-- The default is `{}` and not NULL: "this kind carries nothing" is a real
-- answer, and NULL would make it indistinguishable from "nobody wrote one".

ALTER TABLE agent_lifecycle.proposal
    ADD COLUMN IF NOT EXISTS payload JSONB NOT NULL DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'a_payload_is_an_object'
    ) THEN
        ALTER TABLE agent_lifecycle.proposal
            ADD CONSTRAINT a_payload_is_an_object CHECK (jsonb_typeof(payload) = 'object');
    END IF;
END
$$;
