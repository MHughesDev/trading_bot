-- 0154 — a P&L lot records its side (Set E 2.2).
--
-- `pnl_lots` (migration 0008) stored no side, while `storage::pnl::PnlLot`
-- carries one and the live rollup needs it: a short lot gains when the mark
-- falls, and valuing every lot as a long reports a winning short as a loss of
-- the same size.
--
-- Nullable because rows written before this migration have no recorded side and
-- there is no honest value to backfill. The live rollup refuses to value a lot
-- with a NULL side and surfaces it (`mark_coverage.unreadable_lots`) instead of
-- guessing long.

ALTER TABLE pnl_lots ADD COLUMN IF NOT EXISTS side TEXT;
ALTER TABLE pnl_lots DROP CONSTRAINT IF EXISTS pnl_lots_side_known;
ALTER TABLE pnl_lots
    ADD CONSTRAINT pnl_lots_side_known CHECK (side IS NULL OR side IN ('buy', 'sell'));

GRANT SELECT, INSERT, UPDATE, DELETE ON pnl_lots, pnl_closes TO app_role;
