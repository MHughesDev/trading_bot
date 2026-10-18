-- 0188 — each paper deployment's own account survives a restart (Set U U-4.7).
--
-- The paper engine kept one account per asset class, so every deployment on
-- an asset class shared one balance, one set of positions and one day's loss:
-- two strategies could not be sized, gated or measured apart. A deployment now
-- trades in its own account (`PaperTradingEngine::open_deployment_account`),
-- and this table keeps it across restarts exactly as `paper_account_state`
-- (0153) keeps the asset-class accounts: the engine's `PaperAccountState`
-- export, in the same shape and under the same `state_version`, written when it
-- changes and loaded at boot before any order. A row the binary cannot read
-- refuses boot rather than reseeding the account.

CREATE TABLE IF NOT EXISTS paper_deployment_account_state (
    deployment_id  UUID PRIMARY KEY REFERENCES strategy_deployment (deployment_id),
    asset_class    TEXT NOT NULL,
    state_version  INTEGER NOT NULL CHECK (state_version > 0),
    state          JSONB NOT NULL,
    saved_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_deployment_account_state_is_an_object CHECK (jsonb_typeof(state) = 'object'),
    CONSTRAINT paper_deployment_account_state_names_its_class
        CHECK (state ->> 'asset_class' = asset_class)
);

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') THEN
    GRANT SELECT, INSERT, UPDATE ON paper_deployment_account_state TO app_role;
    -- An account is the deployment's record of money; it is never deleted.
    REVOKE DELETE ON paper_deployment_account_state FROM app_role;
  END IF;
END $$;
