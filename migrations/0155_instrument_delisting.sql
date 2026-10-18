-- Set P P-4.4: delisted instruments are retained, never deleted (DATA-005 §8; DA-08).
--
-- Survivorship bias is invisible in the numbers by construction: a universe built
-- from the instruments that still exist has already selected on the outcome. The
-- only defence that actually works is a deletion policy, because once an
-- instrument's row is gone the bias it introduces is unrecoverable at any cost.
--
-- Before this migration `instruments` (0002) carried an `active` boolean and
-- nothing else, and `app_role` held DELETE on it through 0044's schema-wide grant.
-- `dataplane.instrument` already refuses deletion (0046, `trg_instrument_no_delete`);
-- this brings the operational table to the same rule, and gives delisting a state
-- that is not a deletion.
--
-- Three mechanisms, each because the one before it is not enough alone:
--   1. `status` + `delisted_at`, with a CHECK tying them together and to `active`,
--      so a delisting is a recorded state rather than an absence.
--   2. A row trigger refusing DELETE and a statement trigger refusing TRUNCATE,
--      for every role including the owner — a REVOKE alone does not bind the
--      owner, and the owner is who runs a hand-written cleanup at 3 a.m.
--   3. REVOKE DELETE, TRUNCATE from app_role, so the refusal is also a missing
--      privilege and not only a trigger someone can drop.
-- The static half — no code path deletes an instrument row — is
-- `cargo xtask check-instruments-retained`.

ALTER TABLE instruments
  ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE instruments
  ADD COLUMN IF NOT EXISTS delisted_at TIMESTAMPTZ;
ALTER TABLE instruments
  ADD COLUMN IF NOT EXISTS delisting_reason TEXT;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'instruments_status_known') THEN
    ALTER TABLE instruments ADD CONSTRAINT instruments_status_known
      CHECK (status IN ('active', 'delisted'));
  END IF;
  -- A delisted instrument says when, and is not tradeable. An active one has no
  -- delisting date. Either half disagreeing with the other is a corrupt row.
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'instruments_delisting_consistent') THEN
    ALTER TABLE instruments ADD CONSTRAINT instruments_delisting_consistent
      CHECK (
        (status = 'active'   AND delisted_at IS NULL)
     OR (status = 'delisted' AND delisted_at IS NOT NULL AND NOT active)
      );
  END IF;
END $$;

CREATE OR REPLACE FUNCTION refuse_instrument_removal()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'instruments rows are never deleted: set status = ''delisted'' instead (Set P P-4.4; survivorship bias is unrecoverable once the row is gone)'
    USING ERRCODE = 'restrict_violation';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_instruments_no_delete ON instruments;
CREATE TRIGGER trg_instruments_no_delete
  BEFORE DELETE ON instruments
  FOR EACH ROW EXECUTE FUNCTION refuse_instrument_removal();

DROP TRIGGER IF EXISTS trg_instruments_no_truncate ON instruments;
CREATE TRIGGER trg_instruments_no_truncate
  BEFORE TRUNCATE ON instruments
  FOR EACH STATEMENT EXECUTE FUNCTION refuse_instrument_removal();

-- The one way to delist. Idempotent: delisting an already-delisted instrument
-- keeps the first date rather than moving it, because the first date is the fact.
CREATE OR REPLACE FUNCTION delist_instrument(p_instrument_id TEXT, p_at TIMESTAMPTZ, p_reason TEXT)
RETURNS VOID AS $$
BEGIN
  IF p_reason IS NULL OR btrim(p_reason) = '' THEN
    RAISE EXCEPTION 'a delisting states its reason';
  END IF;
  UPDATE instruments
     SET status = 'delisted',
         active = false,
         delisted_at = COALESCE(delisted_at, p_at),
         delisting_reason = COALESCE(delisting_reason, p_reason),
         updated_at = now()
   WHERE instrument_id = p_instrument_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'no instrument %', p_instrument_id;
  END IF;
END;
$$ LANGUAGE plpgsql;

REVOKE DELETE, TRUNCATE ON instruments FROM app_role;
GRANT SELECT, INSERT, UPDATE ON instruments TO app_role;
GRANT EXECUTE ON FUNCTION delist_instrument(TEXT, TIMESTAMPTZ, TEXT) TO app_role;
