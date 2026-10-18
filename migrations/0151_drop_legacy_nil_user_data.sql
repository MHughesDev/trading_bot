-- 0151 — delete legacy nil-user data (Set E 1.6, locked decision 3).
--
-- Until Set E 1.4 every automation was written under `Uuid::nil()` (the
-- `DEV_USER` placeholder in routes/automations.rs), and backtests run before
-- real sessions were stamped either nil or a UUIDv5 of the bearer string. With
-- every route now owner-scoped, those rows belong to nobody: no caller can list,
-- disarm or delete them, and an armed one would keep evaluating unseen.
--
-- Decision 3 is DELETE, not reassign. Order matters and is not negotiable:
--
--   1. DISARM first. An armed nil-user automation is a live declaration that
--      moves (paper) money; it is set unarmed before anything is removed, in the
--      same transaction, so nothing can be armed-and-orphaned mid-migration.
--      (Boot also re-declares only rows that are still armed — see
--      `resume_armed` — so a disarmed row can never be resumed.)
--   2. DELETE the nil automations, and automations whose owner is not a user at
--      all (the UUIDv5-of-token identity, which no `users` row can match).
--      Stage membership cascades (migration 0010).
--   3. DELETE nil and NULL-owner backtests. Orphaned non-nil backtests are left:
--      they are inert history, not authority, and deleting research results is
--      a larger decision than this cutover.

UPDATE automations SET armed = false
 WHERE armed
   AND (user_id = '00000000-0000-0000-0000-000000000000'
        OR NOT EXISTS (SELECT 1 FROM users u WHERE u.user_id = automations.user_id));

DELETE FROM automations
 WHERE user_id = '00000000-0000-0000-0000-000000000000'
    OR NOT EXISTS (SELECT 1 FROM users u WHERE u.user_id = automations.user_id);

DELETE FROM backtest_jobs
 WHERE user_id IS NULL OR user_id = '00000000-0000-0000-0000-000000000000';

-- No new nil-owned automation can ever be written again: the placeholder is gone
-- from the code, and the database now refuses it too.
ALTER TABLE automations DROP CONSTRAINT IF EXISTS automations_owner_is_not_nil;
ALTER TABLE automations
    ADD CONSTRAINT automations_owner_is_not_nil
    CHECK (user_id <> '00000000-0000-0000-0000-000000000000');

GRANT SELECT, INSERT, UPDATE, DELETE ON automations, backtest_jobs TO app_role;
