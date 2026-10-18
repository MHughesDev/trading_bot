-- 0153 — the paper engine's accounts survive a restart (Set E audit item #5).
--
-- `PaperTradingEngine` held every account's cash, positions, realized P&L,
-- per-day realized tally, fees, trade counts and transaction ledger in memory
-- only. A restart silently reset all of it to the seeded opening balance — and
-- with it the daily-loss figure the risk gate reads. Paper trading runs in the
-- app's own engine (ADR-0048), so this state *is* the paper account.
--
-- One row per asset-class account, holding the engine's `PaperAccountState`
-- export as JSONB. `execution::paper::persist` writes a row whenever the
-- engine's mutation generation moves (at most once a second) and loads every
-- row at boot before the engine takes an order.
--
-- `state_version` is the shape of the JSON. A row whose version the binary does
-- not know is refused at load (the account is NOT silently reseeded), so a
-- downgrade cannot quietly discard positions.
--
-- What is not here: resting (unfilled) paper orders. A restart cancels them —
-- the `orders` table still records each one — and the account state carries no
-- reservation for them, so nothing in this row depends on them.

CREATE TABLE IF NOT EXISTS paper_account_state (
    asset_class    TEXT PRIMARY KEY,
    state_version  INTEGER NOT NULL CHECK (state_version > 0),
    state          JSONB NOT NULL,
    -- The engine's mutation counter when this row was taken; strictly a
    -- diagnostic (how much happened since boot), not a correctness input.
    generation     BIGINT NOT NULL CHECK (generation >= 0),
    saved_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_account_state_is_an_object CHECK (jsonb_typeof(state) = 'object'),
    CONSTRAINT paper_account_state_names_its_class
        CHECK (state ->> 'asset_class' = asset_class)
);

GRANT SELECT, INSERT, UPDATE, DELETE ON paper_account_state TO app_role;
