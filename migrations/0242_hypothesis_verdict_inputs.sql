-- 0242 — a verdict keeps the numbers that decided it (AGENT-013 §6.2, §12;
-- S-10.15 server side).
--
-- `evaluate` writes `status` and `inconclusive_why` and nothing about *why those*:
-- which metric, which direction, what threshold, which measurements, which one
-- fired, and which parts of §6.9's battery the ledger could and could not supply.
-- The desk's hypothesis card is specified to show "the platform's verdict with the
-- numbers that decided it"; with no column holding them every figure on the card
-- would have been reconstructed, and a reconstruction can disagree with what the
-- platform actually read.
--
-- ## Why JSONB and not columns
--
-- The inputs are a small document with a variable number of measurements, and the
-- one reader is the card. It is written once, in the same UPDATE that sets the
-- status, and never edited (the first verdict stands: `status = 'open'` is in that
-- UPDATE's WHERE clause).
--
-- ## NOT VALID on the constraint
--
-- Rows decided before this migration carry no inputs and cannot be given any: the
-- numbers were not kept. `NOT VALID` makes the rule bind every verdict from now on
-- without pretending the old ones had them; a reader that finds NULL on an old row
-- says "not recorded", never zero.

ALTER TABLE agent_lifecycle.hypothesis
    ADD COLUMN IF NOT EXISTS verdict_inputs JSONB;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'a_verdict_keeps_its_inputs'
    ) THEN
        ALTER TABLE agent_lifecycle.hypothesis
            ADD CONSTRAINT a_verdict_keeps_its_inputs CHECK (
                status = 'open' OR verdict_inputs IS NOT NULL
            ) NOT VALID;
    END IF;
END
$$;

-- `agent_lifecycle.hypothesis` is already granted SELECT, INSERT, UPDATE to
-- app_role at table level (0166), which covers the new column; restated for the
-- one column so the grant is visible where the column is added.
GRANT SELECT, INSERT, UPDATE ON agent_lifecycle.hypothesis TO app_role;
