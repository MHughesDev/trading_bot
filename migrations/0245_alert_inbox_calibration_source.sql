-- 0245 -- the alert inbox admits calibration drift as a source (Set R-b RB-5.8).
--
-- 0158 constrained alert_inbox_state.source to ('monitor', 'price'). A forecaster whose
-- interval coverage has left tolerance in two consecutive windows is a third kind of item
-- a person should see (research::refit::coverage_drift over model_quality_series), so the
-- CHECK is widened. ALTER does not touch privileges: app_role keeps SELECT, INSERT, UPDATE
-- and still has no DELETE (0167); the REVOKE is restated so this file is correct on its own.

ALTER TABLE alert_inbox_state DROP CONSTRAINT IF EXISTS alert_inbox_state_source_check;
ALTER TABLE alert_inbox_state
    ADD CONSTRAINT alert_inbox_state_source_check
    CHECK (source IN ('monitor', 'price', 'calibration'));

GRANT SELECT, INSERT, UPDATE ON alert_inbox_state TO app_role;
REVOKE DELETE ON alert_inbox_state FROM app_role;
