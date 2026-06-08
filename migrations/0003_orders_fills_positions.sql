-- Orders, fills, and positions.
-- All price/size/commission columns use NUMERIC — never FLOAT.

CREATE TABLE IF NOT EXISTS orders (
    idempotency_key     UUID PRIMARY KEY,           -- matches OrderIntent.idempotency_key
    account_id          UUID NOT NULL REFERENCES accounts(account_id),
    strategy_id         TEXT,                       -- NULL for manual orders
    instrument_id       TEXT NOT NULL REFERENCES instruments(instrument_id),
    side                TEXT NOT NULL,              -- 'buy' | 'sell'
    order_type          TEXT NOT NULL,              -- 'market' | 'limit' | 'stop_limit'
    size                NUMERIC(30, 10) NOT NULL,   -- never FLOAT
    limit_price         NUMERIC(30, 10),            -- NULL for market orders
    state               TEXT NOT NULL,
    broker_order_id     TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS fills (
    fill_id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    idempotency_key     UUID NOT NULL REFERENCES orders(idempotency_key),
    broker_order_id     TEXT NOT NULL,
    instrument_id       TEXT NOT NULL,
    side                TEXT NOT NULL,
    filled_size         NUMERIC(30, 10) NOT NULL,   -- never FLOAT
    fill_price          NUMERIC(30, 10) NOT NULL,   -- never FLOAT
    commission          NUMERIC(30, 10) NOT NULL DEFAULT 0,
    filled_at           TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS fills_order ON fills(idempotency_key);

CREATE TABLE IF NOT EXISTS positions (
    position_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(account_id),
    instrument_id       TEXT NOT NULL REFERENCES instruments(instrument_id),
    quantity            NUMERIC(30, 10) NOT NULL,           -- positive=long, negative=short
    average_entry_price NUMERIC(30, 10) NOT NULL DEFAULT 0, -- never FLOAT
    unrealized_pnl      NUMERIC(30, 10) NOT NULL DEFAULT 0,
    last_updated        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(account_id, instrument_id)
);
