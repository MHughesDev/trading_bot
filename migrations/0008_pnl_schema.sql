-- FIFO P&L persistence (C-073 / C-105).
-- Open lots and close records for the FifoEngine (see crates/storage/src/pnl.rs).

CREATE TABLE IF NOT EXISTS pnl_lots (
    lot_id          UUID PRIMARY KEY,
    user_id         UUID NOT NULL REFERENCES users(user_id),
    account_mode    TEXT NOT NULL,          -- 'paper' | 'live'
    instrument_id   TEXT NOT NULL REFERENCES instruments(instrument_id),
    open_event_id   UUID NOT NULL,          -- fill event that opened this lot
    open_qty        NUMERIC(30, 10) NOT NULL,
    remaining_qty   NUMERIC(30, 10) NOT NULL,
    open_price      NUMERIC(30, 10) NOT NULL,
    open_usd_rate   NUMERIC(20, 10) NOT NULL DEFAULT 1,
    opened_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pnl_lots_user_mode_inst
    ON pnl_lots(user_id, account_mode, instrument_id)
    WHERE remaining_qty > 0;

CREATE TABLE IF NOT EXISTS pnl_closes (
    close_id        UUID PRIMARY KEY,
    lot_id          UUID NOT NULL REFERENCES pnl_lots(lot_id),
    close_event_id  UUID NOT NULL,          -- fill event that triggered this close
    close_qty       NUMERIC(30, 10) NOT NULL,
    close_price     NUMERIC(30, 10) NOT NULL,
    realized_usd    NUMERIC(30, 10) NOT NULL,
    closed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pnl_closes_lot ON pnl_closes(lot_id);
CREATE INDEX IF NOT EXISTS pnl_closes_user
    ON pnl_closes(close_event_id);
