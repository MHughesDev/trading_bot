-- 0134 — one workflow, one trial family (AGENT-006 §2.2, §6; Set S task S-8.1).
--
-- ## The question it answers
--
-- A workflow is one tool call that runs a fixed battery of five or six looks.
-- Every one of those looks is a multiple comparison, and a family-wise
-- correction computed over a denominator that does not contain them is a
-- correction over the wrong family — which reads as a pass.
--
-- `mlops.trial_reservation` already records *that* a look was authorised. This
-- records *which battery it belonged to*, so the six looks a `leakage_audit`
-- takes are countable as six looks of one battery rather than as six unrelated
-- calls that happened close together in time.
--
-- ## Why not group by time, or by run
--
-- By time, because two batteries can overlap and a gap is not a boundary. By
-- run, because a run takes many batteries and folding them into one family
-- would over-correct — which is the opposite error and just as wrong.
--
-- The id is minted by the workflow engine at the moment the battery starts, so
-- it exists before the first step is admitted and no step can be outside it.
--
-- ## Nullable, deliberately
--
-- Most calls are not inside a battery. A NULL means "a look taken on its own",
-- which is a real and common answer — not "unknown". Making it NOT NULL would
-- mean inventing a family for every standalone call, and a family of one that
-- nothing else will ever join is a denominator that lies in the safe-looking
-- direction.

ALTER TABLE mlops.trial_reservation
    ADD COLUMN IF NOT EXISTS family_id TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'a_family_is_named_or_absent'
    ) THEN
        -- A blank family is worse than none: it groups every blank-family look
        -- on the platform into one denominator.
        ALTER TABLE mlops.trial_reservation
            ADD CONSTRAINT a_family_is_named_or_absent CHECK (
                family_id IS NULL OR btrim(family_id) <> ''
            );
    END IF;
END
$$;

-- "Every look this battery took", which is the question a family-wise
-- correction asks and the one S-8.1's acceptance checks.
CREATE INDEX IF NOT EXISTS trial_reservation_family_idx
    ON mlops.trial_reservation (tenant_id, family_id, created_at)
    WHERE family_id IS NOT NULL;
