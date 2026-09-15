-- Track 16 (the execution path to a fill): make orders and fills durable.
--
-- Before this migration nothing in the repository wrote to `orders` or `fills`.
-- `GET /api/orders/{id}` read a table that was always empty, and order/fill
-- idempotency lived in a 50 000 entry in-memory deque that a restart wiped.
-- SC-7 requires every money-mutating path to be idempotent across reconnects
-- and redeliveries; a restart is a reconnect.
--
-- Three changes, each removing a reason the insert could not be made:

-- 1. Paper orders belong to the in-house paper engine's per-asset-class
--    accounts, not to a row in `accounts` (which models a per-user brokerage
--    account and is empty in a paper-only deployment).  The column stays for
--    the live path; it is now nullable so a paper order can be recorded.
ALTER TABLE orders ALTER COLUMN account_id DROP NOT NULL;

-- 2. The instrument foreign key refused any order on an instrument that was
--    live-traded but never inserted into `instruments`.  An unknown instrument
--    is caught by the risk gate (InstrumentMetadataUnavailable) *before* the
--    order exists, which is the correct place for it — a referential failure at
--    insert time would mean losing the record of an order that was submitted.
ALTER TABLE orders DROP CONSTRAINT IF EXISTS orders_instrument_id_fkey;

-- 3. Paper only, enforced by the database rather than by a comment.  Enabling a
--    live venue is a migration plus a security review, not a code change.
ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS execution_mode TEXT NOT NULL DEFAULT 'paper';
ALTER TABLE orders DROP CONSTRAINT IF EXISTS orders_execution_mode_paper_only;
ALTER TABLE orders
    ADD CONSTRAINT orders_execution_mode_paper_only CHECK (execution_mode = 'paper');

-- The durable idempotency guarantee that `execution::fills` used to *claim*.
-- A redelivered fill inserts zero rows; the duplicate is detected by the
-- database, so it survives a restart, a reconnect and a JetStream redelivery.
--
-- The key is the CUMULATIVE filled quantity, not this fill's size.  Keying on
-- (price, size) -- what the in-memory dedup does -- collapses two legitimate
-- partial fills of the same size at the same price into one, which silently
-- halves a position.  Cumulative quantity is monotonic per order, so two
-- partials are distinct while a redelivery of the same broker state is not.
ALTER TABLE fills
    ADD COLUMN IF NOT EXISTS cumulative_filled NUMERIC(30, 10) NOT NULL DEFAULT 0;
ALTER TABLE fills ALTER COLUMN cumulative_filled DROP DEFAULT;
ALTER TABLE fills DROP CONSTRAINT IF EXISTS fills_idempotent;
ALTER TABLE fills
    ADD CONSTRAINT fills_idempotent
    UNIQUE (idempotency_key, broker_order_id, cumulative_filled);

-- A commission of NULL means the venue did not report one.  Recording an
-- unknown fee as 0 makes every fill on that path look cheaper than it was, and
-- makes a forward fill stream look cheaper than the backtest that justified
-- deploying it -- the one comparison Gate 15 exists to make.
ALTER TABLE fills ALTER COLUMN commission DROP NOT NULL;
ALTER TABLE fills ALTER COLUMN commission DROP DEFAULT;

-- Reading an instrument's recent fills is the reconciliation sweep's hot query.
CREATE INDEX IF NOT EXISTS fills_instrument_time ON fills(instrument_id, filled_at DESC);
CREATE INDEX IF NOT EXISTS orders_instrument_state ON orders(instrument_id, state);
CREATE INDEX IF NOT EXISTS orders_strategy ON orders(strategy_id) WHERE strategy_id IS NOT NULL;
