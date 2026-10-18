-- 0246 — the platform role cannot delete (or rewrite) the append-only tables of the
-- 2026-10-03/04 push (final verification pass, 2026-10-05).
--
-- 0044 ran `ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE
-- ON TABLES TO app_role`, so every table created in `public` afterwards arrives with all
-- four privileges, and a narrower explicit GRANT cannot take one away (0167 found this for
-- alert_inbox_state). The migrations below each say "SELECT, INSERT" (append-only) or
-- "SELECT, INSERT, UPDATE" in their GRANT and nothing in the workspace deletes from them, so
-- the revokes remove nothing anyone uses. Owner-connected tests cannot see the difference.
--
-- Append-only (INSERT only): a row is a fact about the past; a correction is a new row.
REVOKE UPDATE, DELETE ON
    experiment_data_admission,   -- 0157
    agent_idempotency,           -- 0170
    eval_holdout_draw,           -- 0177
    context_policy_promotion,    -- 0177
    deployment_proposal,         -- 0180
    deployment_decision,         -- 0180
    deployment_authorisation,    -- 0180
    deployment_legacy_marker,    -- 0180
    ramp_period,                 -- 0181
    ramp_review,                 -- 0181
    deployment_ceiling,          -- 0182
    context_attribution_run,     -- 0238
    context_attribution_record   -- 0238
FROM app_role;

-- Read-only to the platform (0180 grants it SELECT only; the default privileges added the rest).
REVOKE INSERT, UPDATE, DELETE ON deployment_gate_switch FROM app_role;

-- Written in place by the platform (upserts, status changes) but never deleted.
REVOKE DELETE ON
    agent_token_calibration,     -- 0177
    model_promotion_proposals,   -- 0220
    model_forecasts,             -- 0222
    model_quality_series,        -- 0222
    model_quality_alerts         -- 0222
FROM app_role;
