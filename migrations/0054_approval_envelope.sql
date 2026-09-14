-- 0054 — `approval_spend_usd` is a DEFINE fact (SPEC §15, ADR-P2-20).
--
-- Four actions require a human approval, and one of them — spending — needs a
-- threshold to be measured against. That threshold is declared at DEFINE with
-- everything else the campaign pre-registers, because a spending limit set after
-- seeing the bill is not a limit.
--
-- There is no platform default. Existing rows are backfilled to **0**, which is
-- not a default in the sense that matters: 0 means "ask about every spend", the
-- strictest reading, and it is the only backfill that cannot authorise something
-- nobody authorised. A campaign that wants a larger envelope is a new campaign,
-- because DEFINE facts are immutable.

ALTER TABLE mlops.campaign
    ADD COLUMN IF NOT EXISTS approval_spend_usd NUMERIC(14,2);

UPDATE mlops.campaign SET approval_spend_usd = 0 WHERE approval_spend_usd IS NULL;

ALTER TABLE mlops.campaign
    ALTER COLUMN approval_spend_usd SET NOT NULL;

-- Explicitly no DEFAULT: an INSERT that omits the column fails rather than
-- inheriting a number. That is the mechanism, not an omission.
ALTER TABLE mlops.campaign
    ALTER COLUMN approval_spend_usd DROP DEFAULT;

ALTER TABLE mlops.campaign
    DROP CONSTRAINT IF EXISTS chk_campaign_approval_spend_nonneg;
ALTER TABLE mlops.campaign
    ADD CONSTRAINT chk_campaign_approval_spend_nonneg
    CHECK (approval_spend_usd >= 0);

-- The agent reads the envelope it is operating inside; it does not set it. The
-- column-level grants on `mlops.campaign` were re-issued in 0050 to omit
-- `platform_seed`, so this one is added explicitly rather than inherited.
GRANT SELECT (approval_spend_usd) ON mlops.campaign TO agent_role;
