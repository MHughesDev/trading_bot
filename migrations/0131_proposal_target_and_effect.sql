-- 0131 — a proposal names a target, and an approval records what it did
-- (AGENT-013 §6.8, §7.5; Set S task S-10.9).
--
-- ## Two gaps, found building the approval route
--
-- **A proposal had no target an approval could act on.** `subject` is TEXT —
-- "what is being promoted, armed or changed" — and a human approving
-- `deploy_paper` for the subject `"ema_cross_v3"` leaves the platform holding a
-- string where it needs an automation id. Resolving a name to a row at approval
-- time is the free-form-string bug class this repository keeps paying for: the
-- lookup has to guess, and the guess happens at the one moment the platform is
-- acting on a human's authority.
--
-- `subject_id` is the target. `subject` stays, because it is what the desk
-- renders and what the human reads — an id alone would make the approval screen
-- a UUID and a verb.
--
-- **An approved proposal recorded no effect.** `decision = 'approved'` said a
-- human said yes. It did not say whether anything happened, and those are
-- different facts: the registry write can fail, and the platform may not have a
-- path for the kind at all. Without the distinction an approved-and-inert row
-- reads exactly like an approved-and-performed one — which is the failure
-- `lifecycle.propose` already avoids on the agent's side by answering
-- `"effect": "nothing yet"` out loud, reproduced here on the human's side.
--
-- ## Why `NOT VALID`
--
-- The CHECK requiring `subject_id` is added `NOT VALID`: Postgres enforces it on
-- every new row and does not scan the existing ones. A proposal created before
-- this migration genuinely has no target, and there is no honest value to
-- backfill — inventing one would put a UUID nobody chose in front of a human
-- about to approve something. Those rows are refused at approval with the reason
-- named, which is better than a migration that fails on a database that has one.

ALTER TABLE agent_lifecycle.proposal
    ADD COLUMN IF NOT EXISTS subject_id UUID;

ALTER TABLE agent_lifecycle.proposal
    ADD COLUMN IF NOT EXISTS performed_at TIMESTAMPTZ;

-- What the approval actually did, or why it did nothing. An object, never a
-- string: "what happened" has a shape per kind, and a sentence would be a
-- second thing to parse.
ALTER TABLE agent_lifecycle.proposal
    ADD COLUMN IF NOT EXISTS effect JSONB;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'a_proposal_names_a_target'
    ) THEN
        ALTER TABLE agent_lifecycle.proposal
            ADD CONSTRAINT a_proposal_names_a_target CHECK (subject_id IS NOT NULL)
            NOT VALID;
    END IF;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conname = 'an_effect_belongs_to_an_approval'
    ) THEN
        -- Only an approval performs anything, and a performed row says both when
        -- and what. A `performed_at` with no `effect` is a row claiming something
        -- happened and declining to say what.
        ALTER TABLE agent_lifecycle.proposal
            ADD CONSTRAINT an_effect_belongs_to_an_approval CHECK (
                (performed_at IS NULL AND effect IS NULL)
                OR (decision = 'approved' AND performed_at IS NOT NULL AND effect IS NOT NULL)
            )
            NOT VALID;
    END IF;
END
$$;

-- The desk's queue reads pending proposals; the audit reads decided ones by
-- when they were decided.
CREATE INDEX IF NOT EXISTS proposal_decided_idx
    ON agent_lifecycle.proposal (decided_at DESC)
    WHERE decision <> 'pending';
