-- Connect-your-own-key: verification state and the default choice per user
-- (PROVIDERS track, 2026-10-05; docs/operations/connect-ai-provider.md).
--
-- 1. `vllm` was never in the provider CHECK, so the desk's "Local (vLLM)" card
--    answered every save with a 500: the row the route tried to write was refused
--    by 0034's own constraint. The CHECK now lists every `llm::Provider`.
-- 2. When a key was last proven to work, and why it last failed. A key is verified
--    before it is first stored, but keys are revoked at the provider; the card says
--    "verified <time>" or "Key rejected: <reason>" from these, never from a guess.
-- 3. Which connected provider, and which of its models, the desk starts with. One
--    default per user, held by a partial unique index rather than by the route.
--
-- Nothing here holds secret material: `last_error` is the provider's reason with
-- the key scrubbed out by the route before it is written, and the CHECK on its
-- length stops a provider body being stored whole.

ALTER TABLE llm_credentials DROP CONSTRAINT IF EXISTS llm_credentials_provider_check;
ALTER TABLE llm_credentials
    ADD CONSTRAINT llm_credentials_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'ollama', 'vllm'));

ALTER TABLE llm_credentials
    ADD COLUMN IF NOT EXISTS verified_at     TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_checked_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_error      TEXT,
    ADD COLUMN IF NOT EXISTS default_model   TEXT,
    ADD COLUMN IF NOT EXISTS is_default      BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE llm_credentials DROP CONSTRAINT IF EXISTS llm_credentials_last_error_len;
ALTER TABLE llm_credentials
    ADD CONSTRAINT llm_credentials_last_error_len
    CHECK (last_error IS NULL OR length(last_error) <= 500);

ALTER TABLE llm_credentials DROP CONSTRAINT IF EXISTS llm_credentials_default_model_len;
ALTER TABLE llm_credentials
    ADD CONSTRAINT llm_credentials_default_model_len
    CHECK (default_model IS NULL OR (length(default_model) BETWEEN 1 AND 200));

-- Every row written before this migration was verified against the provider
-- before it was saved (routes/llm.rs has always verified first), so its write time
-- is its verification time.
UPDATE llm_credentials
   SET verified_at = updated_at,
       last_checked_at = updated_at
 WHERE verified_at IS NULL;

-- At most one default per user.
CREATE UNIQUE INDEX IF NOT EXISTS llm_credentials_one_default_per_user
    ON llm_credentials (user_id) WHERE is_default;

-- Covered by 0044's default privileges, restated so the grant does not depend on
-- which role ran 0034.
GRANT SELECT, INSERT, UPDATE, DELETE ON llm_credentials TO app_role;
